//! Staff-side administration of the client portal (portal v1): invite /
//! manage portal users per client and toggle per-board client exposure.
//!
//! Purpose: every privileged portal mutation behind the STAFF permission
//! matrix (`Permission::PortalAdmin`, held by Owner/Admin builtins).
//! Provisioning writes an argon2id hash from the admin-supplied initial
//! password (invite emails are out of scope for v1 — the password travels
//! out-of-band); exposure toggles gate which boards a client's portal
//! users can see at all.
//!
//! Non-purpose: no portal-facing endpoints here (that is `routes::portal`);
//! no client/staff user management; no board status definition.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_domain::entities::{Actor, AuditEvent, PortalRole, PortalUser, PortalUserStatus};
use core_domain::events::types;
use core_domain::policy::Permission;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

fn validate_email(email: &str) -> ApiResult<()> {
    let ok = email.contains('@') && email.contains('.') && !email.contains(' ') && email.len() >= 5;
    if ok {
        Ok(())
    } else {
        Err(ApiError::Validation("invalid email".into()))
    }
}

fn validate_password(pw: &str) -> ApiResult<()> {
    if pw.len() < 10 {
        return Err(ApiError::Validation(
            "password must be at least 10 characters".into(),
        ));
    }
    Ok(())
}

// --- portal users --------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePortalUserRequest {
    pub email: String,
    pub display_name: String,
    /// Initial credential, hashed with argon2id before storage. The admin
    /// delivers it to the contact out-of-band (v1 has no invite email).
    pub password: String,
    pub role: PortalRole,
}

/// GET /api/clients/{id}/portal-users — directory for one client.
#[utoipa::path(operation_id = "portal_users_list", get, path = "/api/clients/{id}/portal-users", tag = "portal-admin",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = [PortalUser])))]
pub async fn list_portal_users(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(client_id): Path<Uuid>,
) -> ApiResult<Json<Vec<PortalUser>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::PortalAdmin)?;
    let users = core_db::portal::list_portal_users(
        &state.pool,
        authed.principal.user.org_id,
        Some(client_id),
    )
    .await?;
    Ok(Json(users))
}

/// POST /api/clients/{id}/portal-users — provision (invite) one identity.
#[utoipa::path(operation_id = "portal_users_create", post, path = "/api/clients/{id}/portal-users", tag = "portal-admin",
    security(("bearer" = [])), params(("id" = Uuid, Path)), request_body = CreatePortalUserRequest,
    responses((status = 200, body = PortalUser), (status = 409, description = "email already provisioned")))]
pub async fn create_portal_user(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(client_id): Path<Uuid>,
    Json(req): Json<CreatePortalUserRequest>,
) -> ApiResult<Json<PortalUser>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::PortalAdmin)?;
    validate_email(&req.email)?;
    validate_password(&req.password)?;
    if req.display_name.trim().is_empty() {
        return Err(ApiError::Validation("display name is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let admin_id = authed.principal.user.id;

    // The client must exist in THIS org (explicit check → clean 404 instead
    // of a raw FK violation on cross-org ids).
    core_db::clients::get_client(&state.pool, org_id, client_id).await?;

    let password_hash = crate::auth::password::hash(&req.password)
        .map_err(|e| ApiError::Validation(format!("password hashing failed: {e}")))?;
    let user = org_tx::with_org_tx(&state, org_id, admin_id, |mut ctx| async move {
        let user = core_db::portal::create_portal_user(
            &mut *ctx.tx,
            core_db::portal::NewPortalUser {
                id: Uuid::now_v7(),
                org_id,
                client_id,
                email: req.email.trim(),
                display_name: req.display_name.trim(),
                password_hash: &password_hash,
                role: req.role,
            },
        )
        .await?;
        ctx.emit(
            types::PORTAL_USER_CREATED,
            "portal_user",
            Some(user.id.to_string().as_str()),
            &serde_json::json!({
                "portal_user_id": user.id,
                "client_id": client_id,
                "email": user.email,
                "role": user.role.as_str(),
                "status": user.status.as_str(),
            }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(admin_id),
                action: "portal.user_created".into(),
                entity_type: Some("portal_user".into()),
                entity_id: Some(user.id.to_string()),
                detail: serde_json::json!({
                    "client_id": client_id,
                    "role": user.role.as_str(),
                    // Never the password or its hash.
                    "email": user.email,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((ctx, user))
    })
    .await?;
    Ok(Json(user))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchPortalUserRequest {
    pub display_name: Option<String>,
    pub role: Option<PortalRole>,
    /// `disabled` blocks login AND every authenticated request immediately.
    pub status: Option<PortalUserStatus>,
    /// Optional credential reset (hashed before storage).
    pub password: Option<String>,
}

/// PATCH /api/portal-users/{id} — rename / re-tier / enable / disable /
/// reset password.
#[utoipa::path(operation_id = "portal_users_patch", patch, path = "/api/portal-users/{id}", tag = "portal-admin",
    security(("bearer" = [])), params(("id" = Uuid, Path)), request_body = PatchPortalUserRequest,
    responses((status = 200, body = PortalUser), (status = 404)))]
pub async fn patch_portal_user(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchPortalUserRequest>,
) -> ApiResult<Json<PortalUser>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::PortalAdmin)?;
    if let Some(pw) = &req.password {
        validate_password(pw)?;
    }
    let password_hash = req
        .password
        .as_deref()
        .map(crate::auth::password::hash)
        .transpose()
        .map_err(|e| ApiError::Validation(format!("password hashing failed: {e}")))?;
    let org_id = authed.principal.user.org_id;
    let admin_id = authed.principal.user.id;

    let user = org_tx::with_org_tx(&state, org_id, admin_id, |mut ctx| async move {
        let user = core_db::portal::update_portal_user(
            &mut *ctx.tx,
            org_id,
            id,
            core_db::portal::PortalUserPatch {
                display_name: req.display_name.clone(),
                role: req.role,
                status: req.status,
                password_hash,
            },
        )
        .await?;
        ctx.emit(
            types::PORTAL_USER_UPDATED,
            "portal_user",
            Some(user.id.to_string().as_str()),
            &serde_json::json!({
                "portal_user_id": user.id,
                "client_id": user.client_id,
                "role": user.role.as_str(),
                "status": user.status.as_str(),
            }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(admin_id),
                action: "portal.user_updated".into(),
                entity_type: Some("portal_user".into()),
                entity_id: Some(user.id.to_string()),
                detail: serde_json::json!({
                    "renamed": req.display_name.is_some(),
                    "role_changed": req.role.is_some(),
                    "status_changed": req.status.is_some(),
                    "password_reset": req.password.is_some(),
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((ctx, user))
    })
    .await?;
    Ok(Json(user))
}

/// DELETE /api/portal-users/{id} — hard removal (sessions cascade). Prefer
/// PATCH status=disabled to preserve history.
#[utoipa::path(operation_id = "portal_users_delete", delete, path = "/api/portal-users/{id}", tag = "portal-admin",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200), (status = 404)))]
pub async fn delete_portal_user(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::PortalAdmin)?;
    let org_id = authed.principal.user.org_id;
    let admin_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, admin_id, |mut ctx| async move {
        core_db::portal::delete_portal_user(&mut *ctx.tx, org_id, id).await?;
        ctx.emit(
            types::PORTAL_USER_DELETED,
            "portal_user",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "portal_user_id": id }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(admin_id),
                action: "portal.user_deleted".into(),
                entity_type: Some("portal_user".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({}),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

// --- board exposure -------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct BoardExposureView {
    pub id: Uuid,
    pub board_id: Uuid,
    pub client_id: Uuid,
    pub exposed: bool,
    pub visible_status_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PutExposureRequest {
    pub board_id: Uuid,
    pub exposed: bool,
    /// Empty = all statuses visible (v1 default). Non-empty narrows to
    /// statuses OF THAT BOARD — unknown ids are rejected, not silently kept.
    pub visible_status_ids: Vec<Uuid>,
}

/// GET /api/clients/{id}/board-exposures — every configured row for the
/// client (exposed or not), so admin UI can render toggles.
#[utoipa::path(operation_id = "board_exposures_list", get, path = "/api/clients/{id}/board-exposures", tag = "portal-admin",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = [BoardExposureView])))]
pub async fn list_board_exposures(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(client_id): Path<Uuid>,
) -> ApiResult<Json<Vec<BoardExposureView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::PortalAdmin)?;
    let rows =
        core_db::portal::list_exposures(&state.pool, authed.principal.user.org_id, Some(client_id))
            .await?;
    Ok(Json(
        rows.into_iter()
            .map(|x| BoardExposureView {
                id: x.id,
                board_id: x.board_id,
                client_id: x.client_id,
                exposed: x.exposed,
                visible_status_ids: x.visible_status_ids,
            })
            .collect(),
    ))
}

/// PUT /api/clients/{id}/board-exposures — upsert one board×client row.
#[utoipa::path(operation_id = "board_exposures_put", put, path = "/api/clients/{id}/board-exposures", tag = "portal-admin",
    security(("bearer" = [])), params(("id" = Uuid, Path)), request_body = PutExposureRequest,
    responses((status = 200, body = BoardExposureView), (status = 404, description = "board or client missing")))]
pub async fn put_board_exposure(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(client_id): Path<Uuid>,
    Json(req): Json<PutExposureRequest>,
) -> ApiResult<Json<BoardExposureView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::PortalAdmin)?;
    let org_id = authed.principal.user.org_id;
    let admin_id = authed.principal.user.id;

    core_db::boards::get_board(&state.pool, org_id, req.board_id).await?;
    core_db::clients::get_client(&state.pool, org_id, client_id).await?;
    // Status narrowing must reference statuses OF that board.
    if !req.visible_status_ids.is_empty() {
        let valid: std::collections::HashSet<Uuid> =
            core_db::boards::list_statuses(&state.pool, req.board_id)
                .await?
                .into_iter()
                .map(|s| s.id)
                .collect();
        if !req.visible_status_ids.iter().all(|s| valid.contains(s)) {
            return Err(ApiError::Validation(
                "visible_status_ids contains a status that is not on this board".into(),
            ));
        }
    }

    let exposure = org_tx::with_org_tx(&state, org_id, admin_id, |mut ctx| async move {
        let exposure = core_db::portal::upsert_exposure(
            &mut *ctx.tx,
            core_db::portal::NewExposure {
                org_id,
                board_id: req.board_id,
                client_id,
                exposed: req.exposed,
                visible_status_ids: req.visible_status_ids.clone(),
            },
        )
        .await?;
        ctx.emit(
            types::BOARD_PORTAL_EXPOSURE_UPDATED,
            "board_portal_exposure",
            Some(exposure.id.to_string().as_str()),
            &serde_json::json!({
                "board_id": exposure.board_id,
                "client_id": exposure.client_id,
                "exposed": exposure.exposed,
                "visible_status_ids": exposure.visible_status_ids,
            }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(admin_id),
                action: "portal.exposure_updated".into(),
                entity_type: Some("board_portal_exposure".into()),
                entity_id: Some(exposure.id.to_string()),
                detail: serde_json::json!({
                    "board_id": exposure.board_id,
                    "client_id": exposure.client_id,
                    "exposed": exposure.exposed,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((ctx, exposure))
    })
    .await?;

    Ok(Json(BoardExposureView {
        id: exposure.id,
        board_id: exposure.board_id,
        client_id: exposure.client_id,
        exposed: exposure.exposed,
        visible_status_ids: exposure.visible_status_ids,
    }))
}
