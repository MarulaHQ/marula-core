//! The portal's project view: plain words, four steps, the date the client
//! cares about, and the updates that were actually emailed.
//!
//! Nothing internal crosses this boundary: no hours, no budget, no rates, no
//! technician names. The portal theme and navigation belong to the web app; this
//! file is the read and the one client act (acknowledging a change order).

use super::*;
use crate::auth::PortalAuthed;
use crate::error::{ApiError, ApiResult};
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

fn portal_authed(extensions: &Extensions) -> ApiResult<PortalAuthed> {
    extensions
        .get::<PortalAuthed>()
        .cloned()
        .ok_or_else(|| ApiError::Unauthorized("not authenticated".into()))
}

/// The client's words for a project status. Never the staff vocabulary.
fn plain_status(status: &str) -> &'static str {
    match status {
        "planning" => "Being planned",
        "active" => "In progress",
        "on_hold" => "On hold",
        "completed" => "Completed",
        _ => "In progress",
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PortalProjectSummary {
    pub id: Uuid,
    pub name: String,
    pub status: String,
    pub status_text: String,
    pub due_date: Option<NaiveDate>,
    pub latest_update_at: Option<DateTime<Utc>>,
    pub waiting_for_you: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PortalProjectList {
    pub projects: Vec<PortalProjectSummary>,
}

/// GET /api/portal/projects
#[utoipa::path(operation_id = "portal_projects_list", get, path = "/api/portal/projects", tag = "portal", security(("bearer" = [])),
    responses((status = 200, body = PortalProjectList)))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<PortalProjectList>> {
    let authed = portal_authed(&extensions)?;
    let org_id = authed.user.org_id;
    let client_id = authed.user.client_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::project_delivery::list_client_projects(&mut *tx, org_id, client_id).await?;
    let mut projects = Vec::with_capacity(rows.len());
    for (id, name, status, _start, due) in rows {
        let updates = core_db::project_delivery::list_client_updates(&mut *tx, org_id, id).await?;
        let latest = updates
            .iter()
            .filter(|u| u.status == "sent")
            .map(|u| u.sent_at.unwrap_or(u.created_at))
            .max();
        let orders = core_db::project_delivery::list_change_orders(&mut *tx, org_id, id).await?;
        let waiting = orders.iter().any(|co| {
            co.client_ack_required
                && co.client_ack_at.is_none()
                && matches!(co.status.as_str(), "approved" | "awaiting_client")
        });
        projects.push(PortalProjectSummary {
            id,
            name,
            status: status.clone(),
            status_text: plain_status(&status).to_string(),
            due_date: due,
            latest_update_at: latest,
            waiting_for_you: waiting,
        });
    }
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(PortalProjectList { projects }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PortalStep {
    pub label: String,
    pub status: String,
    pub status_text: String,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PortalUpdate {
    pub id: Uuid,
    pub sent_at: Option<DateTime<Utc>>,
    pub body: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PortalChangeOrder {
    pub id: Uuid,
    pub reference: String,
    pub title: String,
    pub summary: Option<String>,
    pub amount_minor: Option<i64>,
    pub currency: Option<String>,
    pub changes_date: bool,
    pub acknowledged_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PortalProjectView {
    pub id: Uuid,
    pub name: String,
    pub status: String,
    pub status_text: String,
    /// A client-safe reason, only when the project is on hold.
    pub hold_reason: Option<String>,
    pub due_date: Option<NaiveDate>,
    pub steps: Vec<PortalStep>,
    pub updates: Vec<PortalUpdate>,
    pub change_orders: Vec<PortalChangeOrder>,
    /// False when the plan has not been shared yet (S16.7).
    pub plan_shared: bool,
}

/// GET /api/portal/projects/{id}
#[utoipa::path(operation_id = "portal_projects_get", get, path = "/api/portal/projects/{id}", tag = "portal", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = PortalProjectView),
              (status = 404, description = "not the caller's project")))]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<PortalProjectView>> {
    let authed = portal_authed(&extensions)?;
    let org_id = authed.user.org_id;
    let client_id = authed.user.client_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    let project = core_db::projects::get_project(&mut *tx, org_id, id)
        .await
        .map_err(|_| ApiError::NotFound)?;
    if project.client_id != Some(client_id) {
        return Err(ApiError::NotFound);
    }
    let phases = core_db::projects::list_phases(&mut *tx, org_id, id).await?;
    let updates = core_db::project_delivery::list_client_updates(&mut *tx, org_id, id).await?;
    let orders = core_db::project_delivery::list_change_orders(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let plan_shared = phases.iter().any(|p| p.start_date.is_some());
    let steps: Vec<PortalStep> = if phases.len() <= 4 {
        phases
            .iter()
            .map(|phase| PortalStep {
                label: phase.name.clone(),
                status: phase.status.as_str().to_string(),
                status_text: plain_phase(phase.status.as_str()),
                start_date: phase.start_date,
                end_date: phase.end_date,
            })
            .collect()
    } else {
        // Eighteen tasks are not a client's business; four steps are. The
        // buckets keep order and never hide a phase: the label carries the
        // count it folded in.
        let chunk = phases.len().div_ceil(4);
        phases
            .chunks(chunk)
            .map(|group| {
                let first = &group[0];
                let completed = group.iter().all(|p| p.status.as_str() == "completed");
                let status = if completed {
                    "completed"
                } else {
                    first.status.as_str()
                };
                PortalStep {
                    label: if group.len() == 1 {
                        first.name.clone()
                    } else {
                        format!("{} and {} more", first.name, group.len() - 1)
                    },
                    status: status.to_string(),
                    status_text: plain_phase(status),
                    start_date: group.iter().filter_map(|p| p.start_date).min(),
                    end_date: group.iter().filter_map(|p| p.end_date).max(),
                }
            })
            .collect()
    };

    Ok(Json(PortalProjectView {
        id: project.id,
        name: project.name,
        status: project.status.as_str().to_string(),
        status_text: plain_status(project.status.as_str()).to_string(),
        hold_reason: if project.status.as_str() == "on_hold" {
            Some(
                "Work is paused while the remaining access is arranged. Your contact will confirm the new date."
                    .into(),
            )
        } else {
            None
        },
        due_date: project.due_date,
        steps,
        updates: updates
            .into_iter()
            .filter(|u| u.status == "sent")
            .map(|u| PortalUpdate {
                id: u.id,
                sent_at: u.sent_at,
                body: u.body,
            })
            .collect(),
        change_orders: orders
            .into_iter()
            .filter(|co| {
                co.client_ack_required
                    && matches!(co.status.as_str(), "approved" | "awaiting_client")
            })
            .map(|co| PortalChangeOrder {
                id: co.id,
                reference: co.reference,
                title: co.title,
                summary: co.client_summary,
                amount_minor: co.amount_minor,
                currency: co.currency,
                changes_date: co
                    .scope_changes
                    .get("shift_days")
                    .and_then(|v| v.as_array())
                    .is_some_and(|v| !v.is_empty()),
                acknowledged_at: co.client_ack_at,
            })
            .collect(),
        plan_shared,
    }))
}

fn plain_phase(status: &str) -> String {
    match status {
        "pending" => "Not started".into(),
        "in_progress" => "Under way".into(),
        "blocked" => "Waiting on a decision".into(),
        "completed" => "Done".into(),
        "cancelled" => "Not needed".into(),
        other => other.to_string(),
    }
}

/// POST /api/portal/projects/{id}/change-orders/{change_order_id}/acknowledge
#[utoipa::path(operation_id = "portal_project_change_order_acknowledge", post, path = "/api/portal/projects/{id}/change-orders/{change_order_id}/acknowledge", tag = "portal", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("change_order_id" = Uuid, Path)),
    responses((status = 200, body = serde_json::Value),
              (status = 404, description = "not the caller's project")))]
pub async fn acknowledge(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, change_order_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = portal_authed(&extensions)?;
    let org_id = authed.user.org_id;
    let client_id = authed.user.client_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    let project = core_db::projects::get_project(&mut *tx, org_id, id)
        .await
        .map_err(|_| ApiError::NotFound)?;
    if project.client_id != Some(client_id) {
        return Err(ApiError::NotFound);
    }
    core_db::project_delivery::acknowledge_change_order(&mut tx, org_id, id, change_order_id, None)
        .await
        .map_err(|e| match e {
            core_db::DbError::Conflict(message) => ApiError::Conflict(message),
            _ => ApiError::NotFound,
        })?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({ "acknowledged": true })))
}
