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
    BillingEvaluation, Contract, ContractFixedFee, ContractRate, ContractStatus, ContractType,
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
    pub notes: Option<String>,
    /// T&M rate card (only valid on time_and_materials contracts).
    #[serde(default)]
    pub rates: Vec<RateSpec>,
    /// Fee schedule (only valid on fixed_fee contracts).
    #[serde(default)]
    pub fees: Vec<FeeSpec>,
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
    pub notes: Option<String>,
    /// When present, replaces the whole rate card.
    pub rates: Option<Vec<RateSpec>>,
    /// When present, replaces the whole fee schedule.
    pub fees: Option<Vec<FeeSpec>>,
    /// Instant-first autosave parameters (IF1): idempotency key +
    /// If-Match `version`. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// The contract aggregate as one view: the agreement plus its rate card
/// and fee schedule.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ContractDetail {
    #[serde(flatten)]
    pub contract: Contract,
    pub rates: Vec<ContractRate>,
    pub fees: Vec<ContractFixedFee>,
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

/// Type-strictness the entity validation cannot see: rate cards belong to
/// T&M contracts and fee schedules to fixed-fee contracts. Silent ignoring
/// would hide misconfigured billing from the operator.
fn validate_specs(
    contract_type: ContractType,
    rates: &[ContractRate],
    fees: &[ContractFixedFee],
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

async fn load_detail_from_pool(
    pool: &sqlx::PgPool,
    org_id: Uuid,
    id: Uuid,
) -> ApiResult<ContractDetail> {
    let contract = core_db::contracts::get_contract(pool, org_id, id)
        .await
        .map_err(ApiError::from)?;
    let rates = core_db::contracts::list_rates(pool, org_id, id).await?;
    let fees = core_db::contracts::list_fees(pool, org_id, id).await?;
    Ok(ContractDetail {
        contract,
        rates,
        fees,
    })
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
    Ok(ContractDetail {
        contract,
        rates,
        fees,
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
        notes: req.notes.clone(),
        created_at: now,
        updated_at: now,
    };
    contract.validate()?;
    let rates = rate_rows(org_id, contract.id, &req.rates)?;
    let fees = fee_rows(org_id, contract.id, &req.fees)?;
    validate_specs(contract.contract_type, &rates, &fees)?;

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
    let filters = core_db::contracts::ContractFilters {
        client_id: q.client_id,
        status: q.status,
        effective_on: q.effective_on,
    };
    let contracts =
        core_db::contracts::list_contracts(&state.pool, authed.principal.user.org_id, &filters)
            .await?;
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
    let detail = load_detail_from_pool(&state.pool, authed.principal.user.org_id, id).await?;
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
        validate_specs(resolved.contract_type, &rates, &fees)?;
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
        let detail = ContractDetail {
            contract: updated,
            rates,
            fees,
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
    let org_id = authed.principal.user.org_id;
    let (from, to) = parse_preview_window(&q)?;

    let contract = core_db::contracts::get_contract(&state.pool, org_id, id)
        .await
        .map_err(ApiError::from)?;
    let rates = core_db::contracts::list_rates(&state.pool, org_id, id).await?;
    let fees = core_db::contracts::list_fees(&state.pool, org_id, id).await?;
    let entries = core_db::contracts::list_billable_entries(
        &state.pool,
        org_id,
        contract.client_id,
        from,
        to,
    )
    .await?;
    let inputs: Vec<core_billing::BillableEntry> = entries
        .into_iter()
        .map(|e| core_billing::BillableEntry {
            time_entry_id: e.time_entry_id,
            minutes: e.minutes,
            work_type: e.work_type,
            entry_date: e.entry_date,
        })
        .collect();
    let evaluation = core_billing::rules::evaluate(&contract, &rates, &fees, &inputs, from, to)
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
    let org_id = authed.principal.user.org_id;
    let (from, to) = parse_preview_window(&q)?;

    let contracts = core_db::contracts::list_billable_contracts(&state.pool, org_id, from, to)
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
                    &state.pool,
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
        let rates = core_db::contracts::list_rates(&state.pool, org_id, contract.id).await?;
        let fees = core_db::contracts::list_fees(&state.pool, org_id, contract.id).await?;
        let evaluation = core_billing::rules::evaluate(contract, &rates, &fees, &inputs, from, to)
            .map_err(engine_error)?;
        evaluations.push(evaluation);
    }
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
