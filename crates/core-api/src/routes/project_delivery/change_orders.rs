//! Change orders: the document, its approval and its two separate acts.
//!
//! The pack's rule is the code's shape: approval changes the document, apply
//! changes the plan and captures a baseline, bill releases the money. Each step
//! is its own endpoint so no click can do two of them (S13.6), and a refused
//! release leaves an applied order in `billing_failed` with the reason
//! (S13.8).

use super::*;
use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::routes::projects::validate_money_pair;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

// --- wire shapes ------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct ChangeOrderView {
    pub id: Uuid,
    pub project_id: Uuid,
    pub reference: String,
    pub title: String,
    pub status: String,
    pub source: String,
    pub reason: String,
    pub scope_changes: Value,
    pub client_summary: Option<String>,
    pub evidence: Value,
    pub coverage_note: Option<String>,
    pub confidence: Option<String>,
    pub contract_clause: Option<String>,
    pub amount_minor: Option<i64>,
    pub currency: Option<String>,
    pub submitted_at: Option<chrono::DateTime<Utc>>,
    pub submitted_by: Option<Uuid>,
    pub decided_at: Option<chrono::DateTime<Utc>>,
    pub decided_by: Option<Uuid>,
    pub decision_note: Option<String>,
    pub applied_at: Option<chrono::DateTime<Utc>>,
    pub applied_by: Option<Uuid>,
    pub applied_baseline_id: Option<Uuid>,
    pub billed_at: Option<chrono::DateTime<Utc>>,
    pub billing_release_id: Option<Uuid>,
    pub billing_error: Option<String>,
    pub client_ack_required: bool,
    pub client_ack_at: Option<chrono::DateTime<Utc>>,
    pub created_by: Option<Uuid>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

impl From<core_db::project_delivery::ChangeOrder> for ChangeOrderView {
    fn from(co: core_db::project_delivery::ChangeOrder) -> Self {
        Self {
            id: co.id,
            project_id: co.project_id,
            reference: co.reference,
            title: co.title,
            status: co.status,
            source: co.source,
            reason: co.reason,
            scope_changes: co.scope_changes,
            client_summary: co.client_summary,
            evidence: co.evidence,
            coverage_note: co.coverage_note,
            confidence: co.confidence,
            contract_clause: co.contract_clause,
            amount_minor: co.amount_minor,
            currency: co.currency,
            submitted_at: co.submitted_at,
            submitted_by: co.submitted_by,
            decided_at: co.decided_at,
            decided_by: co.decided_by,
            decision_note: co.decision_note,
            applied_at: co.applied_at,
            applied_by: co.applied_by,
            applied_baseline_id: co.applied_baseline_id,
            billed_at: co.billed_at,
            billing_release_id: co.billing_release_id,
            billing_error: co.billing_error,
            client_ack_required: co.client_ack_required,
            client_ack_at: co.client_ack_at,
            created_by: co.created_by,
            created_at: co.created_at,
            updated_at: co.updated_at,
        }
    }
}

/// An absent optional document body is an empty object, not JSON null: the
/// validators and the apply path both read it as a map.
fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

// --- validation -------------------------------------------------------------

/// The documented scope-change shape. Unknown keys are kept (the document is
/// stored whole) but the two consumed collections are checked here, so a typo
/// in a task id refuses the draft instead of failing halfway through apply.
pub fn validate_scope_changes(value: &Value) -> ApiResult<()> {
    let Some(object) = value.as_object() else {
        return Err(ApiError::Validation(
            "scope_changes must be an object".into(),
        ));
    };
    if let Some(tasks) = object.get("tasks") {
        let Some(tasks) = tasks.as_array() else {
            return Err(ApiError::Validation(
                "scope_changes.tasks must be an array".into(),
            ));
        };
        for task in tasks {
            let title = task
                .get("title")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            if title.is_empty() {
                return Err(ApiError::Validation(
                    "every added task needs a title".into(),
                ));
            }
            if let Some(duration) = task.get("duration_minutes") {
                match duration.as_i64() {
                    Some(v) if v >= 0 => {}
                    _ => {
                        return Err(ApiError::Validation(
                            "duration_minutes must be a non-negative integer".into(),
                        ))
                    }
                }
            }
        }
    }
    if let Some(shifts) = object.get("shift_days") {
        let Some(shifts) = shifts.as_array() else {
            return Err(ApiError::Validation(
                "scope_changes.shift_days must be an array".into(),
            ));
        };
        for shift in shifts {
            let id = shift.get("task_id").and_then(Value::as_str).unwrap_or("");
            if Uuid::parse_str(id).is_err() {
                return Err(ApiError::Validation(
                    "every shift_days entry needs a task_id".into(),
                ));
            }
            if shift.get("days").and_then(Value::as_i64).is_none() {
                return Err(ApiError::Validation(
                    "every shift_days entry needs an integer days".into(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_confidence(value: Option<&str>) -> ApiResult<()> {
    if let Some(value) = value {
        if !matches!(value, "strong" | "fair" | "unsure") {
            return Err(ApiError::Validation(
                "confidence must be strong, fair or unsure".into(),
            ));
        }
    }
    Ok(())
}

fn validate_source(value: &str) -> ApiResult<()> {
    if !matches!(value, "manual" | "scope_signal" | "client_request") {
        return Err(ApiError::Validation(
            "source must be manual, scope_signal or client_request".into(),
        ));
    }
    Ok(())
}

fn reference_for(id: Uuid) -> String {
    let hex: String = id.simple().to_string().chars().take(6).collect();
    format!("CO-{}", hex.to_uppercase())
}

// --- read / create / edit ---------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateProjectChangeOrderRequest {
    pub title: String,
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default = "empty_object")]
    pub scope_changes: Value,
    pub client_summary: Option<String>,
    #[serde(default = "empty_object")]
    pub evidence: Value,
    pub coverage_note: Option<String>,
    pub confidence: Option<String>,
    pub contract_clause: Option<String>,
    pub amount_minor: Option<i64>,
    pub currency: Option<String>,
    #[serde(default)]
    pub client_ack_required: bool,
}

fn default_source() -> String {
    "manual".into()
}

/// GET /api/projects/{id}/change-orders
#[utoipa::path(operation_id = "project_change_orders_list", get, path = "/api/projects/{id}/change-orders", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [ChangeOrderView])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<ChangeOrderView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    core_db::projects::get_project(&mut *tx, org_id, id)
        .await
        .map_err(|e| match e {
            core_db::DbError::NotFound => ApiError::RlsSparse,
            other => other.into(),
        })?;
    let rows = core_db::project_delivery::list_change_orders(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows.into_iter().map(ChangeOrderView::from).collect()))
}

/// POST /api/projects/{id}/change-orders — an empty draft with three ways in.
#[utoipa::path(operation_id = "project_change_orders_create", post, path = "/api/projects/{id}/change-orders", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = CreateProjectChangeOrderRequest,
    responses((status = 200, body = ChangeOrderView),
              (status = 422, description = "validation failed")))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateProjectChangeOrderRequest>,
) -> ApiResult<Json<ChangeOrderView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let title = req.title.trim();
    if title.is_empty() {
        return Err(ApiError::Validation("title is required".into()));
    }
    validate_source(&req.source)?;
    validate_confidence(req.confidence.as_deref())?;
    validate_scope_changes(&req.scope_changes)?;
    validate_money_pair(req.amount_minor, req.currency.as_deref(), "amount")?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let id_new = Uuid::now_v7();
    let reference = reference_for(id_new);

    let row = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::projects::get_project(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| match e {
                core_db::DbError::NotFound => ApiError::RlsSparse,
                other => other.into(),
            })?;
        let row = core_db::project_delivery::insert_change_order(
            &mut ctx.tx,
            core_db::project_delivery::NewChangeOrder {
                id: id_new,
                org_id,
                project_id: id,
                reference: &reference,
                title,
                source: &req.source,
                reason: req.reason.trim(),
                scope_changes: &req.scope_changes,
                client_summary: req.client_summary.as_deref(),
                evidence: &req.evidence,
                coverage_note: req.coverage_note.as_deref(),
                confidence: req.confidence.as_deref(),
                contract_clause: req.contract_clause.as_deref(),
                amount_minor: req.amount_minor,
                currency: req.currency.as_deref(),
                client_ack_required: req.client_ack_required,
                created_by: Some(user_id),
            },
        )
        .await?;
        ctx.emit(
            "project.change_order_created",
            "project_change_order",
            Some(row.id.to_string().as_str()),
            &serde_json::json!({ "change_order_id": row.id, "project_id": id }),
        )
        .await?;
        Ok((ctx, row))
    })
    .await?;
    Ok(Json(ChangeOrderView::from(row)))
}

/// GET /api/projects/{id}/change-orders/{change_order_id}
#[utoipa::path(operation_id = "project_change_order_get", get, path = "/api/projects/{id}/change-orders/{change_order_id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("change_order_id" = Uuid, Path)),
    responses((status = 200, body = ChangeOrderView),
              (status = 404, description = "not visible in scope (rls_sparse)")))]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, change_order_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ChangeOrderView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let row = core_db::project_delivery::get_change_order(&mut *tx, org_id, id, change_order_id)
        .await
        .map_err(|e| match e {
            core_db::DbError::NotFound => ApiError::RlsSparse,
            other => other.into(),
        })?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(ChangeOrderView::from(row)))
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PatchChangeOrderRequest {
    pub title: Option<String>,
    pub reason: Option<String>,
    pub scope_changes: Option<Value>,
    pub client_summary: Option<Option<String>>,
    pub evidence: Option<Value>,
    pub coverage_note: Option<Option<String>>,
    pub confidence: Option<Option<String>>,
    pub contract_clause: Option<Option<String>>,
    pub amount_minor: Option<Option<i64>>,
    pub currency: Option<Option<String>>,
    pub client_ack_required: Option<bool>,
}

/// PATCH /api/projects/{id}/change-orders/{change_order_id} — drafts only.
#[utoipa::path(operation_id = "project_change_order_patch", patch, path = "/api/projects/{id}/change-orders/{change_order_id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("change_order_id" = Uuid, Path)), request_body = PatchChangeOrderRequest,
    responses((status = 200, body = ChangeOrderView),
              (status = 409, description = "the order is no longer a draft")))]
pub async fn patch(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, change_order_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<PatchChangeOrderRequest>,
) -> ApiResult<Json<ChangeOrderView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    if let Some(title) = req.title.as_deref() {
        if title.trim().is_empty() {
            return Err(ApiError::Validation("title must not be empty".into()));
        }
    }
    if let Some(value) = req.scope_changes.as_ref() {
        validate_scope_changes(value)?;
    }
    if let Some(confidence) = req.confidence.as_ref() {
        validate_confidence(confidence.as_deref())?;
    }
    if let Some(Some(amount)) = req.amount_minor {
        validate_money_pair(
            Some(amount),
            req.currency
                .as_ref()
                .and_then(|currency| currency.as_deref()),
            "amount",
        )?;
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let row = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let row = core_db::project_delivery::patch_change_order(
            &mut ctx.tx,
            org_id,
            id,
            change_order_id,
            core_db::project_delivery::ChangeOrderPatch {
                title: req.title.as_deref().map(str::trim),
                reason: req.reason.as_deref().map(str::trim),
                scope_changes: req.scope_changes.as_ref(),
                client_summary: req.client_summary.as_ref().map(|v| v.as_deref()),
                evidence: req.evidence.as_ref(),
                coverage_note: req.coverage_note.as_ref().map(|v| v.as_deref()),
                confidence: req.confidence.as_ref().map(|v| v.as_deref()),
                contract_clause: req.contract_clause.as_ref().map(|v| v.as_deref()),
                amount_minor: req.amount_minor,
                currency: req.currency.as_ref().map(|v| v.as_deref()),
                client_ack_required: req.client_ack_required,
            },
        )
        .await?;
        Ok((ctx, row))
    })
    .await?;
    Ok(Json(ChangeOrderView::from(row)))
}

// --- lifecycle --------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn transition(
    state: AppState,
    extensions: Extensions,
    project_id: Uuid,
    change_order_id: Uuid,
    from: &'static [&'static str],
    to: &'static str,
    note: Option<&str>,
    event: &'static str,
) -> ApiResult<Json<ChangeOrderView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let note = note.map(str::to_string);

    let row = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let row = core_db::project_delivery::transition_change_order(
            &mut ctx.tx,
            org_id,
            project_id,
            change_order_id,
            from,
            to,
            Some(user_id),
            note.as_deref(),
            "",
        )
        .await?;
        ctx.emit(
            event,
            "project_change_order",
            Some(row.id.to_string().as_str()),
            &serde_json::json!({ "change_order_id": row.id, "project_id": project_id, "status": to }),
        )
        .await?;
        Ok((ctx, row))
    })
    .await?;
    Ok(Json(ChangeOrderView::from(row)))
}

/// POST .../submit — the author's last editable moment.
#[utoipa::path(operation_id = "project_change_order_submit", post, path = "/api/projects/{id}/change-orders/{change_order_id}/submit", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("change_order_id" = Uuid, Path)),
    responses((status = 200, body = ChangeOrderView)))]
pub async fn submit(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, change_order_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ChangeOrderView>> {
    transition(
        state,
        extensions,
        id,
        change_order_id,
        &["draft"],
        "submitted",
        None,
        "project.change_order_submitted",
    )
    .await
}

#[derive(Debug, Deserialize, Serialize, ToSchema, Default)]
pub struct ProjectChangeOrderDecisionRequest {
    pub note: Option<String>,
}

/// POST .../approve — approval changes the document only. When the org asks for
/// portal acknowledgement the order waits for the client first (S13.9).
#[utoipa::path(operation_id = "project_change_order_approve", post, path = "/api/projects/{id}/change-orders/{change_order_id}/approve", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("change_order_id" = Uuid, Path)), request_body = ProjectChangeOrderDecisionRequest,
    responses((status = 200, body = ChangeOrderView)))]
pub async fn approve(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, change_order_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<ProjectChangeOrderDecisionRequest>,
) -> ApiResult<Json<ChangeOrderView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let row = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let existing =
            core_db::project_delivery::get_change_order(&mut *ctx.tx, org_id, id, change_order_id)
                .await?;
        let next = if existing.client_ack_required {
            "awaiting_client"
        } else {
            "approved"
        };
        let row = core_db::project_delivery::transition_change_order(
            &mut ctx.tx,
            org_id,
            id,
            change_order_id,
            &["submitted"],
            next,
            Some(user_id),
            req.note.as_deref(),
            "",
        )
        .await?;
        ctx.emit(
            "project.change_order_approved",
            "project_change_order",
            Some(row.id.to_string().as_str()),
            &serde_json::json!({ "change_order_id": row.id, "status": next }),
        )
        .await?;
        Ok((ctx, row))
    })
    .await?;
    Ok(Json(ChangeOrderView::from(row)))
}

/// POST .../reject — the reason is required and stays on the record.
#[utoipa::path(operation_id = "project_change_order_reject", post, path = "/api/projects/{id}/change-orders/{change_order_id}/reject", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("change_order_id" = Uuid, Path)), request_body = ProjectChangeOrderDecisionRequest,
    responses((status = 200, body = ChangeOrderView),
              (status = 422, description = "a rejection needs a reason")))]
pub async fn reject(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, change_order_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<ProjectChangeOrderDecisionRequest>,
) -> ApiResult<Json<ChangeOrderView>> {
    let note = req
        .note
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| ApiError::Validation("a rejection needs a reason".into()))?;
    transition(
        state,
        extensions,
        id,
        change_order_id,
        &["submitted"],
        "rejected",
        Some(note),
        "project.change_order_rejected",
    )
    .await
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApplyResponse {
    pub change_order: ChangeOrderView,
    pub tasks_added: i32,
    pub tasks_shifted: i32,
    pub baseline_id: Option<Uuid>,
}

/// POST .../apply — updates the plan and captures a baseline. It never bills.
#[utoipa::path(operation_id = "project_change_order_apply", post, path = "/api/projects/{id}/change-orders/{change_order_id}/apply", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("change_order_id" = Uuid, Path)),
    responses((status = 200, body = ApplyResponse),
              (status = 409, description = "not approved, or still waiting for the client")))]
pub async fn apply(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, change_order_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ApplyResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let (row, tasks_added, tasks_shifted, baseline_id) =
        org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
            let existing = core_db::project_delivery::get_change_order(
                &mut *ctx.tx,
                org_id,
                id,
                change_order_id,
            )
            .await?;
            if existing.status == "awaiting_client" {
                return Err(ApiError::Conflict(
                    "the client has not acknowledged this change yet".into(),
                ));
            }
            if existing.status != "approved" {
                return Err(ApiError::Conflict(
                    "only an approved change order can be applied".into(),
                ));
            }

            let mut added = 0i32;
            let mut shifted = 0i32;
            if let Some(tasks) = existing
                .scope_changes
                .get("tasks")
                .and_then(Value::as_array)
            {
                for (index, task) in tasks.iter().enumerate() {
                    let title = task
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("Change order task")
                        .trim()
                        .to_string();
                    let phase_id = task
                        .get("phase_id")
                        .and_then(Value::as_str)
                        .and_then(|s| Uuid::parse_str(s).ok());
                    let duration = task
                        .get("duration_minutes")
                        .and_then(Value::as_i64)
                        .map(|v| v as i32);
                    let assignee = task
                        .get("assignee_id")
                        .and_then(Value::as_str)
                        .and_then(|s| Uuid::parse_str(s).ok());
                    let effort = task
                        .get("effort_estimate_minutes")
                        .and_then(Value::as_i64)
                        .map(|v| v as i32);
                    core_db::projects::insert_task(
                        &mut *ctx.tx,
                        core_db::projects::NewTask {
                            id: Uuid::now_v7(),
                            org_id,
                            project_id: id,
                            phase_id,
                            title: &title,
                            description: None,
                            assignee_id: assignee,
                            sort_order: 10_000 + index as i32,
                            is_milestone: false,
                            milestone_amount_minor: None,
                            milestone_currency: None,
                            due_at: None,
                            start_at: None,
                            duration_minutes: duration,
                            effort_estimate_minutes: effort,
                            effort_remaining_minutes: effort,
                            work_type: None,
                            priority: None,
                            role_key: None,
                            required_skills: &[],
                            is_critical: false,
                            external_owner: None,
                            calendar_pin: None,
                        },
                    )
                    .await?;
                    added += 1;
                }
            }
            if let Some(shifts) = existing
                .scope_changes
                .get("shift_days")
                .and_then(Value::as_array)
            {
                for shift in shifts {
                    let Some(task_id) = shift
                        .get("task_id")
                        .and_then(Value::as_str)
                        .and_then(|s| Uuid::parse_str(s).ok())
                    else {
                        continue;
                    };
                    let days = shift.get("days").and_then(Value::as_i64).unwrap_or(0);
                    if days == 0 {
                        continue;
                    }
                    let task = core_db::projects::get_task(&mut *ctx.tx, org_id, task_id).await?;
                    if task.project_id != id {
                        return Err(ApiError::Validation(
                            "a shifted task does not belong to this project".into(),
                        ));
                    }
                    if let Some(start) = task.start_at {
                        core_db::projects::update_task(
                            &mut *ctx.tx,
                            org_id,
                            task_id,
                            core_db::projects::TaskPatch {
                                start_at: Some(Some(start + Duration::days(days))),
                                ..Default::default()
                            },
                        )
                        .await?;
                        shifted += 1;
                    }
                }
            }

            let baseline = core_db::projects::capture_baseline(
                &mut ctx.tx,
                org_id,
                id,
                Some(user_id),
                Some(&format!("Applied change order {}", existing.reference)),
            )
            .await?;

            let extra = format!(", applied_baseline_id = '{}'", baseline.id);
            let row = core_db::project_delivery::transition_change_order(
                &mut ctx.tx,
                org_id,
                id,
                change_order_id,
                &["approved"],
                "applied",
                Some(user_id),
                None,
                &extra,
            )
            .await?;
            ctx.emit(
                "project.change_order_applied",
                "project_change_order",
                Some(row.id.to_string().as_str()),
                &serde_json::json!({
                    "change_order_id": row.id,
                    "baseline_id": baseline.id,
                    "tasks_added": added,
                    "tasks_shifted": shifted,
                }),
            )
            .await?;
            Ok((ctx, (row, added, shifted, Some(baseline.id))))
        })
        .await?;

    Ok(Json(ApplyResponse {
        change_order: ChangeOrderView::from(row),
        tasks_added,
        tasks_shifted,
        baseline_id,
    }))
}

/// POST .../bill — releases the money through the existing billing seam. A
/// refused release leaves the order applied and records why (S13.8).
#[utoipa::path(operation_id = "project_change_order_bill", post, path = "/api/projects/{id}/change-orders/{change_order_id}/bill", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("change_order_id" = Uuid, Path)),
    responses((status = 200, body = ChangeOrderView),
              (status = 422, description = "the change order carries no amount")))]
pub async fn bill(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, change_order_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ChangeOrderView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let row = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let existing =
            core_db::project_delivery::get_change_order(&mut *ctx.tx, org_id, id, change_order_id)
                .await?;
        if existing.status != "applied" && existing.status != "billing_failed" {
            return Err(ApiError::Conflict(
                "only an applied change order can be billed".into(),
            ));
        }
        let (Some(amount), Some(currency)) = (existing.amount_minor, existing.currency.as_deref())
        else {
            return Err(ApiError::Validation(
                "this change order carries no amount, so there is nothing to release".into(),
            ));
        };

        let release = core_db::projects::insert_release(
            &mut *ctx.tx,
            &core_db::projects::NewRelease {
                org_id,
                project_id: id,
                phase_id: None,
                task_id: None,
                label: format!("{} {}", existing.reference, existing.title),
                amount_minor: amount,
                currency: currency.to_string(),
                released_on: Utc::now().date_naive(),
                source_event_id: None,
            },
        )
        .await;

        let row = match release {
            Ok(release) => {
                let extra = format!(
                    ", billing_release_id = '{}', billing_error = NULL",
                    release.id
                );
                core_db::project_delivery::transition_change_order(
                    &mut ctx.tx,
                    org_id,
                    id,
                    change_order_id,
                    &["applied", "billing_failed"],
                    "billed",
                    Some(user_id),
                    None,
                    &extra,
                )
                .await?
            }
            Err(error) => {
                let message = error.to_string().replace('\'', "''");
                let extra = format!(
                    ", billing_error = '{}'",
                    message.chars().take(400).collect::<String>()
                );
                core_db::project_delivery::transition_change_order(
                    &mut ctx.tx,
                    org_id,
                    id,
                    change_order_id,
                    &["applied", "billing_failed"],
                    "billing_failed",
                    Some(user_id),
                    None,
                    &extra,
                )
                .await?
            }
        };
        ctx.emit(
            "project.change_order_billed",
            "project_change_order",
            Some(row.id.to_string().as_str()),
            &serde_json::json!({ "change_order_id": row.id, "status": row.status }),
        )
        .await?;
        Ok((ctx, row))
    })
    .await?;
    Ok(Json(ChangeOrderView::from(row)))
}
