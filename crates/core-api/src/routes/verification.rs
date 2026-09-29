//! Billing verification workspace (Wave 39 lane 2, migration 0111): the
//! pre-invoice review surface, its in-context entry actions, and the
//! close ritual.
//!
//! One module, four surfaces:
//!
//! 1. **`GET /billing/verification`** — the workspace: every entry and
//!    its cost for one contract over one window, GROUPED BY TICKET,
//!    each with its disposition badge, running totals, the unclassified
//!    / draft / non-billable counters, and the retainer block-consumption
//!    panel read off lane 3's `contract_blocks` + draw ledger (which
//!    entries drew the block versus billed overage, the remaining
//!    balance, and a depletion alert). Read-only.
//!
//! 2. **Entry actions** (`/billing/verification/entries/{id}/action`) —
//!    approve, reject, flip-billable-with-reason, adjust minutes,
//!    recategorize, add note — in one gated endpoint so every action
//!    shares the SAME enforcement: reason required, audit row (V9),
//!    pre-run only, invoiced entries locked.
//!
//! 3. **Close sign-off** (`POST /billing/verification/signoff`) — the
//!    reviewer's attestation (entries dispositioned, zero unclassified,
//!    preview seen) recorded per contract window, with a run-window
//!    OVERLAP WARNING when an issued run already covers the same dates.
//!
//! 4. **Block panel** rides the read; the per-entry pricing uses the
//!    ENGINE's own `prorate` so Σ(entry costs) == the invoice line.
//!
//! Non-purpose: no approval state machine (approvals.rs owns it), no
//! rate/pricing writes, no invoice mutation. Suggestions from lane 1's
//! `decision_billing_audit` chips ride their existing read endpoints.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    routing::{get, post},
    Json, Router,
};
use core_db::verification::VerificationEntryRow;
use core_domain::entities::{
    Actor, AuditEvent, DispositionStatus, TimeEntryAdjustment, TimeRejectionReason, UnbillableReason,
};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

/// Upper bound on a verification window, matching the billing preview's
/// 366-day contract (one leap year of daily granularity).
const MAX_WINDOW_DAYS: i64 = 366;
const REASON_MAX: usize = 500;
const NOTE_MAX: usize = 1000;

pub fn routes(gen: crate::routes::ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/billing/verification"),
            get(workspace),
        )
        .route(
            &format!("{base}/billing/verification/entries/{{id}}/action"),
            post(entry_action),
        )
        .route(
            &format!("{base}/billing/verification/signoff"),
            post(signoff),
        )
}

fn time_perm(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Time,
        action,
    }
}

fn manage_billing() -> Permission {
    Permission::ManageBilling
}

// --- wire types --------------------------------------------------------------

/// Window + scope selector for the workspace read.
#[derive(Debug, Clone, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct VerificationQuery {
    /// The contract under review. Required: the workspace is
    /// per-contract because pricing, rate card, and blocks are.
    pub contract_id: Uuid,
    /// Inclusive period start (YYYY-MM-DD).
    pub from: chrono::NaiveDate,
    /// Inclusive period end (YYYY-MM-DD).
    pub to: chrono::NaiveDate,
}

/// One entry inside a ticket group: identity, cost cell, badge.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VerificationEntryView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    pub user_name: String,
    #[schema(value_type = String)]
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// Raw logged minutes.
    pub minutes: i64,
    /// Minutes after verification adjustments — what the run will price.
    pub effective_minutes: i64,
    /// Signed adjustment total (negative or zero; 0 when untouched).
    pub adjustment_minutes: i64,
    /// Priced cost in contract minor units at the resolved card rate.
    pub cost_minor: i64,
    /// The card rate this entry priced at, per hour.
    pub rate_minor: i64,
    /// The category slug, or null (unclassified — a blocker, not a blank).
    pub work_type: Option<String>,
    /// Approval state (draft/submitted/approved/rejected).
    pub approval_status: String,
    /// Typed unbillable reason when the entry is non-billable.
    pub unbillable_reason: Option<String>,
    /// The current human disposition, when one was recorded.
    pub disposition: Option<String>,
    /// Free-text note on the entry.
    pub notes: Option<String>,
    /// True when an invoice already consumed this entry (locked).
    pub invoiced: bool,
    /// Project phase this entry scopes to, when project-scoped.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub phase_id: Option<Uuid>,
}

/// One ticket (or project phase) group with its running totals.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VerificationGroupView {
    /// Stable grouping key: the ticket id, or the phase id for
    /// project-scoped entries (Wave 39 lane 4 gave entries a second
    /// parent, so a phase group has no ticket number).
    #[schema(value_type = uuid::Uuid)]
    pub group_id: Uuid,
    /// Human label: ticket number, else "Project / phase".
    pub label: String,
    pub project_name: Option<String>,
    pub entries: Vec<VerificationEntryView>,
    /// Group minutes after adjustments.
    pub total_minutes: i64,
    /// Group cost in contract minor units.
    pub total_cost_minor: i64,
}

/// The retainer / prepaid block-consumption panel (report V7): which
/// entries drew the included block versus billed as overage, the
/// remaining balance, and the depletion alert.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BlockPanelView {
    #[schema(value_type = uuid::Uuid)]
    pub block_id: Uuid,
    pub label: String,
    pub total_minutes: i64,
    /// Minutes drawn BEFORE this window (earlier runs).
    pub prior_drawn_minutes: i64,
    /// Minutes this window's entries drew.
    pub drawn_minutes: i64,
    /// Balance still available (never negative).
    pub remaining_minutes: i64,
    /// Minutes of this window's work that found no open block — they
    /// bill at card rates (the overage the admin must see).
    pub overage_minutes: i64,
    /// Per-entry draws: which entries consumed the block.
    pub draws: Vec<BlockDrawView>,
    /// Expiry, when the block carries one.
    pub expires_on: Option<chrono::NaiveDate>,
    /// `depleted` when the balance is gone and work is spilling over;
    /// `expiring` when a balance remains but the block expires in-window;
    /// `none` when healthy.
    pub alert: String,
}

/// One draw: which entry consumed how many block minutes.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BlockDrawView {
    #[schema(value_type = Option<uuid::Uuid>)]
    pub time_entry_id: Option<Uuid>,
    pub minutes: i64,
    pub spilled_minutes: i64,
}

/// The workspace counters that gate the close checklist.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VerificationCounters {
    pub entries: i64,
    pub billable: i64,
    pub non_billable: i64,
    /// Approved + billable + no category slug: these price at the
    /// contract default rate, which is exactly the silent billing the
    /// close ritual exists to catch.
    pub unclassified: i64,
    /// Entries nobody has dispositioned yet.
    pub undispositioned: i64,
    pub draft: i64,
    pub rejected: i64,
    pub invoiced: i64,
    pub adjusted: i64,
}

/// The full workspace response.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VerificationWorkspace {
    #[schema(value_type = uuid::Uuid)]
    pub contract_id: Uuid,
    pub contract_number: String,
    pub contract_name: String,
    pub contract_type: String,
    pub client_id: Uuid,
    pub client_name: Option<String>,
    pub currency: String,
    pub from: chrono::NaiveDate,
    pub to: chrono::NaiveDate,
    pub groups: Vec<VerificationGroupView>,
    pub counters: VerificationCounters,
    /// Running total across the whole window, in contract minor units.
    pub total_minor: i64,
    /// Running total minutes across the window (effective).
    pub total_minutes: i64,
    /// The block-consumption panel (empty when the contract has no
    /// blocks — retainer overage panels render nothing, not a zero).
    pub blocks: Vec<BlockPanelView>,
    /// What the engine says this window would bill. The workspace's
    /// own entry sum and this are computed independently on purpose:
    /// they must agree, and a disagreement is the signal that a
    /// disposition was missed.
    pub preview_total_minor: i64,
    /// Issued runs whose window overlaps this one (report V8).
    pub overlapping_runs: Vec<RunWindowView>,
    /// The sign-off recorded for this window, if any.
    pub signoff: Option<SignoffView>,
}

/// One issued run window in the overlap warning.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RunWindowView {
    #[schema(value_type = uuid::Uuid)]
    pub run_id: Uuid,
    pub number: String,
    pub period_from: chrono::NaiveDate,
    pub period_to: chrono::NaiveDate,
}

/// The close checklist as recorded.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SignoffView {
    pub entries_dispositioned: bool,
    pub zero_unclassified: bool,
    pub preview_seen: bool,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub reviewer_id: Option<Uuid>,
    pub reviewer_note: Option<String>,
    #[schema(value_type = String)]
    pub signed_at: chrono::DateTime<chrono::Utc>,
    /// The run that claimed this sign-off, when one has.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub run_id: Option<Uuid>,
}

/// The body of an entry action. One endpoint, one envelope, every
/// in-context edit — so the reason requirement, the audit row, the
/// pre-run guard, and the invoiced lock cannot be forgotten on one
/// branch.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct EntryActionRequest {
    /// `approve` | `reject` | `flip_billable` | `adjust_minutes` |
    /// `recategorize` | `add_note`.
    pub action: String,
    /// Required by `reject`, `flip_billable`, and `adjust_minutes`:
    /// the human explanation that rides the audit row (V9). Omitting
    /// it is a 422, not a silent write.
    #[serde(default)]
    pub reason: Option<String>,
    /// `reject` only: the typed rejection code.
    #[serde(default)]
    pub reason_code: Option<String>,
    /// `adjust_minutes` only: the NEW effective minutes for the entry
    /// (0..=current effective). The signed delta is derived here.
    #[serde(default)]
    pub minutes: Option<i64>,
    /// `flip_billable` only: required when flipping to non-billable.
    #[serde(default)]
    pub unbillable_reason: Option<String>,
    /// `recategorize` only: a category slug from the org taxonomy.
    #[serde(default)]
    pub work_type: Option<String>,
    /// `add_note` only: the note text.
    #[serde(default)]
    pub note: Option<String>,
    /// IF1-IF3 mutation parameters (idempotency key / version).
    #[serde(flatten)]
    pub instant: instant::MutationParams,
}

/// The applied action result.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EntryActionResponse {
    #[schema(value_type = uuid::Uuid)]
    pub entry_id: Uuid,
    /// The action actually applied.
    pub action: String,
    /// The entry's current disposition after the write.
    pub disposition: Option<String>,
    /// Effective minutes after the write.
    pub effective_minutes: i64,
    /// The adjustment row this write produced, when it produced one.
    pub adjustment: Option<TimeEntryAdjustment>,
    /// The audit action name written to `audit_events` (V9).
    pub audit_action: String,
    pub mutation: instant::MutationMeta,
}

/// The close sign-off body.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct SignoffRequest {
    #[schema(value_type = uuid::Uuid)]
    pub contract_id: Uuid,
    pub from: chrono::NaiveDate,
    pub to: chrono::NaiveDate,
    pub entries_dispositioned: bool,
    pub zero_unclassified: bool,
    pub preview_seen: bool,
    #[serde(default)]
    pub reviewer_note: Option<String>,
    #[serde(flatten)]
    pub instant: instant::MutationParams,
}

/// The sign-off response.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SignoffResponse {
    pub signoff: SignoffView,
    /// Issued runs overlapping this window — a WARNING, not a block:
    /// re-running an overlapping window is a real operator need, but
    /// it must never be silent.
    pub overlapping_runs: Vec<RunWindowView>,
    pub mutation: instant::MutationMeta,
}

// --- helpers -----------------------------------------------------------------

fn parse_window(from: chrono::NaiveDate, to: chrono::NaiveDate) -> ApiResult<()> {
    if to < from {
        return Err(ApiError::Validation("to must not precede from".into()));
    }
    if (to - from).num_days() > MAX_WINDOW_DAYS {
        return Err(ApiError::Validation(
            "verification window exceeds 366 days; split the range".into(),
        ));
    }
    Ok(())
}

fn require_reason(reason: Option<&str>, action: &str) -> ApiResult<String> {
    let reason = reason.unwrap_or_default().trim().to_string();
    if reason.is_empty() {
        return Err(ApiError::Validation(format!(
            "{action} requires a reason"
        )));
    }
    if reason.chars().count() > REASON_MAX {
        return Err(ApiError::Validation(format!(
            "reason must be at most {REASON_MAX} characters"
        )));
    }
    Ok(reason)
}

fn engine_error(e: core_billing::BillingError) -> ApiError {
    match e {
        core_billing::BillingError::InvalidWindow { from, to } => {
            ApiError::Validation(format!("window is inverted: {from} is after {to}"))
        }
        core_billing::BillingError::AmountOverflow => {
            ApiError::Conflict("billing amounts overflow; narrow the window".into())
        }
    }
}

/// Resolves the card rate for one entry: the exact `work_type` row
/// first, then the contract default. The same precedence the engine
/// uses in `capped_labor_lines` — a read model that priced differently
/// would show a total the invoice never matches.
fn rate_for(
    rates: &[core_domain::entities::ContractRate],
    default_rate_minor: i64,
    work_type: Option<&str>,
) -> i64 {
    work_type
        .and_then(|wt| rates.iter().find(|r| r.work_type == wt))
        .map(|r| r.rate_minor)
        .unwrap_or(default_rate_minor)
}

/// Build the workspace for one contract window. Read-only, RLS-scoped.
async fn build_workspace(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    contract_id: Uuid,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
) -> ApiResult<VerificationWorkspace> {
    parse_window(from, to)?;
    let contract = core_db::contracts::get_contract(&mut *tx, org_id, contract_id)
        .await
        .map_err(ApiError::from)?;
    let client_name: Option<String> =
        sqlx::query_scalar("SELECT name FROM clients WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(contract.client_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(ApiError::from)?;
    let rates = core_db::contracts::list_rates(&mut *tx, org_id, contract_id).await?;
    let fees = core_db::contracts::list_fees(&mut *tx, org_id, contract_id).await?;
    let rows = core_db::verification::list_client_entries(
        &mut *tx,
        org_id,
        contract.client_id,
        from,
        to,
    )
    .await?;

    // The engine's own evaluation: block consumption, and the total the
    // run would produce. Shares `load_eval_inputs` with the preview and
    // the run, so a preview agrees with the run it mirrors.
    let eval_inputs =
        crate::routes::billing::load_eval_inputs(&mut *tx, org_id, &contract, from, to).await?;
    let billable_rows = core_db::contracts::list_billable_entries(
        &mut *tx,
        org_id,
        contract.client_id,
        from,
        to,
    )
    .await?;
    let engine_entries: Vec<core_billing::BillableEntry> = billable_rows
        .into_iter()
        .map(|e| core_billing::BillableEntry {
            time_entry_id: e.time_entry_id,
            minutes: e.minutes,
            work_type: e.work_type,
            entry_date: e.entry_date,
        })
        .collect();
    let evaluation = core_billing::rules::evaluate_with(
        &contract,
        &rates,
        &fees,
        &engine_entries,
        from,
        to,
        &eval_inputs,
    )
    .map_err(engine_error)?;

    // Grouped by ticket (or phase for project-scoped entries), in the
    // read order, so the reviewer reads the window the way it happened.
    let default_rate = contract.hourly_rate_minor.unwrap_or(0);
    let mut groups: Vec<VerificationGroupView> = Vec::new();
    let mut counters = VerificationCounters {
        entries: 0,
        billable: 0,
        non_billable: 0,
        unclassified: 0,
        undispositioned: 0,
        draft: 0,
        rejected: 0,
        invoiced: 0,
        adjusted: 0,
    };
    let mut total_minor: i64 = 0;
    let mut total_minutes: i64 = 0;

    for row in &rows {
        let view = entry_view(row, &rates, default_rate);
        // The SAME population the cost cell prices, so rows, groups and
        // the window total cannot disagree with each other.
        let counts_toward_total = row.billable && row.approval_status == "approved";
        counters.entries += 1;
        if row.billable {
            counters.billable += 1;
        } else {
            counters.non_billable += 1;
        }
        if row.approval_status == "draft" || row.approval_status == "submitted" {
            counters.draft += 1;
        }
        if row.approval_status == "rejected" {
            counters.rejected += 1;
        }
        if row.invoiced {
            counters.invoiced += 1;
        }
        if row.adjustment_minutes != 0 {
            counters.adjusted += 1;
        }
        // An unclassified APPROVED billable entry prices at the
        // contract default — the silent-billing case the counters
        // exist to make impossible to miss.
        if row.billable && row.work_type.is_none() && row.approval_status == "approved" {
            counters.unclassified += 1;
        }
        if row.disposition.is_none() && !row.invoiced {
            counters.undispositioned += 1;
        }
        // The running total is what this window WILL bill, so it sums
        // the same population the engine prices: billable AND approved.
        // Adding draft or unapproved minutes here would make the
        // reviewer's headline number disagree with the preview sitting
        // beside it — and an e2e run proved exactly that, so the
        // discipline is: this figure foots, or the contract is doing
        // work the entry table cannot show (base fees, block sales,
        // caps), which the UI names explicitly.
        if row.billable && row.approval_status == "approved" {
            total_minor += view.cost_minor;
            total_minutes += view.effective_minutes;
        }

        // Group key: ticket when there is one, else the phase.
        let (group_id, label, project_name) = match (row.ticket_id, row.phase_id) {
            (Some(ticket_id), _) => (
                ticket_id,
                row.ticket_number.clone().unwrap_or_else(|| "Ticket".into()),
                row.project_name.clone(),
            ),
            (None, Some(phase_id)) => (
                phase_id,
                format!(
                    "{} / {}",
                    row.project_name.clone().unwrap_or_else(|| "Project".into()),
                    row.phase_name.clone().unwrap_or_else(|| "Phase".into())
                ),
                row.project_name.clone(),
            ),
            (None, None) => (row.entry_id, "Unassigned".into(), None),
        };
        match groups.iter_mut().find(|g| g.group_id == group_id) {
            Some(group) => {
                if counts_toward_total {
                    group.total_minutes += view.effective_minutes;
                    group.total_cost_minor += view.cost_minor;
                }
                group.entries.push(view);
            }
            None => groups.push(VerificationGroupView {
                group_id,
                label,
                project_name,
                total_minutes: if counts_toward_total {
                    view.effective_minutes
                } else {
                    0
                },
                total_cost_minor: if counts_toward_total {
                    view.cost_minor
                } else {
                    0
                },
                entries: vec![view],
            }),
        }
    }

    let blocks = block_panel(&eval_inputs, &evaluation, from, to);
    let signoff = core_db::verification::get_run_signoff(
        &mut *tx,
        org_id,
        contract_id,
        from,
        to,
    )
    .await?
    .map(|s| SignoffView {
        entries_dispositioned: s.entries_dispositioned,
        zero_unclassified: s.zero_unclassified,
        preview_seen: s.preview_seen,
        reviewer_id: s.reviewer_id,
        reviewer_note: s.reviewer_note,
        signed_at: s.signed_at,
        run_id: s.run_id,
    });
    let overlapping_runs = overlapping_windows(&mut *tx, org_id, from, to).await?;

    // A read is not a mutation: the caller is authenticated, not
    // written. `user_id` is unused by the body but keeps the signature
    // honest about the audit boundary.
    let _ = user_id;
    Ok(VerificationWorkspace {
        contract_id: contract.id,
        contract_number: contract.number.clone(),
        contract_name: contract.name.clone(),
        contract_type: contract.contract_type.as_str().to_string(),
        client_id: contract.client_id,
        client_name,
        currency: contract.currency.clone(),
        from,
        to,
        groups,
        counters,
        total_minor,
        total_minutes,
        blocks,
        preview_total_minor: evaluation.total_minor,
        overlapping_runs,
        signoff,
    })
}

/// One entry's row. `counts_toward_total` decides whether the cost cell
/// carries money: a non-billable or not-yet-approved entry renders at
/// zero rather than a "what if this were approved" figure, because a
/// table whose rows do not sum to the total above it teaches the reader
/// not to trust either number. The minutes still show, the badge still
/// says why it will not bill.
fn entry_view(
    row: &VerificationEntryRow,
    rates: &[core_domain::entities::ContractRate],
    default_rate_minor: i64,
) -> VerificationEntryView {
    let rate = rate_for(rates, default_rate_minor, row.work_type.as_deref());
    let counts_toward_total = row.billable && row.approval_status == "approved";
    let cost_minor = if counts_toward_total {
        core_billing::prorate(row.effective_minutes, rate).unwrap_or(0)
    } else {
        0
    };
    VerificationEntryView {
        id: row.entry_id,
        user_id: row.user_id,
        user_name: row.user_name.clone(),
        started_at: row.started_at,
        minutes: row.minutes,
        effective_minutes: row.effective_minutes,
        adjustment_minutes: row.adjustment_minutes,
        cost_minor,
        rate_minor: rate,
        work_type: row.work_type.clone(),
        approval_status: row.approval_status.clone(),
        unbillable_reason: row.unbillable_reason.clone(),
        disposition: row.disposition.map(|d| d.as_str().to_string()),
        notes: row.notes.clone(),
        invoiced: row.invoiced,
        phase_id: row.phase_id,
    }
}

/// The block-consumption panel (report V7). Reads the engine's own
/// `BlockConsumption` for THIS window (which entries drew the block)
/// and the persisted draw ledger for the PRIOR balance, so the number
/// an admin checks is the number the next run will use.
fn block_panel(
    inputs: &core_billing::EvalInputs<'_>,
    evaluation: &core_domain::entities::BillingEvaluation,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
) -> Vec<BlockPanelView> {
    let Some(consumption) = evaluation.block_consumption.as_ref() else {
        return Vec::new();
    };
    inputs
        .blocks
        .iter()
        .map(|block| {
            let prior = inputs
                .prior_draws
                .and_then(|d| d.get(&block.id).copied())
                .unwrap_or(0);
            let drawn: i64 = consumption
                .draws
                .iter()
                .filter(|d| d.block_id == block.id)
                .map(|d| d.minutes as i64)
                .sum();
            let overage: i64 = consumption
                .draws
                .iter()
                .filter(|d| d.block_id == block.id)
                .map(|d| d.spilled_minutes as i64)
                .sum();
            let total = block.total_minutes as i64;
            let remaining = (total - prior - drawn).max(0);
            // Expiry INSIDE the window with a balance left is its own
            // alert: the work is fine, the money is about to be gone.
            let expires_in_window = block
                .expires_on
                .is_some_and(|e| e >= from && e <= to && remaining > 0);
            let alert = if remaining == 0 && (drawn > 0 || overage > 0) {
                "depleted"
            } else if expires_in_window {
                "expiring"
            } else {
                "none"
            };
            BlockPanelView {
                block_id: block.id,
                label: block.label.clone(),
                total_minutes: total,
                prior_drawn_minutes: prior,
                drawn_minutes: drawn,
                remaining_minutes: remaining,
                overage_minutes: overage,
                draws: consumption
                    .draws
                    .iter()
                    .filter(|d| d.block_id == block.id)
                    .map(|d| BlockDrawView {
                        time_entry_id: d.time_entry_id,
                        minutes: d.minutes as i64,
                        spilled_minutes: d.spilled_minutes as i64,
                    })
                    .collect(),
                expires_on: block.expires_on,
                alert: alert.to_string(),
            }
        })
        .filter(|b| b.drawn_minutes > 0 || b.prior_drawn_minutes > 0 || b.total_minutes > 0)
        .collect()
}

/// Issued runs whose window overlaps `[from, to]` (report V8). The
/// `invoice_runs` table IS the run-window ledger.
async fn overlapping_windows(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
) -> ApiResult<Vec<RunWindowView>> {
    let rows = core_db::verification::list_run_windows(&mut *tx, org_id).await?;
    Ok(rows
        .into_iter()
        .filter(|(_, _, run_from, run_to)| *run_from <= to && *run_to >= from)
        .map(|(run_id, number, period_from, period_to)| RunWindowView {
            run_id,
            number,
            period_from,
            period_to,
        })
        .collect())
}

async fn audit_entry(
    ctx: &mut org_tx::OrgTx<'static>,
    org_id: Uuid,
    actor: Uuid,
    action: &str,
    entry_id: Uuid,
    detail: Value,
) -> ApiResult<()> {
    core_db::audit::insert(
        &mut *ctx.tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(actor),
            action: action.into(),
            entity_type: Some("time_entry".into()),
            entity_id: Some(entry_id.to_string()),
            detail,
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    Ok(())
}

// --- handlers ----------------------------------------------------------------

/// GET /api/billing/verification — the workspace for one contract over
/// one window. READ-ONLY: no entry changes, no side effects.
#[utoipa::path(operation_id = "billing_verification", get, path = "/api/billing/verification", tag = "billing", security(("bearer" = [])),
    params(VerificationQuery),
    responses((status = 200, body = VerificationWorkspace),
              (status = 404, description = "contract not visible in scope (rls_sparse)"),
              (status = 422, description = "inverted or oversized window")))]
pub async fn workspace(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<VerificationQuery>,
) -> ApiResult<Json<VerificationWorkspace>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_billing())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let workspace = build_workspace(
        &mut tx,
        org_id,
        user_id,
        q.contract_id,
        q.from,
        q.to,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(workspace))
}

/// POST /api/billing/verification/entries/{id}/action — every in-context
/// entry edit, one envelope. Gated on the instant-first mutation
/// contract: same key + same payload replays, different payload 409s.
#[utoipa::path(operation_id = "billing_verification_entry_action", post, path = "/api/billing/verification/entries/{id}/action", tag = "billing", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Time entry id")),
    request_body = EntryActionRequest,
    responses((status = 200, body = EntryActionResponse),
              (status = 404, description = "entry not visible in scope (rls_sparse)"),
              (status = 409, description = "entry already invoiced; verification is pre-run only"),
              (status = 422, description = "missing reason, unknown action, or invalid value")))]
pub async fn entry_action(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<EntryActionRequest>,
) -> ApiResult<Json<EntryActionResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    // Billing decisions on someone's time: the time matrix gates the
    // write, `manage_billing` gates the money call, and `approve_time`
    // gates the approval transitions specifically.
    require_permission(&authed.principal, time_perm(Action::Edit))?;
    require_permission(&authed.principal, manage_billing())?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let key = req.instant.idempotency_key.clone();
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);

    let outcome = org_tx::with_org_tx(&state, org_id, actor, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "time_entry",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        // Load the entry, then the invoiced lock — one read, one typed
        // refusal. RLS proves visibility: an invisible entry is not
        // found, never a 403.
        let entry = core_db::timers::get_time_entry(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        if core_db::verification::entry_is_invoiced(&mut *ctx.tx, org_id, id).await? {
            return Err(ApiError::Conflict(
                "entry is already invoiced and locked; verification actions are pre-run only"
                    .into(),
            ));
        }
        let current_adjustment =
            core_db::verification::total_adjustment_minutes(&mut *ctx.tx, org_id, id).await?;
        let current_effective = entry.minutes + current_adjustment;

        let action = req.action.trim().to_lowercase();
        let mut disposition: Option<DispositionStatus> = None;
        let mut adjustment: Option<TimeEntryAdjustment> = None;
        // Every branch below assigns the audit action name it wrote.
        let audit_action;
        let mut effective_minutes = current_effective;
        let detail;
        let approval_note = req.note.clone().unwrap_or_default().trim().to_string();
        let approval_note = if approval_note.is_empty() {
            None
        } else {
            if approval_note.chars().count() > NOTE_MAX {
                return Err(ApiError::Validation(format!(
                    "note must be at most {NOTE_MAX} characters"
                )));
            }
            Some(approval_note)
        };

        match action.as_str() {
            "approve" | "reject" => {
                // The approval state machine belongs to approvals.rs
                // (draft → submitted → approved | rejected). The
                // verification workspace calls it rather than
                // re-implementing transitions, so the queue and the
                // workspace can never disagree about what "approved"
                // means. A rejection carries the typed reason code.
                if action == "reject" {
                    let code = req
                        .reason_code
                        .as_deref()
                        .and_then(TimeRejectionReason::parse)
                        .ok_or_else(|| {
                            ApiError::Validation(
                                "reject requires a typed reason_code (missing_notes, wrong_ticket, duplicate, not_work, other)".into(),
                            )
                        })?;
                    let _ = require_reason(req.reason.as_deref(), "reject")?;
                    let _ = code;
                }
                let applied = core_db::approvals::decide(
                    &mut ctx.tx,
                    org_id,
                    &[id],
                    action == "approve",
                    actor,
                    req.reason_code.as_deref(),
                    approval_note.as_deref(),
                    chrono::Utc::now(),
                )
                .await?;
                let decided = applied.first().ok_or_else(|| {
                    ApiError::Conflict(format!(
                        "entry is not awaiting a decision (status {})",
                        entry.approval().as_str()
                    ))
                })?;
                disposition = Some(if action == "approve" {
                    DispositionStatus::Approved
                } else {
                    DispositionStatus::Rejected
                });
                audit_action = format!("billing_verification.{action}");
                detail = serde_json::json!({
                    "approval_status": decided.approval().as_str(),
                    "reason_code": req.reason_code,
                    "note": approval_note,
                });
            }
            "flip_billable" => {
                // Money call: flipping to non-billable needs a typed
                // reason (V3), flipping back to billable clears it.
                // Both are audit rows, and the disposition lands so
                // the badge follows the flip.
                let reason = require_reason(req.reason.as_deref(), "flip_billable")?;
                let to_billable = !entry.billable;
                let unbillable_reason = if to_billable {
                    None
                } else {
                    let code = req
                        .unbillable_reason
                        .as_deref()
                        .and_then(UnbillableReason::parse)
                        .ok_or_else(|| {
                            ApiError::Validation(
                                "flipping to non-billable requires unbillable_reason (warranty, courtesy, in_retainer_block, sla_covered, internal, mistake)".into(),
                            )
                        })?;
                    Some(code.as_str().to_string())
                };
                core_db::timers::update_time_entry(
                    &mut *ctx.tx,
                    org_id,
                    id,
                    entry.started_at,
                    entry.ended_at,
                    entry.minutes,
                    to_billable,
                    entry.notes.as_deref(),
                    entry.work_type.as_deref(),
                    unbillable_reason.as_deref(),
                )
                .await?;
                disposition = Some(if to_billable {
                    DispositionStatus::Approved
                } else {
                    DispositionStatus::NonBillable
                });
                audit_action = "billing_verification.flip_billable".into();
                detail = serde_json::json!({
                    "from": entry.billable,
                    "to": to_billable,
                    "unbillable_reason": unbillable_reason,
                    "reason": reason,
                });
            }
            "adjust_minutes" => {
                // The additive-ledger action (V2): the request carries
                // the NEW effective minutes; the signed delta is
                // derived and the ORIGINAL minutes ride the row as
                // evidence. Reducing only — verification can lower a
                // bill, never raise it; raising is an entry edit.
                let reason = require_reason(req.reason.as_deref(), "adjust_minutes")?;
                let target = req.minutes.ok_or_else(|| {
                    ApiError::Validation("adjust_minutes requires 'minutes' (the new total)".into())
                })?;
                if target < 0 {
                    return Err(ApiError::Validation("minutes must not be negative".into()));
                }
                if target > current_effective {
                    return Err(ApiError::Validation(format!(
                        "adjust_minutes reduces or zeroes: {target} exceeds the current effective {current_effective}"
                    )));
                }
                let delta = target - current_effective;
                adjustment = Some(
                    core_db::verification::insert_adjustment(
                        &mut ctx.tx,
                        core_db::verification::NewAdjustment {
                            org_id,
                            time_entry_id: id,
                            run_id: None,
                            original_minutes: current_effective as i32,
                            delta_minutes: delta as i32,
                            reason: &reason,
                            actor_id: actor,
                        },
                    )
                    .await?,
                );
                effective_minutes = target;
                // An adjusted entry has been reviewed: that IS the
                // disposition (the reviewer chose these minutes).
                disposition = Some(DispositionStatus::Approved);
                audit_action = "billing_verification.adjust_minutes".into();
                detail = serde_json::json!({
                    "from_minutes": current_effective,
                    "to_minutes": target,
                    "delta_minutes": delta,
                    "reason": reason,
                });
            }
            "recategorize" => {
                // The taxonomy key (lane 1): slug-first, then the kept
                // legacy mapping; anything unknown is refused, never
                // guessed. Recategorizing alone does not disposition
                // the entry — the reviewer still owes a decision.
                let raw = req
                    .work_type
                    .as_deref()
                    .ok_or_else(|| ApiError::Validation("recategorize requires 'work_type'".into()))?;
                let slug = core_db::time_categories::resolve_work_type_slug(
                    &mut ctx.tx,
                    org_id,
                    raw,
                )
                .await?
                .ok_or_else(|| {
                    ApiError::Validation(format!("unknown work_type '{raw}'"))
                })?;
                let reason = req
                    .reason
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("recategorized during billing verification")
                    .to_string();
                core_db::timers::update_time_entry(
                    &mut *ctx.tx,
                    org_id,
                    id,
                    entry.started_at,
                    entry.ended_at,
                    entry.minutes,
                    entry.billable,
                    entry.notes.as_deref(),
                    Some(&slug),
                    entry.unbillable_reason.as_deref(),
                )
                .await?;
                audit_action = "billing_verification.recategorize".into();
                detail = serde_json::json!({
                    "from": entry.work_type,
                    "to": slug,
                    "reason": reason,
                });
            }
            "add_note" => {
                // Pure annotation: never dispositions, never touches
                // minutes or billability.
                let note = approval_note.ok_or_else(|| {
                    ApiError::Validation("add_note requires 'note'".into())
                })?;
                core_db::timers::update_time_entry(
                    &mut *ctx.tx,
                    org_id,
                    id,
                    entry.started_at,
                    entry.ended_at,
                    entry.minutes,
                    entry.billable,
                    Some(&note),
                    entry.work_type.as_deref(),
                    entry.unbillable_reason.as_deref(),
                )
                .await?;
                audit_action = "billing_verification.add_note".into();
                detail = serde_json::json!({ "note": note });
            }
            other => {
                return Err(ApiError::Validation(format!(
                    "unknown verification action '{other}'"
                )));
            }
        }

        // The disposition row is the workspace's badge source, and it
        // enforces the invoiced lock for its own write.
        if let Some(status) = disposition {
            let reason_for_disposition = req.reason.as_deref().map(str::trim);
            core_db::verification::upsert_disposition(
                &mut ctx.tx,
                org_id,
                id,
                status,
                if status.requires_unbillable_reason() {
                    req.unbillable_reason.as_deref()
                } else {
                    None
                },
                reason_for_disposition.filter(|r| !r.is_empty()),
                actor,
            )
            .await?;
        }

        audit_entry(&mut ctx, org_id, actor, &audit_action, id, detail).await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "time_entry",
                entity_id: id,
                version: None,
            },
        )
        .await?;
        let stored = EntryActionResponse {
            entry_id: id,
            action: action.clone(),
            disposition: disposition.map(|d| d.as_str().to_string()),
            effective_minutes,
            adjustment: adjustment.clone(),
            audit_action: audit_action.clone(),
            mutation: instant::MutationMeta {
                seq: write_seq,
                version: write_seq,
                updated_at: chrono::Utc::now(),
                before: None,
                after: serde_json::Value::Null,
                idempotency_key: key.clone(),
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "time_entry",
                entity_id: id,
                op: "mutation",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&stored).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &stored,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(EntryActionResponse {
                mutation,
                ..stored
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/billing/verification/signoff — the close ritual (report
/// V8). The checklist booleans are RE-VERIFIED against the live
/// workspace, never taken on trust: signing off "every entry
/// dispositioned" while a draft entry sits in the window is a 422, not
/// a stored lie. The overlap warning rides the response.
#[utoipa::path(operation_id = "billing_verification_signoff", post, path = "/api/billing/verification/signoff", tag = "billing", security(("bearer" = [])),
    request_body = SignoffRequest,
    responses((status = 200, body = SignoffResponse),
              (status = 404, description = "contract not visible in scope (rls_sparse)"),
              (status = 422, description = "checklist claim does not match the live window")))]
pub async fn signoff(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<SignoffRequest>,
) -> ApiResult<Json<SignoffResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_billing())?;
    let org_id = authed.principal.user.org_id;
    let reviewer = authed.principal.user.id;
    let key = req.instant.idempotency_key.clone();
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);

    let outcome = org_tx::with_org_tx(&state, org_id, reviewer, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "invoice_run_signoff",
                entity_id: Some(req.contract_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        let workspace = build_workspace(
            &mut ctx.tx,
            org_id,
            reviewer,
            req.contract_id,
            req.from,
            req.to,
        )
        .await?;

        // A reviewer can attest only what is true. Both claims are
        // checked against the counters computed a moment earlier in
        // this same transaction.
        let undispositioned = workspace.counters.undispositioned;
        let unclassified = workspace.counters.unclassified;
        if req.entries_dispositioned && undispositioned > 0 {
            return Err(ApiError::Validation(format!(
                "checklist claim 'entries_dispositioned' is false: {undispositioned} entries in the window are still undispositioned"
            )));
        }
        if req.zero_unclassified && unclassified > 0 {
            return Err(ApiError::Validation(format!(
                "checklist claim 'zero_unclassified' is false: {unclassified} approved billable entries have no category"
            )));
        }
        if !req.entries_dispositioned || !req.zero_unclassified || !req.preview_seen {
            return Err(ApiError::Validation(
                "a close sign-off attests all three checklist items (entries_dispositioned, zero_unclassified, preview_seen)".into(),
            ));
        }
        if let Some(note) = req.reviewer_note.as_deref() {
            if note.chars().count() > NOTE_MAX {
                return Err(ApiError::Validation(format!(
                    "reviewer_note must be at most {NOTE_MAX} characters"
                )));
            }
        }

        core_db::verification::upsert_run_signoff(
            &mut *ctx.tx,
            org_id,
            req.contract_id,
            req.from,
            req.to,
            true,
            true,
            true,
            reviewer,
            req.reviewer_note.as_deref(),
        )
        .await?;

        let overlap = workspace.overlapping_runs.clone();
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(reviewer),
                action: "billing_verification.signed_off".into(),
                entity_type: Some("contract".into()),
                entity_id: Some(req.contract_id.to_string()),
                detail: serde_json::json!({
                    "from": req.from,
                    "to": req.to,
                    "entries": workspace.counters.entries,
                    "undispositioned": undispositioned,
                    "unclassified": unclassified,
                    "preview_total_minor": workspace.preview_total_minor,
                    "overlapping_runs": overlap
                        .iter()
                        .map(|r| r.number.as_str())
                        .collect::<Vec<_>>(),
                    "reviewer_note": req.reviewer_note,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "invoice_run_signoff",
                entity_id: req.contract_id,
                version: None,
            },
        )
        .await?;
        let stored = SignoffResponse {
            signoff: SignoffView {
                entries_dispositioned: true,
                zero_unclassified: true,
                preview_seen: true,
                reviewer_id: Some(reviewer),
                reviewer_note: req.reviewer_note.clone(),
                signed_at: chrono::Utc::now(),
                run_id: None,
            },
            overlapping_runs: overlap,
            mutation: instant::MutationMeta {
                seq: write_seq,
                version: write_seq,
                updated_at: chrono::Utc::now(),
                before: None,
                after: serde_json::Value::Null,
                idempotency_key: key.clone(),
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "invoice_run_signoff",
                entity_id: req.contract_id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&stored).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &stored,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(SignoffResponse {
                mutation,
                ..stored
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

