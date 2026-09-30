//! Native invoicing v1 (TASKLIST Phase 3, Wave 9 lane INVOICING).
//!
//! Purpose: contract → invoice run → document → payments → dunning
//! states. `POST /api/billing/invoice-runs` evaluates every billable
//! contract over a window through the SAME engine the previews expose
//! (`core_billing::rules::evaluate`), then persists one invoice per
//! contract that produced at least one candidate line — lines, time-
//! entry consumption links, and the run roll-up all commit in ONE
//! transaction (any failure rolls the whole run back).
//!
//! "Billed once, ever" invariants:
//! * period lines (recurring/retainer base + overage) — one per
//!   (contract, kind, period), DB-enforced by a partial unique index;
//!   the run skips already-billed candidates before insert.
//! * fixed-fee lines — one per (contract, due date, label); stable
//!   across fee-card replacement (fee ids churn on contract edits).
//! * time entries — linked to the consuming invoice at insert time;
//!   `core_db::invoices::list_unbilled_entries` excludes linked entries
//!   from every later evaluation, even across overlapping runs and
//!   contracts sharing a client.
//!
//! Payments flow through the `PaymentProvider` seam
//! (`core_billing::PaymentProvider`); v1 resolves the built-in
//! none-mode provider, which approves locally without external PSP
//! calls. Dunning states are DATA: a forward-only ladder on the invoice
//! row moved by explicit API writes (events + write trail), never a
//! background side effect.
//!
//! Documents: PDF output deliberately ships as print-ready HTML (a
//! heavyweight PDF toolchain would violate the dependency budget; the
//! gap is recorded in TASKLIST).
//!
//! Instant-first contract (docs/instant-first.md): the run, void,
//! payment, and dunning writes are idempotency-keyed with read-your-
//! write `mutation` metadata; all failures return the central typed
//! error envelope.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use core_billing::ChargeRequest;
use core_domain::entities::{
    BillingCandidateLine, BillingLineKind, ContractType, DunningState, Invoice, InvoiceLine,
    InvoicePayment, InvoiceRun, InvoiceStatus,
};
use core_domain::events::{
    types, InvoiceCreated, InvoiceDunningChanged, InvoicePaymentRecorded, InvoiceRunCompleted,
    InvoiceVoided,
};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::str::FromStr;
use utoipa::ToSchema;
use uuid::Uuid;

fn commercial(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Commercial,
        action,
    }
}

/// Upper bound on a run window (same contract as the previews): the
/// engine is deterministic but an unbounded scan is not a contract.
const MAX_RUN_DAYS: i64 = 366;

/// Default payment terms for issued invoices.
const DEFAULT_DUE_IN_DAYS: i32 = 30;

// --- wire structs ------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateInvoiceRunRequest {
    /// Window start (inclusive), ISO date.
    pub from: chrono::NaiveDate,
    /// Window end (inclusive), ISO date.
    pub to: chrono::NaiveDate,
    /// Restrict the run to one client's billable contracts.
    pub client_id: Option<Uuid>,
    /// Payment terms for the issued invoices, in days (default 30).
    pub due_in_days: Option<i32>,
    /// Instant-first autosave parameters (IF1): idempotency key.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// One issued invoice: the document plus its lines.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct IssuedInvoice {
    pub invoice: Invoice,
    pub lines: Vec<InvoiceLine>,
}

/// Instant-first mutation response (IF2) for an invoice run.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InvoiceRunResponse {
    pub run: InvoiceRun,
    pub invoices: Vec<IssuedInvoice>,
    pub mutation: instant::MutationMeta,
}

/// The run's stored payload (everything but the mutation metadata, which
/// `instant::record` merges in for byte-identical replays).
#[derive(Debug, Serialize)]
struct InvoiceRunPayload<'a> {
    run: &'a InvoiceRun,
    invoices: &'a [IssuedInvoice],
}

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct InvoiceListQuery {
    pub client_id: Option<Uuid>,
    pub contract_id: Option<Uuid>,
    /// Exact status: `draft|issued|paid|void`.
    pub status: Option<String>,
    /// Exact dunning state:
    /// `none|reminder_sent|overdue|final_notice|collections`.
    pub dunning_state: Option<String>,
}

/// The invoice aggregate: document, lines, and payment receipts.
#[derive(Debug, Serialize, ToSchema)]
pub struct InvoiceDetail {
    #[serde(flatten)]
    pub invoice: Invoice,
    pub lines: Vec<InvoiceLine>,
    pub payments: Vec<InvoicePayment>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct VoidInvoiceRequest {
    /// Instant-first autosave parameters (IF1): idempotency key +
    /// If-Match `version`.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RecordPaymentRequest {
    /// Amount to collect, minor units; must not exceed the outstanding
    /// balance. Currency is always the invoice's own.
    pub amount_minor: i64,
    pub notes: Option<String>,
    /// Instant-first autosave parameters (IF1): idempotency key +
    /// If-Match `version` on the invoice.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct DunningTransitionRequest {
    /// The target state; must be strictly forward on the ladder
    /// (skipping rungs is allowed).
    pub to_state: DunningState,
    pub note: Option<String>,
    /// Instant-first autosave parameters (IF1): idempotency key +
    /// If-Match `version` on the invoice.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// POST /api/invoices/{id}/dunning/statement body.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SendStatementRequest {
    /// Instant-first autosave parameters (IF1): idempotency key.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// POST /api/invoices/{id}/dunning/statement response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SendStatementResponse {
    pub client_id: Uuid,
    pub currency: String,
    /// Open invoices on the statement (number, due date, outstanding).
    pub open_invoices: Vec<StatementLine>,
    /// Sum outstanding across the statement, minor units.
    pub total_outstanding_minor: i64,
    /// Addresses the statement email was queued to (client contacts
    /// with an address). The message rides the outbound ledger; in
    /// none-mode transport it stays queued like every other leg.
    pub queued_to: Vec<String>,
}

/// One open invoice on the statement.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StatementLine {
    pub number: String,
    #[schema(value_type = String)]
    pub due_on: chrono::NaiveDate,
    pub total_minor: i64,
    pub outstanding_minor: i64,
}

/// Instant-first mutation response (IF2) for invoice mutations.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InvoiceMutationResponse {
    #[serde(flatten)]
    pub invoice: Invoice,
    pub mutation: instant::MutationMeta,
}

/// Payment receipt response. The invoice is nested (not flattened) so
/// the stored replay payload — `{payment, invoice}` plus the merged
/// mutation metadata — deserializes into this shape byte-identically.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PaymentMutationResponse {
    pub payment: InvoicePayment,
    pub invoice: Invoice,
    pub mutation: instant::MutationMeta,
}

// --- helpers -----------------------------------------------------------------

fn parse_run_window(from: chrono::NaiveDate, to: chrono::NaiveDate) -> ApiResult<()> {
    if to < from {
        return Err(ApiError::Validation("to must not precede from".into()));
    }
    if (to - from).num_days() > MAX_RUN_DAYS {
        return Err(ApiError::Validation(
            "run window exceeds 366 days; split the range".into(),
        ));
    }
    Ok(())
}

fn fx_provider_error(e: core_domain::fx::FxError) -> ApiError {
    match e {
        // Fail-closed (ADR 0002): a run hitting a cross-currency fact
        // without rate coverage is a preflight refusal — nothing commits,
        // so the typed signal is a conflict, not a validation error.
        core_domain::fx::FxError::NoRate { from, to, as_of } => ApiError::Conflict(format!(
            "no fx rate recorded for {from}→{to} effective {as_of}; \
             record one before invoicing in a non-base currency"
        )),
        other => ApiError::Internal(anyhow::anyhow!("fx provider error: {other}")),
    }
}

fn engine_error(e: core_billing::BillingError) -> ApiError {
    match e {
        core_billing::BillingError::InvalidWindow { from, to } => ApiError::Validation(format!(
            "evaluation window is inverted: {from} is after {to}"
        )),
        core_billing::BillingError::AmountOverflow => {
            ApiError::Conflict("billing line amounts overflow; narrow the window".into())
        }
    }
}

/// Formats minor units with an assumed two-decimal exponent for the
/// print document (the currency code is always rendered alongside).
fn format_minor(minor: i64) -> String {
    let sign = if minor < 0 { "-" } else { "" };
    let abs = minor.unsigned_abs();
    format!("{sign}{}.{:02}", abs / 100, abs % 100)
}

/// Minimal HTML escaping for the print document.
fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Loads the invoice aggregate from the pool.
async fn load_detail(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> ApiResult<InvoiceDetail> {
    let invoice = core_db::invoices::get_invoice(&mut *conn, org_id, id).await?;
    let lines = core_db::invoices::list_lines(&mut *conn, org_id, id).await?;
    let payments = core_db::invoices::list_payments(&mut *conn, org_id, id).await?;
    Ok(InvoiceDetail {
        invoice,
        lines,
        payments,
    })
}

// --- handlers: invoice runs --------------------------------------------------

/// POST /api/billing/invoice-runs — evaluate every billable contract over
/// the window (optionally one client's) and persist one invoice per
/// contract with at least one candidate line. Idempotency-keyed (IF1):
/// the same key with the same body replays the stored run.
#[utoipa::path(operation_id = "billing_invoice_runs_create", post, path = "/api/billing/invoice-runs", tag = "billing", security(("bearer" = [])),
    request_body = CreateInvoiceRunRequest,
    responses((status = 200, body = InvoiceRunResponse),
              (status = 409, description = "mixed-currency run or idempotency key reused"),
              (status = 422, description = "validation failure")))]
pub async fn create_run(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateInvoiceRunRequest>,
) -> ApiResult<Json<InvoiceRunResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    parse_run_window(req.from, req.to)?;
    let due_in_days = req.due_in_days.unwrap_or(DEFAULT_DUE_IN_DAYS);
    if !(0..=365).contains(&due_in_days) {
        return Err(ApiError::Validation(
            "due_in_days must be between 0 and 365".into(),
        ));
    }

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let window = (req.from, req.to);
    let client_filter = req.client_id;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "invoice_run",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        // The population the Wave 8b preview endpoints evaluate (the
        // engine refuses cancelled contracts; windows must intersect).
        let mut contracts =
            core_db::contracts::list_billable_contracts(&mut *ctx.tx, org_id, window.0, window.1)
                .await?;
        if let Some(client) = client_filter {
            contracts.retain(|c| c.client_id == client);
        }

        let contract_ids: Vec<Uuid> = contracts.iter().map(|c| c.id).collect();
        let billed_keys =
            core_db::invoices::list_billed_keys(&mut *ctx.tx, org_id, &contract_ids).await?;
        // Period lines dedupe on (contract, kind, period_start); the
        // description-keyed kinds (fixed fees, block sales/drawdowns, cap
        // overages, change orders, SLA credits, per-incident, escalators)
        // dedupe on (contract, period_start, description). Per-unit and
        // minimum true-up are kind+period-identity kinds.
        let period_kinds = |kind: &str| {
            matches!(
                core_domain::entities::BillingLineKind::from_str(kind),
                Ok(k) if k.is_period_kind()
            )
        };
        let billed_periods: std::collections::HashSet<(Uuid, String, chrono::NaiveDate)> =
            billed_keys
                .iter()
                .filter(|k| period_kinds(&k.kind))
                .map(|k| (k.contract_id, k.kind.clone(), k.period_start))
                .collect();
        let billed_fees: std::collections::HashSet<(Uuid, chrono::NaiveDate, String)> = billed_keys
            .iter()
            .filter(|k| !period_kinds(&k.kind))
            .filter(|k| {
                k.kind != BillingLineKind::TimeMaterials.as_str()
                    && k.kind != BillingLineKind::RetainerOverage.as_str()
            })
            .map(|k| (k.contract_id, k.period_start, k.description.clone()))
            .collect();

        let today = chrono::Utc::now().date_naive();
        let due_on = today + chrono::Duration::days(due_in_days as i64);
        let mut issued: Vec<IssuedInvoice> = Vec::new();
        let mut currencies: Vec<String> = Vec::new();
        let mut total_minor: i64 = 0;

        // The run row exists up front so invoices can reference it; the
        // roll-up counters fill in after the sweep (everything shares
        // this one transaction).
        let run_id = Uuid::now_v7();
        let run_number = core_db::invoices::next_number(&mut *ctx.tx, org_id, "RUN-").await?;
        core_db::invoices::create_run(
            &mut *ctx.tx,
            &InvoiceRun {
                id: run_id,
                org_id,
                number: run_number,
                period_from: window.0,
                period_to: window.1,
                invoice_count: 0,
                total_minor: 0,
                currency: None,
                due_in_days,
                created_by: Some(user_id),
                created_at: chrono::Utc::now(),
            },
        )
        .await?;

        // Contracts arrive ordered by number: per-client sequential
        // processing means later contracts re-read UNBILLED entries and
        // see the consumption links this run already wrote.
        for contract in &contracts {
            let entries = core_db::invoices::list_unbilled_entries(
                &mut *ctx.tx,
                org_id,
                contract.client_id,
                window.0,
                window.1,
            )
            .await?;
            let inputs: Vec<core_billing::BillableEntry> = entries
                .iter()
                .map(|e| core_billing::BillableEntry {
                    time_entry_id: e.time_entry_id,
                    minutes: e.minutes,
                    work_type: e.work_type.clone(),
                    entry_date: e.entry_date,
                })
                .collect();
            let rates = core_db::contracts::list_rates(&mut *ctx.tx, org_id, contract.id).await?;
            let fees = core_db::contracts::list_fees(&mut *ctx.tx, org_id, contract.id).await?;
            let eval_inputs = crate::routes::billing::load_eval_inputs(
                &mut ctx.tx,
                org_id,
                contract,
                window.0,
                window.1,
            )
            .await?;
            let evaluation = core_billing::rules::evaluate_with(
                contract,
                &rates,
                &fees,
                &inputs,
                window.0,
                window.1,
                &eval_inputs,
            )
            .map_err(engine_error)?;

            // Skip already-billed candidates; the partial unique indexes
            // remain the concurrency backstop.
            let lines: Vec<BillingCandidateLine> = evaluation
                .lines
                .iter()
                .filter(|l| match l.kind {
                    BillingLineKind::TimeMaterials => true,
                    _ => !billed_periods.contains(&(
                        contract.id,
                        l.kind.as_str().to_string(),
                        l.period_start,
                    )),
                })
                .filter(|l| {
                    l.kind != BillingLineKind::FixedFee
                        || !billed_fees.contains(&(
                            contract.id,
                            l.period_start,
                            l.description.clone(),
                        ))
                })
                .cloned()
                .collect();
            if lines.is_empty() {
                continue;
            }

            // Mixed-currency runs are refused before anything commits
            // (one transaction: an error here rolls the whole run back).
            if !currencies.contains(&contract.currency) {
                currencies.push(contract.currency.clone());
                if currencies.len() > 1 {
                    return Err(ApiError::Conflict(
                        "invoice run mixes currencies; run one window per currency".into(),
                    ));
                }
            }
            total_minor = total_minor
                .checked_add(evaluation.total_minor)
                .ok_or_else(|| ApiError::Conflict("invoice run totals overflow".into()))?;

            // Entry consumption: T&M invoices consume the entries in the
            // engine's effective slice; retainers consume the entries
            // dated inside their RETAINED periods; recurring/fixed-fee
            // invoices price by period/schedule and consume none.
            let lo = window.0.max(contract.effective_from);
            let hi = match contract.effective_until {
                Some(until) => window.1.min(until),
                None => window.1,
            };
            let retained_periods: Vec<(chrono::NaiveDate, chrono::NaiveDate)> = lines
                .iter()
                .filter(|l| {
                    matches!(
                        l.kind,
                        BillingLineKind::RecurringBase
                            | BillingLineKind::RetainerBase
                            | BillingLineKind::RetainerOverage
                    )
                })
                .map(|l| (l.period_start, l.period_end))
                .collect();
            let consumed: Vec<Uuid> = match contract.contract_type {
                ContractType::TimeAndMaterials => entries
                    .iter()
                    .filter(|e| e.entry_date >= lo && e.entry_date <= hi)
                    .map(|e| e.time_entry_id)
                    .collect(),
                ContractType::Retainer => entries
                    .iter()
                    .filter(|e| {
                        retained_periods.iter().any(|(start, end)| {
                            e.entry_date >= (*start).max(lo)
                                && e.entry_date < *end
                                && e.entry_date <= hi
                        })
                    })
                    .map(|e| e.time_entry_id)
                    .collect(),
                _ => Vec::new(),
            };

            // Invoice carries the contract's PO reference forward (F):
            // each document shows what the client's AP matches it to.
            let po_number = contract.po_number.clone();
            let number = core_db::invoices::next_number(&mut *ctx.tx, org_id, "INV-").await?;
            let (period_from, period_to) = match contract.contract_type {
                ContractType::FixedFee => (None, None),
                _ => {
                    let from = lines.iter().map(|l| l.period_start).min();
                    let to = lines.iter().map(|l| l.period_end).max();
                    (from, to)
                }
            };
            // ADR 0002 lock moments: this run creates the document
            // already ISSUED, so the invoice-issue snapshot locks here —
            // and each priced line locks its time-entry-costing snapshot
            // (same resolved rate, per-line amount). Single-currency facts
            // (contract currency = org base) carry no snapshot. The rate
            // resolves ON the org-bound transaction — never a nested
            // pool transaction (connection starvation under load).
            let base = core_db::fx::base_currency(&mut *ctx.tx, org_id)
                .await
                .map_err(ApiError::from)?;
            let provider = crate::routes::fx::ManualFxRateProvider::new();
            let issue_rate: Option<core_domain::fx::FxRate> = if contract.currency != base {
                Some(
                    provider
                        .resolve_in_tx(
                            &mut ctx.tx,
                            &core_domain::fx::FxRateRequest {
                                org_id,
                                from: core_domain::money::CurrencyCode::new(&contract.currency)
                                    .map_err(ApiError::from)?,
                                to: core_domain::money::CurrencyCode::new(&base)
                                    .map_err(ApiError::from)?,
                                as_of: today,
                            },
                        )
                        .await
                        .map_err(fx_provider_error)?,
                )
            } else {
                None
            };
            let issue_rate_columns = issue_rate
                .as_ref()
                .map(|rate| {
                    let original = core_domain::money::Money::new(
                        evaluation.total_minor,
                        core_domain::money::CurrencyCode::new(&contract.currency)
                            .map_err(ApiError::from)?,
                    );
                    core_domain::fx::FxSnapshot::lock(
                        &original,
                        &core_domain::money::CurrencyCode::new(&base).map_err(ApiError::from)?,
                        rate.clone(),
                        today,
                        provider.name(),
                    )
                    .map_err(ApiError::from)
                })
                .transpose()?;
            let base_of = |minor: i64| -> ApiResult<i64> {
                match &issue_rate {
                    Some(rate) => {
                        let m = core_domain::money::Money::new(
                            minor,
                            core_domain::money::CurrencyCode::new(&contract.currency)
                                .map_err(ApiError::from)?,
                        );
                        Ok(m.convert(rate).map_err(ApiError::from)?.minor)
                    }
                    None => Ok(minor),
                }
            };
            // Header aggregation convention (reconciliation coherence):
            // the document's base total is the SUM of the per-line base
            // snapshots — one canonical aggregation point. Rounding the
            // header total independently of the lines would let
            // Σ(line base) drift from the header by up to ⌊N/2⌋ minor
            // units, and the reconciliation read model cross-foots them.
            let line_bases: Vec<i64> = lines
                .iter()
                .map(|l| base_of(l.amount_minor))
                .collect::<ApiResult<Vec<i64>>>()?;
            let base_total = line_bases.iter().sum::<i64>();
            let invoice = Invoice {
                id: Uuid::now_v7(),
                org_id,
                run_id: Some(run_id),
                client_id: contract.client_id,
                contract_id: contract.id,
                number,
                status: InvoiceStatus::Issued,
                currency: contract.currency.clone(),
                period_from,
                period_to,
                issued_at: today,
                due_on,
                total_minor: evaluation.total_minor,
                total_paid_minor: 0,
                fx_rate_used: issue_rate_columns.as_ref().map(|s| s.columns().0),
                fx_rate_date: issue_rate_columns.as_ref().map(|s| s.rate_date),
                fx_source: issue_rate_columns.as_ref().map(|s| s.source.clone()),
                base_total_minor: issue_rate_columns.as_ref().map(|_| base_total),
                dunning_state: DunningState::None,
                dunning_updated_at: None,
                dunning_note: None,
                po_number: po_number.clone(),
                notes: None,
                created_by: Some(user_id),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };
            invoice.validate()?;
            let line_rows: Vec<InvoiceLine> = lines
                .iter()
                .enumerate()
                .map(|(position, l)| {
                    let base_minor = line_bases[position];
                    Ok(InvoiceLine {
                        id: Uuid::now_v7(),
                        org_id,
                        invoice_id: invoice.id,
                        contract_id: contract.id,
                        kind: l.kind,
                        description: l.description.clone(),
                        period_start: l.period_start,
                        period_end: l.period_end,
                        quantity_minutes: l.quantity_minutes,
                        unit_amount_minor: l.unit_amount_minor,
                        amount_minor: l.amount_minor,
                        fx_rate_used: issue_rate_columns.as_ref().map(|s| s.columns().0),
                        fx_rate_date: issue_rate_columns.as_ref().map(|s| s.rate_date),
                        fx_source: issue_rate_columns.as_ref().map(|s| s.source.clone()),
                        base_amount_minor: issue_rate_columns.as_ref().map(|_| base_minor),
                        position: position as i32,
                    })
                })
                .collect::<ApiResult<Vec<InvoiceLine>>>()?;
            core_db::invoices::insert_invoice(&mut *ctx.tx, &invoice).await?;
            core_db::invoices::insert_lines(&mut *ctx.tx, org_id, invoice.id, &line_rows).await?;
            if !consumed.is_empty() {
                core_db::invoices::insert_entry_links(&mut *ctx.tx, org_id, invoice.id, &consumed)
                    .await?;
            }
            // Wave-39 aggregate side effects (same transaction: a failure
            // rolls the whole run back):
            // * block draws persist per consumed entry (the verification
            //   read model computes remaining balances from this ledger),
            // * applied SLA credits flip pending → applied with the
            //   consuming invoice (never silent, never double-applied),
            // * billed change orders flip approved → billed,
            // * project releases flip pending → billed with the consuming
            //   invoice + line (Wave 39 lane 4: the ledger seatbelt that
            //   makes a replayed milestone completion bill exactly once).
            if let Some(consumption) = &evaluation.block_consumption {
                for draw in &consumption.draws {
                    let Some(entry_id) = draw.time_entry_id else {
                        continue;
                    };
                    core_db::contracts::insert_block_draw(
                        &mut ctx.tx,
                        org_id,
                        draw.block_id,
                        entry_id,
                        invoice.id,
                        draw.minutes,
                    )
                    .await?;
                }
            }
            let applied_credit_ids: Vec<Uuid> = evaluation
                .lines
                .iter()
                .filter(|l| l.kind == BillingLineKind::SlaCredit)
                .filter_map(|l| {
                    l.description
                        .split('[')
                        .nth(1)
                        .and_then(|rest| rest.split(']').next())
                        .and_then(|id| Uuid::parse_str(id).ok())
                })
                .collect();
            core_db::contracts::mark_sla_credits_applied(
                &mut *ctx.tx,
                org_id,
                &applied_credit_ids,
                invoice.id,
            )
            .await?;
            let billed_order_ids: Vec<Uuid> = eval_inputs
                .change_orders
                .iter()
                .filter(|o| o.status == "approved")
                .filter(|o| {
                    o.approved_at
                        .map(|a| a.date_naive() >= window.0 && a.date_naive() <= window.1)
                        .unwrap_or(false)
                })
                .map(|o| o.id)
                .collect();
            for order_id in billed_order_ids {
                core_db::contracts::update_change_order_status(
                    &mut *ctx.tx,
                    org_id,
                    contract.id,
                    order_id,
                    "billed",
                    None,
                    None,
                    Some(invoice.id),
                )
                .await?;
            }
            // Project releases: every project_release line this evaluation
            // produced flips its ledger row (release id rides the
            // description in [id] brackets, same convention as SLA
            // credits) to billed with the consuming invoice + line.
            for (position, line) in line_rows.iter().enumerate() {
                if line.kind != BillingLineKind::ProjectRelease {
                    continue;
                }
                let release_id = line
                    .description
                    .split('[')
                    .nth(1)
                    .and_then(|rest| rest.split(']').next())
                    .and_then(|id| Uuid::parse_str(id).ok());
                let Some(release_id) = release_id else {
                    return Err(ApiError::Conflict(
                        "project_release line without a parseable release id".into(),
                    ));
                };
                core_db::projects::mark_release_billed(
                    &mut ctx.tx,
                    org_id,
                    release_id,
                    invoice.id,
                    line_rows[position].id,
                )
                .await?;
            }
            ctx.emit(
                types::INVOICE_CREATED,
                "invoice",
                Some(invoice.id.to_string().as_str()),
                &serde_json::to_value(InvoiceCreated {
                    invoice: invoice.clone(),
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;
            // Allocate the invoice's version-1 write seq at creation so
            // later If-Match mutations (void/payment/dunning) count from
            // a coherent baseline, exactly like contract creates.
            instant::entity_seq(
                &mut ctx,
                instant::SeqSpec {
                    entity: "invoice",
                    entity_id: invoice.id,
                    version: None,
                },
            )
            .await?;
            issued.push(IssuedInvoice {
                invoice,
                lines: line_rows,
            });
        }

        core_db::invoices::finalize_run(
            &mut *ctx.tx,
            org_id,
            run_id,
            issued.len() as i32,
            total_minor,
            currencies.first().cloned(),
        )
        .await?;
        // Wave 39 lane 2 (report V8): claim the close sign-offs this
        // run's invoices cover. A run with no sign-off still issues —
        // the ritual is evidence, not a hard gate — but when a reviewer
        // signed the window, the run row is what the sign-off points at
        // from now on, so "was this period verified?" is one query.
        let claimed_signoffs =
            core_db::verification::attach_signoffs_to_run(&mut *ctx.tx, org_id, run_id).await?;
        if claimed_signoffs > 0 {
            tracing::debug!(run = %run_id, claimed_signoffs, "claimed billing verification sign-offs");
        }
        let run = core_db::invoices::get_run(&mut *ctx.tx, org_id, run_id).await?;
        ctx.emit(
            types::INVOICE_RUN_COMPLETED,
            "invoice_run",
            Some(run.id.to_string().as_str()),
            &serde_json::to_value(InvoiceRunCompleted { run: run.clone() })
                .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "invoice_run",
                entity_id: run_id,
                version: None,
            },
        )
        .await?;
        let payload = InvoiceRunPayload {
            run: &run,
            invoices: &issued,
        };
        let meta = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "invoice_run",
                entity_id: run_id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&payload).unwrap_or(Value::Null),
                updated_at: run.created_at,
                key: key.as_deref(),
                payload: &payload,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(InvoiceRunResponse {
                run,
                invoices: issued,
                mutation: meta,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// GET /api/billing/invoice-runs — most recent runs first.
#[utoipa::path(operation_id = "billing_invoice_runs_list", get, path = "/api/billing/invoice-runs", tag = "billing", security(("bearer" = [])),
    responses((status = 200, body = [core_domain::entities::InvoiceRun])))]
pub async fn list_runs(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<InvoiceRun>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let runs = core_db::invoices::list_runs(&mut *tx, authed.principal.user.org_id, 100).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(runs))
}

/// GET /api/billing/invoice-runs/{id} — one run's roll-up.
#[utoipa::path(operation_id = "billing_invoice_runs_get", get, path = "/api/billing/invoice-runs/{id}", tag = "billing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = core_domain::entities::InvoiceRun),
              (status = 404, description = "run not found")))]
pub async fn get_run(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<InvoiceRun>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let run = core_db::invoices::get_run(&mut *tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(run))
}

// --- handlers: invoices ------------------------------------------------------

/// GET /api/invoices — list with client/contract/status/dunning filters.
#[utoipa::path(operation_id = "invoices_list", get, path = "/api/invoices", tag = "billing", security(("bearer" = [])),
    params(InvoiceListQuery),
    responses((status = 200, body = [core_domain::entities::Invoice])))]
pub async fn list_invoices(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<InvoiceListQuery>,
) -> ApiResult<Json<Vec<Invoice>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let filters = core_db::invoices::InvoiceFilters {
        client_id: q.client_id,
        contract_id: q.contract_id,
        status: q.status,
        dunning_state: q.dunning_state,
    };
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let invoices =
        core_db::invoices::list_invoices(&mut *tx, authed.principal.user.org_id, &filters, 200)
            .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(invoices))
}

/// GET /api/invoices/{id} — the invoice aggregate (document, lines,
/// payment receipts).
#[utoipa::path(operation_id = "invoices_get_one", get, path = "/api/invoices/{id}", tag = "billing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = InvoiceDetail),
              (status = 404, description = "invoice not found")))]
pub async fn get_invoice(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<InvoiceDetail>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let detail = load_detail(&mut tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(detail))
}

/// GET /api/invoices/{id}/document — the print-ready HTML invoice
/// document (print-to-PDF from any browser; v1 ships no PDF binary
/// output — see TASKLIST).
#[utoipa::path(operation_id = "invoices_document", get, path = "/api/invoices/{id}/document", tag = "billing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, content_type = "text/html", body = String),
              (status = 404, description = "invoice not found")))]
pub async fn get_document(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::response::Html<String>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let invoice = core_db::invoices::get_invoice(&mut *tx, org_id, id).await?;
    let lines = core_db::invoices::list_lines(&mut *tx, org_id, id).await?;
    let client = core_db::clients::get_client(&mut *tx, org_id, invoice.client_id)
        .await
        .map_err(ApiError::from)?;
    let html = render_invoice_html(&mut tx, org_id, &invoice, &lines, &client.name).await;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(axum::response::Html(html))
}

/// The print-ready HTML document for one loaded invoice aggregate —
/// shared verbatim by the staff endpoint above and the Portal v2
/// client-scoped view (`routes::portal_v2`), so a client sees exactly
/// what staff print. `payments` reads ride org RLS; call only after the
/// caller's scope gate has passed.
pub(crate) async fn render_invoice_html(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    invoice: &Invoice,
    lines: &[InvoiceLine],
    client_name: &str,
) -> String {
    let status = match invoice.status {
        InvoiceStatus::Paid => "PAID".to_string(),
        InvoiceStatus::Void => "VOID".to_string(),
        other => {
            if invoice.total_paid_minor > 0 {
                format!(
                    "PARTIALLY PAID ({})",
                    format_minor(invoice.total_paid_minor)
                )
            } else {
                other.as_str().to_uppercase()
            }
        }
    };
    let rows: String = lines
        .iter()
        .map(|l| {
            format!(
                "<tr><td>{desc}</td><td class=\"num\">{qty}</td><td class=\"num\">{unit}</td>\
                 <td class=\"num\">{amount}</td></tr>",
                desc = esc(&l.description),
                qty = if l.quantity_minutes > 0 {
                    format!("{} min", l.quantity_minutes)
                } else {
                    String::new()
                },
                unit = format_minor(l.unit_amount_minor),
                amount = format_minor(l.amount_minor),
            )
        })
        .collect();
    let payments: String = invoice_payments_html(&mut *conn, org_id, invoice.id).await;
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{number}</title><style>\
         body{{font-family:-apple-system,'Segoe UI',sans-serif;margin:48px auto;max-width:720px;color:#111}}\
         h1{{font-size:20px;margin:0}} .muted{{color:#666}}\
         table{{border-collapse:collapse;width:100%;margin-top:24px}}\
         th,td{{border-bottom:1px solid #ddd;padding:8px 4px;text-align:left;font-size:14px}}\
         th{{border-bottom:2px solid #111}} .num{{text-align:right;font-variant-numeric:tabular-nums}}\
         .total td{{font-weight:700;font-size:16px;border-bottom:none}}\
         .badge{{display:inline-block;margin-top:8px;padding:2px 10px;border:1px solid #111;\
         border-radius:999px;font-size:12px;letter-spacing:.08em}}\
         .stamp{{float:right;text-align:right}}\
         @media print{{body{{margin:12mm}}}}</style></head><body>\
         <div class=\"stamp\"><h1>Invoice {number}</h1>\
         <div class=\"muted\">Issued {issued} · Due {due}</div>\
         <span class=\"badge\">{status}</span></div>\
         <div><strong>Bill to:</strong> {client}</div>\
         <table><thead><tr><th>Description</th><th class=\"num\">Qty</th>\
         <th class=\"num\">Unit</th><th class=\"num\">Amount</th></tr></thead>\
         <tbody>{rows}<tr class=\"total\"><td>Total ({currency})</td><td></td><td></td>\
         <td class=\"num\">{total}</td></tr></tbody></table>\
         {payments}\
         <p class=\"muted\">Marula invoice {number} · generated {now}</p>\
         </body></html>",
        number = esc(&invoice.number),
        issued = invoice.issued_at,
        due = invoice.due_on,
        status = esc(&status),
        client = esc(client_name),
        rows = rows,
        currency = esc(&invoice.currency),
        total = format_minor(invoice.total_minor),
        payments = payments,
        now = chrono::Utc::now().date_naive(),
    )
}

async fn invoice_payments_html(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    invoice_id: Uuid,
) -> String {
    let payments = core_db::invoices::list_payments(&mut *conn, org_id, invoice_id)
        .await
        .unwrap_or_default();
    if payments.is_empty() {
        return String::new();
    }
    let rows: String = payments
        .iter()
        .map(|p| {
            format!(
                "<tr><td>{}</td><td class=\"num\">{}</td><td class=\"muted\">{}</td></tr>",
                p.received_at.date_naive(),
                format_minor(p.amount_minor),
                esc(&p.provider),
            )
        })
        .collect();
    format!(
        "<table><thead><tr><th>Payments</th><th class=\"num\">Amount</th>\
         <th>Provider</th></tr></thead><tbody>{rows}</tbody></table>"
    )
}

/// POST /api/invoices/{id}/void — withdraw an ISSUED invoice. Terminal:
/// consumption markers stay, so re-running the window can never
/// double-bill. Instant-first gated (IF1/IF2).
#[utoipa::path(operation_id = "invoices_void", post, path = "/api/invoices/{id}/void", tag = "billing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = VoidInvoiceRequest,
    responses((status = 200, body = InvoiceMutationResponse),
              (status = 404, description = "invoice not visible in scope"),
              (status = 409, description = "invoice not voidable, version conflict, or idempotency key reused")))]
pub async fn void_invoice(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<VoidInvoiceRequest>,
) -> ApiResult<Json<InvoiceMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "invoice",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::invoices::get_invoice(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if before.status != InvoiceStatus::Issued {
            return Err(ApiError::Conflict(format!(
                "only issued invoices can be voided; this one is {}",
                before.status.as_str()
            )));
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "invoice",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let voided = core_db::invoices::void_invoice(&mut *ctx.tx, org_id, id)
            .await?
            .ok_or_else(|| ApiError::Conflict("invoice left its voidable state; retry".into()))?;
        ctx.emit(
            types::INVOICE_VOIDED,
            "invoice",
            Some(id.to_string().as_str()),
            &serde_json::to_value(InvoiceVoided {
                invoice_id: id,
                number: voided.number.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        let meta = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "invoice",
                entity_id: id,
                op: "void",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&voided).unwrap_or(Value::Null),
                updated_at: voided.updated_at,
                key: key.as_deref(),
                payload: &voided,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(InvoiceMutationResponse {
                invoice: voided,
                mutation: meta,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- handlers: payments ------------------------------------------------------

/// POST /api/invoices/{id}/payments — record a payment through the
/// `PaymentProvider` seam. The charge runs INSIDE the mutation
/// transaction so receipt, payment row, and invoice status commit (or
/// roll back) atomically. Instant-first gated (IF1/IF2).
#[utoipa::path(operation_id = "invoices_payments_create", post, path = "/api/invoices/{id}/payments", tag = "billing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = RecordPaymentRequest,
    responses((status = 200, body = PaymentMutationResponse),
              (status = 404, description = "invoice not visible in scope"),
              (status = 409, description = "invoice not payable, overpayment, version conflict, or idempotency key reused"),
              (status = 422, description = "validation failure")))]
pub async fn record_payment(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<RecordPaymentRequest>,
) -> ApiResult<Json<PaymentMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    if req.amount_minor <= 0 {
        return Err(ApiError::Validation("amount_minor must be positive".into()));
    }

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "invoice_payment",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let invoice = core_db::invoices::get_invoice(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        match invoice.status {
            InvoiceStatus::Issued => {}
            InvoiceStatus::Paid => {
                return Err(ApiError::Conflict("invoice is already fully paid".into()))
            }
            other => {
                return Err(ApiError::Conflict(format!(
                    "invoice is not payable in state {}",
                    other.as_str()
                )))
            }
        }
        let outstanding = invoice.total_minor.saturating_sub(invoice.total_paid_minor);
        if req.amount_minor > outstanding {
            return Err(ApiError::Conflict(format!(
                "payment exceeds the outstanding balance ({outstanding} minor units)"
            )));
        }

        // The seam hands the charge to the provider. Staff recording
        // stays LEDGER-ONLY (none-mode): a staff-row payment is an
        // external fact being RECORDED (wire/transfer/check), not a
        // card charge — PSP charging belongs to the portal pay flow.
        let provider: std::sync::Arc<dyn core_billing::PaymentProvider> =
            std::sync::Arc::new(core_billing::NonePaymentProvider);
        let receipt = provider
            .charge(&ChargeRequest {
                org_id,
                invoice_id: id,
                invoice_number: invoice.number.clone(),
                amount_minor: req.amount_minor,
                currency: invoice.currency.clone(),
                payment_method: None,
            })
            .await
            .map_err(|e| match e {
                core_billing::PaymentError::Declined { reason, code } => {
                    ApiError::PaymentDeclined {
                        code: code.as_str().into(),
                        provider_reason: reason,
                    }
                }
                core_billing::PaymentError::Unavailable(m) => ApiError::Unavailable(m),
            })?;

        // ADR 0002 lock moment (payment settlement): the receipt locks its
        // base equivalent at settlement when currency ≠ org base.
        let snap = crate::routes::fx::lock_snapshot(
            &mut ctx.tx,
            org_id,
            req.amount_minor,
            &invoice.currency,
            chrono::Utc::now().date_naive(),
        )
        .await?;
        let payment = InvoicePayment {
            id: Uuid::now_v7(),
            org_id,
            invoice_id: id,
            amount_minor: req.amount_minor,
            currency: invoice.currency.clone(),
            fx_rate_used: snap.as_ref().map(|s| s.rate_used.clone()),
            fx_rate_date: snap.as_ref().map(|s| s.rate_date),
            fx_source: snap.as_ref().map(|s| s.source.clone()),
            base_amount_minor: snap.as_ref().map(|s| s.base_amount_minor),
            provider: receipt.provider,
            provider_ref: receipt.provider_ref,
            notes: req.notes.clone(),
            received_at: chrono::Utc::now(),
            created_by: Some(user_id),
        };
        core_db::invoices::insert_payment(&mut *ctx.tx, &payment).await?;
        let after =
            core_db::invoices::apply_payment_total(&mut *ctx.tx, org_id, id, req.amount_minor)
                .await?
                .ok_or_else(|| {
                    ApiError::Conflict("invoice left its payable state; retry".into())
                })?;

        ctx.emit(
            types::INVOICE_PAYMENT_RECORDED,
            "invoice_payment",
            Some(payment.id.to_string().as_str()),
            &serde_json::to_value(InvoicePaymentRecorded {
                payment: payment.clone(),
                invoice_status: after.status.as_str().to_string(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "invoice",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let meta = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "invoice",
                entity_id: id,
                op: "payment_record",
                seq: write_seq,
                before: Some(serde_json::to_value(&invoice).unwrap_or(Value::Null)),
                after: serde_json::to_value(&after).unwrap_or(Value::Null),
                updated_at: after.updated_at,
                key: key.as_deref(),
                payload: &PaymentPayload {
                    payment: &payment,
                    invoice: &after,
                },
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(PaymentMutationResponse {
                payment,
                invoice: after,
                mutation: meta,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// The stored payment payload (mutation metadata merges in on record).
#[derive(Debug, Serialize)]
struct PaymentPayload<'a> {
    payment: &'a InvoicePayment,
    invoice: &'a Invoice,
}

/// GET /api/invoices/{id}/payments — the payment receipts.
#[utoipa::path(operation_id = "invoices_payments_list", get, path = "/api/invoices/{id}/payments", tag = "billing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::InvoicePayment]),
              (status = 404, description = "invoice not found")))]
pub async fn list_payments(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<InvoicePayment>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    // A missing invoice and an empty payment list are indistinguishable
    // through this lens; existence is validated by the 404 on the read.
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    core_db::invoices::get_invoice(&mut *tx, authed.principal.user.org_id, id).await?;
    let payments =
        core_db::invoices::list_payments(&mut *tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(payments))
}

// --- handlers: dunning -------------------------------------------------------

/// POST /api/invoices/{id}/dunning — move an ISSUED invoice's dunning
/// state forward (none → reminder_sent → overdue → final_notice →
/// collections; skipping rungs is allowed, backwards is refused).
/// States are data: the write emits `invoice.dunning_changed` and lands
/// in the write trail. Instant-first gated (IF1/IF2).
#[utoipa::path(operation_id = "invoices_dunning_transition", post, path = "/api/invoices/{id}/dunning", tag = "billing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = DunningTransitionRequest,
    responses((status = 200, body = InvoiceMutationResponse),
              (status = 404, description = "invoice not visible in scope"),
              (status = 409, description = "invoice not issuable, version conflict, or idempotency key reused"),
              (status = 422, description = "backwards or same-state transition")))]
pub async fn transition_dunning(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<DunningTransitionRequest>,
) -> ApiResult<Json<InvoiceMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "invoice_dunning",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::invoices::get_invoice(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if before.status != InvoiceStatus::Issued {
            return Err(ApiError::Conflict(format!(
                "dunning applies to issued invoices; this one is {}",
                before.status.as_str()
            )));
        }
        DunningState::transition(before.dunning_state, req.to_state)?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "invoice",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let after = core_db::invoices::set_dunning(
            &mut *ctx.tx,
            org_id,
            id,
            req.to_state,
            req.note.clone(),
        )
        .await?
        .ok_or_else(|| ApiError::Conflict("invoice left its issuable state; retry".into()))?;
        ctx.emit(
            types::INVOICE_DUNNING_CHANGED,
            "invoice",
            Some(id.to_string().as_str()),
            &serde_json::to_value(InvoiceDunningChanged {
                invoice_id: id,
                number: after.number.clone(),
                from_state: before.dunning_state.as_str().to_string(),
                to_state: req.to_state.as_str().to_string(),
                note: req.note.clone(),
                changed_at: chrono::Utc::now(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        let meta = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "invoice",
                entity_id: id,
                op: "dunning_transition",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&after).unwrap_or(Value::Null),
                updated_at: after.updated_at,
                key: key.as_deref(),
                payload: &after,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(InvoiceMutationResponse {
                invoice: after,
                mutation: meta,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- handlers: dunning statement ----------------------------------------------

/// POST /api/invoices/{id}/dunning/statement — generate an account
/// statement (the client's OPEN invoices with outstanding balances) and
/// queue it to the client's contacts through the outbound ledger
/// (`MailTransport`; none-mode stays queued like every other leg). The
/// statement rides the dunning path but is a document delivery, not a
/// dunning-state change. Instant-first gated (IF1).
#[utoipa::path(operation_id = "invoices_dunning_statement", post, path = "/api/invoices/{id}/dunning/statement", tag = "billing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = SendStatementRequest,
    responses((status = 200, body = SendStatementResponse),
              (status = 404, description = "invoice not visible in scope"),
              (status = 409, description = "invoice not issuable or idempotency key reused"),
              (status = 422, description = "no contact emails on the client to deliver to")))]
pub async fn send_statement(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<SendStatementRequest>,
) -> ApiResult<Json<SendStatementResponse>> {
    use crate::email::{enqueue_outbound, OutboundEnqueue};

    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let mail = std::sync::Arc::clone(&state.mail);

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "invoice_statement",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let invoice = core_db::invoices::get_invoice(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        if invoice.status != InvoiceStatus::Issued {
            return Err(ApiError::Conflict(format!(
                "statements are generated from the dunning path of issued invoices; this one is {}",
                invoice.status.as_str()
            )));
        }
        // The statement covers the client's whole open account, not just
        // the triggering invoice (PSA-standard statement semantics).
        let open = core_db::invoices::list_invoices(
            &mut *ctx.tx,
            org_id,
            &core_db::invoices::InvoiceFilters {
                client_id: Some(invoice.client_id),
                contract_id: None,
                status: Some("issued".into()),
                dunning_state: None,
            },
            200,
        )
        .await?
        .into_iter()
        .filter(|i| i.total_minor > i.total_paid_minor)
        .collect::<Vec<_>>();
        if open.is_empty() {
            return Err(ApiError::Conflict(
                "the client has no open invoices; nothing to state".into(),
            ));
        }
        let lines: Vec<StatementLine> = open
            .iter()
            .map(|i| StatementLine {
                number: i.number.clone(),
                due_on: i.due_on,
                total_minor: i.total_minor,
                outstanding_minor: i.total_minor - i.total_paid_minor,
            })
            .collect();
        let total_outstanding_minor: i64 = lines.iter().map(|l| l.outstanding_minor).sum();

        // Delivery targets: the client's contacts with an email address.
        let contacts = core_db::clients::list_contacts(&mut *ctx.tx, org_id, invoice.client_id)
            .await?;
        let mut queued_to: Vec<String> = Vec::new();
        for email in contacts.into_iter().filter_map(|c| c.email) {
            let normalized = email.trim().to_ascii_lowercase();
            if !normalized.is_empty() && !queued_to.contains(&normalized) {
                queued_to.push(normalized);
            }
        }
        if queued_to.is_empty() {
            return Err(ApiError::Validation(
                "the client has no contacts with an email address; add one before sending the statement"
                    .into(),
            ));
        }

        let client = core_db::clients::get_client(&mut *ctx.tx, org_id, invoice.client_id).await?;
        let mut invoice_lines = Vec::with_capacity(lines.len());
        for line in &lines {
            invoice_lines.push(format!(
                "{} · due {} · total {} · outstanding {}",
                line.number,
                line.due_on,
                format_minor(line.total_minor),
                format_minor(line.outstanding_minor),
            ));
        }
        enqueue_outbound(
            &mut ctx.tx,
            OutboundEnqueue {
                mail: &mail,
                org_id,
                to_addresses: queued_to.clone(),
                kind: &core_mail::outbound::OutboundKind::InvoiceStatement {
                    client_name: client.name,
                    currency: invoice.currency.clone(),
                    total_outstanding_minor,
                    invoice_lines,
                },
                in_reply_to: None,
                references: vec![],
                ticket_id: None,
                // The member who ran the dunning sweep is the person the
                // client is corresponding with, so their signature resolves
                // (their default, or the org's when they have none).
                signer: Some(user_id),
                overrides: crate::email::EnvelopeOverrides::default(),
            },
        )
        .await?;

        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: "billing.statement_sent".to_string(),
                entity_type: Some("invoice".to_string()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "client_id": invoice.client_id,
                    "open_invoices": lines.len(),
                    "total_outstanding_minor": total_outstanding_minor,
                    "queued_to": queued_to,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "invoice_statement",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let payload = StatementPayload {
            response: &SendStatementResponse {
                client_id: invoice.client_id,
                currency: invoice.currency.clone(),
                open_invoices: lines,
                total_outstanding_minor,
                queued_to: queued_to.clone(),
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "invoice_statement",
                entity_id: id,
                op: "statement_sent",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&payload).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &payload,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(StatementMutation {
                response: payload.response.clone(),
                mutation,
            }),
        ))
    })
    .await?;

    let applied = outcome.into_response()?;
    Ok(Json(applied.response))
}

/// The stored statement payload (mutation metadata merges in on record).
/// The response flattens so a replay deserializes into
/// [`StatementMutation`] (flattened response fields + `mutation`)
/// byte-identically.
#[derive(Debug, Serialize)]
struct StatementPayload<'a> {
    #[serde(flatten)]
    response: &'a SendStatementResponse,
}

/// The applied statement result carried through the mutation outcome
/// (flattened response + mutation metadata).
#[derive(Debug, Serialize, Deserialize)]
struct StatementMutation {
    #[serde(flatten)]
    response: SendStatementResponse,
    mutation: instant::MutationMeta,
}
