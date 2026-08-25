//! Audit log endpoint (admin-only, §6: ViewAuditLog).

use crate::auth::middleware::{authed, require_permission};
use crate::error::ApiResult;
use crate::state::AppState;
use axum::{extract::{Query, State}, http::Extensions, Json};
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct AuditQuery {
    pub before: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    100
}

/// GET /api/audit-events — append-only audit trail, newest first.
#[utoipa::path(get, path = "/api/audit-events", tag = "audit", security(("bearer" = [])),
    params(AuditQuery),
    responses((status = 200, body = [core_domain::entities::AuditEvent])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<Vec<core_domain::entities::AuditEvent>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, core_domain::policy::Permission::ViewAuditLog)?;
    let events = core_db::audit::list(
        &state.pool,
        authed.principal.user.org_id,
        q.limit.clamp(1, 500),
        q.before,
    )
    .await?;
    Ok(Json(events))
}
