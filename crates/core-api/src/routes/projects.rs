//! Projects endpoints (the private build tracker Phase 3 projects line): projects → phases
//! → tasks, task dependencies, saved task-list views, and reusable
//! templates — plus the milestone billing seam.
//!
//! Contract notes:
//! * Typed errors only: every failure returns the central IF3 envelope
//!   with machine-readable reason codes (`error.rs`); completion blocked
//!   by unfinished dependencies is `task_dependencies_unsatisfied` (409).
//! * Instant-first (IF1/IF2): project/phase/task mutations carry the
//!   flattened `MutationParams` (idempotency key + If-Match version) and
//!   return the read-your-write `mutation` metadata. The idempotency gate
//!   doubles as the billing replay guard — a retried milestone completion
//!   can never double-emit `project.milestone_completed`.
//! * Milestone billing seam: completing a milestone phase/task emits
//!   `project.milestone_completed` through the transactional outbox in the
//!   SAME transaction as the completion write. core-billing (Wave 9)
//!   consumes the envelope; nothing here imports billing internals.
//! * Views/templates are configuration CRUD (webhook-admin pattern): plain
//!   typed JSON, outbox change events, no autosave gating.
//!
//! Non-purpose: no invoicing rows or payment state (core-billing), no
//! labor-actual rollups (reporting track read models), no Gantt scheduling.

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
    BillingMethod, PhaseStatus, Project, ProjectPhase, ProjectStatus, ProjectTask, ProjectTemplate,
    ProjectView, TaskStatus,
};
use core_domain::events::types;
use core_domain::money::CurrencyCode;
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

fn projects(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Projects,
        action,
    }
}

// --- shared wire helpers ------------------------------------------------------

/// Instant-first mutation response for the project entity (IF2): applied
/// snapshot at the top level plus the additive `mutation` metadata.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProjectMutationResponse {
    #[serde(flatten)]
    pub project: Project,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PhaseMutationResponse {
    #[serde(flatten)]
    pub phase: ProjectPhase,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TaskMutationResponse {
    #[serde(flatten)]
    pub task: ProjectTask,
    pub mutation: instant::MutationMeta,
}

/// A task's direct finish-to-start dependencies (full rows, so the client
/// renders "blocked by" without extra fetches).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DependencyListResponse {
    #[schema(value_type = uuid::Uuid)]
    pub task_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub project_id: Uuid,
    pub depends_on: Vec<ProjectTask>,
}

fn valid_currency(code: &str) -> bool {
    CurrencyCode::new(code).is_ok()
}

/// Validates the project→contract billing link (Wave 39 lane 4, report
/// P2): the contract must be visible in this org AND share the
/// project's client — a project bills its client's agreement, never a
/// sibling client's. Cancelled contracts refuse as a link target.
async fn validate_billing_link(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    project_client_id: Option<Uuid>,
    contract_id: Uuid,
) -> ApiResult<()> {
    let contract = core_db::contracts::get_contract(tx, org_id, contract_id)
        .await
        .map_err(|e| match e {
            core_db::DbError::NotFound => {
                ApiError::Validation("billing_contract_id: contract not found in org".into())
            }
            other => other.into(),
        })?;
    if contract.status == core_domain::entities::ContractStatus::Cancelled {
        return Err(ApiError::Validation(
            "billing_contract_id: cancelled contracts cannot bill projects".into(),
        ));
    }
    if project_client_id != Some(contract.client_id) {
        return Err(ApiError::Validation(
            "billing_contract_id: contract must belong to the project's client".into(),
        ));
    }
    Ok(())
}

/// An amount column requires its currency (the project's currency is a
/// valid fallback), and a lone currency with no amount is a typo — both
/// 422 before any write.
/// A phase window is valid when it is absent, or when BOTH ends are present
/// and the end does not precede the start.
///
/// One end on its own is rejected rather than defaulted. A phase with a
/// start and no end has no length, and every Gantt rendering has to invent
/// one — "start + 1 day", a zero-width bar, or a bar running to infinity.
/// All three assert a duration nobody planned, which is the exact dishonesty
/// the nullable columns exist to prevent.
fn validate_phase_dates(
    start: Option<chrono::NaiveDate>,
    end: Option<chrono::NaiveDate>,
) -> ApiResult<()> {
    match (start, end) {
        (None, None) => Ok(()),
        (Some(s), Some(e)) if e >= s => Ok(()),
        (Some(s), Some(e)) => Err(ApiError::Validation(format!(
            "end_date {e} must not precede start_date {s}"
        ))),
        (s, e) => Err(ApiError::Validation(format!(
            "a phase needs both start_date and end_date to be dated (got start={s:?}, end={e:?})"
        ))),
    }
}

fn validate_money_pair(minor: Option<i64>, currency: Option<&str>, label: &str) -> ApiResult<()> {
    match (minor, currency) {
        (Some(_), Some(code)) => {
            if !valid_currency(code) {
                return Err(ApiError::Validation(format!(
                    "invalid ISO 4217 currency code for {label}: {code}"
                )));
            }
            Ok(())
        }
        (Some(_), None) => Err(ApiError::Validation(format!(
            "{label} amount requires {label}_currency"
        ))),
        (None, Some(_)) => Err(ApiError::Validation(format!(
            "{label}_currency set without {label} amount"
        ))),
        (None, None) => Ok(()),
    }
}

/// Validates the milestone amount/currency pair against the entity's
/// currency and the project-level fallback. Returns the effective
/// (amount, currency) after fallback resolution.
fn resolve_amount(
    amount: Option<i64>,
    currency: Option<&str>,
    fallback: Option<&str>,
) -> ApiResult<Option<(i64, String)>> {
    match amount {
        Some(minor) => {
            let code = currency.or(fallback).ok_or_else(|| {
                ApiError::Validation(
                    "milestone amount requires a currency (explicit or the project's)".into(),
                )
            })?;
            if !valid_currency(code) {
                return Err(ApiError::Validation(format!(
                    "invalid ISO 4217 currency code: {code}"
                )));
            }
            Ok(Some((minor, code.to_string())))
        }
        None => {
            if currency.is_some() {
                return Err(ApiError::Validation(
                    "milestone_currency set without milestone_amount_minor".into(),
                ));
            }
            Ok(None)
        }
    }
}

/// The milestone billing seam payload (`project.milestone_completed`).
/// Emitted only for entities flagged `is_milestone`, in the completion
/// transaction. `amount_minor`/`currency` come pre-resolved by
/// [`resolve_amount`]; `None` = informational milestone.
#[allow(clippy::too_many_arguments)]
async fn emit_milestone_completed(
    ctx: &mut org_tx::OrgTx<'_>,
    project: &Project,
    name: &str,
    phase_id: Option<Uuid>,
    task_id: Option<Uuid>,
    amount: Option<(i64, String)>,
    at: chrono::DateTime<chrono::Utc>,
) -> ApiResult<()> {
    ctx.emit(
        types::PROJECT_MILESTONE_COMPLETED,
        "project",
        Some(project.id.to_string().as_str()),
        &serde_json::to_value(core_domain::events::ProjectMilestoneCompleted {
            org_id: project.org_id,
            project_id: project.id,
            phase_id,
            task_id,
            name: name.to_string(),
            amount: amount.map(|(minor, code)| {
                CurrencyCode::new(&code)
                    .map(|c| core_domain::money::Money::new(minor, c))
                    .unwrap_or_else(|_| {
                        core_domain::money::Money::new(
                            minor,
                            CurrencyCode::new("USD").expect("USD"),
                        )
                    })
            }),
            billing_method: project.billing_method.to_string(),
            completed_at: at,
        })
        .unwrap_or_default(),
    )
    .await
}

// --- projects -------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateProjectRequest {
    pub name: String,
    pub description: Option<String>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub client_id: Option<Uuid>,
    #[serde(default)]
    pub billing_method: BillingMethod,
    pub budget_minor: Option<i64>,
    pub currency: Option<String>,
    pub start_date: Option<chrono::NaiveDate>,
    pub due_date: Option<chrono::NaiveDate>,
    /// The contract this project bills through (Wave 39 lane 4, report
    /// P2). Its rate card/blocks/caps price the project's T&M entries and
    /// its runs carry the project's released milestones. Validated to
    /// belong to the same client.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub billing_contract_id: Option<Uuid>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PatchProjectRequest {
    pub name: Option<String>,
    /// `Some(Some(v))` sets; `Some(None)` clears; `None` = no change.
    pub description: Option<Option<String>>,
    pub client_id: Option<Option<Uuid>>,
    pub status: Option<ProjectStatus>,
    pub billing_method: Option<BillingMethod>,
    pub budget_minor: Option<Option<i64>>,
    pub currency: Option<Option<String>>,
    pub start_date: Option<Option<chrono::NaiveDate>>,
    pub due_date: Option<Option<chrono::NaiveDate>>,
    /// `Some(Some(v))` links the contract; `Some(None)` unlinks;
    /// `None` = no change (Wave 39 lane 4, report P2).
    pub billing_contract_id: Option<Option<Uuid>>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct ProjectListQuery {
    pub status: Option<ProjectStatus>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub client_id: Option<Uuid>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

/// GET /api/projects — filtered, newest-first list.
#[utoipa::path(operation_id = "projects_list", get, path = "/api/projects", tag = "projects", security(("bearer" = [])),
    params(ProjectListQuery),
    responses((status = 200, body = [Project])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ProjectListQuery>,
) -> ApiResult<Json<Vec<Project>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let rows = core_db::projects::list_projects(
        &mut *tx,
        authed.principal.user.org_id,
        core_db::projects::ProjectFilter {
            status: q.status,
            client_id: q.client_id,
            limit: q.limit,
            offset: q.offset,
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// POST /api/projects — creates the project and emits `project.created` in
/// one transaction. Instant-first (IF1/IF2): idempotency-keyed,
/// version-gated, read-your-write response.
#[utoipa::path(operation_id = "projects_create", post, path = "/api/projects", tag = "projects", security(("bearer" = [])),
    request_body = CreateProjectRequest,
    responses((status = 200, body = ProjectMutationResponse),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "validation failed")))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateProjectRequest>,
) -> ApiResult<Json<ProjectMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Create))?;
    if req.name.trim().is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    if let Some(code) = req.currency.as_deref() {
        if !valid_currency(code) {
            return Err(ApiError::Validation(format!(
                "invalid ISO 4217 currency code: {code}"
            )));
        }
    }
    if req.budget_minor.is_some() && req.currency.is_none() {
        return Err(ApiError::Validation(
            "budget_minor requires currency".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "project",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        if let Some(contract_id) = req.billing_contract_id {
            validate_billing_link(&mut ctx.tx, org_id, req.client_id, contract_id).await?;
        }
        let project = core_db::projects::insert_project(
            &mut *ctx.tx,
            core_db::projects::NewProject {
                id: Uuid::now_v7(),
                org_id,
                client_id: req.client_id,
                name: req.name.trim(),
                description: req.description.as_deref(),
                billing_method: req.billing_method,
                budget_minor: req.budget_minor,
                currency: req.currency.as_deref(),
                start_date: req.start_date,
                due_date: req.due_date,
                billing_contract_id: req.billing_contract_id,
            },
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "project",
                entity_id: project.id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_CREATED,
            "project",
            Some(project.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectCreated {
                project: project.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "project",
                entity_id: project.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&project).unwrap_or(Value::Null),
                updated_at: project.updated_at,
                key: key.as_deref(),
                payload: &project,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ProjectMutationResponse { project, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// GET /api/projects/{id}
#[utoipa::path(operation_id = "projects_get_one", get, path = "/api/projects/{id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Project),
              (status = 404, description = "not found")))]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Project>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let project =
        core_db::projects::get_project(&mut *tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(project))
}

/// PATCH /api/projects/{id} — tri-state partial update. Instant-first
/// (IF1/IF2): idempotency-keyed, version-gated, returns prior + new values.
#[utoipa::path(operation_id = "projects_patch", patch, path = "/api/projects/{id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = PatchProjectRequest,
    responses((status = 200, body = ProjectMutationResponse),
              (status = 404, description = "project not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "validation failed")))]
pub async fn patch(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchProjectRequest>,
) -> ApiResult<Json<ProjectMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    if let Some(name) = req.name.as_deref() {
        if name.trim().is_empty() {
            return Err(ApiError::Validation("name must not be empty".into()));
        }
    }
    if let Some(Some(code)) = req.currency.as_ref().map(|o| o.as_deref()) {
        if !valid_currency(code) {
            return Err(ApiError::Validation(format!(
                "invalid ISO 4217 currency code: {code}"
            )));
        }
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "project",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::projects::get_project(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        // Effective budget/currency pairing after this patch lands.
        let eff_budget = req.budget_minor.unwrap_or(before.budget_minor);
        let eff_currency: Option<String> = req
            .currency
            .clone()
            .unwrap_or_else(|| before.currency.clone());
        if eff_budget.is_some() && eff_currency.is_none() {
            return Err(ApiError::Validation(
                "budget_minor requires currency".into(),
            ));
        }
        // The link lands against the EFFECTIVE client (a patch may move
        // both together), so validate the pair after applying the patch's
        // client election.
        let eff_client = req.client_id.flatten().or(before.client_id);
        if let Some(contract_id) = req.billing_contract_id.flatten() {
            validate_billing_link(&mut ctx.tx, org_id, eff_client, contract_id).await?;
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "project",
                entity_id: id,
                version,
            },
        )
        .await?;
        let after = core_db::projects::update_project(
            &mut *ctx.tx,
            org_id,
            id,
            core_db::projects::ProjectPatch {
                name: req.name.as_deref().map(str::trim),
                description: req.description.as_ref().map(|o| o.as_deref()),
                client_id: req.client_id,
                status: req.status,
                billing_method: req.billing_method,
                budget_minor: req.budget_minor,
                currency: req.currency.as_ref().map(|o| o.as_deref()),
                start_date: req.start_date,
                due_date: req.due_date,
                billing_contract_id: req.billing_contract_id,
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_UPDATED,
            "project",
            Some(after.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectUpdated {
                project_id: after.id,
            })
            .unwrap_or_default(),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "project",
                entity_id: id,
                op: "update",
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
            MutationOutcome::Applied(ProjectMutationResponse {
                project: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- phases ------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreatePhaseRequest {
    pub name: String,
    pub description: Option<String>,
    /// The planned window (Wave 34 claims close-out, claim 6). Both ends
    /// optional: a phase created without dates lands in the ordered list,
    /// NOT on the Gantt, and the UI says which it is. Supplying only one
    /// end is rejected at the database CHECK, because a phase with a start
    /// and no length has no bar to draw.
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub sort_order: i32,
    #[serde(default)]
    pub is_milestone: bool,
    pub milestone_amount_minor: Option<i64>,
    pub milestone_currency: Option<String>,
    /// Wave 39 lane 4 (report C/D): the phase's billing budget and/or the
    /// fixed fee a COMPLETED non-milestone phase releases. Milestone
    /// phases keep using `milestone_amount_*`.
    #[serde(default)]
    pub budget_minor: Option<i64>,
    #[serde(default)]
    pub budget_currency: Option<String>,
    #[serde(default)]
    pub fee_minor: Option<i64>,
    #[serde(default)]
    pub fee_currency: Option<String>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PatchPhaseRequest {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    /// Tri-state, matching the budget fields below: `Some(Some(d))` sets a
    /// date, `Some(None)` takes the phase off the Gantt, and an absent field
    /// leaves it alone. Without the distinction, renaming a phase would
    /// quietly erase its planned window.
    pub start_date: Option<Option<chrono::NaiveDate>>,
    pub end_date: Option<Option<chrono::NaiveDate>>,
    pub sort_order: Option<i32>,
    pub status: Option<PhaseStatus>,
    pub is_milestone: Option<bool>,
    pub milestone_amount_minor: Option<Option<i64>>,
    pub milestone_currency: Option<Option<String>>,
    /// Wave 39 lane 4: tri-state budget/fee updates on the phase's
    /// billing record (`Some(Some(v))` sets, `Some(None)` clears,
    /// `None` = no change; `clear_*` drops the whole record when both
    /// fields would be empty).
    pub budget_minor: Option<Option<i64>>,
    pub budget_currency: Option<Option<String>>,
    pub fee_minor: Option<Option<i64>>,
    pub fee_currency: Option<Option<String>>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// GET /api/projects/{id}/phases — ordered by sort_order.
#[utoipa::path(operation_id = "projects_list_phases", get, path = "/api/projects/{id}/phases", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [ProjectPhase])))]
pub async fn list_phases(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<ProjectPhase>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let rows = core_db::projects::list_phases(&mut *tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// POST /api/projects/{id}/phases — adds a phase; emits
/// `project.phase_created`.
#[utoipa::path(operation_id = "projects_create_phase", post, path = "/api/projects/{id}/phases", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = CreatePhaseRequest,
    responses((status = 200, body = PhaseMutationResponse),
              (status = 404, description = "project not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "validation failed")))]
pub async fn create_phase(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreatePhaseRequest>,
) -> ApiResult<Json<PhaseMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Create))?;
    if req.name.trim().is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "project_phase",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // Parent must be visible (mutation target's parent left scope ⇒
        // rls_sparse per IF3).
        let project = core_db::projects::get_project(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let amount = resolve_amount(
            req.milestone_amount_minor,
            req.milestone_currency.as_deref(),
            project.currency.as_deref(),
        )?;
        validate_money_pair(req.budget_minor, req.budget_currency.as_deref(), "budget")?;
        validate_money_pair(req.fee_minor, req.fee_currency.as_deref(), "fee")?;
        validate_phase_dates(req.start_date, req.end_date)?;
        let phase = core_db::projects::insert_phase(
            &mut *ctx.tx,
            core_db::projects::NewPhase {
                id: Uuid::now_v7(),
                org_id,
                project_id: id,
                name: req.name.trim(),
                description: req.description.as_deref(),
                sort_order: req.sort_order,
                is_milestone: req.is_milestone,
                milestone_amount_minor: amount.as_ref().map(|(m, _)| *m),
                milestone_currency: amount.as_ref().map(|(_, c)| c.as_str()),
                start_date: req.start_date,
                end_date: req.end_date,
            },
        )
        .await?;
        if req.budget_minor.is_some() || req.fee_minor.is_some() {
            core_db::projects::upsert_phase_budget(
                &mut *ctx.tx,
                org_id,
                phase.id,
                core_db::projects::NewPhaseBudget {
                    budget_minor: req.budget_minor,
                    currency: req.budget_currency.as_deref(),
                    fee_minor: req.fee_minor,
                    fee_currency: req.fee_currency.as_deref(),
                },
            )
            .await?;
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "project_phase",
                entity_id: phase.id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_PHASE_CREATED,
            "project_phase",
            Some(phase.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectPhaseCreated {
                phase: phase.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "project_phase",
                entity_id: phase.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&phase).unwrap_or(Value::Null),
                updated_at: phase.updated_at,
                key: key.as_deref(),
                payload: &phase,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(PhaseMutationResponse { phase, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/projects/{id}/phases/{phase_id} — tri-state partial update.
#[utoipa::path(operation_id = "projects_patch_phase", patch, path = "/api/projects/{id}/phases/{phase_id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("phase_id" = Uuid, Path)), request_body = PatchPhaseRequest,
    responses((status = 200, body = PhaseMutationResponse),
              (status = 404, description = "phase not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "validation failed")))]
pub async fn patch_phase(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, phase_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<PatchPhaseRequest>,
) -> ApiResult<Json<PhaseMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    if let Some(name) = req.name.as_deref() {
        if name.trim().is_empty() {
            return Err(ApiError::Validation("name must not be empty".into()));
        }
    }
    if let Some(Some(code)) = req.milestone_currency.as_ref().map(|o| o.as_deref()) {
        if !valid_currency(code) {
            return Err(ApiError::Validation(format!(
                "invalid ISO 4217 currency code: {code}"
            )));
        }
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "project_phase",
                entity_id: Some(phase_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::projects::get_phase(&mut *ctx.tx, org_id, phase_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if before.project_id != id {
            return Err(ApiError::RlsSparse);
        }
        let project = core_db::projects::get_project(&mut *ctx.tx, org_id, id).await?;
        let eff_amount = req
            .milestone_amount_minor
            .unwrap_or(before.milestone_amount_minor);
        let eff_currency: Option<String> = req
            .milestone_currency
            .clone()
            .unwrap_or_else(|| before.milestone_currency.clone());
        let _validated_amount = resolve_amount(
            eff_amount,
            eff_currency.as_deref(),
            project.currency.as_deref(),
        )?;
        // Effective budget/fee record after this patch lands (tri-state
        // over the existing row; a cleared pair drops the whole record).
        let existing = core_db::projects::get_phase_budget(&mut *ctx.tx, org_id, phase_id).await?;
        let eff_budget: Option<i64> = req
            .budget_minor
            .unwrap_or(existing.as_ref().and_then(|b| b.budget_minor));
        let eff_budget_ccy: Option<String> = req
            .budget_currency
            .clone()
            .flatten()
            .or_else(|| existing.as_ref().and_then(|b| b.currency.clone()));
        let eff_fee: Option<i64> = req
            .fee_minor
            .unwrap_or(existing.as_ref().and_then(|b| b.fee_minor));
        let eff_fee_ccy: Option<String> = req
            .fee_currency
            .clone()
            .flatten()
            .or_else(|| existing.as_ref().and_then(|b| b.fee_currency.clone()));
        validate_money_pair(eff_budget, eff_budget_ccy.as_deref(), "budget")?;
        validate_money_pair(eff_fee, eff_fee_ccy.as_deref(), "fee")?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "project_phase",
                entity_id: phase_id,
                version,
            },
        )
        .await?;
        let after = core_db::projects::update_phase(
            &mut *ctx.tx,
            org_id,
            phase_id,
            core_db::projects::PhasePatch {
                name: req.name.as_deref().map(str::trim),
                description: req.description.as_ref().map(|o| o.as_deref()),
                sort_order: req.sort_order,
                status: req.status,
                is_milestone: req.is_milestone,
                milestone_amount_minor: req.milestone_amount_minor,
                milestone_currency: req.milestone_currency.as_ref().map(|o| o.as_deref()),
                start_date: req.start_date,
                end_date: req.end_date,
            },
        )
        .await?;
        // Budget/fee record maintenance rides the same transaction:
        // both cleared → drop the row; any value present → upsert.
        if eff_budget.is_none() && eff_fee.is_none() {
            core_db::projects::delete_phase_budget(&mut *ctx.tx, org_id, phase_id).await?;
        } else {
            core_db::projects::upsert_phase_budget(
                &mut *ctx.tx,
                org_id,
                phase_id,
                core_db::projects::NewPhaseBudget {
                    budget_minor: eff_budget,
                    currency: eff_budget_ccy.as_deref(),
                    fee_minor: eff_fee,
                    fee_currency: eff_fee_ccy.as_deref(),
                },
            )
            .await?;
        }
        ctx.emit(
            types::PROJECT_PHASE_UPDATED,
            "project_phase",
            Some(after.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectPhaseUpdated {
                phase_id: after.id,
                project_id: after.project_id,
            })
            .unwrap_or_default(),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "project_phase",
                entity_id: phase_id,
                op: "update",
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
            MutationOutcome::Applied(PhaseMutationResponse {
                phase: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/projects/{id}/phases/{phase_id}/complete — stamps the phase
/// completed and, when it is a billing milestone, emits
/// `project.milestone_completed` in the same transaction (the core-billing
/// seam). Instant-first: the idempotency key guarantees a retried
/// completion can never double-emit the billing event.
#[utoipa::path(operation_id = "projects_complete_phase", post, path = "/api/projects/{id}/phases/{phase_id}/complete", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("phase_id" = Uuid, Path)),
    responses((status = 200, body = PhaseMutationResponse),
              (status = 404, description = "phase not visible in scope (rls_sparse)"),
              (status = 409, description = "already completed, version conflict, or idempotency key reused")))]
pub async fn complete_phase(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, phase_id)): Path<(Uuid, Uuid)>,
    body: Option<Json<instant::MutationParams>>,
) -> ApiResult<Json<PhaseMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let req: instant::MutationParams = body.map(|Json(p)| p).unwrap_or_default();
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.idempotency_key.clone();
    let version = req.version;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "project_phase",
                entity_id: Some(phase_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::projects::get_phase(&mut *ctx.tx, org_id, phase_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if before.project_id != id {
            return Err(ApiError::RlsSparse);
        }
        if before.status == PhaseStatus::Completed {
            return Err(ApiError::Conflict("phase already completed".into()));
        }
        let project = core_db::projects::get_project(&mut *ctx.tx, org_id, id).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "project_phase",
                entity_id: phase_id,
                version,
            },
        )
        .await?;
        let at = chrono::Utc::now();
        let after = core_db::projects::complete_phase(&mut *ctx.tx, org_id, phase_id, at).await?;
        ctx.emit(
            types::PROJECT_PHASE_COMPLETED,
            "project_phase",
            Some(after.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectPhaseCompleted {
                phase: after.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        if after.is_milestone {
            let amount = resolve_amount(
                after.milestone_amount_minor,
                after.milestone_currency.as_deref(),
                project.currency.as_deref(),
            )?;
            emit_milestone_completed(
                &mut ctx,
                &project,
                &after.name,
                Some(after.id),
                None,
                amount,
                at,
            )
            .await?;
        } else {
            // Fixed-fee phase (report P4): a NON-milestone phase whose
            // budget record carries a fee releases it on completion —
            // directly into the ledger in this same transaction (the
            // phase-completion idempotency replay guard covers this too:
            // a retried completion replays the same mutation without
            // double-staging, guarded by the ledger's unique index).
            if let Some(budget) =
                core_db::projects::get_phase_budget(&mut *ctx.tx, org_id, phase_id).await?
            {
                if let (Some(fee_minor), Some(currency)) = (budget.fee_minor, budget.fee_currency) {
                    core_db::projects::insert_release(
                        &mut *ctx.tx,
                        &core_db::projects::NewRelease {
                            org_id,
                            project_id: after.project_id,
                            phase_id: Some(after.id),
                            task_id: None,
                            label: after.name.clone(),
                            amount_minor: fee_minor,
                            currency: currency.to_string(),
                            released_on: at.date_naive(),
                            source_event_id: None,
                        },
                    )
                    .await?;
                }
            }
        }
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "project_phase",
                entity_id: phase_id,
                op: "complete",
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
            MutationOutcome::Applied(PhaseMutationResponse {
                phase: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- phase burn + billing releases (Wave 39 lane 4) -------------------------

/// GET /api/projects/{id}/phase-burn — the burn read model (report P3):
/// consumed approved billable minutes and card-priced value per phase
/// against the phase's budget, with the typed `none | warn | over`
/// alert state (`warn` at ≥80%, `over` at ≥100%).
#[utoipa::path(operation_id = "projects_phase_burn", get, path = "/api/projects/{id}/phase-burn", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::PhaseBurn]),
              (status = 404, description = "project not visible in scope (rls_sparse)")))]
pub async fn phase_burn(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::PhaseBurn>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let burns = core_db::projects::phase_burn(&mut *tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(burns))
}

/// GET /api/projects/{id}/billing-releases — the release ledger: every
/// staged milestone/fee for this project with its status (`pending`
/// bills on the next run reaching its window; `billed` carries the
/// consuming invoice; `cancelled` terminal).
#[utoipa::path(operation_id = "projects_billing_releases", get, path = "/api/projects/{id}/billing-releases", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::ProjectBillingRelease])))]
pub async fn list_billing_releases(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::ProjectBillingRelease>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let rows =
        core_db::projects::list_releases_for_project(&mut *tx, authed.principal.user.org_id, id)
            .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// POST /api/projects/{id}/billing-releases/{release_id}/cancel — cancel
/// a PENDING release (admin correction; terminal). Billed releases are
/// immutable — they ride an invoice, and invoices correct through credit
/// notes.
#[utoipa::path(operation_id = "projects_billing_release_cancel", post, path = "/api/projects/{id}/billing-releases/{release_id}/cancel", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("release_id" = Uuid, Path)),
    responses((status = 200, body = core_domain::entities::ProjectBillingRelease),
              (status = 404, description = "release not visible/already settled (rls_sparse)")))]
pub async fn cancel_billing_release(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, release_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<core_domain::entities::ProjectBillingRelease>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    // RLS-scoped transaction; the project id scopes the read so a
    // release from another project 404s (rls_sparse).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    core_db::projects::get_project(&mut *tx, org_id, id).await?;
    let release = core_db::projects::cancel_release(&mut tx, org_id, release_id)
        .await
        .map_err(|_| ApiError::RlsSparse)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(release))
}

// --- tasks ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateTaskRequest {
    pub title: String,
    pub description: Option<String>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub phase_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub assignee_id: Option<Uuid>,
    #[serde(default)]
    pub sort_order: i32,
    #[serde(default)]
    pub is_milestone: bool,
    pub milestone_amount_minor: Option<i64>,
    pub milestone_currency: Option<String>,
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Finish-to-start dependencies to create with the task; every id must
    /// be a task of the SAME project. Cycles are impossible at creation
    /// (nothing can depend on the not-yet-existing task) but membership is
    /// validated.
    #[serde(default)]
    pub depends_on: Vec<Uuid>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PatchTaskRequest {
    pub title: Option<String>,
    pub description: Option<Option<String>>,
    pub phase_id: Option<Option<Uuid>>,
    /// Lifecycle moves other than completion (pending / in_progress /
    /// blocked / cancelled). Reaching `completed` MUST go through the
    /// complete endpoint so dependency gating + billing events cannot be
    /// skipped.
    pub status: Option<TaskStatus>,
    pub assignee_id: Option<Option<Uuid>>,
    pub sort_order: Option<i32>,
    pub is_milestone: Option<bool>,
    pub milestone_amount_minor: Option<Option<i64>>,
    pub milestone_currency: Option<Option<String>>,
    pub due_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct TaskListQuery {
    #[schema(value_type = Option<uuid::Uuid>)]
    pub phase_id: Option<Uuid>,
    pub status: Option<TaskStatus>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub assignee_id: Option<Uuid>,
    pub milestone: Option<bool>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

/// GET /api/projects/{id}/tasks — filtered by phase/status/assignee and
/// the milestone flag, sort_order-ordered.
#[utoipa::path(operation_id = "projects_list_tasks", get, path = "/api/projects/{id}/tasks", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), TaskListQuery),
    responses((status = 200, body = [ProjectTask])))]
pub async fn list_tasks(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Query(q): Query<TaskListQuery>,
) -> ApiResult<Json<Vec<ProjectTask>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let rows = core_db::projects::list_tasks(
        &mut *tx,
        authed.principal.user.org_id,
        id,
        core_db::projects::TaskFilter {
            phase_id: q.phase_id,
            status: q.status,
            assignee_id: q.assignee_id,
            milestone: q.milestone,
            limit: q.limit,
            offset: q.offset,
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// POST /api/projects/{id}/tasks — creates the task (optionally with its
/// dependency set) and emits `project.task_created` in one transaction.
#[utoipa::path(operation_id = "projects_create_task", post, path = "/api/projects/{id}/tasks", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = CreateTaskRequest,
    responses((status = 200, body = TaskMutationResponse),
              (status = 404, description = "project not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "validation failed")))]
pub async fn create_task(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateTaskRequest>,
) -> ApiResult<Json<TaskMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Create))?;
    if req.title.trim().is_empty() {
        return Err(ApiError::Validation("title is required".into()));
    }
    if let Some(code) = req.milestone_currency.as_deref() {
        if !valid_currency(code) {
            return Err(ApiError::Validation(format!(
                "invalid ISO 4217 currency code: {code}"
            )));
        }
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "project_task",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let project = core_db::projects::get_project(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if let Some(phase_id) = req.phase_id {
            let phase = core_db::projects::get_phase(&mut *ctx.tx, org_id, phase_id)
                .await
                .map_err(|e| instant::mutation_scope_err(e.into()))?;
            if phase.project_id != id {
                return Err(ApiError::Validation(
                    "phase_id belongs to a different project".into(),
                ));
            }
        }
        let amount = resolve_amount(
            req.milestone_amount_minor,
            req.milestone_currency.as_deref(),
            project.currency.as_deref(),
        )?;
        let new_id = Uuid::now_v7();
        // Dependency membership: every dep must live in the SAME project.
        // A fresh task cannot close a cycle (nothing points at it yet),
        // but the shared validator keeps the rules in one place.
        let existing = core_db::projects::list_project_dep_edges(&mut *ctx.tx, org_id, id).await?;
        let mut deps: Vec<Uuid> = Vec::with_capacity(req.depends_on.len());
        for dep in &req.depends_on {
            let dep_task = core_db::projects::get_task(&mut *ctx.tx, org_id, *dep).await?;
            if dep_task.project_id != id {
                return Err(ApiError::Validation(format!(
                    "dependency {dep} belongs to a different project"
                )));
            }
            if deps.contains(dep) {
                continue;
            }
            core_projects::deps::validate_add(&existing, new_id, *dep, true)
                .map_err(|r| ApiError::Validation(r.to_string()))?;
            deps.push(*dep);
        }
        let task = core_db::projects::insert_task(
            &mut *ctx.tx,
            core_db::projects::NewTask {
                id: new_id,
                org_id,
                project_id: id,
                phase_id: req.phase_id,
                title: req.title.trim(),
                description: req.description.as_deref(),
                assignee_id: req.assignee_id,
                sort_order: req.sort_order,
                is_milestone: req.is_milestone,
                milestone_amount_minor: amount.as_ref().map(|(m, _)| *m),
                milestone_currency: amount.as_ref().map(|(_, c)| c.as_str()),
                due_at: req.due_at,
            },
        )
        .await?;
        if !deps.is_empty() {
            core_db::projects::set_task_deps(&mut ctx.tx, org_id, task.id, &deps).await?;
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "project_task",
                entity_id: task.id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_TASK_CREATED,
            "project_task",
            Some(task.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectTaskCreated { task: task.clone() })
                .unwrap_or_default(),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "project_task",
                entity_id: task.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&task).unwrap_or(Value::Null),
                updated_at: task.updated_at,
                key: key.as_deref(),
                payload: &task,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TaskMutationResponse { task, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/projects/{id}/tasks/{task_id} — tri-state partial update.
/// `status: "completed"` is rejected: completion runs through the complete
/// endpoint (dependency gating + the billing seam).
#[utoipa::path(operation_id = "projects_patch_task", patch, path = "/api/projects/{id}/tasks/{task_id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("task_id" = Uuid, Path)), request_body = PatchTaskRequest,
    responses((status = 200, body = TaskMutationResponse),
              (status = 404, description = "task not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "validation failed")))]
pub async fn patch_task(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, task_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<PatchTaskRequest>,
) -> ApiResult<Json<TaskMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    if let Some(title) = req.title.as_deref() {
        if title.trim().is_empty() {
            return Err(ApiError::Validation("title must not be empty".into()));
        }
    }
    if req.status == Some(TaskStatus::Completed) {
        return Err(ApiError::Validation(
            "task completion goes through POST .../complete (dependency gating + billing events)"
                .into(),
        ));
    }
    if let Some(Some(code)) = req.milestone_currency.as_ref().map(|o| o.as_deref()) {
        if !valid_currency(code) {
            return Err(ApiError::Validation(format!(
                "invalid ISO 4217 currency code: {code}"
            )));
        }
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "project_task",
                entity_id: Some(task_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::projects::get_task(&mut *ctx.tx, org_id, task_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if before.project_id != id {
            return Err(ApiError::RlsSparse);
        }
        if let Some(Some(phase_id)) = req.phase_id {
            let phase = core_db::projects::get_phase(&mut *ctx.tx, org_id, phase_id)
                .await
                .map_err(|e| instant::mutation_scope_err(e.into()))?;
            if phase.project_id != id {
                return Err(ApiError::Validation(
                    "phase_id belongs to a different project".into(),
                ));
            }
        }
        let project = core_db::projects::get_project(&mut *ctx.tx, org_id, id).await?;
        let eff_amount = req
            .milestone_amount_minor
            .unwrap_or(before.milestone_amount_minor);
        let eff_currency: Option<String> = req
            .milestone_currency
            .clone()
            .unwrap_or_else(|| before.milestone_currency.clone());
        let _validated_amount = resolve_amount(
            eff_amount,
            eff_currency.as_deref(),
            project.currency.as_deref(),
        )?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "project_task",
                entity_id: task_id,
                version,
            },
        )
        .await?;
        let after = core_db::projects::update_task(
            &mut *ctx.tx,
            org_id,
            task_id,
            core_db::projects::TaskPatch {
                title: req.title.as_deref().map(str::trim),
                description: req.description.as_ref().map(|o| o.as_deref()),
                phase_id: req.phase_id,
                status: req.status,
                assignee_id: req.assignee_id,
                sort_order: req.sort_order,
                is_milestone: req.is_milestone,
                milestone_amount_minor: req.milestone_amount_minor,
                milestone_currency: req.milestone_currency.as_ref().map(|o| o.as_deref()),
                due_at: req.due_at,
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_TASK_UPDATED,
            "project_task",
            Some(after.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectTaskUpdated {
                task_id: after.id,
                project_id: after.project_id,
            })
            .unwrap_or_default(),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "project_task",
                entity_id: task_id,
                op: "update",
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
            MutationOutcome::Applied(TaskMutationResponse {
                task: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/projects/{id}/tasks/{task_id}/complete — the guarded
/// completion: every finish-to-start dependency must already be completed
/// (typed 409 `task_dependencies_unsatisfied` naming the blockers), then
/// `project.task_completed` fires — plus `project.milestone_completed`
/// when the task is a billing milestone. Instant-first: the idempotency
/// key makes retried completions replay instead of double-billing.
#[utoipa::path(operation_id = "projects_complete_task", post, path = "/api/projects/{id}/tasks/{task_id}/complete", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("task_id" = Uuid, Path)),
    responses((status = 200, body = TaskMutationResponse),
              (status = 404, description = "task not visible in scope (rls_sparse)"),
              (status = 409, description = "dependencies unsatisfied, already completed, version conflict, or idempotency key reused")))]
pub async fn complete_task(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, task_id)): Path<(Uuid, Uuid)>,
    body: Option<Json<instant::MutationParams>>,
) -> ApiResult<Json<TaskMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let req: instant::MutationParams = body.map(|Json(p)| p).unwrap_or_default();
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.idempotency_key.clone();
    let version = req.version;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "project_task",
                entity_id: Some(task_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::projects::get_task(&mut *ctx.tx, org_id, task_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if before.project_id != id {
            return Err(ApiError::RlsSparse);
        }
        if before.status == TaskStatus::Completed {
            return Err(ApiError::Conflict("task already completed".into()));
        }
        if before.status == TaskStatus::Cancelled {
            return Err(ApiError::Conflict(
                "cancelled tasks cannot be completed".into(),
            ));
        }
        let blocking = core_db::projects::incomplete_deps(&mut *ctx.tx, org_id, task_id).await?;
        if !blocking.is_empty() {
            return Err(ApiError::TaskDependenciesUnsatisfied { task_id, blocking });
        }
        let project = core_db::projects::get_project(&mut *ctx.tx, org_id, id).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "project_task",
                entity_id: task_id,
                version,
            },
        )
        .await?;
        let at = chrono::Utc::now();
        let after = core_db::projects::complete_task(&mut *ctx.tx, org_id, task_id, at).await?;
        ctx.emit(
            types::PROJECT_TASK_COMPLETED,
            "project_task",
            Some(after.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectTaskCompleted {
                task: after.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        if after.is_milestone {
            let amount = resolve_amount(
                after.milestone_amount_minor,
                after.milestone_currency.as_deref(),
                project.currency.as_deref(),
            )?;
            emit_milestone_completed(
                &mut ctx,
                &project,
                &after.title,
                None,
                Some(after.id),
                amount,
                at,
            )
            .await?;
        }
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "project_task",
                entity_id: task_id,
                op: "complete",
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
            MutationOutcome::Applied(TaskMutationResponse {
                task: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- task dependencies -----------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SetDependenciesRequest {
    /// The FULL finish-to-start dependency set for the task (PUT
    /// semantics): every id must be a task of the same project; the set
    /// must not create a cycle (typed 422 names the offending edge).
    pub depends_on: Vec<Uuid>,
}

/// GET /api/projects/{id}/tasks/{task_id}/dependencies
#[utoipa::path(operation_id = "projects_get_dependencies", get, path = "/api/projects/{id}/tasks/{task_id}/dependencies", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("task_id" = Uuid, Path)),
    responses((status = 200, body = DependencyListResponse),
              (status = 404, description = "not found")))]
pub async fn get_deps(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, task_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DependencyListResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let task = core_db::projects::get_task(&mut *tx, org_id, task_id).await?;
    if task.project_id != id {
        return Err(ApiError::NotFound);
    }
    let depends_on = core_db::projects::list_task_deps(&mut *tx, org_id, task_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(DependencyListResponse {
        task_id,
        project_id: id,
        depends_on,
    }))
}

/// PUT /api/projects/{id}/tasks/{task_id}/dependencies — replaces the
/// whole dependency set, cycle-checked over the project graph BEFORE any
/// write; emits `project.task_updated`.
#[utoipa::path(operation_id = "projects_set_dependencies", put, path = "/api/projects/{id}/tasks/{task_id}/dependencies", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("task_id" = Uuid, Path)), request_body = SetDependenciesRequest,
    responses((status = 200, body = DependencyListResponse),
              (status = 404, description = "task not visible in scope (rls_sparse)"),
              (status = 422, description = "dependency set rejected (self-dep, cross-project, or cycle)")))]
pub async fn set_deps(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, task_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<SetDependenciesRequest>,
) -> ApiResult<Json<DependencyListResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let task = core_db::projects::get_task(&mut *ctx.tx, org_id, task_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if task.project_id != id {
            return Err(ApiError::RlsSparse);
        }
        let existing = core_db::projects::list_project_dep_edges(&mut *ctx.tx, org_id, id).await?;
        let members: std::collections::HashSet<Uuid> =
            core_db::projects::list_project_task_ids(&mut *ctx.tx, org_id, id)
                .await?
                .into_iter()
                .collect();
        let validated =
            core_projects::deps::validate_replace(&existing, task_id, &req.depends_on, &members)
                .map_err(|r| ApiError::Validation(r.to_string()))?;
        core_db::projects::set_task_deps(&mut ctx.tx, org_id, task_id, &validated).await?;
        ctx.emit(
            types::PROJECT_TASK_UPDATED,
            "project_task",
            Some(task.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectTaskUpdated {
                task_id: task.id,
                project_id: task.project_id,
            })
            .unwrap_or_default(),
        )
        .await?;
        let depends_on = core_db::projects::list_task_deps(&mut *ctx.tx, org_id, task_id).await?;
        Ok((
            ctx,
            DependencyListResponse {
                task_id,
                project_id: id,
                depends_on,
            },
        ))
    })
    .await
    .map(Json)
}

// --- views ------------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateViewRequest {
    pub name: String,
    /// `None` = org-wide view (spans every project).
    #[schema(value_type = Option<uuid::Uuid>)]
    pub project_id: Option<Uuid>,
    /// Versioned, additive-only view spec (filter/group/sort). Consumers
    /// tolerate unknown fields.
    #[serde(default)]
    pub spec: Value,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PatchViewRequest {
    pub name: Option<String>,
    pub spec: Option<Value>,
    pub is_default: Option<bool>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct ViewListQuery {
    /// Scope to one project's views; omit for the org's ENTIRE view set.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub project_id: Option<Uuid>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ViewMutationResponse {
    #[serde(flatten)]
    pub view: ProjectView,
}

/// GET /api/project-views — one project's views, or all org views when
/// `project_id` is omitted.
#[utoipa::path(operation_id = "project_views_list", get, path = "/api/project-views", tag = "projects", security(("bearer" = [])),
    params(ViewListQuery),
    responses((status = 200, body = [ProjectView])))]
pub async fn list_views(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ViewListQuery>,
) -> ApiResult<Json<Vec<ProjectView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let rows =
        core_db::projects::list_views(&mut *tx, authed.principal.user.org_id, q.project_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// POST /api/project-views — saves a view; emits `project.view_updated`.
#[utoipa::path(operation_id = "project_views_create", post, path = "/api/project-views", tag = "projects", security(("bearer" = [])),
    request_body = CreateViewRequest,
    responses((status = 200, body = ViewMutationResponse),
              (status = 404, description = "project not visible in scope (rls_sparse)"),
              (status = 409, description = "view name already in use in the scope"),
              (status = 422, description = "validation failed")))]
pub async fn create_view(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateViewRequest>,
) -> ApiResult<Json<ViewMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Create))?;
    if req.name.trim().is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    if !req.spec.is_object() {
        return Err(ApiError::Validation("spec must be a JSON object".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let Some(pid) = req.project_id {
            core_db::projects::get_project(&mut *ctx.tx, org_id, pid)
                .await
                .map_err(|e| instant::mutation_scope_err(e.into()))?;
        }
        // Name uniqueness per scope (org-wide views have no DB UNIQUE —
        // Postgres treats NULLs as distinct — so the app enforces both).
        let scope_taken = core_db::projects::list_views(&mut *ctx.tx, org_id, req.project_id)
            .await?
            .iter()
            .any(|v| v.name.eq_ignore_ascii_case(req.name.trim()));
        if scope_taken {
            return Err(ApiError::Conflict(
                "a view with this name already exists in the scope".into(),
            ));
        }
        let view = core_db::projects::insert_view(
            &mut *ctx.tx,
            core_db::projects::NewView {
                id: Uuid::now_v7(),
                org_id,
                project_id: req.project_id,
                name: req.name.trim(),
                spec: &req.spec,
                is_default: req.is_default,
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_VIEW_UPDATED,
            "project_view",
            Some(view.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectViewUpdated {
                view_id: view.id,
                project_id: view.project_id,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, ViewMutationResponse { view }))
    })
    .await
    .map(Json)
}

/// PATCH /api/project-views/{id} — renames / respecs a view; emits
/// `project.view_updated`.
#[utoipa::path(operation_id = "project_views_patch", patch, path = "/api/project-views/{id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = PatchViewRequest,
    responses((status = 200, body = ViewMutationResponse),
              (status = 404, description = "view not visible in scope (rls_sparse)"),
              (status = 422, description = "validation failed")))]
pub async fn patch_view(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchViewRequest>,
) -> ApiResult<Json<ViewMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    if let Some(name) = req.name.as_deref() {
        if name.trim().is_empty() {
            return Err(ApiError::Validation("name must not be empty".into()));
        }
    }
    if let Some(spec) = &req.spec {
        if !spec.is_object() {
            return Err(ApiError::Validation("spec must be a JSON object".into()));
        }
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let view = core_db::projects::update_view(
            &mut *ctx.tx,
            org_id,
            id,
            core_db::projects::ViewPatch {
                name: req.name.as_deref().map(str::trim),
                spec: req.spec.as_ref(),
                is_default: req.is_default,
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_VIEW_UPDATED,
            "project_view",
            Some(view.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectViewUpdated {
                view_id: view.id,
                project_id: view.project_id,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, ViewMutationResponse { view }))
    })
    .await
    .map(Json)
}

/// DELETE /api/project-views/{id} — emits `project.view_removed`.
#[utoipa::path(operation_id = "project_views_delete", delete, path = "/api/project-views/{id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200),
              (status = 404, description = "view not visible in scope (rls_sparse)")))]
pub async fn delete_view(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let deleted = core_db::projects::delete_view(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if !deleted {
            return Err(ApiError::RlsSparse);
        }
        ctx.emit(
            types::PROJECT_VIEW_REMOVED,
            "project_view",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectViewRemoved { view_id: id })
                .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, serde_json::json!({ "deleted": true })))
    })
    .await
    .map(Json)
}

// --- templates -----------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateTemplateRequest {
    pub name: String,
    pub description: Option<String>,
    /// Versioned template spec (core-projects `template`, spec_version 1).
    /// Validated eagerly so broken blueprints never persist.
    pub spec: Value,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PatchTemplateRequest {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub spec: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TemplateMutationResponse {
    #[serde(flatten)]
    pub template: ProjectTemplate,
}

/// GET /api/project-templates
#[utoipa::path(operation_id = "project_templates_list", get, path = "/api/project-templates", tag = "projects", security(("bearer" = [])),
    responses((status = 200, body = [ProjectTemplate])))]
pub async fn list_templates(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<ProjectTemplate>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let rows = core_db::projects::list_templates(&mut *tx, authed.principal.user.org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// POST /api/project-templates — stores a blueprint (spec validated);
/// emits `project.template_updated`.
#[utoipa::path(operation_id = "project_templates_create", post, path = "/api/project-templates", tag = "projects", security(("bearer" = [])),
    request_body = CreateTemplateRequest,
    responses((status = 200, body = TemplateMutationResponse),
              (status = 409, description = "template name already in use"),
              (status = 422, description = "invalid template spec")))]
pub async fn create_template(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateTemplateRequest>,
) -> ApiResult<Json<TemplateMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Create))?;
    if req.name.trim().is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    // Eager spec validation: templates are data; broken data never ships.
    core_projects::template::parse_spec(&req.spec)
        .map_err(|e| ApiError::Validation(e.to_string()))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let taken = core_db::projects::list_templates(&mut *ctx.tx, org_id)
            .await?
            .iter()
            .any(|t| t.name.eq_ignore_ascii_case(req.name.trim()));
        if taken {
            return Err(ApiError::Conflict(
                "a template with this name already exists".into(),
            ));
        }
        let template = core_db::projects::insert_template(
            &mut *ctx.tx,
            core_db::projects::NewTemplate {
                id: Uuid::now_v7(),
                org_id,
                name: req.name.trim(),
                description: req.description.as_deref(),
                spec: &req.spec,
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_TEMPLATE_UPDATED,
            "project_template",
            Some(template.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectTemplateUpdated {
                template_id: template.id,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, TemplateMutationResponse { template }))
    })
    .await
    .map(Json)
}

/// PATCH /api/project-templates/{id} — emits `project.template_updated`.
#[utoipa::path(operation_id = "project_templates_patch", patch, path = "/api/project-templates/{id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = PatchTemplateRequest,
    responses((status = 200, body = TemplateMutationResponse),
              (status = 404, description = "template not visible in scope (rls_sparse)"),
              (status = 422, description = "invalid template spec")))]
pub async fn patch_template(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchTemplateRequest>,
) -> ApiResult<Json<TemplateMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    if let Some(name) = req.name.as_deref() {
        if name.trim().is_empty() {
            return Err(ApiError::Validation("name must not be empty".into()));
        }
    }
    if let Some(spec) = &req.spec {
        core_projects::template::parse_spec(spec)
            .map_err(|e| ApiError::Validation(e.to_string()))?;
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let template = core_db::projects::update_template(
            &mut *ctx.tx,
            org_id,
            id,
            core_db::projects::TemplatePatch {
                name: req.name.as_deref().map(str::trim),
                description: req.description.as_ref().map(|o| o.as_deref()),
                spec: req.spec.as_ref(),
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_TEMPLATE_UPDATED,
            "project_template",
            Some(template.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectTemplateUpdated {
                template_id: template.id,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, TemplateMutationResponse { template }))
    })
    .await
    .map(Json)
}

/// DELETE /api/project-templates/{id} — blueprints are data, not live
/// references: projects already instantiated are untouched; emits
/// `project.template_removed`.
#[utoipa::path(operation_id = "project_templates_delete", delete, path = "/api/project-templates/{id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200),
              (status = 404, description = "template not visible in scope (rls_sparse)")))]
pub async fn delete_template(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let deleted = core_db::projects::delete_template(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if !deleted {
            return Err(ApiError::RlsSparse);
        }
        ctx.emit(
            types::PROJECT_TEMPLATE_REMOVED,
            "project_template",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectTemplateRemoved { template_id: id })
                .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, serde_json::json!({ "deleted": true })))
    })
    .await
    .map(Json)
}

// --- template instantiation ---------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct InstantiateTemplateRequest {
    pub name: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub client_id: Option<Uuid>,
    pub start_date: Option<chrono::NaiveDate>,
    pub due_date: Option<chrono::NaiveDate>,
    /// Resolves template `assignee_key`s to user ids at instantiation
    /// time; unmapped keys simply leave those tasks unassigned.
    #[serde(default)]
    pub assignee_map: HashMap<String, Uuid>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version (of the newly created project). Absent in legacy clients —
    /// purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// The instantiation result: the applied project plus the expansion
/// counts (clients render "created N phases, M tasks" without refetching).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProjectInstantiateResponse {
    #[serde(flatten)]
    pub project: Project,
    pub phase_count: i64,
    pub task_count: i64,
    pub mutation: instant::MutationMeta,
}

/// POST /api/project-templates/{id}/instantiate — expands the template
/// into a REAL project: phases, tasks, dependencies, and every lifecycle
/// event, all in ONE transaction (claim → expand → insert → events →
/// record). Rollback on any failure leaves zero partial rows.
#[utoipa::path(operation_id = "project_templates_instantiate", post, path = "/api/project-templates/{id}/instantiate", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = InstantiateTemplateRequest,
    responses((status = 200, body = ProjectInstantiateResponse),
              (status = 404, description = "template not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "invalid template spec or request")))]
pub async fn instantiate(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<InstantiateTemplateRequest>,
) -> ApiResult<Json<ProjectInstantiateResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Create))?;
    if req.name.trim().is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "project",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let template = core_db::projects::get_template(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let spec = core_projects::template::parse_spec(&template.spec)
            .map_err(|e| ApiError::Validation(e.to_string()))?;
        let plan = spec.expand();

        let billing_method = plan
            .billing_method
            .as_deref()
            .map(|s| s.parse::<BillingMethod>())
            .transpose()
            .map_err(|e| ApiError::Validation(e.to_string()))?
            .unwrap_or(BillingMethod::NonBillable);
        if let Some(code) = plan.currency.as_deref() {
            if !valid_currency(code) {
                return Err(ApiError::Validation(format!(
                    "template currency is not a valid ISO 4217 code: {code}"
                )));
            }
        }
        if core_db::projects::project_name_taken(&mut *ctx.tx, org_id, req.name.trim()).await? {
            return Err(ApiError::Conflict(
                "a project with this name already exists".into(),
            ));
        }
        let project = core_db::projects::insert_project(
            &mut *ctx.tx,
            core_db::projects::NewProject {
                id: Uuid::now_v7(),
                org_id,
                client_id: req.client_id,
                name: req.name.trim(),
                description: template.description.as_deref(),
                billing_method,
                budget_minor: None,
                currency: plan.currency.as_deref(),
                start_date: req.start_date,
                due_date: req.due_date,
                // Templates stay unlinked: a fresh instantiation bills
                // nothing until someone attaches a client + contract.
                billing_contract_id: None,
            },
        )
        .await?;

        // Phases first (tasks reference their ids).
        let mut phase_ids: HashMap<String, Uuid> = HashMap::new();
        let mut phase_count: i64 = 0;
        for phase in &plan.phases {
            let phase_id = Uuid::now_v7();
            let inserted = core_db::projects::insert_phase(
                &mut *ctx.tx,
                core_db::projects::NewPhase {
                    id: phase_id,
                    org_id,
                    project_id: project.id,
                    name: &phase.name,
                    description: phase.description.as_deref(),
                    sort_order: phase.sort_order,
                    is_milestone: phase.is_milestone,
                    milestone_amount_minor: phase.milestone_amount_minor,
                    milestone_currency: phase.milestone_currency.as_deref(),
                    // A template seeds no dates: the dates a real project
                    // gets depend on when it is scheduled, and baking
                    // template dates in would put a plan on a Gantt that
                    // nobody chose for this engagement.
                    start_date: None,
                    end_date: None,
                },
            )
            .await?;
            ctx.emit(
                types::PROJECT_PHASE_CREATED,
                "project_phase",
                Some(inserted.id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::ProjectPhaseCreated { phase: inserted })
                    .unwrap_or_default(),
            )
            .await?;
            phase_ids.insert(phase.key.clone(), phase_id);
            phase_count += 1;
        }

        // Tasks second, keyed for the dependency pass.
        let mut task_ids: HashMap<String, Uuid> = HashMap::new();
        let mut inserted_tasks: Vec<ProjectTask> = Vec::with_capacity(plan.tasks.len());
        for task in &plan.tasks {
            let task_id = Uuid::now_v7();
            let phase_id = task.phase_key.as_deref().map(|key| {
                phase_ids
                    .get(key)
                    .copied()
                    .expect("spec validation guarantees phase keys resolve")
            });
            let due_at = match (req.start_date, task.due_offset_days) {
                (Some(start), Some(days)) => start
                    .and_hms_opt(0, 0, 0)
                    .map(|t| t.and_utc() + chrono::Duration::days(days)),
                _ => None,
            };
            let assignee_id = task
                .assignee_key
                .as_deref()
                .and_then(|k| req.assignee_map.get(k).copied());
            let inserted = core_db::projects::insert_task(
                &mut *ctx.tx,
                core_db::projects::NewTask {
                    id: task_id,
                    org_id,
                    project_id: project.id,
                    phase_id,
                    title: &task.title,
                    description: task.description.as_deref(),
                    assignee_id,
                    sort_order: task.sort_order,
                    is_milestone: task.is_milestone,
                    milestone_amount_minor: task.milestone_amount_minor,
                    milestone_currency: task.milestone_currency.as_deref(),
                    due_at,
                },
            )
            .await?;
            ctx.emit(
                types::PROJECT_TASK_CREATED,
                "project_task",
                Some(inserted.id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::ProjectTaskCreated {
                    task: inserted.clone(),
                })
                .unwrap_or_default(),
            )
            .await?;
            task_ids.insert(task.key.clone(), inserted.id);
            inserted_tasks.push(inserted);
        }

        // Dependencies last: every edge now resolves to a real task id.
        for task in &plan.tasks {
            let task_id = task_ids
                .get(&task.key)
                .copied()
                .expect("inserted above under its key");
            for dep_key in &task.depends_on {
                let dep_id = task_ids
                    .get(dep_key)
                    .copied()
                    .expect("spec validation guarantees dep keys resolve");
                core_db::projects::add_task_dep(&mut *ctx.tx, org_id, task_id, dep_id).await?;
            }
        }

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "project",
                entity_id: project.id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_CREATED,
            "project",
            Some(project.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectCreated {
                project: project.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        let task_count = inserted_tasks.len() as i64;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "project",
                entity_id: project.id,
                op: "instantiate",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&project).unwrap_or(Value::Null),
                updated_at: project.updated_at,
                key: key.as_deref(),
                payload: &project,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ProjectInstantiateResponse {
                project,
                phase_count,
                task_count,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- dated Gantt (Wave 34 claims close-out, claim 6) -----------------------

/// One phase as the Gantt draws it, with its budget burn attached.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PhaseGanttView {
    pub id: Uuid,
    pub name: String,
    pub sort_order: i32,
    pub status: String,
    pub is_milestone: bool,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    /// Planned days, inclusive of both ends. `None` when undated.
    pub planned_days: Option<i64>,
    pub completed_at: Option<String>,
    pub budget_minor: Option<i64>,
    pub currency: Option<String>,
    pub consumed_minor: i64,
    pub consumed_minutes: i64,
    /// `none` under budget, `warn` at/over 80%, `over` at/over 100%.
    /// `None` for an unbudgeted phase, because "no budget" is not "under
    /// budget" and rendering it green would be a false reassurance.
    pub burn_alert: Option<String>,
}

/// The project's Gantt: dated phases as bars, undated ones as list items.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectGanttView {
    pub project_id: Uuid,
    /// Phases with BOTH dates set, ordered by start then sort_order.
    pub dated: Vec<PhaseGanttView>,
    /// Phases with a missing end, in sort_order. The UI shows these in the
    /// ordered list and says they are not on the timeline, rather than
    /// placing them at an invented date.
    pub undated: Vec<PhaseGanttView>,
    /// The union of dated windows, so the chart can scale to the plan
    /// instead of to "today".
    pub window_from: Option<String>,
    pub window_to: Option<String>,
}

/// GET /api/projects/{id}/gantt — the dated timeline plus its undated
/// remainder, each phase carrying its budget burn.
#[utoipa::path(operation_id = "projects_gantt", get, path = "/api/projects/{id}/gantt",
    tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ProjectGanttView)))]
pub async fn gantt(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ProjectGanttView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::projects::phase_gantt(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let to_view = |r: core_db::projects::PhaseGanttRow| PhaseGanttView {
        id: r.id,
        name: r.name,
        sort_order: r.sort_order,
        status: r.status,
        is_milestone: r.is_milestone,
        planned_days: match (r.start_date, r.end_date) {
            (Some(s), Some(e)) => Some((e - s).num_days() + 1),
            _ => None,
        },
        start_date: r.start_date.map(|d| d.to_string()),
        end_date: r.end_date.map(|d| d.to_string()),
        completed_at: r.completed_at.map(|t| t.to_rfc3339()),
        // The alert is computed here rather than in SQL so the thresholds
        // live in one place next to the rendering that depends on them.
        burn_alert: r.budget_minor.map(|budget| {
            if budget <= 0 {
                // A zero or negative budget has no meaningful percentage, and
                // dividing by it would be the classic "infinite overspend".
                "none"
            } else if r.consumed_minor >= budget {
                "over"
            } else if r.consumed_minor.saturating_mul(100) >= budget.saturating_mul(80) {
                "warn"
            } else {
                "none"
            }
            .to_string()
        }),
        budget_minor: r.budget_minor,
        currency: r.currency,
        consumed_minor: r.consumed_minor,
        consumed_minutes: r.consumed_minutes,
    };

    // The split is the point: a bar is a plan, a list row is not a plan.
    // Everything the chart draws is dated, and everything undated is
    // reported as undated rather than being quietly placed.
    let mut dated = Vec::new();
    let mut undated = Vec::new();
    for r in rows {
        let view = to_view(r);
        if view.start_date.is_some() && view.end_date.is_some() {
            dated.push(view);
        } else {
            undated.push(view);
        }
    }
    dated.sort_by(|a, b| {
        a.start_date
            .cmp(&b.start_date)
            .then(a.sort_order.cmp(&b.sort_order))
    });

    let window_from = dated.iter().filter_map(|v| v.start_date.clone()).min();
    let window_to = dated.iter().filter_map(|v| v.end_date.clone()).max();

    Ok(Json(ProjectGanttView {
        project_id: id,
        dated,
        undated,
        window_from,
        window_to,
    }))
}
