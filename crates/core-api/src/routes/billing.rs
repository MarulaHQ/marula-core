//! Contracts + recurring-billing rule evaluation (TASKLIST Phase 3,
//! Wave 8b lane BILLING).
//!
//! Purpose: contract CRUD across the four commercial types (recurring /
//! time_and_materials / retainer / fixed_fee) with effective dating, plus
//! READ-ONLY rule evaluation: the billing rules engine
//! (`core_billing::rules::evaluate`) turns a contract and its supporting
//! rows into draft invoice-line candidates for a window. Two preview
//! surfaces ship: per-contract and org-wide (the shape a Wave-9 invoice
//! run will consume).
//!
//! Instant-first contract (docs/instant-first.md): creates and updates go
//! through the shared claim/entity_seq/record layer — idempotency-keyed,
//! If-Match versioned, read-your-write `mutation` metadata on the
//! response. All failures return the central typed error envelope.
//!
//! Non-purpose: NO invoice persistence (no invoice/line/payment rows, no
//! `billed` markers) — invoice run, payments, and dunning are Wave 9.
//! Rates and fees are managed as part of the contract aggregate (replace
//! semantics inside the mutation transaction), not as standalone CRUD
//! surfaces.

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
use core_domain::entities::{
    BillingEvaluation, Contract, ContractBlock, ContractCap, ContractChangeOrder,
    ContractEscalator, ContractFixedFee, ContractRate, ContractSlaCredit, ContractStatus,
    ContractType,
};
use core_domain::events::{types, ContractCreated, ContractRemoved, ContractUpdated};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

fn commercial(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Commercial,
        action,
    }
}

/// RB2 (ADR 0004): contract rates are financial data. Every rate-bearing
/// surface (list/detail/preview, and the mutations that carry rate
/// fields) additionally requires `financial:read` — dispatchers and
/// technicians hold the commercial matrix but NOT financial visibility
/// by default. Removal stays commercial-gated only: deleting is not a
/// rate read.
fn financial_read() -> Permission {
    Permission::ViewFinancials
}

/// Upper bound on a preview window: the engine is deterministic but an
/// unbounded scan is not a contract. One leap year of daily granularity.
const MAX_PREVIEW_DAYS: i64 = 366;

// --- wire structs ------------------------------------------------------------

/// T&M rate-card row spec (hourly minor units per `work_type`).
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, PartialEq)]
pub struct RateSpec {
    pub work_type: String,
    pub rate_minor: i64,
}

/// Fixed-fee schedule entry spec.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, PartialEq)]
pub struct FeeSpec {
    pub label: String,
    pub amount_minor: i64,
    pub due_on: chrono::NaiveDate,
    pub notes: Option<String>,
}

/// Prepaid-block spec (A): an hour bank sold up front.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, PartialEq)]
pub struct BlockSpec {
    pub label: String,
    pub total_minutes: i32,
    pub sale_price_minor: i64,
    /// Optional discounted draw-down rate, minor units per hour.
    pub block_rate_minor: Option<i64>,
    pub expires_on: Option<chrono::NaiveDate>,
    /// `expire|rollover|none`.
    pub rollover_policy: String,
}

/// NTE cap spec (B): typed not-to-exceed behavior.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, PartialEq)]
pub struct CapSpec {
    /// `period|contract`.
    pub scope: String,
    pub cap_minor: i64,
    /// `stop|warn|overage_billable`.
    pub reached_behavior: core_domain::entities::CapReachedBehavior,
    pub overage_rate_minor: Option<i64>,
    /// Optional work_type bucket the cap covers (`None` = all labor).
    pub work_type: Option<String>,
}

/// Scheduled escalator spec (E): uplift bps per billing-annual
/// anniversary.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, PartialEq)]
pub struct EscalatorSpec {
    pub uplift_bps: i32,
    /// First anniversary the uplift applies from (`None` = the anchor's
    /// first full period).
    pub first_uplift_on: Option<chrono::NaiveDate>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateContractRequest {
    pub client_id: Uuid,
    pub number: String,
    pub name: String,
    pub contract_type: ContractType,
    /// Defaults to `draft`.
    pub status: Option<ContractStatus>,
    /// ISO 4217 code (e.g. "USD"); every amount uses this currency.
    pub currency: String,
    pub effective_from: chrono::NaiveDate,
    pub effective_until: Option<chrono::NaiveDate>,
    /// Required for recurring/retainer.
    pub billing_frequency: Option<core_domain::entities::BillingFrequency>,
    /// Required for recurring/retainer; first billing period's start.
    pub billing_anchor: Option<chrono::NaiveDate>,
    /// Per-period price (recurring/retainer), minor units.
    pub amount_minor: Option<i64>,
    /// T&M default hourly rate, minor units.
    pub hourly_rate_minor: Option<i64>,
    /// Retainer included minutes per period.
    pub included_minutes: Option<i32>,
    /// Retainer hourly overage rate, minor units.
    pub overage_rate_minor: Option<i64>,
    /// Per-unit recurring (C): price per unit per period.
    pub unit_price_minor: Option<i64>,
    /// Per-unit recurring (C): `devices|users`.
    pub unit_basis: Option<core_domain::entities::UnitBasis>,
    /// Per-incident (D): fixed price per closed ticket.
    pub incident_price_minor: Option<i64>,
    /// Per-incident (D): `any` or `p1`..`p4`.
    pub incident_priority: Option<String>,
    /// Client PO reference printed on produced invoices (F).
    pub po_number: Option<String>,
    pub notes: Option<String>,
    /// T&M rate card (only valid on time_and_materials contracts).
    #[serde(default)]
    pub rates: Vec<RateSpec>,
    /// Fee schedule (only valid on fixed_fee contracts).
    #[serde(default)]
    pub fees: Vec<FeeSpec>,
    /// Prepaid hour banks (only valid on T&M and retainer contracts).
    #[serde(default)]
    pub blocks: Vec<BlockSpec>,
    /// NTE caps (only valid on time_and_materials contracts).
    #[serde(default)]
    pub caps: Vec<CapSpec>,
    /// Minimum commitment per period (E); `None` = none.
    pub minimum_commitment_minor: Option<i64>,
    /// Scheduled escalator (E); `None` = none.
    pub escalator: Option<EscalatorSpec>,
    /// Instant-first autosave parameters (IF1): idempotency key. Absent
    /// in legacy clients — purely additive. (Creates carry no version.)
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PatchContractRequest {
    pub client_id: Option<Uuid>,
    pub number: Option<String>,
    pub name: Option<String>,
    pub contract_type: Option<ContractType>,
    pub status: Option<ContractStatus>,
    pub currency: Option<String>,
    pub effective_from: Option<chrono::NaiveDate>,
    pub effective_until: Option<chrono::NaiveDate>,
    pub billing_frequency: Option<core_domain::entities::BillingFrequency>,
    pub billing_anchor: Option<chrono::NaiveDate>,
    pub amount_minor: Option<i64>,
    pub hourly_rate_minor: Option<i64>,
    pub included_minutes: Option<i32>,
    pub overage_rate_minor: Option<i64>,
    pub unit_price_minor: Option<i64>,
    pub unit_basis: Option<core_domain::entities::UnitBasis>,
    pub incident_price_minor: Option<i64>,
    pub incident_priority: Option<String>,
    pub po_number: Option<String>,
    pub notes: Option<String>,
    /// When present, replaces the whole rate card.
    pub rates: Option<Vec<RateSpec>>,
    /// When present, replaces the whole fee schedule.
    pub fees: Option<Vec<FeeSpec>>,
    /// When present, replaces the whole prepaid-block set.
    pub blocks: Option<Vec<BlockSpec>>,
    /// When present, replaces the whole cap set.
    pub caps: Option<Vec<CapSpec>>,
    /// When present, sets/clears the minimum commitment (`None` clears).
    pub minimum_commitment_minor: Option<i64>,
    /// Explicit clear for the minimum commitment: PATCH cannot
    /// distinguish "absent" from "null" on the flat field, so removing
    /// a minimum sends `clear_minimum: true`.
    #[serde(default)]
    pub clear_minimum: Option<bool>,
    /// When present, sets the escalator; `clear_escalator: true` removes it.
    pub escalator: Option<EscalatorSpec>,
    #[serde(default)]
    pub clear_escalator: Option<bool>,
    /// Instant-first autosave parameters (IF1): idempotency key +
    /// If-Match `version`. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// The contract aggregate as one view: the agreement plus its rate card,
/// fee schedule, prepaid blocks, and caps.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ContractDetail {
    #[serde(flatten)]
    pub contract: Contract,
    pub rates: Vec<ContractRate>,
    pub fees: Vec<ContractFixedFee>,
    pub blocks: Vec<ContractBlock>,
    pub caps: Vec<ContractCap>,
    /// Per-period minimum commitment, minor units (`None` = none).
    pub minimum_commitment_minor: Option<i64>,
    pub escalator: Option<ContractEscalator>,
    /// Lifecycle aggregates ride as counts on the detail (their own
    /// surfaces list them; this keeps the aggregate view honest about
    /// attached commercial state).
    pub change_orders: Vec<ContractChangeOrder>,
    pub sla_credits: Vec<ContractSlaCredit>,
}

/// Instant-first mutation response (IF2): contract aggregate at the top
/// level plus the additive `mutation` metadata.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ContractMutationResponse {
    #[serde(flatten)]
    pub detail: ContractDetail,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct ContractListQuery {
    pub client_id: Option<Uuid>,
    /// Exact status: `draft|active|ended|cancelled`.
    pub status: Option<String>,
    /// Only contracts effective on this date.
    pub effective_on: Option<chrono::NaiveDate>,
}

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct PreviewQuery {
    /// Window start (inclusive), ISO date.
    pub from: Option<chrono::NaiveDate>,
    /// Window end (inclusive), ISO date.
    pub to: Option<chrono::NaiveDate>,
}

/// Org-wide preview: every billable contract's evaluation for the window
/// plus the grand total — the shape a Wave-9 invoice run will consume.
#[derive(Debug, Serialize, ToSchema)]
pub struct OrgBillingPreview {
    pub from: chrono::NaiveDate,
    pub to: chrono::NaiveDate,
    pub evaluations: Vec<BillingEvaluation>,
    /// Sum over evaluations, minor units (mixed currencies reject).
    pub total_minor: i64,
}

// --- helpers -----------------------------------------------------------------

fn rate_rows(org_id: Uuid, contract_id: Uuid, specs: &[RateSpec]) -> ApiResult<Vec<ContractRate>> {
    specs
        .iter()
        .map(|r| {
            if r.work_type.trim().is_empty() {
                return Err(ApiError::Validation("rate work_type is required".into()));
            }
            if r.rate_minor < 0 {
                return Err(ApiError::Validation(
                    "rate_minor must not be negative".into(),
                ));
            }
            Ok(ContractRate {
                id: Uuid::now_v7(),
                org_id,
                contract_id,
                work_type: r.work_type.trim().to_string(),
                rate_minor: r.rate_minor,
                created_at: chrono::Utc::now(),
            })
        })
        .collect()
}

fn fee_rows(
    org_id: Uuid,
    contract_id: Uuid,
    specs: &[FeeSpec],
) -> ApiResult<Vec<ContractFixedFee>> {
    specs
        .iter()
        .map(|f| {
            if f.label.trim().is_empty() {
                return Err(ApiError::Validation("fee label is required".into()));
            }
            if f.amount_minor < 0 {
                return Err(ApiError::Validation(
                    "fee amount_minor must not be negative".into(),
                ));
            }
            Ok(ContractFixedFee {
                id: Uuid::now_v7(),
                org_id,
                contract_id,
                label: f.label.trim().to_string(),
                amount_minor: f.amount_minor,
                due_on: f.due_on,
                notes: f.notes.clone(),
                created_at: chrono::Utc::now(),
            })
        })
        .collect()
}

fn block_rows(
    org_id: Uuid,
    contract_id: Uuid,
    specs: &[BlockSpec],
) -> ApiResult<Vec<ContractBlock>> {
    specs
        .iter()
        .map(|b| {
            if b.label.trim().is_empty() {
                return Err(ApiError::Validation("block label is required".into()));
            }
            if b.total_minutes <= 0 {
                return Err(ApiError::Validation(
                    "block total_minutes must be positive".into(),
                ));
            }
            if b.sale_price_minor < 0 {
                return Err(ApiError::Validation(
                    "block sale_price_minor must not be negative".into(),
                ));
            }
            if b.rollover_policy != "expire"
                && b.rollover_policy != "rollover"
                && b.rollover_policy != "none"
            {
                return Err(ApiError::Validation(format!(
                    "invalid block rollover_policy: {}",
                    b.rollover_policy
                )));
            }
            if b.rollover_policy == "none" && b.expires_on.is_some() {
                return Err(ApiError::Validation(
                    "a block with rollover_policy=none must not carry an expiry".into(),
                ));
            }
            Ok(ContractBlock {
                id: Uuid::now_v7(),
                org_id,
                contract_id,
                label: b.label.trim().to_string(),
                total_minutes: b.total_minutes,
                sale_price_minor: b.sale_price_minor,
                block_rate_minor: b.block_rate_minor,
                expires_on: b.expires_on,
                rollover_policy: b.rollover_policy.clone(),
                created_at: chrono::Utc::now(),
            })
        })
        .collect()
}

fn cap_rows(org_id: Uuid, contract_id: Uuid, specs: &[CapSpec]) -> ApiResult<Vec<ContractCap>> {
    specs
        .iter()
        .map(|c| {
            if c.scope != "period" && c.scope != "contract" {
                return Err(ApiError::Validation(format!(
                    "invalid cap scope: {}",
                    c.scope
                )));
            }
            if c.cap_minor <= 0 {
                return Err(ApiError::Validation(
                    "cap cap_minor must be positive".into(),
                ));
            }
            if c.overage_rate_minor.is_some_and(|r| r < 0) {
                return Err(ApiError::Validation(
                    "cap overage_rate_minor must not be negative".into(),
                ));
            }
            Ok(ContractCap {
                id: Uuid::now_v7(),
                org_id,
                contract_id,
                scope: c.scope.clone(),
                cap_minor: c.cap_minor,
                reached_behavior: c.reached_behavior,
                overage_rate_minor: c.overage_rate_minor,
                work_type: c.work_type.clone(),
                created_at: chrono::Utc::now(),
            })
        })
        .collect()
}

fn escalator_row(org_id: Uuid, contract_id: Uuid, spec: &EscalatorSpec) -> ApiResult<ContractEscalator> {
    if !(0 < spec.uplift_bps && spec.uplift_bps <= 5_000) {
        return Err(ApiError::Validation(
            "escalator uplift_bps must be between 1 and 5000".into(),
        ));
    }
    Ok(ContractEscalator {
        id: Uuid::now_v7(),
        org_id,
        contract_id,
        uplift_bps: spec.uplift_bps,
        first_uplift_on: spec.first_uplift_on,
        created_at: chrono::Utc::now(),
    })
}

/// Type-strictness the entity validation cannot see: rate cards belong to
/// T&M contracts and fee schedules to fixed-fee contracts. Blocks ride
/// the labor-consumption types (T&M + retainer); caps are T&M-only.
/// Per-unit and per-incident carry their own fields instead. Silent
/// ignoring would hide misconfigured billing from the operator.
fn validate_specs(
    contract_type: ContractType,
    rates: &[ContractRate],
    fees: &[ContractFixedFee],
    blocks: &[ContractBlock],
    caps: &[ContractCap],
) -> ApiResult<()> {
    if contract_type != ContractType::TimeAndMaterials && !rates.is_empty() {
        return Err(ApiError::Validation(
            "rate cards are only valid on time_and_materials contracts".into(),
        ));
    }
    if contract_type != ContractType::FixedFee && !fees.is_empty() {
        return Err(ApiError::Validation(
            "fee schedules are only valid on fixed_fee contracts".into(),
        ));
    }
    let block_bearing = matches!(
        contract_type,
        ContractType::TimeAndMaterials | ContractType::Retainer
    );
    if !block_bearing && !blocks.is_empty() {
        return Err(ApiError::Validation(
            "prepaid blocks are only valid on time_and_materials or retainer contracts".into(),
        ));
    }
    if contract_type != ContractType::TimeAndMaterials && !caps.is_empty() {
        return Err(ApiError::Validation(
            "caps are only valid on time_and_materials contracts".into(),
        ));
    }
    Ok(())
}

fn parse_preview_window(q: &PreviewQuery) -> ApiResult<(chrono::NaiveDate, chrono::NaiveDate)> {
    let from = q
        .from
        .ok_or_else(|| ApiError::Validation("from is required".into()))?;
    let to =
        q.to.ok_or_else(|| ApiError::Validation("to is required".into()))?;
    if to < from {
        return Err(ApiError::Validation("to must not precede from".into()));
    }
    if (to - from).num_days() > MAX_PREVIEW_DAYS {
        return Err(ApiError::Validation(
            "preview window exceeds 366 days; split the range".into(),
        ));
    }
    Ok((from, to))
}

async fn load_detail_from_tx(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> ApiResult<ContractDetail> {
    let contract = core_db::contracts::get_contract(&mut *tx, org_id, id)
        .await
        .map_err(ApiError::from)?;
    let rates = core_db::contracts::list_rates(&mut *tx, org_id, id).await?;
    let fees = core_db::contracts::list_fees(&mut *tx, org_id, id).await?;
    let blocks = core_db::contracts::list_blocks(&mut *tx, org_id, id).await?;
    let caps = core_db::contracts::list_caps(&mut *tx, org_id, id).await?;
    let minimum_commitment_minor = core_db::contracts::get_minimum(&mut *tx, org_id, id).await?;
    let escalator = core_db::contracts::get_escalator(&mut *tx, org_id, id).await?;
    let change_orders = core_db::contracts::list_change_orders(&mut *tx, org_id, id).await?;
    let sla_credits = core_db::contracts::list_sla_credits(&mut *tx, org_id, id).await?;
    Ok(ContractDetail {
        contract,
        rates,
        fees,
        blocks,
        caps,
        minimum_commitment_minor,
        escalator,
        change_orders,
        sla_credits,
    })
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

// --- handlers ----------------------------------------------------------------

/// POST /api/contracts — instant-first (IF1/IF2): idempotency-keyed and
/// returns the read-your-write `mutation` metadata.
#[utoipa::path(operation_id = "contracts_create", post, path = "/api/contracts", tag = "billing", security(("bearer" = [])),
    request_body = CreateContractRequest,
    responses((status = 200, body = ContractMutationResponse),
              (status = 409, description = "duplicate contract number, idempotency key reused, or version conflict"),
              (status = 422, description = "validation failure")))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateContractRequest>,
) -> ApiResult<Json<ContractMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    require_permission(&authed.principal, financial_read())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    // Fail-fast validation before the transaction: a 422 should not need
    // a rollback to be honest.
    let now = chrono::Utc::now();
    let contract = Contract {
        id: Uuid::now_v7(),
        org_id,
        client_id: req.client_id,
        number: req.number.trim().to_string(),
        name: req.name.trim().to_string(),
        contract_type: req.contract_type,
        status: req.status.unwrap_or(ContractStatus::Draft),
        currency: req.currency.trim().to_uppercase(),
        effective_from: req.effective_from,
        effective_until: req.effective_until,
        billing_frequency: req.billing_frequency,
        billing_anchor: req.billing_anchor,
        amount_minor: req.amount_minor,
        hourly_rate_minor: req.hourly_rate_minor,
        included_minutes: req.included_minutes,
        overage_rate_minor: req.overage_rate_minor,
        unit_price_minor: req.unit_price_minor,
        unit_basis: req.unit_basis,
        incident_price_minor: req.incident_price_minor,
        incident_priority: req.incident_priority.clone().map(|p| p.trim().to_lowercase()),
        po_number: req.po_number.clone().map(|p| p.trim().to_string()),
        notes: req.notes.clone(),
        created_at: now,
        updated_at: now,
    };
    contract.validate()?;
    let rates = rate_rows(org_id, contract.id, &req.rates)?;
    let fees = fee_rows(org_id, contract.id, &req.fees)?;
    let blocks = block_rows(org_id, contract.id, &req.blocks)?;
    let caps = cap_rows(org_id, contract.id, &req.caps)?;
    let escalator = match &req.escalator {
        Some(spec) => Some(escalator_row(org_id, contract.id, spec)?),
        None => None,
    };
    validate_specs(contract.contract_type, &rates, &fees, &blocks, &caps)?;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "contract",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // The client must be visible in the caller's scope: under RLS a
        // foreign client id is indistinguishable from a missing one.
        core_db::clients::get_client(&mut *ctx.tx, org_id, contract.client_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        core_db::contracts::create_contract(&mut *ctx.tx, &contract).await?;
        core_db::contracts::replace_rates(&mut ctx.tx, org_id, contract.id, &rates).await?;
        core_db::contracts::replace_fees(&mut ctx.tx, org_id, contract.id, &fees).await?;
        core_db::contracts::replace_blocks(&mut ctx.tx, org_id, contract.id, &blocks).await?;
        core_db::contracts::replace_caps(&mut ctx.tx, org_id, contract.id, &caps).await?;
        core_db::contracts::set_minimum(
            &mut ctx.tx,
            org_id,
            contract.id,
            req.minimum_commitment_minor,
        )
        .await?;
        core_db::contracts::set_escalator(
            &mut ctx.tx,
            org_id,
            contract.id,
            escalator.as_ref(),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "contract",
                entity_id: contract.id,
                version: None,
            },
        )
        .await?;
        ctx.emit(
            types::CONTRACT_CREATED,
            "contract",
            Some(contract.id.to_string().as_str()),
            &serde_json::to_value(ContractCreated {
                contract: contract.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        let detail = ContractDetail {
            contract: contract.clone(),
            rates,
            fees,
            blocks,
            caps,
            minimum_commitment_minor: req.minimum_commitment_minor,
            escalator,
            change_orders: Vec::new(),
            sla_credits: Vec::new(),
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "contract",
                entity_id: contract.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                updated_at: contract.created_at,
                key: key.as_deref(),
                payload: &detail,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ContractMutationResponse { detail, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// GET /api/contracts — list with optional client/status/effective-date
/// filters, ordered by contract number.
#[utoipa::path(operation_id = "contracts_list", get, path = "/api/contracts", tag = "billing", security(("bearer" = [])),
    params(ContractListQuery),
    responses((status = 200, body = [core_domain::entities::Contract])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ContractListQuery>,
) -> ApiResult<Json<Vec<Contract>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    require_permission(&authed.principal, financial_read())?;
    let filters = core_db::contracts::ContractFilters {
        client_id: q.client_id,
        status: q.status,
        effective_on: q.effective_on,
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
    let contracts =
        core_db::contracts::list_contracts(&mut *tx, authed.principal.user.org_id, &filters)
            .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(contracts))
}

/// GET /api/contracts/{id} — the contract aggregate (agreement + rate
/// card + fee schedule).
#[utoipa::path(operation_id = "contracts_get_one", get, path = "/api/contracts/{id}", tag = "billing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ContractDetail)))]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ContractDetail>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    require_permission(&authed.principal, financial_read())?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let detail = load_detail_from_tx(&mut tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(detail))
}

/// PATCH /api/contracts/{id} — instant-first (IF1/IF2) partial update.
/// Absent fields keep their values; `rates`/`fees`, when present,
/// replace the whole card/schedule. The version gate rejects stale
/// writes with a typed 409.
#[utoipa::path(operation_id = "contracts_patch", patch, path = "/api/contracts/{id}", tag = "billing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = PatchContractRequest,
    responses((status = 200, body = ContractMutationResponse),
              (status = 404, description = "contract not visible in scope"),
              (status = 409, description = "version conflict, duplicate number, or idempotency key reused"),
              (status = 422, description = "validation failure")))]
pub async fn patch(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchContractRequest>,
) -> ApiResult<Json<ContractMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    require_permission(&authed.principal, financial_read())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "contract",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::contracts::get_contract(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        // Resolve the patched entity, then validate the WHOLE thing: a
        // partial change can never smuggle the aggregate out of invariant.
        let resolved = Contract {
            client_id: req.client_id.unwrap_or(before.client_id),
            number: req
                .number
                .map(|n| n.trim().to_string())
                .unwrap_or(before.number.clone()),
            name: req
                .name
                .map(|n| n.trim().to_string())
                .unwrap_or(before.name.clone()),
            contract_type: req.contract_type.unwrap_or(before.contract_type),
            status: req.status.unwrap_or(before.status),
            currency: req
                .currency
                .map(|c| c.trim().to_uppercase())
                .unwrap_or(before.currency.clone()),
            effective_from: req.effective_from.unwrap_or(before.effective_from),
            effective_until: req.effective_until.or(before.effective_until),
            billing_frequency: req.billing_frequency.or(before.billing_frequency),
            billing_anchor: req.billing_anchor.or(before.billing_anchor),
            amount_minor: req.amount_minor.or(before.amount_minor),
            hourly_rate_minor: req.hourly_rate_minor.or(before.hourly_rate_minor),
            included_minutes: req.included_minutes.or(before.included_minutes),
            overage_rate_minor: req.overage_rate_minor.or(before.overage_rate_minor),
            unit_price_minor: req.unit_price_minor.or(before.unit_price_minor),
            unit_basis: req.unit_basis.or(before.unit_basis),
            incident_price_minor: req.incident_price_minor.or(before.incident_price_minor),
            incident_priority: req
                .incident_priority
                .map(|p| p.trim().to_lowercase())
                .or_else(|| before.incident_priority.clone()),
            po_number: req
                .po_number
                .map(|p| p.trim().to_string())
                .or_else(|| before.po_number.clone()),
            notes: req.notes.clone().or(before.notes.clone()),
            ..before.clone()
        };
        resolved.validate()?;
        let rates = match &req.rates {
            Some(specs) => rate_rows(org_id, id, specs)?,
            None => core_db::contracts::list_rates(&mut *ctx.tx, org_id, id).await?,
        };
        let fees = match &req.fees {
            Some(specs) => fee_rows(org_id, id, specs)?,
            None => core_db::contracts::list_fees(&mut *ctx.tx, org_id, id).await?,
        };
        let blocks = match &req.blocks {
            Some(specs) => block_rows(org_id, id, specs)?,
            None => core_db::contracts::list_blocks(&mut *ctx.tx, org_id, id).await?,
        };
        let caps = match &req.caps {
            Some(specs) => cap_rows(org_id, id, specs)?,
            None => core_db::contracts::list_caps(&mut *ctx.tx, org_id, id).await?,
        };
        let escalator = match &req.escalator {
            Some(spec) => Some(escalator_row(org_id, id, spec)?),
            None if req.clear_escalator == Some(true) => None,
            None => core_db::contracts::get_escalator(&mut *ctx.tx, org_id, id).await?,
        };
        let prior_minimum = core_db::contracts::get_minimum(&mut *ctx.tx, org_id, id).await?;
        let minimum_commitment_minor = match (prior_minimum, req.clear_minimum) {
            (Some(_), Some(true)) => None,
            (prior, _) => req.minimum_commitment_minor.or(prior),
        };
        validate_specs(resolved.contract_type, &rates, &fees, &blocks, &caps)?;
        if resolved.client_id != before.client_id {
            core_db::clients::get_client(&mut *ctx.tx, org_id, resolved.client_id)
                .await
                .map_err(|e| instant::mutation_scope_err(e.into()))?;
        }

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "contract",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let updated = core_db::contracts::update_contract(&mut *ctx.tx, &resolved).await?;
        core_db::contracts::replace_rates(&mut ctx.tx, org_id, id, &rates).await?;
        core_db::contracts::replace_fees(&mut ctx.tx, org_id, id, &fees).await?;
        core_db::contracts::replace_blocks(&mut ctx.tx, org_id, id, &blocks).await?;
        core_db::contracts::replace_caps(&mut ctx.tx, org_id, id, &caps).await?;
        core_db::contracts::set_minimum(&mut ctx.tx, org_id, id, minimum_commitment_minor).await?;
        core_db::contracts::set_escalator(&mut ctx.tx, org_id, id, escalator.as_ref()).await?;
        ctx.emit(
            types::CONTRACT_UPDATED,
            "contract",
            Some(id.to_string().as_str()),
            &serde_json::to_value(ContractUpdated {
                contract: updated.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        let change_orders = core_db::contracts::list_change_orders(&mut *ctx.tx, org_id, id).await?;
        let sla_credits = core_db::contracts::list_sla_credits(&mut *ctx.tx, org_id, id).await?;
        let detail = ContractDetail {
            contract: updated,
            rates,
            fees,
            blocks,
            caps,
            minimum_commitment_minor,
            escalator,
            change_orders,
            sla_credits,
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "contract",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                updated_at: detail.contract.updated_at,
                key: key.as_deref(),
                payload: &detail,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ContractMutationResponse { detail, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/contracts/{id} — removes the contract aggregate. A typed
/// 404 (`rls_sparse`) means the target was never visible in scope.
/// The Wave 9 invoicing lane's billing-lock guard refuses the delete
/// once invoices reference the contract (typed 409).
#[utoipa::path(operation_id = "contracts_delete", delete, path = "/api/contracts/{id}", tag = "billing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ContractDetail),
              (status = 404, description = "contract not visible in scope"),
              (status = 409, description = "contract has invoices")))]
pub async fn remove(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ContractDetail>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let detail = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let detail = load_detail_from_tx(&mut ctx.tx, org_id, id).await?;
        if core_db::invoices::contract_has_invoices(&mut *ctx.tx, org_id, id).await? {
            return Err(ApiError::Conflict(
                "contract has invoices; void the invoices or keep the contract".into(),
            ));
        }
        core_db::contracts::delete_contract(&mut *ctx.tx, org_id, id).await?;
        ctx.emit(
            types::CONTRACT_REMOVED,
            "contract",
            Some(id.to_string().as_str()),
            &serde_json::to_value(ContractRemoved {
                contract_id: id,
                number: detail.contract.number.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        Ok((ctx, detail))
    })
    .await?;
    Ok(Json(detail))
}

/// Loads every Wave-39 evaluation input for one contract inside an open
/// transaction: blocks + prior draws, caps, minimum, escalator, change
/// orders, pending SLA credits, closed-ticket and unit-count feeds (the
/// per-type data the pure engine expects).
pub(crate) async fn load_eval_inputs(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    contract: &Contract,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
) -> ApiResult<core_billing::EvalInputs<'static>> {
    let blocks = core_db::contracts::list_blocks(&mut *tx, org_id, contract.id).await?;
    let draw_rows = core_db::contracts::list_block_draws(&mut *tx, org_id, contract.id).await?;
    let caps = core_db::contracts::list_caps(&mut *tx, org_id, contract.id).await?;
    let minimum = core_db::contracts::get_minimum(&mut *tx, org_id, contract.id).await?;
    let escalator = core_db::contracts::get_escalator(&mut *tx, org_id, contract.id).await?;
    let orders = core_db::contracts::list_change_orders(&mut *tx, org_id, contract.id).await?;
    let credits = core_db::contracts::list_sla_credits(&mut *tx, org_id, contract.id).await?;
    let tickets = core_db::contracts::list_closed_tickets(
        &mut *tx,
        org_id,
        contract.client_id,
        from,
        to,
        contract
            .incident_priority
            .as_deref()
            .filter(|p| !p.is_empty() && *p != "any"),
    )
    .await?;
    let unit_counts = match contract.contract_type {
        ContractType::PerUnit => Some(core_db::contracts::count_units(
            &mut *tx,
            org_id,
            contract.client_id,
        )
        .await?),
        _ => None,
    };
    // Leak into 'static: the engine borrows these slices for the
    // evaluation call only; the API handler returns the owned
    // evaluation, never the inputs. Bounded per-contract row counts —
    // the same profile the request already holds in memory.
    let prior_draws: std::collections::BTreeMap<Uuid, i64> = draw_rows
        .iter()
        .fold(std::collections::BTreeMap::new(), |mut acc, d| {
            *acc.entry(d.block_id).or_insert(0) += d.minutes as i64;
            acc
        });
    Ok(core_billing::EvalInputs {
        blocks: Box::leak(blocks.into_boxed_slice()),
        prior_draws: Some(Box::leak(prior_draws.into())),
        caps: Box::leak(caps.into_boxed_slice()),
        minimum_commitment_minor: minimum,
        escalator: escalator.map(|e| &*Box::leak(Box::new(e))),
        change_orders: Box::leak(orders.into_boxed_slice()),
        sla_credits: Box::leak(credits.into_boxed_slice()),
        closed_tickets: Box::leak(
            tickets
                .into_iter()
                .map(|t| core_billing::IncidentTicket {
                    ticket_id: t.ticket_id,
                    number: t.number,
                    priority: t.priority,
                    resolved_on: t.resolved_on,
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        ),
        unit_counts: unit_counts
            .map(|u| core_billing::UnitCounts {
                devices: u.devices,
                users: u.users,
            })
            .unwrap_or_default(),
    })
}

/// GET /api/contracts/{id}/billing-preview — evaluate this contract's
/// billing rules over `[from, to]` and return the draft invoice-line
/// candidates. READ-ONLY: nothing persists, no billed markers exist.
#[utoipa::path(operation_id = "contracts_billing_preview", get, path = "/api/contracts/{id}/billing-preview", tag = "billing", security(("bearer" = [])),
    params(("id" = Uuid, Path), PreviewQuery),
    responses((status = 200, body = core_domain::entities::BillingEvaluation),
              (status = 404, description = "contract not found"),
              (status = 422, description = "missing or inverted window")))]
pub async fn preview_contract(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Query(q): Query<PreviewQuery>,
) -> ApiResult<Json<BillingEvaluation>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    require_permission(&authed.principal, financial_read())?;
    let org_id = authed.principal.user.org_id;
    let (from, to) = parse_preview_window(&q)?;

    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let contract = core_db::contracts::get_contract(&mut *tx, org_id, id)
        .await
        .map_err(ApiError::from)?;
    let rates = core_db::contracts::list_rates(&mut *tx, org_id, id).await?;
    let fees = core_db::contracts::list_fees(&mut *tx, org_id, id).await?;
    let eval_inputs = load_eval_inputs(&mut tx, org_id, &contract, from, to).await?;
    let entries =
        core_db::contracts::list_billable_entries(&mut *tx, org_id, contract.client_id, from, to)
            .await?;
    tx.commit().await.map_err(ApiError::from)?;
    let inputs: Vec<core_billing::BillableEntry> = entries
        .into_iter()
        .map(|e| core_billing::BillableEntry {
            time_entry_id: e.time_entry_id,
            minutes: e.minutes,
            work_type: e.work_type,
            entry_date: e.entry_date,
        })
        .collect();
    let evaluation =
        core_billing::rules::evaluate_with(&contract, &rates, &fees, &inputs, from, to, &eval_inputs)
            .map_err(engine_error)?;
    Ok(Json(evaluation))
}

/// GET /api/billing/preview — evaluate EVERY billable contract in the
/// org over `[from, to]` (cancelled contracts excluded; effective windows
/// must intersect). READ-ONLY: the org-wide dry-run of a Wave-9 invoice
/// run.
#[utoipa::path(operation_id = "billing_preview", get, path = "/api/billing/preview", tag = "billing", security(("bearer" = [])),
    params(PreviewQuery),
    responses((status = 200, body = OrgBillingPreview),
              (status = 422, description = "missing or inverted window")))]
pub async fn preview_org(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<PreviewQuery>,
) -> ApiResult<Json<OrgBillingPreview>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    require_permission(&authed.principal, financial_read())?;
    let org_id = authed.principal.user.org_id;
    let (from, to) = parse_preview_window(&q)?;

    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let contracts = core_db::contracts::list_billable_contracts(&mut *tx, org_id, from, to)
        .await
        .map_err(ApiError::from)?;
    // Entries are per client; contracts usually cluster on few clients, so
    // fetch once per client and share.
    let mut entry_cache: std::collections::HashMap<Uuid, Vec<core_billing::BillableEntry>> =
        std::collections::HashMap::new();
    let mut evaluations = Vec::with_capacity(contracts.len());
    for contract in &contracts {
        let inputs = match entry_cache.get(&contract.client_id) {
            Some(cached) => cached.clone(),
            None => {
                let rows = core_db::contracts::list_billable_entries(
                    &mut *tx,
                    org_id,
                    contract.client_id,
                    from,
                    to,
                )
                .await?;
                let inputs: Vec<core_billing::BillableEntry> = rows
                    .into_iter()
                    .map(|e| core_billing::BillableEntry {
                        time_entry_id: e.time_entry_id,
                        minutes: e.minutes,
                        work_type: e.work_type,
                        entry_date: e.entry_date,
                    })
                    .collect();
                entry_cache.insert(contract.client_id, inputs.clone());
                inputs
            }
        };
        let rates = core_db::contracts::list_rates(&mut *tx, org_id, contract.id).await?;
        let fees = core_db::contracts::list_fees(&mut *tx, org_id, contract.id).await?;
        let eval_inputs = load_eval_inputs(&mut tx, org_id, contract, from, to).await?;
        let evaluation =
            core_billing::rules::evaluate_with(contract, &rates, &fees, &inputs, from, to, &eval_inputs)
                .map_err(engine_error)?;
        evaluations.push(evaluation);
    }
    tx.commit().await.map_err(ApiError::from)?;
    // Mixed-currency totals are meaningless; every org contract shares a
    // currency in practice, and a violation is a typed conflict rather
    // than a silently wrong number.
    let mut currencies: Vec<&str> = evaluations.iter().map(|e| e.currency.as_str()).collect();
    currencies.sort_unstable();
    currencies.dedup();
    let total_minor = if currencies.len() <= 1 {
        evaluations.iter().try_fold(0_i64, |acc, e| {
            acc.checked_add(e.total_minor)
                .ok_or_else(|| ApiError::Conflict("billing totals overflow".into()))
        })?
    } else {
        return Err(ApiError::Conflict(
            "org-wide preview mixes currencies; total withheld".into(),
        ));
    };
    Ok(Json(OrgBillingPreview {
        from,
        to,
        evaluations,
        total_minor,
    }))
}

// --- Wave 39: change orders + SLA credits (lifecycle surfaces) ---------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateChangeOrderRequest {
    /// Org-unique per contract ("CO-2026-004").
    pub number: String,
    pub title: String,
    pub description: Option<String>,
    pub amount_minor: i64,
    /// Instant-first autosave parameters (IF1). (Creates carry no version.)
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct TransitionChangeOrderRequest {
    /// `approve|reject|cancel`. `billed` is run-owned only (never a
    /// manual write — the invoice run flips it with the invoice link).
    pub action: String,
    /// Instant-first autosave parameters (IF1): idempotency key +
    /// If-Match `version`.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// POST /api/contracts/{id}/change-orders — create a DRAFT order. The
/// lifecycle forward-path (approve → billed at run time) keeps billing
/// honest: only APPROVED orders ever price, and the run itself records
/// which invoice consumed them.
#[utoipa::path(operation_id = "contracts_change_orders_create", post, path = "/api/contracts/{id}/change-orders", tag = "billing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CreateChangeOrderRequest,
    responses((status = 200, body = ContractChangeOrder),
              (status = 404, description = "contract not visible in scope"),
              (status = 409, description = "duplicate change order number or idempotency key reused"),
              (status = 422, description = "validation failure")))]
pub async fn create_change_order(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateChangeOrderRequest>,
) -> ApiResult<Json<ContractChangeOrder>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    require_permission(&authed.principal, financial_read())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    if req.number.trim().is_empty() {
        return Err(ApiError::Validation("number is required".into()));
    }
    if req.title.trim().is_empty() {
        return Err(ApiError::Validation("title is required".into()));
    }
    if req.amount_minor < 0 {
        return Err(ApiError::Validation(
            "amount_minor must not be negative".into(),
        ));
    }
    let order = ContractChangeOrder {
        id: Uuid::now_v7(),
        org_id,
        contract_id: id,
        number: req.number.trim().to_string(),
        title: req.title.trim().to_string(),
        description: req.description.clone(),
        amount_minor: req.amount_minor,
        status: "draft".into(),
        approved_by: None,
        approved_at: None,
        billed_invoice_id: None,
        created_by: Some(user_id),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let built = order.clone();
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::contracts::get_contract(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        core_db::contracts::insert_change_order(&mut *ctx.tx, &order).await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(built))
}

/// POST /api/contracts/{id}/change-orders/{order_id}/transition — move a
/// draft order through its lifecycle. `approve` stamps the approver +
/// time (the audit trail's who/when); `reject`/`cancel` are terminal.
#[utoipa::path(operation_id = "contracts_change_orders_transition", post, path = "/api/contracts/{id}/change-orders/{order_id}/transition", tag = "billing", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("order_id" = Uuid, Path)),
    request_body = TransitionChangeOrderRequest,
    responses((status = 200, body = ContractChangeOrder),
              (status = 404, description = "order not visible in scope"),
              (status = 409, description = "transition not allowed from the current status"),
              (status = 422, description = "validation failure")))]
pub async fn transition_change_order(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, order_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<TransitionChangeOrderRequest>,
) -> ApiResult<Json<ContractChangeOrder>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    require_permission(&authed.principal, financial_read())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let (status, approved_by, approved_at, billed_invoice_id) = match req.action.as_str() {
        "approve" => ("approved", Some(user_id), Some(chrono::Utc::now()), None),
        "reject" => ("rejected", None, None, None),
        "cancel" => ("cancelled", None, None, None),
        other => {
            return Err(ApiError::Validation(format!(
                "invalid transition action: {other} (approve|reject|cancel; billed is run-owned)"
            )))
        }
    };
    let updated = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let existing = core_db::contracts::list_change_orders(&mut *ctx.tx, org_id, id)
            .await?
            .into_iter()
            .find(|o| o.id == order_id)
            .ok_or_else(|| instant::mutation_scope_err(core_db::DbError::NotFound.into()))?;
        // Typed lifecycle guard: transitions only from the legal
        // predecessor statuses (billed is terminal, run-owned).
        let allowed = match req.action.as_str() {
            "approve" | "reject" => existing.status == "draft",
            "cancel" => matches!(existing.status.as_str(), "draft" | "approved"),
            _ => false,
        };
        if !allowed {
            return Err(ApiError::Conflict(format!(
                "cannot {} a {} change order",
                req.action, existing.status
            )));
        }
        core_db::contracts::update_change_order_status(
            &mut *ctx.tx,
            org_id,
            id,
            order_id,
            status,
            approved_by,
            approved_at,
            billed_invoice_id,
        )
        .await?;
        let updated = core_db::contracts::list_change_orders(&mut *ctx.tx, org_id, id)
            .await?
            .into_iter()
            .find(|o| o.id == order_id)
            .expect("just updated row re-reads");
        Ok((ctx, updated))
    })
    .await?;
    Ok(Json(updated))
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateSlaCreditRequest {
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    /// What was missed — printed on the credit line verbatim ("resolution
    /// target breached by 2d 4h"). Required: a silent credit is exactly
    /// the failure mode the ledger refuses.
    pub reason: String,
    pub amount_minor: i64,
    pub currency: String,
    pub applies_on: chrono::NaiveDate,
    /// Instant-first autosave parameters (IF1). (Creates carry no version.)
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// POST /api/contracts/{id}/sla-credits — record a pending service
/// credit. NEGATIVE at billing: the next run whose window covers
/// `applies_on` prices it as the invoice's `sla_credit` line.
#[utoipa::path(operation_id = "contracts_sla_credits_create", post, path = "/api/contracts/{id}/sla-credits", tag = "billing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CreateSlaCreditRequest,
    responses((status = 200, body = ContractSlaCredit),
              (status = 404, description = "contract not visible in scope"),
              (status = 422, description = "validation failure")))]
pub async fn create_sla_credit(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateSlaCreditRequest>,
) -> ApiResult<Json<ContractSlaCredit>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    require_permission(&authed.principal, financial_read())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    if req.reason.trim().is_empty() {
        return Err(ApiError::Validation("reason is required".into()));
    }
    if req.amount_minor <= 0 {
        return Err(ApiError::Validation(
            "amount_minor must be positive (direction is the line kind's meaning)".into(),
        ));
    }
    core_domain::money::CurrencyCode::new(&req.currency)?;
    let credit = ContractSlaCredit {
        id: Uuid::now_v7(),
        org_id,
        contract_id: id,
        ticket_id: req.ticket_id,
        reason: req.reason.trim().to_string(),
        amount_minor: req.amount_minor,
        currency: req.currency.trim().to_uppercase(),
        applies_on: req.applies_on,
        status: "pending".into(),
        applied_invoice_id: None,
        created_by: Some(user_id),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let built = credit.clone();
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::contracts::get_contract(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        core_db::contracts::insert_sla_credit(&mut *ctx.tx, &credit).await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(built))
}

/// POST /api/contracts/{id}/sla-credits/{credit_id}/cancel — withdraw a
/// pending credit before any run consumes it. Applied credits are
/// immutable (additive-ledger rule: corrections issue NEW credits).
#[utoipa::path(operation_id = "contracts_sla_credits_cancel", post, path = "/api/contracts/{id}/sla-credits/{credit_id}/cancel", tag = "billing", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("credit_id" = Uuid, Path)),
    responses((status = 200, body = ContractSlaCredit),
              (status = 404, description = "credit not visible in scope"),
              (status = 409, description = "credit already applied")))]
pub async fn cancel_sla_credit(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, credit_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ContractSlaCredit>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    require_permission(&authed.principal, financial_read())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let cancelled = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let existing = core_db::contracts::list_sla_credits(&mut *ctx.tx, org_id, id)
            .await?
            .into_iter()
            .find(|c| c.id == credit_id)
            .ok_or_else(|| instant::mutation_scope_err(core_db::DbError::NotFound.into()))?;
        if existing.status == "applied" {
            return Err(ApiError::Conflict(
                "applied credits are immutable; issue a new credit to correct".into(),
            ));
        }
        if existing.status == "cancelled" {
            return Ok((ctx, existing));
        }
        core_db::contracts::cancel_sla_credit(&mut ctx.tx, org_id, credit_id).await?;
        let updated = core_db::contracts::list_sla_credits(&mut *ctx.tx, org_id, id)
            .await?
            .into_iter()
            .find(|c| c.id == credit_id)
            .expect("just updated row re-reads");
        Ok((ctx, updated))
    })
    .await?;
    Ok(Json(cancelled))
}
