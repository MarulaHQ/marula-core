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

pub(crate) use crate::auth::middleware::{authed, require_permission};
pub(crate) use crate::error::{ApiError, ApiResult};
pub(crate) use crate::instant::{self, InstantClaim, MutationOutcome};
pub(crate) use crate::org_tx;
pub(crate) use crate::state::AppState;
pub(crate) use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
pub(crate) use core_domain::entities::{
    BillingMethod, PhaseStatus, Project, ProjectPhase, ProjectStatus, ProjectTask, ProjectTemplate,
    ProjectView, TaskStatus,
};
pub(crate) use core_domain::events::types;
pub(crate) use core_domain::money::CurrencyCode;
pub(crate) use core_domain::policy::{Action, Module, Permission};
pub(crate) use serde::{Deserialize, Serialize};
pub(crate) use serde_json::Value;
pub(crate) use std::collections::HashMap;
pub(crate) use utoipa::ToSchema;
pub(crate) use uuid::Uuid;

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

/// A task's start and its length are one fact in two columns: a start with no
/// length is not a bar on a timeline, and a length with no start has nowhere to
/// sit. Neither is required — an undated task stays undated.
pub(crate) fn validate_task_time(has_start: bool, duration_minutes: Option<i32>) -> ApiResult<()> {
    match (has_start, duration_minutes) {
        (true, None) => Err(ApiError::Validation(
            "start_at and duration_minutes must be set together".into(),
        )),
        (false, Some(_)) => Err(ApiError::Validation(
            "duration_minutes requires start_at".into(),
        )),
        (true, Some(minutes)) if minutes < 0 => Err(ApiError::Validation(
            "duration_minutes must not be negative".into(),
        )),
        _ => Ok(()),
    }
}

/// Effort is a separate fact from duration: two hours of wall time can be ten
/// minutes of work. Both figures are optional, and neither may be negative.
pub(crate) fn validate_task_effort(estimate: Option<i32>, remaining: Option<i32>) -> ApiResult<()> {
    for (label, value) in [
        ("effort_estimate_minutes", estimate),
        ("effort_remaining_minutes", remaining),
    ] {
        if matches!(value, Some(v) if v < 0) {
            return Err(ApiError::Validation(format!(
                "{label} must not be negative"
            )));
        }
    }
    Ok(())
}

/// Normalises a skill list: trimmed, empty entries dropped, duplicates
/// removed case-insensitively, and the list bounded so a single task cannot
/// carry an unbounded document.
pub(crate) fn normalise_skills(skills: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(skills.len().min(32));
    for raw in skills {
        let skill = raw.trim();
        if skill.is_empty() || skill.chars().count() > 64 {
            continue;
        }
        if out.iter().any(|kept| kept.eq_ignore_ascii_case(skill)) {
            continue;
        }
        out.push(skill.to_string());
        if out.len() == 32 {
            break;
        }
    }
    out
}

pub(crate) fn default_limit() -> i64 {
    50
}

pub(crate) fn valid_currency(code: &str) -> bool {
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
pub(crate) fn validate_phase_dates(
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

pub(crate) fn validate_money_pair(
    minor: Option<i64>,
    currency: Option<&str>,
    label: &str,
) -> ApiResult<()> {
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
pub(crate) fn resolve_amount(
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

pub mod money;
pub mod plan;
pub mod risk;
pub mod tasks;
pub mod templates;

pub use money::*;
pub use plan::*;
pub use risk::*;
pub use tasks::*;
pub use templates::*;
