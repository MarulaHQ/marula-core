//! Reply snippets — the canned-response library for staff composition
//! (Wave 34 lane 3, `p34-canned-templates`).
//!
//! Purpose: the Stage 4 service-desk close-out. A snippet is a short
//! reusable reply body staff insert into the ticket composer or a
//! client-visible reply (the portal reply path renders the result).
//! Storage is the settings domain (scope `org`, one typed row per
//! snippet — per-board scoping is a field on the snippet, so no table);
//! variable tokens reuse the template registry's closed ticket
//! vocabulary and render engine. Provenance rides the templates
//! fork-on-edit pattern: `fork` duplicates a snippet and records
//! `forked_from` on the copy; the origin is untouched.
//!
//! Permissions: reads behind `tickets:view` OR `admin:view` (staff compose
//! with the library); create/fork/patch/delete behind the admin matrix
//! (matching the template registry). Mutations ride the IF1–IF3
//! instant-first envelope and emit `reply_snippet.*` events + audit rows
//! in the same commit.
//!
//! Non-purpose: ticket TEMPLATES (`/api/v1/templates`) stay a separate
//! typed library — snippets never merge into them; no client/portal
//! composition surface (snippets are staff-only).

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
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use core_domain::policy::{check_any, Action, Module, Permission};
use core_domain::reply_snippets::{self, ReplySnippet};
use core_domain::templates as tmpl;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

const ENTITY: &str = "reply_snippet";

fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

fn tickets(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Tickets,
        action,
    }
}

fn require_staff_reader(principal: &core_domain::entities::Principal) -> ApiResult<()> {
    check_any(
        &principal.permissions,
        &[tickets(Action::View), admin(Action::View)],
    )
    .expect()
    .map_err(ApiError::Forbidden)
}

// --- wire structs ------------------------------------------------------------

/// One canned reply in the org's snippet library.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReplySnippetView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    pub name: String,
    pub body: String,
    /// Boards the snippet is offered on. Empty = every board.
    #[schema(value_type = Vec<uuid::Uuid>)]
    pub board_ids: Vec<Uuid>,
    /// Fork-on-edit provenance: the origin snippet this copy was forked
    /// from, when it is a fork.
    #[schema(value_type = Option<uuid::Uuid>, nullable = true)]
    pub forked_from: Option<Uuid>,
    #[schema(value_type = uuid::Uuid)]
    pub updated_by: Uuid,
    #[schema(value_type = String)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<ReplySnippet> for ReplySnippetView {
    fn from(s: ReplySnippet) -> Self {
        Self {
            id: s.id,
            name: s.name,
            body: s.body,
            board_ids: s.board_ids,
            forked_from: s.forked_from,
            updated_by: s.updated_by,
            updated_at: s.updated_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ReplySnippetListQuery {
    /// Offer-board filter: only snippets scoped to every board or this
    /// board (the composer passes the ticket's board).
    #[schema(value_type = Option<uuid::Uuid>)]
    pub board_id: Option<Uuid>,
    /// Search-in-composer predicate: case-insensitive substring over the
    /// snippet's name and body.
    #[serde(default)]
    pub q: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReplySnippetsListResponse {
    pub snippets: Vec<ReplySnippetView>,
    /// The closed variable vocabulary snippets may use (ticket context).
    pub vocabulary: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateReplySnippetRequest {
    pub name: String,
    pub body: String,
    #[serde(default)]
    #[schema(value_type = Vec<uuid::Uuid>)]
    pub board_ids: Vec<Uuid>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateReplySnippetRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    #[schema(value_type = Option<Vec<uuid::Uuid>>)]
    pub board_ids: Option<Vec<Uuid>>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ForkReplySnippetRequest {
    pub name: String,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Instant-first mutation response (IF2): the snippet at the top level
/// plus the additive `mutation` metadata.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReplySnippetMutationResponse {
    #[serde(flatten)]
    pub snippet: ReplySnippetView,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RenderReplySnippetRequest {
    /// The ticket the variables resolve from.
    #[schema(value_type = uuid::Uuid)]
    pub context_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RenderedReplySnippet {
    pub body: String,
}

// --- helpers ---------------------------------------------------------------

fn build_snippet(
    id: Uuid,
    name: &str,
    body: &str,
    board_ids: Vec<Uuid>,
    actor: Uuid,
) -> ApiResult<ReplySnippet> {
    let snippet = ReplySnippet {
        id,
        name: name.trim().to_string(),
        body: body.to_string(),
        board_ids,
        forked_from: None,
        updated_by: actor,
        updated_at: chrono::Utc::now(),
    };
    core_domain::reply_snippets::to_settings_value(&snippet)
        .map(|_| snippet)
        .map_err(|e| ApiError::Validation(e.to_string()))
}

fn render_error(e: tmpl::RenderError) -> ApiError {
    match e {
        tmpl::RenderError::UnknownVariable(key, _) => ApiError::Validation(format!(
            "unknown variable {{{{{key}}}}} — snippets render from the ticket vocabulary"
        )),
        other => ApiError::Validation(other.to_string()),
    }
}

// --- routes -----------------------------------------------------------------

/// GET /api/v1/reply-snippets — the org's snippet library with the
/// search-in-composer predicate applied (`board_id` offer filter + `q`
/// case-insensitive substring over name and body).
#[utoipa::path(get, path = "/api/reply-snippets", tag = "admin", security(("bearer" = [])),
    params(ReplySnippetListQuery),
    responses((status = 200, body = ReplySnippetsListResponse),
              (status = 403, description = "requires tickets:view or admin:view")))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(query): Query<ReplySnippetListQuery>,
) -> ApiResult<Json<ReplySnippetsListResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_staff_reader(&authed.principal)?;
    let org_id = authed.principal.user.org_id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let all = core_db::reply_snippets::list(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let snippets: Vec<ReplySnippetView> = all
        .into_iter()
        .filter(|s| reply_snippets::matches(s, query.board_id, query.q.as_deref().unwrap_or("")))
        .map(ReplySnippetView::from)
        .collect();
    Ok(Json(ReplySnippetsListResponse {
        snippets,
        vocabulary: reply_snippets::variable_vocabulary()
            .iter()
            .map(|s| s.to_string())
            .collect(),
    }))
}

/// GET /api/v1/reply-snippets/{id} — fetch one snippet (cross-lane fix,
/// Wave 34 lane 4: the fork-on-edit flow reads the origin after forking;
/// the route was missing so reads 405'd).
#[utoipa::path(get, path = "/api/reply-snippets/{id}", tag = "admin", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = ReplySnippetView),
              (status = 404),
              (status = 403, description = "requires tickets:view or admin:view")))]
pub async fn get_by_id(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ReplySnippetView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_staff_reader(&authed.principal)?;
    let org_id = authed.principal.user.org_id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let snippet = core_db::reply_snippets::get(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(ReplySnippetView::from(snippet)))
}

/// POST /api/v1/reply-snippets — add a snippet to the library.
#[utoipa::path(post, path = "/api/reply-snippets", tag = "admin", security(("bearer" = [])),
    request_body = CreateReplySnippetRequest,
    responses((status = 200, body = ReplySnippetMutationResponse),
              (status = 403, description = "requires admin:create"),
              (status = 422, description = "validation failure (name/body, or out-of-vocabulary variable)")))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateReplySnippetRequest>,
) -> ApiResult<Json<ReplySnippetMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec { entity: ENTITY, entity_id: None, key: key.as_deref(), fingerprint: &fingerprint },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let snippet = build_snippet(Uuid::now_v7(), &req.name, &req.body, req.board_ids.clone(), actor_id)?;
        core_db::reply_snippets::upsert(&mut ctx.tx, org_id, &snippet).await?;

        ctx.emit(
            types::REPLY_SNIPPET_CREATED,
            ENTITY,
            Some(snippet.id.to_string().as_str()),
            &serde_json::json!({ "snippet_id": snippet.id, "name": snippet.name, "boards": snippet.board_ids.len() }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(actor_id),
                action: "reply_snippet.created".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(snippet.id.to_string()),
                detail: serde_json::json!({ "name": snippet.name, "board_ids": snippet.board_ids }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec { entity: ENTITY, entity_id: snippet.id, version: None },
        )
        .await?;
        let view = ReplySnippetView::from(snippet);
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: view.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((ctx, MutationOutcome::Applied(ReplySnippetMutationResponse { snippet: view, mutation })))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/v1/reply-snippets/{id}/fork — duplicate a snippet into a new
/// editable copy (templates fork-on-edit provenance pattern). The origin
/// is untouched; the copy records `forked_from`.
#[utoipa::path(post, path = "/api/reply-snippets/{id}/fork", tag = "admin", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ForkReplySnippetRequest,
    responses((status = 200, body = ReplySnippetMutationResponse),
              (status = 403, description = "requires admin:create"),
              (status = 404, description = "snippet not visible in scope"),
              (status = 422, description = "validation failure")))]
pub async fn fork(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ForkReplySnippetRequest>,
) -> ApiResult<Json<ReplySnippetMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Create))?;
    let name = req.name.trim().to_string();
    if name.is_empty() || name.len() > reply_snippets::NAME_MAX {
        return Err(ApiError::Validation(format!(
            "snippet name must be 1..={} characters",
            reply_snippets::NAME_MAX
        )));
    }
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec { entity: ENTITY, entity_id: Some(id), key: key.as_deref(), fingerprint: &fingerprint },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let origin = core_db::reply_snippets::get(&mut *ctx.tx, org_id, id)
            .await
            .map_err(ApiError::from)
            .map_err(instant::mutation_scope_err)?;
        let mut copy = origin.clone();
        copy.id = Uuid::now_v7();
        copy.name = name;
        copy.forked_from = Some(origin.id);
        copy.updated_by = actor_id;
        copy.updated_at = chrono::Utc::now();
        core_db::reply_snippets::upsert(&mut ctx.tx, org_id, &copy).await?;

        ctx.emit(
            types::REPLY_SNIPPET_FORKED,
            ENTITY,
            Some(copy.id.to_string().as_str()),
            &serde_json::json!({ "snippet_id": copy.id, "from_snippet_id": origin.id, "name": copy.name }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(actor_id),
                action: "reply_snippet.forked".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(copy.id.to_string()),
                detail: serde_json::json!({ "name": copy.name, "from_snippet_id": origin.id }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec { entity: ENTITY, entity_id: copy.id, version: None },
        )
        .await?;
        let view = ReplySnippetView::from(copy);
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: view.id,
                op: "fork",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((ctx, MutationOutcome::Applied(ReplySnippetMutationResponse { snippet: view, mutation })))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/v1/reply-snippets/{id} — edit a snippet in place (the
/// library is org-owned data; fork-on-edit applies when orgs want a
/// divergent copy, via the fork endpoint).
#[utoipa::path(patch, path = "/api/reply-snippets/{id}", tag = "admin", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateReplySnippetRequest,
    responses((status = 200, body = ReplySnippetMutationResponse),
              (status = 403, description = "requires admin:edit"),
              (status = 404, description = "snippet not visible in scope"),
              (status = 409, description = "version gate rejected the write (IF3)"),
              (status = 422, description = "validation failure")))]
pub async fn update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateReplySnippetRequest>,
) -> ApiResult<Json<ReplySnippetMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::reply_snippets::get(&mut *ctx.tx, org_id, id)
            .await
            .map_err(ApiError::from)
            .map_err(instant::mutation_scope_err)?;
        let mut after = before.clone();
        if let Some(name) = &req.name {
            after.name = name.trim().to_string();
        }
        if let Some(body) = &req.body {
            after.body = body.clone();
        }
        if let Some(board_ids) = &req.board_ids {
            after.board_ids = board_ids.clone();
        }
        after.forked_from = before.forked_from;
        after.updated_by = actor_id;
        after.updated_at = chrono::Utc::now();
        core_domain::reply_snippets::to_settings_value(&after)
            .map_err(|e| ApiError::Validation(e.to_string()))?;
        core_db::reply_snippets::upsert(&mut ctx.tx, org_id, &after).await?;

        ctx.emit(
            types::REPLY_SNIPPET_UPDATED,
            ENTITY,
            Some(after.id.to_string().as_str()),
            &serde_json::json!({ "snippet_id": after.id, "name": after.name }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(actor_id),
                action: "reply_snippet.updated".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(after.id.to_string()),
                detail: serde_json::json!({ "name": after.name }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: after.id,
                version: None,
            },
        )
        .await?;
        let view = ReplySnippetView::from(after);
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: view.id,
                op: "update",
                seq: write_seq,
                before: Some(
                    serde_json::to_value(ReplySnippetView::from(before)).unwrap_or(Value::Null),
                ),
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ReplySnippetMutationResponse {
                snippet: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/v1/reply-snippets/{id} — remove a snippet from the library.
#[utoipa::path(delete, path = "/api/reply-snippets/{id}", tag = "admin", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ReplySnippetMutationResponse),
              (status = 403, description = "requires admin:delete"),
              (status = 404, description = "snippet not visible in scope")))]
pub async fn delete(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ReplySnippetMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        let before = core_db::reply_snippets::get(&mut *ctx.tx, org_id, id)
            .await
            .map_err(ApiError::from)
            .map_err(instant::mutation_scope_err)?;
        core_db::reply_snippets::delete(&mut ctx.tx, org_id, id).await?;

        ctx.emit(
            types::REPLY_SNIPPET_REMOVED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::json!({ "snippet_id": id, "name": before.name }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(actor_id),
                action: "reply_snippet.removed".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "name": before.name }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: None,
            },
        )
        .await?;
        let view = ReplySnippetView::from(before);
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: view.id,
                op: "delete",
                seq: write_seq,
                before: Some(serde_json::to_value(&view).unwrap_or(Value::Null)),
                after: Value::Null,
                updated_at: chrono::Utc::now(),
                key: None,
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ReplySnippetMutationResponse {
                snippet: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/v1/reply-snippets/{id}/render — resolve the snippet's
/// variable tokens against a real ticket context and return the rendered
/// body (the composer inserts the RENDERED text; the portal reply renders
/// the result). Out-of-vocabulary placeholders are typed 400s.
#[utoipa::path(post, path = "/api/reply-snippets/{id}/render", tag = "admin", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = RenderReplySnippetRequest,
    responses((status = 200, body = RenderedReplySnippet),
              (status = 400, description = "out-of-vocabulary or malformed variable"),
              (status = 403, description = "requires tickets:view or admin:view"),
              (status = 404, description = "snippet or ticket not visible in scope")))]
pub async fn render(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<RenderReplySnippetRequest>,
) -> ApiResult<Json<RenderedReplySnippet>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_staff_reader(&authed.principal)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let snippet = core_db::reply_snippets::get(&mut *tx, org_id, id).await?;
    let context =
        core_db::templates::ticket_render_context(&mut *tx, org_id, req.context_id, user_id)
            .await?;
    tx.commit().await.map_err(ApiError::from)?;

    let (_, body) = tmpl::render_template(
        core_domain::entities::templates::TemplateKind::Ticket,
        None,
        &snippet.body,
        &context.to_map(),
    )
    .map_err(render_error)?;
    Ok(Json(RenderedReplySnippet { body }))
}
