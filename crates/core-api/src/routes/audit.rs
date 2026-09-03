//! Audit log endpoint (admin-only, §6): filtered listing with keyset
//! cursor pagination over the append-only trail.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    extract::{Query, State},
    http::Extensions,
    Json,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use core_db::audit::AuditFilter;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct AuditQuery {
    /// Legacy cursor (kept for compatibility): strictly older than this.
    pub before: Option<chrono::DateTime<chrono::Utc>>,
    /// Keyset cursor from a previous page's `next_cursor`.
    pub cursor: Option<String>,
    /// Actor filter: user uuid or "system".
    pub actor: Option<String>,
    /// Action prefix filter, e.g. "auth" matches every auth.* event.
    pub action: Option<String>,
    /// Inclusive lower bound on occurred_at.
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    /// Inclusive upper bound on occurred_at.
    pub to: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    100
}

fn decode_cursor(raw: &str) -> Option<(chrono::DateTime<chrono::Utc>, Uuid)> {
    let decoded = B64.decode(raw.as_bytes()).ok()?;
    let text = String::from_utf8(decoded).ok()?;
    let (ts, id) = text.split_once('|')?;
    Some((ts.parse().ok()?, id.parse().ok()?))
}

fn encode_cursor(at: chrono::DateTime<chrono::Utc>, id: Uuid) -> String {
    B64.encode(format!("{}|{}", at.to_rfc3339(), id).as_bytes())
}

/// GET /api/audit-events — append-only audit trail, newest first, filterable
/// by actor / action prefix / time range with opaque cursor pagination.
#[utoipa::path(operation_id = "audit_list", get, path = "/api/audit-events", tag = "audit", security(("bearer" = [])),
    params(AuditQuery),
    responses((status = 200, body = AuditPage)))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<AuditPage>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        core_domain::policy::Permission::ViewAuditLog,
    )?;
    let cursor = q.cursor.as_deref().and_then(decode_cursor);
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let events = core_db::audit::list_filtered(
        &mut *tx,
        authed.principal.user.org_id,
        AuditFilter {
            actor: q.actor,
            action: q.action,
            from: q.from,
            to: q.to,
            before: q.before,
            cursor,
        },
        q.limit.clamp(1, 500),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    // A full page hints another exists; anchor it to the last row.
    let next_cursor = if events.len() as i64 >= q.limit.clamp(1, 500) {
        events.last().map(|e| encode_cursor(e.occurred_at, e.id))
    } else {
        None
    };
    Ok(Json(AuditPage {
        events,
        next_cursor,
    }))
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct AuditPage {
    pub events: Vec<core_domain::entities::AuditEvent>,
    /// Opaque cursor for the next page; absent when exhausted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}
