//! Email inbound routing settings (admin).
//!
//! Purpose: manage the minimal routing config — the org-level catch-all
//! mailbox (address + target board) and per-board inbound addresses.
//! Addresses are globally unique by index: they are how an inbound webhook
//! resolves WHICH org and board receive a message. Not in OpenAPI/SDK this
//! wave (no UI yet); documented gap tracked in the lane report.
//!
//! Non-purpose: no mailbox provisioning, no DNS guidance, no provider
//! wiring — operators point their mail provider's webhook at Meridian and
//! configure addresses here to match.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

#[derive(Debug, Serialize)]
pub struct InboundSettingsResponse {
    pub catchall_address: Option<String>,
    pub catchall_board_id: Option<Uuid>,
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct PutInboundSettingsRequest {
    pub catchall_address: String,
    pub catchall_board_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct PutBoardAddressRequest {
    /// `None` clears the address.
    pub address: Option<String>,
}

/// GET /api/email/inbound-settings — current org catch-all, if configured.
pub async fn get_inbound_settings(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<InboundSettingsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org = authed.principal.user.org_id;
    let settings = core_db::emails::get_inbound_settings(&state.pool, org).await?;
    Ok(Json(InboundSettingsResponse {
        catchall_address: settings.as_ref().map(|s| s.catchall_address.clone()),
        catchall_board_id: settings.as_ref().map(|s| s.catchall_board_id),
        enabled: settings.as_ref().is_some_and(|s| s.enabled),
    }))
}

/// PUT /api/email/inbound-settings — upsert the org catch-all route.
pub async fn put_inbound_settings(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PutInboundSettingsRequest>,
) -> ApiResult<Json<InboundSettingsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org = authed.principal.user.org_id;
    if req.catchall_address.trim().is_empty() || !req.catchall_address.contains('@') {
        return Err(ApiError::Validation(
            "catchall_address must be a mailbox address".into(),
        ));
    }
    // Board must exist inside THIS org before it can receive mail.
    core_db::boards::get_board(&state.pool, org, req.catchall_board_id)
        .await
        .map_err(|_| ApiError::Validation("catchall_board_id not found in org".into()))?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org).await?;
    core_db::emails::upsert_inbound_settings(
        &mut tx,
        org,
        &req.catchall_address,
        req.catchall_board_id,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    get_inbound_settings(State(state), extensions).await
}

/// PUT /api/boards/{id}/inbound-address — set/clear a board-level address.
pub async fn put_board_inbound_address(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(board_id): Path<Uuid>,
    Json(req): Json<PutBoardAddressRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org = authed.principal.user.org_id;
    if let Some(addr) = req.address.as_deref() {
        if addr.trim().is_empty() || !addr.contains('@') {
            return Err(ApiError::Validation(
                "address must be a mailbox address (or null to clear)".into(),
            ));
        }
    }
    // Board-level RBAC (deny wins): a restrictive membership row on this
    // board blocks even org-level admins.
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, org)
        .await
        .map_err(ApiError::from)?;
    crate::routes::board_access::authorize_board_op(
        &mut tx,
        &authed.principal,
        board_id,
        core_domain::policy::BoardOp::ConfigureBoard,
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    match core_db::emails::set_board_inbound_address(
        &state.pool,
        org,
        board_id,
        req.address.as_deref(),
    )
    .await
    {
        Ok(()) => Ok(Json(serde_json::json!({ "inbound_address": req.address }))),
        Err(core_db::DbError::NotFound) => Err(ApiError::NotFound),
        // Unique-index violation: another board/org already owns it.
        Err(core_db::DbError::Sql(sqlx::Error::Database(db))) if db.constraint().is_some() => {
            Err(ApiError::Conflict("address already in use".into()))
        }
        Err(e) => Err(e.into()),
    }
}
