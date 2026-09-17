//! Client-portal HTTP surface (portal v1): login/refresh with a DEDICATED
//! token family, own-client ticket list/detail, and replies that always
//! create `client_visible` notes under portal-tier audit provenance.
//!
//! Purpose: the only endpoints portal users may call. Every authenticated
//! route sits behind `require_portal_auth` (kind:"portal" JWT + fresh
//! per-request user load) and every read/write is scoped to the caller's
//! client_id on top of org RLS. Comment visibility flows through
//! `core_domain::notes::visible_kinds` — the it_manager tier widens to
//! in-house updates; `internal` is unreachable from ANY portal role.
//!
//! Non-purpose: staff-side administration of portal users (that is
//! `routes::portal_admin`, behind the staff permission matrix); no MFA for
//! portal v1 (documented deferral — TOTP/WebAuthn enrollment is a staff
//! feature); no ticket creation or status changes from the portal.

use crate::auth::jwt;
use crate::auth::PortalAuthed;
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{header::AUTHORIZATION, Extensions},
    middleware::Next,
    Json,
};
use core_domain::entities::{Actor, AuditEvent, PortalRole, TicketComment};
use core_domain::events::types;
use core_domain::policy::check_portal;
use core_domain::status::{NoteKind, TicketStatusCategory};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Axum middleware for /api/portal/*: verifies the PORTAL token family and
/// loads the caller fresh per request. Staff tokens fail verification by
/// family, disabled/missing users fail the fresh load — revocation is
/// instant.
pub async fn require_portal_auth(
    State(state): State<crate::state::AppState>,
    mut request: axum::extract::Request,
    next: Next,
) -> Result<axum::response::Response, ApiError> {
    let token = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError::Unauthorized("missing bearer token".into()))?
        .to_string();

    let claims = jwt::verify_portal_access_token(&state.config, &token)
        .ok_or_else(|| ApiError::Unauthorized("invalid or expired token".into()))?;
    let (portal_user_id, org_id, client_id, session_id) = claims;

    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, portal_user_id)
        .await
        .map_err(ApiError::from)?;
    let user = core_db::portal::get_portal_user(&mut *tx, org_id, portal_user_id)
        .await
        .map_err(|_| ApiError::Unauthorized("portal user not found".into()))?;
    tx.commit().await.map_err(ApiError::from)?;
    if user.status != core_domain::entities::PortalUserStatus::Active {
        return Err(ApiError::Unauthorized("account disabled".into()));
    }
    // Defense in depth: the claim's client binding must match the stored row.
    if user.client_id != client_id {
        return Err(ApiError::Unauthorized("client mismatch".into()));
    }

    request.extensions_mut().insert(PortalAuthed {
        user,
        session_id: Some(session_id),
    });
    Ok(next.run(request).await)
}

fn portal_actor(role: PortalRole, id: Uuid) -> Actor {
    match role {
        PortalRole::Contact => Actor::PortalContact(id),
        PortalRole::Approver => Actor::PortalApprover(id),
        PortalRole::ItManager => Actor::PortalItManager(id),
    }
}

// --- auth ---------------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct PortalLoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PortalLoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub session_id: Uuid,
    pub expires_in: i64,
    pub portal_user: PortalProfile,
}

/// The signed-in portal identity (also served by GET /api/portal/me).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PortalProfile {
    pub id: Uuid,
    pub org_id: Uuid,
    pub client_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: PortalRole,
}

/// POST /api/portal/login — argon2 verify against `portal_users`. No MFA in
/// portal v1 (documented deferral). Disabled accounts resolve to nothing →
/// uniform 401, never revealing which half failed.
#[utoipa::path(operation_id = "portal_login", post, path = "/api/portal/login", tag = "portal",
    request_body = PortalLoginRequest, responses(
        (status = 200, body = PortalLoginResponse),
        (status = 401, description = "bad credentials")
))]
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<PortalLoginRequest>,
) -> ApiResult<Json<PortalLoginResponse>> {
    // Narrow SECURITY DEFINER bridge (migration 0021): identity only.
    let resolved: Option<(Uuid, Uuid, Uuid)> =
        sqlx::query_as("SELECT portal_user_id, org_id, client_id FROM portal_resolve_login($1)")
            .bind(&req.email)
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
    let (user_id, org_id, _client_id) =
        resolved.ok_or_else(|| ApiError::Unauthorized("invalid credentials".into()))?;

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, org_id).await?;
    let creds = core_db::portal::find_portal_credentials_by_email(&mut *tx, org_id, &req.email)
        .await?
        .filter(|c| c.id == user_id)
        .ok_or_else(|| ApiError::Unauthorized("invalid credentials".into()))?;
    if !crate::auth::password::verify(&req.password, &creds.password_hash) {
        audit_best_effort(
            &state,
            org_id,
            portal_actor(creds.role, creds.id),
            "portal.login_failed",
            Some("portal_user"),
            Some(creds.id.to_string()),
            serde_json::json!({}),
        )
        .await;
        return Err(ApiError::Unauthorized("invalid credentials".into()));
    }
    // First successful login activates an invited identity.
    core_db::portal::activate_on_first_login(&mut *tx, org_id, creds.id).await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let identity = PortalIdentity {
        id: creds.id,
        org_id,
        client_id: creds.client_id,
        email: creds.email.clone(),
        display_name: creds.display_name.clone(),
        role: creds.role,
    };
    let response = issue_portal_tokens(&state, &identity, None).await?;
    audit_best_effort(
        &state,
        org_id,
        portal_actor(creds.role, creds.id),
        "portal.login",
        Some("portal_user"),
        Some(creds.id.to_string()),
        serde_json::json!({ "method": "password" }),
    )
    .await;
    Ok(Json(response))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PortalRefreshRequest {
    pub refresh_token: String,
    pub session_id: Uuid,
}

/// POST /api/portal/refresh — rotation within the portal session family.
/// A presented token resolves ONLY via its live session row; rotation
/// revokes that row, so replaying an old refresh token fails.
#[utoipa::path(operation_id = "portal_refresh", post, path = "/api/portal/refresh", tag = "portal",
    request_body = PortalRefreshRequest, responses((status = 200, body = PortalLoginResponse)))]
pub async fn refresh(
    State(state): State<AppState>,
    Json(req): Json<PortalRefreshRequest>,
) -> ApiResult<Json<PortalLoginResponse>> {
    let hash = jwt::hash_refresh_token(&req.refresh_token);
    let resolved: Option<(Uuid, Uuid, Uuid)> = sqlx::query_as(
        "SELECT portal_user_id, org_id, client_id FROM portal_resolve_session($1, $2)",
    )
    .bind(req.session_id)
    .bind(hash)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;
    let (user_id, org_id, client_id) =
        resolved.ok_or_else(|| ApiError::Unauthorized("invalid refresh token".into()))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut rtx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut rtx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let user = core_db::portal::get_portal_user(&mut *rtx, org_id, user_id)
        .await
        .map_err(|_| ApiError::Unauthorized("invalid refresh token".into()))?;
    rtx.commit().await.map_err(ApiError::from)?;
    if user.status != core_domain::entities::PortalUserStatus::Active {
        return Err(ApiError::Unauthorized("account disabled".into()));
    }

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, org_id).await?;
    // Retire the presented session BEFORE minting its successor.
    core_db::portal::revoke_session(&mut *tx, req.session_id).await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let identity = PortalIdentity {
        id: user.id,
        org_id: user.org_id,
        client_id,
        email: user.email.clone(),
        display_name: user.display_name.clone(),
        role: user.role,
    };
    let response = issue_portal_tokens(&state, &identity, None).await?;
    Ok(Json(response))
}

/// Everything the token issuer needs to mint both halves of the pair and
/// echo the profile back.
struct PortalIdentity {
    id: Uuid,
    org_id: Uuid,
    client_id: Uuid,
    email: String,
    display_name: String,
    role: PortalRole,
}

async fn issue_portal_tokens(
    state: &AppState,
    identity: &PortalIdentity,
    user_agent: Option<&str>,
) -> ApiResult<PortalLoginResponse> {
    let session_id = Uuid::now_v7();
    let refresh = jwt::new_refresh_token();
    let refresh_hash = jwt::hash_refresh_token(&refresh);
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(state.config.refresh_ttl_secs);
    // RLS-scoped write (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, identity.org_id, identity.id)
        .await
        .map_err(ApiError::from)?;
    core_db::portal::create_session(
        &mut *tx,
        session_id,
        identity.org_id,
        identity.id,
        &refresh_hash,
        user_agent,
        expires_at,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    let access = jwt::issue_portal_access_token(
        &state.config,
        identity.id,
        identity.org_id,
        identity.client_id,
        session_id,
    )
    .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(PortalLoginResponse {
        access_token: access,
        refresh_token: refresh,
        session_id,
        expires_in: state.config.access_ttl_secs,
        portal_user: PortalProfile {
            id: identity.id,
            org_id: identity.org_id,
            client_id: identity.client_id,
            email: identity.email.clone(),
            display_name: identity.display_name.clone(),
            role: identity.role,
        },
    })
}

pub(crate) async fn audit_best_effort(
    state: &AppState,
    org_id: Uuid,
    actor: Actor,
    action: &str,
    entity_type: Option<&str>,
    entity_id: Option<String>,
    detail: serde_json::Value,
) {
    let event = AuditEvent {
        id: Uuid::now_v7(),
        org_id,
        actor,
        action: action.into(),
        entity_type: entity_type.map(String::from),
        entity_id,
        detail,
        ip: None,
        occurred_at: chrono::Utc::now(),
    };
    // RLS-scoped write (pool carries no app.org_id GUC); best effort.
    let result = async {
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        match &event.actor {
            Actor::Human(id) => {
                core_db::set_org_and_user(&mut tx, org_id, *id).await?;
            }
            _ => {
                core_db::set_org(&mut tx, org_id).await?;
            }
        }
        core_db::audit::insert(&mut *tx, &event).await?;
        tx.commit().await.map_err(ApiError::from)?;
        Ok::<(), ApiError>(())
    }
    .await;
    // Best effort, but LOUD (§6 — the audit trail is the security
    // authority): a swallowed audit failure must at least log its cause.
    if let Err(e) = result {
        tracing::error!(error = %e, action = %event.action, "portal audit insert failed");
    }
}

// --- profile ---------------------------------------------------------------

/// GET /api/portal/me — the signed-in portal identity.
#[utoipa::path(operation_id = "portal_me", get, path = "/api/portal/me", tag = "portal",
    security(("bearer" = [])), responses((status = 200, body = PortalProfile)))]
pub async fn me(extensions: Extensions) -> ApiResult<Json<PortalProfile>> {
    let authed = portal_authed(&extensions)?;
    Ok(Json(PortalProfile {
        id: authed.user.id,
        org_id: authed.user.org_id,
        client_id: authed.user.client_id,
        email: authed.user.email.clone(),
        display_name: authed.user.display_name.clone(),
        role: authed.user.role,
    }))
}

fn portal_authed(extensions: &Extensions) -> ApiResult<PortalAuthed> {
    extensions
        .get::<PortalAuthed>()
        .cloned()
        .ok_or_else(|| ApiError::Unauthorized("not authenticated".into()))
}

// --- tickets -----------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct PortalTicketListQuery {
    pub board_id: Option<Uuid>,
    pub category: Option<TicketStatusCategory>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

/// GET /api/portal/tickets — the caller's client's tickets across EXPOSED
/// boards only. Unexposed boards are invisible, not empty-filtered.
#[utoipa::path(operation_id = "portal_list_tickets", get, path = "/api/portal/tickets", tag = "portal",
    security(("bearer" = [])), params(PortalTicketListQuery),
    responses((status = 200, body = [core_domain::entities::Ticket])))]
pub async fn list_tickets(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<PortalTicketListQuery>,
) -> ApiResult<Json<Vec<core_domain::entities::Ticket>>> {
    let authed = portal_authed(&extensions)?;
    check_portal(
        authed.user.role,
        core_domain::policy::PortalAction::ViewTickets,
    )
    .expect()
    .map_err(ApiError::Forbidden)?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, authed.user.org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    let tickets = core_db::portal::list_client_tickets(
        &mut *tx,
        authed.user.org_id,
        authed.user.client_id,
        core_db::portal::PortalTicketFilter {
            board_id: q.board_id,
            category: q.category,
            limit: q.limit.clamp(1, 200),
            offset: q.offset.max(0),
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(tickets))
}

/// GET /api/portal/tickets/{id} — detail plus comments through the
/// notes-taxonomy whitelist for the caller's tier. A ticket from another
/// client (or an unexposed board) is a 404, indistinguishable from absent.
#[utoipa::path(operation_id = "portal_get_ticket", get, path = "/api/portal/tickets/{id}", tag = "portal",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = PortalTicketDetail), (status = 404)))]
pub async fn get_ticket(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<PortalTicketDetail>> {
    let authed = portal_authed(&extensions)?;
    check_portal(
        authed.user.role,
        core_domain::policy::PortalAction::ViewTickets,
    )
    .expect()
    .map_err(ApiError::Forbidden)?;
    let org_id = authed.user.org_id;
    let client_id = authed.user.client_id;
    let role = authed.user.role;

    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    let ticket = core_db::portal::get_client_ticket(&mut *tx, org_id, client_id, id).await?;
    let audience = audience_for(role);
    let comments = core_db::tickets::list_comments(
        &mut *tx,
        org_id,
        id,
        Some(core_domain::notes::visible_kinds(audience)),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(PortalTicketDetail {
        ticket,
        comments,
        can_view_in_house_updates: matches!(
            audience,
            core_domain::notes::NoteAudience::PortalItManager
        ),
    }))
}

/// Portal tier → note-visibility audience. THE mapping of the v1 rule:
/// contact/approver see client-visible only; it_manager additionally sees
/// in-house updates; internal is unreachable for all three.
fn audience_for(role: PortalRole) -> core_domain::notes::NoteAudience {
    match role {
        PortalRole::Contact | PortalRole::Approver => core_domain::notes::NoteAudience::Portal,
        PortalRole::ItManager => core_domain::notes::NoteAudience::PortalItManager,
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PortalTicketDetail {
    pub ticket: core_domain::entities::Ticket,
    /// Only the kinds this tier may read, filtered by the domain policy.
    pub comments: Vec<TicketComment>,
    pub can_view_in_house_updates: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PortalReplyRequest {
    pub body: String,
}

/// POST /api/portal/tickets/{id}/reply — ALWAYS creates a `client_visible`
/// comment regardless of tier, authored under portal audit provenance
/// (`actor_kind = portal_contact | portal_approver | portal_it_manager`;
/// `author_id` stays NULL because portal identities are not staff rows).
/// Emits the SAME `ticket.comment_added` event as staff replies so every
/// downstream notification path works unchanged.
#[utoipa::path(operation_id = "portal_reply", post, path = "/api/portal/tickets/{id}/reply", tag = "portal",
    security(("bearer" = [])), params(("id" = Uuid, Path)), request_body = PortalReplyRequest,
    responses((status = 200, body = TicketComment), (status = 404)))]
pub async fn reply(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PortalReplyRequest>,
) -> ApiResult<Json<TicketComment>> {
    let authed = portal_authed(&extensions)?;
    check_portal(
        authed.user.role,
        core_domain::policy::PortalAction::PostReply,
    )
    .expect()
    .map_err(ApiError::Forbidden)?;
    if req.body.trim().is_empty() {
        return Err(ApiError::Validation("reply body is required".into()));
    }
    let org_id = authed.user.org_id;
    let portal_user_id = authed.user.id;
    let client_id = authed.user.client_id;
    let role = authed.user.role;

    let comment = org_tx::with_org_tx(&state, org_id, portal_user_id, |mut ctx| async move {
        // Client+exposure scoped fetch: replying into another client's
        // ticket (or an unexposed board) is a NotFound before any write.
        let ticket =
            core_db::portal::get_client_ticket(&mut *ctx.tx, org_id, client_id, id).await?;
        let comment = core_db::tickets::insert_comment(
            &mut *ctx.tx,
            Uuid::now_v7(),
            org_id,
            id,
            None,
            NoteKind::ClientVisible,
            req.body.trim(),
            None,
        )
        .await?;

        // Append-only provenance: WHO KIND of actor replied.
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: portal_actor(role, portal_user_id),
                action: "portal.ticket_reply".into(),
                entity_type: Some("ticket_comment".into()),
                entity_id: Some(comment.id.to_string()),
                detail: serde_json::json!({
                    "ticket_id": id,
                    "ticket_number": ticket.number.0,
                    "client_id": client_id,
                    "portal_role": role.as_str(),
                    "comment_kind": NoteKind::ClientVisible.as_str(),
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        // Normal comment event — staff notification paths unchanged.
        ctx.emit(
            types::TICKET_COMMENT_ADDED,
            "ticket_comment",
            Some(comment.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketCommentAdded {
                comment: comment.clone(),
                mentioned_user_ids: Vec::new(),
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, comment))
    })
    .await?;

    Ok(Json(comment))
}

// --- portal attachments (TASKLIST 2.14) -------------------------------------

/// Portal attachment view: metadata only (bytes stream on download).
#[derive(Debug, Serialize, ToSchema)]
pub struct PortalAttachmentView {
    pub id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

fn portal_attachment_visible_kinds(role: PortalRole) -> Vec<String> {
    core_domain::notes::visible_kinds(audience_for(role))
        .iter()
        .map(|k| k.as_str().to_string())
        .collect()
}

/// GET /api/portal/tickets/{id}/attachments — attachments the portal
/// caller may see: ticket-level files always; comment files only when the
/// owning note kind is visible to this tier.
#[utoipa::path(operation_id = "portal_list_attachments", get,
    path = "/api/portal/tickets/{id}/attachments", tag = "portal",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = [PortalAttachmentView]), (status = 404)))]
pub async fn list_attachments(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<PortalAttachmentView>>> {
    let authed = portal_authed(&extensions)?;
    check_portal(
        authed.user.role,
        core_domain::policy::PortalAction::ViewTickets,
    )
    .expect()
    .map_err(ApiError::Forbidden)?;
    let org_id = authed.user.org_id;
    let client_id = authed.user.client_id;

    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    // Exposure + ownership gate (also 404s hidden tickets).
    core_db::portal::get_client_ticket(&mut *tx, org_id, client_id, id).await?;
    let kinds = portal_attachment_visible_kinds(authed.user.role);
    let items = core_db::attachments::list_portal_visible(&mut *tx, org_id, id, &kinds)
        .await?
        .into_iter()
        .map(|a| PortalAttachmentView {
            id: a.id,
            filename: a.filename,
            content_type: a.content_type,
            size_bytes: a.size_bytes,
            sha256: a.sha256,
            created_at: a.created_at,
        })
        .collect();
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(items))
}

/// GET /api/portal/tickets/{id}/attachments/{attachment_id}/download —
/// streams one visible attachment through the API. Hidden or foreign
/// attachments are plain 404s (no existence leak).
#[utoipa::path(operation_id = "portal_download_attachment", get,
    path = "/api/portal/tickets/{id}/attachments/{attachment_id}/download", tag = "portal",
    security(("bearer" = [])),
    params(("id" = Uuid, Path), ("attachment_id" = Uuid, Path)),
    responses((status = 200, body = Vec<u8>, content_type = "application/octet-stream"), (status = 404)))]
pub async fn download_attachment(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, attachment_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<axum::response::Response> {
    use futures_util::StreamExt;
    let authed = portal_authed(&extensions)?;
    check_portal(
        authed.user.role,
        core_domain::policy::PortalAction::ViewTickets,
    )
    .expect()
    .map_err(ApiError::Forbidden)?;
    let org_id = authed.user.org_id;
    let client_id = authed.user.client_id;

    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    core_db::portal::get_client_ticket(&mut *tx, org_id, client_id, id).await?;
    let kinds = portal_attachment_visible_kinds(authed.user.role);
    let Some(attachment) =
        core_db::attachments::find_portal_visible(&mut *tx, org_id, id, attachment_id, &kinds)
            .await?
    else {
        return Err(ApiError::NotFound);
    };
    tx.commit().await.map_err(ApiError::from)?;

    let obj = state
        .store
        .get(&attachment.storage_key)
        .await
        .map_err(crate::routes::attachments::store_error)?;
    let content_disposition = format!(
        "attachment; filename=\"{}\"",
        attachment.filename.replace(['"', '\\'], "")
    );
    let stream = obj.stream.map(|chunk| chunk.map_err(axum::Error::new));
    axum::response::Response::builder()
        .status(axum::http::StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, obj.content_type)
        .header(axum::http::header::CONTENT_DISPOSITION, content_disposition)
        .body(axum::body::Body::from_stream(stream))
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))
}
