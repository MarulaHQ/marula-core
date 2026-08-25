//! Notification center endpoints.

use crate::auth::middleware::authed;
use crate::error::ApiResult;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct ListQuery {
    #[serde(default)]
    pub unread_only: bool,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    50
}

/// GET /api/notifications — the caller's notifications, newest first.
#[utoipa::path(get, path = "/api/notifications", tag = "notifications", security(("bearer" = [])),
    params(ListQuery),
    responses((status = 200, body = [core_domain::entities::Notification])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<core_domain::entities::Notification>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let items = core_db::notifications::list_for_user(
        &state.pool,
        authed.principal.user.org_id,
        authed.principal.user.id,
        q.unread_only,
        q.limit.clamp(1, 200),
    )
    .await?;
    Ok(Json(items))
}

/// GET /api/notifications/unread-count
#[utoipa::path(get, path = "/api/notifications/unread-count", tag = "notifications",
    security(("bearer" = [])), responses((status = 200, body = UnreadCount)))]
pub async fn unread_count(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<UnreadCount>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let count = core_db::notifications::unread_count(
        &state.pool,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await?;
    Ok(Json(UnreadCount { count }))
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct UnreadCount {
    pub count: i64,
}

/// POST /api/notifications/{id}/read
#[utoipa::path(post, path = "/api/notifications/{id}/read", tag = "notifications",
    security(("bearer" = [])), params(("id" = Uuid, Path)), responses((status = 204)))]
pub async fn mark_read(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    core_db::notifications::mark_read(
        &state.pool,
        authed.principal.user.org_id,
        authed.principal.user.id,
        id,
    )
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
