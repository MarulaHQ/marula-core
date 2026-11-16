//! Task pods: typed blockers, promotion to a ticket, personal my-work, and the
//! workbench's pod documents.
//!
//! `Blocked` is an action with a type and an owner (S3.9); `Waiting on` is a
//! dependency and stays its own state (S10.4). Promotion creates one ticket and
//! one link, and asking twice returns the first (S10.7).

use super::*;
use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use chrono::Utc;
use core_domain::status::Priority;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// The pods a project task workbench can show. Stable keys, because a person's
/// saved order names them.
pub const POD_KEYS: [&str; 11] = [
    "status",
    "chain",
    "needs",
    "notes",
    "composer",
    "timer",
    "time",
    "schedule",
    "materials",
    "client",
    "links",
];

fn validate_pods(pods: &[String]) -> ApiResult<()> {
    for pod in pods {
        if !POD_KEYS.contains(&pod.as_str()) {
            return Err(ApiError::Validation(format!(
                "unknown pod \"{pod}\"; expected one of {}",
                POD_KEYS.join(", ")
            )));
        }
    }
    Ok(())
}

// --- blockers ---------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SetBlockerRequest {
    /// `client`, `task`, `vendor`, `access`, `parts` or `other`.
    pub kind: String,
    pub note: Option<String>,
    pub owner_id: Option<Uuid>,
}

/// PUT /api/projects/{id}/tasks/{task_id}/blocker
#[utoipa::path(operation_id = "project_task_blocker_set", put, path = "/api/projects/{id}/tasks/{task_id}/blocker", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("task_id" = Uuid, Path)), request_body = SetBlockerRequest,
    responses((status = 200, body = serde_json::Value),
              (status = 422, description = "unknown blocker kind")))]
pub async fn set_blocker(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, task_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<SetBlockerRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    const KINDS: [&str; 6] = ["client", "task", "vendor", "access", "parts", "other"];
    if !KINDS.contains(&req.kind.as_str()) {
        return Err(ApiError::Validation(format!(
            "blocker kind must be one of {}",
            KINDS.join(", ")
        )));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let updated = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let changed = core_db::project_delivery::set_task_blocker(
            &mut ctx.tx,
            org_id,
            id,
            task_id,
            core_db::project_delivery::TaskBlocker {
                kind: &req.kind,
                note: req.note.as_deref().map(str::trim),
                owner_id: req.owner_id,
            },
        )
        .await?;
        if !changed {
            return Err(ApiError::RlsSparse);
        }
        Ok((ctx, true))
    })
    .await?;
    Ok(Json(serde_json::json!({ "blocked": updated })))
}

/// DELETE /api/projects/{id}/tasks/{task_id}/blocker
#[utoipa::path(operation_id = "project_task_blocker_clear", delete, path = "/api/projects/{id}/tasks/{task_id}/blocker", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("task_id" = Uuid, Path)),
    responses((status = 200, body = serde_json::Value)))]
pub async fn clear_blocker(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, task_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let cleared = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let changed =
            core_db::project_delivery::clear_task_blocker(&mut ctx.tx, org_id, id, task_id).await?;
        Ok((ctx, changed))
    })
    .await?;
    Ok(Json(serde_json::json!({ "cleared": cleared })))
}

// --- promotion --------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PromoteRequest {
    pub board_id: Option<Uuid>,
    /// `p1`..`p4`; defaults to the task's own priority, then `p3`.
    pub priority: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PromoteResponse {
    pub task_id: Uuid,
    pub project_id: Uuid,
    pub ticket_id: Uuid,
    pub ticket_number: String,
    /// False when the task already had a ticket; the first ticket is returned.
    pub created: bool,
}

/// POST /api/projects/{id}/tasks/{task_id}/promote
#[utoipa::path(operation_id = "project_task_promote", post, path = "/api/projects/{id}/tasks/{task_id}/promote", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("task_id" = Uuid, Path)), request_body = PromoteRequest,
    responses((status = 200, body = PromoteResponse),
              (status = 422, description = "no board exists for the ticket")))]
pub async fn promote_task(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, task_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<PromoteRequest>,
) -> ApiResult<Json<PromoteResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    if let Some(priority) = req.priority.as_deref() {
        priority
            .parse::<Priority>()
            .map_err(|e| ApiError::Validation(e.to_string()))?;
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let response = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let task = core_db::projects::get_task(&mut *ctx.tx, org_id, task_id)
            .await
            .map_err(|e| match e {
                core_db::DbError::NotFound => ApiError::RlsSparse,
                other => other.into(),
            })?;
        if task.project_id != id {
            return Err(ApiError::RlsSparse);
        }

        // One task, one ticket: a second promotion returns the first.
        if let Some(link) =
            core_db::project_delivery::link_exists_for_task(&mut *ctx.tx, org_id, task_id).await?
        {
            let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, link.ticket_id).await?;
            return Ok((
                ctx,
                PromoteResponse {
                    task_id,
                    project_id: id,
                    ticket_id: ticket.id,
                    ticket_number: ticket.number.to_string(),
                    created: false,
                },
            ));
        }

        let board = match req.board_id {
            Some(board_id) => core_db::boards::get_board(&mut *ctx.tx, org_id, board_id).await?,
            None => {
                let boards = core_db::boards::list_boards(&mut *ctx.tx, org_id).await?;
                boards
                    .iter()
                    .find(|b| b.is_default)
                    .or_else(|| boards.first())
                    .cloned()
                    .ok_or_else(|| {
                        ApiError::Validation(
                            "no ticket board exists, so this task cannot be promoted".into(),
                        )
                    })?
            }
        };
        let status = core_db::boards::default_status_for_board(&mut *ctx.tx, board.id).await?;
        let (prefix, seq) = core_db::orgs::next_ticket_number(&mut *ctx.tx, org_id).await?;
        let number = format!("{prefix}-{seq}");
        let priority = match req.priority.as_deref() {
            Some(value) => value
                .parse::<Priority>()
                .map_err(|e| ApiError::Validation(e.to_string()))?,
            None => task.priority.unwrap_or(Priority::P3),
        };
        let project = core_db::projects::get_project(&mut *ctx.tx, org_id, id).await?;

        let ticket_id = Uuid::now_v7();
        let ticket = core_db::tickets::insert_ticket(
            &mut ctx.tx,
            core_db::tickets::NewTicket {
                id: ticket_id,
                org_id,
                number: &number,
                board_id: board.id,
                status_id: status.id,
                title: &task.title,
                body: task.description.as_deref(),
                priority,
                client_id: project.client_id,
                site_id: None,
                contact_id: None,
                assignee_id: task.assignee_id,
                requester_id: Some(user_id),
                due_at: task.due_at,
            },
        )
        .await?;
        core_db::project_delivery::insert_ticket_link(
            &mut ctx.tx,
            org_id,
            id,
            task_id,
            ticket.id,
            Some(user_id),
        )
        .await?;
        ctx.emit(
            "project.task_promoted",
            "project_task",
            Some(task_id.to_string().as_str()),
            &serde_json::json!({
                "task_id": task_id, "project_id": id, "ticket_id": ticket.id,
            }),
        )
        .await?;
        Ok((
            ctx,
            PromoteResponse {
                task_id,
                project_id: id,
                ticket_id: ticket.id,
                ticket_number: ticket.number.to_string(),
                created: true,
            },
        ))
    })
    .await?;
    Ok(Json(response))
}

// --- pod settings -----------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct PodSettingsView {
    pub surface: String,
    pub role_key: String,
    pub pods: Vec<String>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
pub struct SurfaceQuery {
    #[serde(default = "default_surface")]
    pub surface: String,
}

fn default_surface() -> String {
    "project_task".into()
}

/// GET /api/project-pods/settings?surface=project_task
#[utoipa::path(operation_id = "project_pod_settings", get, path = "/api/project-pods/settings", tag = "projects", security(("bearer" = [])),
    params(("surface" = Option<String>, Query)),
    responses((status = 200, body = [PodSettingsView])))]
pub async fn settings(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(query): Query<SurfaceQuery>,
) -> ApiResult<Json<Vec<PodSettingsView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows =
        core_db::project_delivery::list_pod_settings(&mut *tx, org_id, &query.surface).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        rows.into_iter()
            .map(|row| PodSettingsView {
                surface: row.surface,
                role_key: row.role_key,
                pods: row.pods,
                updated_at: row.updated_at,
            })
            .collect(),
    ))
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PutPodSettingsRequest {
    pub role_key: String,
    pub pods: Vec<String>,
    #[serde(default = "default_surface")]
    pub surface: String,
}

/// PUT /api/project-pods/settings — the matrix is an org document.
#[utoipa::path(operation_id = "project_pod_settings_put", put, path = "/api/project-pods/settings", tag = "projects", security(("bearer" = [])),
    request_body = PutPodSettingsRequest,
    responses((status = 200, body = PodSettingsView),
              (status = 422, description = "unknown pod")))]
pub async fn put_settings(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PutPodSettingsRequest>,
) -> ApiResult<Json<PodSettingsView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let role_key = req.role_key.trim();
    if role_key.is_empty() {
        return Err(ApiError::Validation("role_key is required".into()));
    }
    validate_pods(&req.pods)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let row = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let row = core_db::project_delivery::upsert_pod_settings(
            &mut ctx.tx,
            org_id,
            &req.surface,
            role_key,
            &req.pods,
            user_id,
        )
        .await?;
        Ok((ctx, row))
    })
    .await?;
    Ok(Json(PodSettingsView {
        surface: row.surface,
        role_key: row.role_key,
        pods: row.pods,
        updated_at: row.updated_at,
    }))
}

// --- personal prefs ---------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct PodPrefsView {
    pub surface: String,
    pub hidden: Vec<String>,
    pub pod_order: Vec<String>,
}

/// GET /api/project-pods/prefs
#[utoipa::path(operation_id = "project_pod_prefs", get, path = "/api/project-pods/prefs", tag = "projects", security(("bearer" = [])),
    params(("surface" = Option<String>, Query)),
    responses((status = 200, body = PodPrefsView)))]
pub async fn prefs(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(query): Query<SurfaceQuery>,
) -> ApiResult<Json<PodPrefsView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let row =
        core_db::project_delivery::get_pod_prefs(&mut *tx, org_id, user_id, &query.surface).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(match row {
        Some(row) => PodPrefsView {
            surface: row.surface,
            hidden: row.hidden,
            pod_order: row.pod_order,
        },
        None => PodPrefsView {
            surface: query.surface,
            hidden: Vec::new(),
            pod_order: Vec::new(),
        },
    }))
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PutPodPrefsRequest {
    #[serde(default = "default_surface")]
    pub surface: String,
    #[serde(default)]
    pub hidden: Vec<String>,
    #[serde(default)]
    pub pod_order: Vec<String>,
}

/// PUT /api/project-pods/prefs
#[utoipa::path(operation_id = "project_pod_prefs_put", put, path = "/api/project-pods/prefs", tag = "projects", security(("bearer" = [])),
    request_body = PutPodPrefsRequest,
    responses((status = 200, body = PodPrefsView)))]
pub async fn put_prefs(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PutPodPrefsRequest>,
) -> ApiResult<Json<PodPrefsView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    validate_pods(&req.hidden)?;
    validate_pods(&req.pod_order)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let surface = req.surface.clone();
    let hidden = req.hidden.clone();
    let pod_order = req.pod_order.clone();
    let response = PodPrefsView {
        surface: surface.clone(),
        hidden: hidden.clone(),
        pod_order: pod_order.clone(),
    };
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::project_delivery::upsert_pod_prefs(
            &mut ctx.tx,
            org_id,
            user_id,
            &surface,
            &hidden,
            &pod_order,
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(response))
}

// --- my work ----------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct MyTaskView {
    pub task_id: Uuid,
    pub project_id: Uuid,
    pub project_name: String,
    pub phase_name: Option<String>,
    pub title: String,
    pub status: String,
    pub blocker_type: Option<String>,
    pub blocker_note: Option<String>,
    pub blocked_since: Option<chrono::DateTime<Utc>>,
    pub due_at: Option<chrono::DateTime<Utc>>,
    pub start_at: Option<chrono::DateTime<Utc>>,
    pub duration_minutes: Option<i32>,
    pub effort_remaining_minutes: Option<i32>,
    pub work_type: Option<String>,
    pub is_milestone: bool,
    pub ticket_id: Option<Uuid>,
}

/// GET /api/projects/my-work — the project-shaped day, without opening a
/// project. Waiting (`blocked`) is its own state and says what it waits on.
#[utoipa::path(operation_id = "project_my_work", get, path = "/api/projects/my-work", tag = "projects", security(("bearer" = [])),
    responses((status = 200, body = [MyTaskView])))]
pub async fn my_work(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<MyTaskView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::project_delivery::list_my_tasks(&mut *tx, org_id, user_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        rows.into_iter()
            .map(|row| MyTaskView {
                task_id: row.task_id,
                project_id: row.project_id,
                project_name: row.project_name,
                phase_name: row.phase_name,
                title: row.title,
                status: row.status,
                blocker_type: row.blocker_type,
                blocker_note: row.blocker_note,
                blocked_since: row.blocked_at,
                due_at: row.due_at,
                start_at: row.start_at,
                duration_minutes: row.duration_minutes,
                effort_remaining_minutes: row.effort_remaining_minutes,
                work_type: row.work_type,
                is_milestone: row.is_milestone,
                ticket_id: row.ticket_id,
            })
            .collect(),
    ))
}
