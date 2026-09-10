//! Tags (Wave 11 lane 9, migration 0056): org tag taxonomy CRUD +
//! per-ticket application + the ticket-list filter seam.
//!
//! Tags are flat org-wide labels (unique per name, optional color). CRUD
//! is admin-gated (`ManageBoards`); applying/removing on a ticket needs
//! the ticket Update matrix permission (board-level RBAC still applies
//! through the ticket's board). The ticket list integrates filtering via
//! `tickets::list_tickets`'s tag clause (AND across requested ids).
//! Mutations ride the IF1–IF3 instant-first envelope; every mutation
//! emits its contract event + audit row in the same commit.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::board_access::authorize_board_op;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    routing::{delete, get, patch},
    Json, Router,
};
use core_domain::entities::{Actor, AuditEvent, Tag};
use core_domain::events::types;
use core_domain::policy::{BoardOp, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

pub fn routes(gen: crate::routes::ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(&format!("{base}/tags"), get(list_tags).post(create_tag))
        .route(
            &format!("{base}/tags/{{id}}"),
            patch(update_tag).delete(delete_tag),
        )
        .route(
            &format!("{base}/tickets/{{id}}/tags"),
            get(list_ticket_tags).post(apply_tag),
        )
        .route(
            &format!("{base}/tickets/{{id}}/tags/{{tag_id}}"),
            delete(remove_tag),
        )
}

fn manage_boards() -> Permission {
    Permission::ManageBoards
}

fn tickets(action: core_domain::policy::Action) -> Permission {
    Permission::Matrix {
        module: Module::Tickets,
        action,
    }
}

const NAME_MAX: usize = 80;
const COLOR_MAX: usize = 32;

fn validate_name(name: &str) -> ApiResult<String> {
    let trimmed = name.trim().to_string();
    if trimmed.is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    if trimmed.len() > NAME_MAX {
        return Err(ApiError::Validation(format!(
            "name must be at most {NAME_MAX} characters"
        )));
    }
    Ok(trimmed)
}

/// One tag + how many tickets currently carry it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TagView {
    pub tag: Tag,
    pub applied_count: i64,
}

impl From<Tag> for TagView {
    fn from(tag: Tag) -> Self {
        Self {
            tag,
            applied_count: 0,
        }
    }
}

#[derive(Debug, Serialize, ToSchema, serde::Deserialize)]
pub struct TagMutationResponse {
    pub tag: TagView,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct CreateTagRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct UpdateTagRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    pub name: Option<String>,
    /// Some → set (null clears); absent → no change.
    #[serde(default)]
    pub color: Option<Option<String>>,
}

/// GET /api/tags — the org's tag taxonomy with application counts.
#[utoipa::path(operation_id = "tags_list", get, path = "/api/tags", tag = "tags", security(("bearer" = [])),
    responses((status = 200, body = [TagView])))]
pub async fn list_tags(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<TagView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let tags = core_db::tags::list_tags(&mut *tx, org_id).await?;
    let counts: std::collections::HashMap<Uuid, i64> =
        core_db::tags::tag_application_counts(&mut *tx, org_id)
            .await?
            .into_iter()
            .collect();
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        tags.into_iter()
            .map(|t| TagView {
                applied_count: counts.get(&t.id).copied().unwrap_or(0),
                tag: t,
            })
            .collect(),
    ))
}

/// POST /api/tags — create a tag.
#[utoipa::path(operation_id = "tags_create", post, path = "/api/tags", tag = "tags", security(("bearer" = [])),
    request_body = CreateTagRequest,
    responses((status = 200, body = TagMutationResponse),
              (status = 409, description = "duplicate tag name or idempotency key reused")))]
pub async fn create_tag(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateTagRequest>,
) -> ApiResult<Json<TagMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_boards())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let name = validate_name(&req.name)?;
    if let Some(c) = req.color.as_deref() {
        if c.len() > COLOR_MAX {
            return Err(ApiError::Validation(format!(
                "color must be at most {COLOR_MAX} characters"
            )));
        }
    }
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "tag",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let id = Uuid::now_v7();
        core_db::tags::insert_tag(
            &mut *ctx.tx,
            &core_db::tags::NewTag {
                id,
                org_id,
                name: &name,
                color: req.color.as_deref(),
                created_by: Some(user_id),
            },
        )
        .await
        .map_err(|e| match e {
            core_db::DbError::Sql(sqlx::Error::Database(db)) if db.constraint().is_some() => {
                ApiError::Conflict("a tag with this name already exists".into())
            }
            other => other.into(),
        })?;
        let tag = core_db::tags::get_tag(&mut *ctx.tx, org_id, id).await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "tag.created".into(),
                entity_type: Some("tag".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "name": tag.name }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            types::TAG_CREATED,
            "tag",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TagCreated { tag: tag.clone() })
                .unwrap_or_default(),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "tag",
                entity_id: id,
                version: None,
            },
        )
        .await?;
        let view = TagView {
            tag: tag.clone(),
            applied_count: 0,
        };
        let stored = TagMutationResponse {
            tag: view.clone(),
            mutation: instant::MutationMeta {
                seq: write_seq,
                version: write_seq,
                updated_at: chrono::Utc::now(),
                before: None,
                after: Value::Null,
                idempotency_key: None,
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "tag",
                entity_id: id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&tag).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &stored,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TagMutationResponse {
                tag: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/tags/{id} — rename / recolor.
#[utoipa::path(operation_id = "tags_update", patch, path = "/api/tags/{id}", tag = "tags", security(("bearer" = [])),
    request_body = UpdateTagRequest,
    responses((status = 200, body = TagMutationResponse),
              (status = 404, description = "tag not found"),
              (status = 409, description = "version conflict or duplicate name")))]
pub async fn update_tag(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTagRequest>,
) -> ApiResult<Json<TagMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_boards())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let name = req.name.as_deref().map(validate_name).transpose()?;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "tag",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::tags::get_tag(&mut *ctx.tx, org_id, id).await?;
        core_db::tags::update_tag(
            &mut *ctx.tx,
            org_id,
            id,
            name.as_deref(),
            req.color.as_ref().map(|c| c.as_deref()),
        )
        .await?;
        let after = core_db::tags::get_tag(&mut *ctx.tx, org_id, id).await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "tag.updated".into(),
                entity_type: Some("tag".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "name": after.name }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            types::TAG_UPDATED,
            "tag",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TagUpdated { tag: after.clone() })
                .unwrap_or_default(),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "tag",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let view = TagView {
            tag: after.clone(),
            applied_count: 0,
        };
        let before_json = serde_json::to_value(&before).unwrap_or(Value::Null);
        let stored = TagMutationResponse {
            tag: view.clone(),
            mutation: instant::MutationMeta {
                seq: write_seq,
                version: write_seq,
                updated_at: chrono::Utc::now(),
                before: Some(before_json.clone()),
                after: Value::Null,
                idempotency_key: None,
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "tag",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(before_json),
                after: serde_json::to_value(&after).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &stored,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TagMutationResponse {
                tag: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/tags/{id} — removes the tag and every ticket application.
#[utoipa::path(operation_id = "tags_delete", delete, path = "/api/tags/{id}", tag = "tags", security(("bearer" = [])),
    responses((status = 200, body = Value),
              (status = 404, description = "tag not found")))]
pub async fn delete_tag(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_boards())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let before = core_db::tags::get_tag(&mut *ctx.tx, org_id, id).await?;
        let applied_count = core_db::tags::delete_tag(&mut ctx.tx, org_id, id).await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "tag.deleted".into(),
                entity_type: Some("tag".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "name": before.name, "applied_count": applied_count }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            types::TAG_DELETED,
            "tag",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TagDeleted {
                tag_id: id,
                name: before.name.clone(),
                applied_count,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, serde_json::json!({ "deleted": true })))
    })
    .await
    .map(Json)
}

// --- per-ticket application ------------------------------------------------------

/// GET /api/tickets/{id}/tags — the ticket's applied tags.
#[utoipa::path(operation_id = "tickets_list_tags", get, path = "/api/tickets/{id}/tags", tag = "tags", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [Tag])))]
pub async fn list_ticket_tags(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<Tag>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let ticket = core_db::tickets::get_ticket(&mut *tx, org_id, id).await?;
    authorize_board_op(&mut tx, &authed.principal, ticket.board_id, BoardOp::View).await?;
    let tags = core_db::tags::list_ticket_tags(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(tags))
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct ApplyTagRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    pub tag_id: Uuid,
}

/// POST /api/tickets/{id}/tags — apply a tag to a ticket (idempotent).
#[utoipa::path(operation_id = "tickets_apply_tag", post, path = "/api/tickets/{id}/tags", tag = "tags", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ApplyTagRequest,
    responses((status = 200, body = Value),
              (status = 404, description = "ticket or tag not found"),
              (status = 409, description = "idempotency key reused")))]
pub async fn apply_tag(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ApplyTagRequest>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "ticket_tag",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            ticket.board_id,
            BoardOp::WorkTicket,
        )
        .await?;
        let tag = core_db::tags::get_tag(&mut *ctx.tx, org_id, req.tag_id).await?;
        core_db::tags::apply_tag(&mut *ctx.tx, org_id, id, req.tag_id, Some(user_id)).await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "tag.applied".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "tag_id": req.tag_id, "tag_name": tag.name }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            types::TICKET_TAGS_CHANGED,
            "ticket",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketTagsChanged {
                ticket_id: id,
                board_id: ticket.board_id,
                tag_id: req.tag_id,
                applied: true,
                changed_by: Some(user_id),
            })
            .unwrap_or_default(),
        )
        .await?;
        let outcome = MutationOutcome::Applied(serde_json::json!({ "applied": true }));
        Ok((ctx, outcome))
    })
    .await?
    .into_response()
    .map(Json)
}

/// DELETE /api/tickets/{id}/tags/{tag_id} — remove one applied tag.
#[utoipa::path(operation_id = "tickets_remove_tag", delete, path = "/api/tickets/{id}/tags/{tag_id}", tag = "tags", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("tag_id" = Uuid, Path)),
    responses((status = 200, body = Value),
              (status = 404, description = "ticket or tag not in scope")))]
pub async fn remove_tag(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, tag_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            ticket.board_id,
            BoardOp::WorkTicket,
        )
        .await?;
        let _removed = core_db::tags::remove_tag(&mut *ctx.tx, org_id, id, tag_id).await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "tag.removed".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "tag_id": tag_id }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            types::TICKET_TAGS_CHANGED,
            "ticket",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketTagsChanged {
                ticket_id: id,
                board_id: ticket.board_id,
                tag_id,
                applied: false,
                changed_by: Some(user_id),
            })
            .unwrap_or_default(),
        )
        .await?;
        let outcome = MutationOutcome::Applied(serde_json::json!({ "removed": true }));
        Ok((ctx, outcome))
    })
    .await?
    .into_response()
    .map(Json)
}
