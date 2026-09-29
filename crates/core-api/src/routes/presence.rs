//! Ephemeral presence endpoints (Wave 12 lane 12): who is viewing/editing
//! what, with a short TTL. Heartbeats keep a viewer alive; roster reads
//! answer "is someone else working on this ticket right now".
//!
//! Presence is EPHEMERAL and transport-adjacent: entries live in process
//! memory ([`crate::realtime`]), expire on TTL, are never persisted, never
//! enter the event trail, and never register in the contract registry.
//! Transitions (join / editing flip) are pushed as lightweight
//! `type: "presence"` stream frames — not contract events.
//!
//! Multi-replica note (Wave 13 lane 10): the store is per-core-api-process
//! but heartbeats are RELAYED to the other replicas over the pg NOTIFY
//! channel `marula_presence` (`realtime::publish_heartbeat_notify`) —
//! each replica applies the relayed heartbeat locally, so rosters
//! converge and transition frames carry the identical shape everywhere.
//! Frame shape stays identical to the documented contract in
//! `realtime.rs`.

use crate::auth::middleware::authed;
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::realtime::{self, PresenceViewer};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// POST /api/presence/heartbeat body. One heartbeat per viewer per entity
/// instance every ~30s; entries expire after 90s without one (TTL is
/// server-side, see `realtime::presence`).
#[derive(Debug, Deserialize, ToSchema)]
pub struct HeartbeatRequest {
    /// Entity family (e.g. `"ticket"`).
    pub entity: String,
    /// The entity instance id.
    pub entity_id: String,
    /// True while the caller is actively editing (vs. just viewing).
    #[serde(default)]
    pub editing: bool,
}

/// GET /api/tickets/{id}/presence response.
#[derive(Debug, Serialize, ToSchema)]
pub struct PresenceRoster {
    pub viewers: Vec<PresenceViewer>,
}

fn validate_entity(entity: &str, entity_id: &str) -> ApiResult<()> {
    if entity.is_empty() || entity.len() > 64 {
        return Err(ApiError::Validation(
            "entity must be 1-64 characters".into(),
        ));
    }
    if entity
        .chars()
        .any(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
    {
        return Err(ApiError::Validation(
            "entity must be lowercase snake_case (e.g. ticket)".into(),
        ));
    }
    Uuid::parse_str(entity_id)
        .map_err(|_| ApiError::Validation("entity_id must be a uuid".into()))?;
    Ok(())
}

fn roster(viewers: Vec<PresenceViewer>) -> PresenceRoster {
    PresenceRoster { viewers }
}

/// POST /api/presence/heartbeat — keep this viewer's presence alive on
/// {entity, entity_id}; flips the editing flag when it changed. Returns
/// the current roster so the caller can paint it immediately.
#[utoipa::path(
    operation_id = "presence_heartbeat",
    post,
    path = "/api/presence/heartbeat",
    tag = "presence",
    security(("bearer" = [])),
    request_body = HeartbeatRequest,
    responses(
        (status = 200, body = PresenceRoster),
        (status = 401, description = "missing or invalid bearer token"),
        (status = 422, description = "malformed entity/entity_id"),
    )
)]
pub async fn heartbeat(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<HeartbeatRequest>,
) -> ApiResult<Json<PresenceRoster>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    validate_entity(&req.entity, &req.entity_id)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let notify = realtime::HeartbeatNotify {
        org_id,
        user_id,
        name: authed.principal.user.display_name.clone(),
        entity: req.entity.clone(),
        entity_id: req.entity_id.clone(),
        editing: req.editing,
        at: chrono::Utc::now(),
    };
    let viewers = realtime::presence().heartbeat(
        org_id,
        user_id,
        notify.name.clone(),
        req.entity,
        req.entity_id,
        req.editing,
    );
    // Cross-replica relay: fire-and-forget after the local upsert — the
    // response never waits on (or fails with) the relay; a missed relay
    // only delays cross-replica convergence by one heartbeat.
    if let Err(e) = realtime::publish_heartbeat_notify(&state.pool, &notify).await {
        tracing::warn!(error = %e, "presence relay NOTIFY failed; this replica only");
    }
    Ok(Json(roster(viewers)))
}

/// GET /api/tickets/{id}/presence — current viewers/editors of the
/// ticket, TTL-pruned. The ticket must exist in the caller's org (RLS);
/// the roster is org-keyed, so it can never leak across orgs.
#[utoipa::path(
    operation_id = "ticket_presence",
    get,
    path = "/api/tickets/{id}/presence",
    tag = "presence",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Ticket id")),
    responses(
        (status = 200, body = PresenceRoster),
        (status = 401, description = "missing or invalid bearer token"),
        (status = 404, description = "ticket not visible in caller's org"),
    )
)]
pub async fn ticket_presence(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<PresenceRoster>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    // Visibility gate: the ticket must resolve inside the caller's org
    // (RLS-scoped read). A miss is a plain not_found — reads keep
    // not_found; only mutations collapse to rls_sparse.
    org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut ctx| async move {
            if core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id)
                .await
                .is_err()
            {
                return Err(ApiError::NotFound);
            }
            Ok((ctx, ()))
        },
    )
    .await?;
    let viewers = realtime::presence().roster(org_id, "ticket", &id.to_string());
    Ok(Json(roster(viewers)))
}
