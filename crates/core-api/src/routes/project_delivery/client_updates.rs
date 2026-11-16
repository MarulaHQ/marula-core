//! Client status updates: assembled from records, approved by a person, sent by
//! a person.
//!
//! The draft is deterministic (completed work in the period, nothing else), so
//! the same week produces the same proposal. Withheld is a real answer: when
//! nothing moved the record says so and keeps the reason (S2.12).

use super::*;
use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
pub struct ClientUpdateView {
    pub id: Uuid,
    pub project_id: Uuid,
    pub status: String,
    pub body: String,
    pub facts: Value,
    pub period_start: Option<NaiveDate>,
    pub period_end: Option<NaiveDate>,
    pub withheld_reason: Option<String>,
    pub approved_by: Option<Uuid>,
    pub approved_at: Option<DateTime<Utc>>,
    pub sent_at: Option<DateTime<Utc>>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<core_db::project_delivery::ClientUpdate> for ClientUpdateView {
    fn from(update: core_db::project_delivery::ClientUpdate) -> Self {
        Self {
            id: update.id,
            project_id: update.project_id,
            status: update.status,
            body: update.body,
            facts: update.facts,
            period_start: update.period_start,
            period_end: update.period_end,
            withheld_reason: update.withheld_reason,
            approved_by: update.approved_by,
            approved_at: update.approved_at,
            sent_at: update.sent_at,
            created_by: update.created_by,
            created_at: update.created_at,
            updated_at: update.updated_at,
        }
    }
}

/// The deterministic prose. It names what moved, in the order a client reads
/// it, and never mentions money, hours or staff names. `None` movement is an
/// honest sentence rather than an empty document.
pub fn compose_body(project: &str, movements: &[(&str, &str)], closing: Option<&str>) -> String {
    let mut body = String::new();
    body.push_str(&format!("Update on {project}.\n\n"));
    if movements.is_empty() {
        body.push_str("No movement since the last update. The next milestone is unchanged.\n");
    } else {
        body.push_str("What moved since the last update:\n");
        for (kind, name) in movements {
            body.push_str(&format!("- {kind}: {name}\n"));
        }
    }
    if let Some(closing) = closing {
        body.push_str(&format!("\n{closing}\n"));
    }
    body
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct DraftUpdateRequest {
    pub period_start: Option<NaiveDate>,
    pub period_end: Option<NaiveDate>,
    /// What to say about what comes next, when the PM wants to add it. The
    /// draft never invents a promise.
    pub closing: Option<String>,
}

/// GET /api/projects/{id}/client-updates
#[utoipa::path(operation_id = "project_client_updates_list", get, path = "/api/projects/{id}/client-updates", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [ClientUpdateView])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<ClientUpdateView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::project_delivery::list_client_updates(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows.into_iter().map(ClientUpdateView::from).collect()))
}

/// POST /api/projects/{id}/client-updates — drafts from what moved in the
/// period. With no movement it records a withheld update with the reason and
/// returns it; it never produces a cheerful empty letter.
#[utoipa::path(operation_id = "project_client_updates_draft", post, path = "/api/projects/{id}/client-updates", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = DraftUpdateRequest,
    responses((status = 200, body = ClientUpdateView),
              (status = 422, description = "the period ends before it starts")))]
pub async fn draft(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<DraftUpdateRequest>,
) -> ApiResult<Json<ClientUpdateView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let period_end = req.period_end.unwrap_or_else(|| Utc::now().date_naive());
    let period_start = req
        .period_start
        .unwrap_or(period_end - chrono::Duration::days(7));
    if period_end < period_start {
        return Err(ApiError::Validation(
            "period_end must not precede period_start".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let closing = req
        .closing
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());

    let update = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let project = core_db::projects::get_project(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| match e {
                core_db::DbError::NotFound => ApiError::RlsSparse,
                other => other.into(),
            })?;
        let tasks = core_db::projects::list_tasks(
            &mut *ctx.tx,
            org_id,
            id,
            core_db::projects::TaskFilter {
                phase_id: None,
                status: None,
                assignee_id: None,
                milestone: None,
                limit: 200,
                offset: 0,
            },
        )
        .await?;
        let phases = core_db::projects::list_phases(&mut *ctx.tx, org_id, id).await?;
        let change_orders =
            core_db::project_delivery::list_change_orders(&mut *ctx.tx, org_id, id).await?;

        let since = period_start
            .and_hms_opt(0, 0, 0)
            .map(|t| t.and_utc())
            .unwrap_or_else(Utc::now);

        let mut movements: Vec<(String, String)> = Vec::new();
        let mut facts: Vec<Value> = Vec::new();
        for phase in &phases {
            if let Some(done) = phase.completed_at {
                if done >= since {
                    movements.push(("Phase complete".into(), phase.name.clone()));
                    facts.push(serde_json::json!({
                        "kind": "phase", "id": phase.id, "name": phase.name,
                    }));
                }
            }
        }
        for task in &tasks {
            if let Some(done) = task.completed_at {
                if done >= since {
                    movements.push(("Work complete".into(), task.title.clone()));
                    facts.push(serde_json::json!({
                        "kind": "task", "id": task.id, "name": task.title,
                    }));
                }
            }
        }
        for co in &change_orders {
            if matches!(co.status.as_str(), "applied" | "billed") && co.updated_at >= since {
                movements.push(("Change agreed".into(), co.title.clone()));
                facts.push(serde_json::json!({
                    "kind": "change_order", "id": co.id, "reference": co.reference,
                    "name": co.title,
                }));
            }
        }

        let borrowed: Vec<(&str, &str)> = movements
            .iter()
            .map(|(kind, name)| (kind.as_str(), name.as_str()))
            .collect();
        if movements.is_empty() {
            let withheld_body = compose_body(&project.name, &[], None);
            let update = core_db::project_delivery::insert_client_update(
                &mut ctx.tx,
                core_db::project_delivery::NewClientUpdate {
                    id: Uuid::now_v7(),
                    org_id,
                    project_id: id,
                    status: "withheld",
                    body: &withheld_body,
                    facts: &Value::Array(facts),
                    period_start: Some(period_start),
                    period_end: Some(period_end),
                    withheld_reason: Some(
                        "Nothing moved in this period, so there is nothing to tell the client",
                    ),
                    created_by: Some(user_id),
                },
            )
            .await?;
            return Ok((ctx, update));
        }

        let draft_body = compose_body(&project.name, &borrowed, closing);
        let update = core_db::project_delivery::insert_client_update(
            &mut ctx.tx,
            core_db::project_delivery::NewClientUpdate {
                id: Uuid::now_v7(),
                org_id,
                project_id: id,
                status: "draft",
                body: &draft_body,
                facts: &Value::Array(facts),
                period_start: Some(period_start),
                period_end: Some(period_end),
                withheld_reason: None,
                created_by: Some(user_id),
            },
        )
        .await?;
        Ok((ctx, update))
    })
    .await?;
    Ok(Json(ClientUpdateView::from(update)))
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct EditUpdateRequest {
    pub body: String,
}

/// PATCH /api/projects/{id}/client-updates/{update_id} — drafts only.
#[utoipa::path(operation_id = "project_client_updates_edit", patch, path = "/api/projects/{id}/client-updates/{update_id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("update_id" = Uuid, Path)), request_body = EditUpdateRequest,
    responses((status = 200, body = ClientUpdateView),
              (status = 422, description = "the body is empty")))]
pub async fn edit(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, update_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<EditUpdateRequest>,
) -> ApiResult<Json<ClientUpdateView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let body = req.body.trim();
    if body.is_empty() {
        return Err(ApiError::Validation(
            "an update cannot be sent with an empty body".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let update = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let update = core_db::project_delivery::update_client_update_body(
            &mut ctx.tx,
            org_id,
            id,
            update_id,
            body,
        )
        .await?;
        Ok((ctx, update))
    })
    .await?;
    Ok(Json(ClientUpdateView::from(update)))
}

/// POST .../approve
#[utoipa::path(operation_id = "project_client_updates_approve", post, path = "/api/projects/{id}/client-updates/{update_id}/approve", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("update_id" = Uuid, Path)),
    responses((status = 200, body = ClientUpdateView)))]
pub async fn approve(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, update_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ClientUpdateView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let update = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let update = core_db::project_delivery::approve_client_update(
            &mut ctx.tx,
            org_id,
            id,
            update_id,
            user_id,
        )
        .await?;
        Ok((ctx, update))
    })
    .await?;
    Ok(Json(ClientUpdateView::from(update)))
}

/// POST .../send — the send is the person's act. It is recorded here as the
/// state change; the delivery channel consumes the sent row, so no model can
/// put words in front of a client.
#[utoipa::path(operation_id = "project_client_updates_send", post, path = "/api/projects/{id}/client-updates/{update_id}/send", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("update_id" = Uuid, Path)),
    responses((status = 200, body = ClientUpdateView),
              (status = 409, description = "the update was not approved")))]
pub async fn send(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, update_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ClientUpdateView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let update = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let update =
            core_db::project_delivery::send_client_update(&mut ctx.tx, org_id, id, update_id)
                .await?;
        ctx.emit(
            "project.client_update_sent",
            "project_client_update",
            Some(update.id.to_string().as_str()),
            &serde_json::json!({ "project_id": id, "update_id": update.id }),
        )
        .await?;
        Ok((ctx, update))
    })
    .await?;
    Ok(Json(ClientUpdateView::from(update)))
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct WithholdUpdateRequest {
    pub reason: String,
}

/// POST .../withhold — records the honest refusal and why.
#[utoipa::path(operation_id = "project_client_updates_withhold", post, path = "/api/projects/{id}/client-updates/{update_id}/withhold", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("update_id" = Uuid, Path)), request_body = WithholdUpdateRequest,
    responses((status = 200, body = ClientUpdateView),
              (status = 422, description = "a reason is required")))]
pub async fn withhold(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, update_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<WithholdUpdateRequest>,
) -> ApiResult<Json<ClientUpdateView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let reason = req.reason.trim();
    if reason.is_empty() {
        return Err(ApiError::Validation(
            "say why the update is withheld".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let update = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let update = core_db::project_delivery::withhold_client_update(
            &mut ctx.tx,
            org_id,
            id,
            update_id,
            reason,
        )
        .await?;
        Ok((ctx, update))
    })
    .await?;
    Ok(Json(ClientUpdateView::from(update)))
}
