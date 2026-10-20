//! Notification center endpoints: cursor-paginated inbox, unread badge,
//! single/bulk/all mark-read, and per-channel preferences with quiet hours.
//!
//! Purpose: the caller's own notification surface. Reads are user-scoped;
//! every mutation runs in an org-bound transaction that emits the matching
//! outbox event (`notification.read`) so other sessions reconcile over WS.
//!
//! Non-purpose: no fan-out or delivery decisions here (core-domain rules +
//! worker notifier own those), no org-default administration UI surface
//! (org defaults are read through the effective chain, managed via seed).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use core_domain::entities::NotificationKind;
use core_domain::notifications::QuietHours;
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct ListQuery {
    #[serde(default)]
    pub unread_only: bool,
    #[serde(default = "default_limit")]
    pub limit: i64,
    /// Opaque keyset cursor from a previous page's `next_cursor`.
    #[serde(default)]
    pub cursor: Option<String>,
}

fn default_limit() -> i64 {
    50
}

/// One page of the inbox plus the cursor to continue with.
#[derive(Debug, Serialize, ToSchema)]
pub struct NotificationPage {
    pub items: Vec<core_domain::entities::Notification>,
    /// Pass as `cursor` on the next request; `None` means exhausted.
    pub next_cursor: Option<String>,
}

fn encode_cursor(created_at: chrono::DateTime<chrono::Utc>, id: Uuid) -> String {
    hex::encode(format!("{}|{id}", created_at.to_rfc3339()))
}

fn decode_cursor(raw: &str) -> ApiResult<core_db::notifications::NotificationCursor> {
    let bytes =
        hex::decode(raw).map_err(|_| ApiError::Validation("invalid notification cursor".into()))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| ApiError::Validation("invalid notification cursor".into()))?;
    let parts: Vec<&str> = text.split('|').collect();
    if parts.len() != 2 {
        return Err(ApiError::Validation("invalid notification cursor".into()));
    }
    let created_at: chrono::DateTime<chrono::Utc> = parts[0]
        .parse()
        .map_err(|_| ApiError::Validation("invalid notification cursor".into()))?;
    let id = Uuid::parse_str(parts[1])
        .map_err(|_| ApiError::Validation("invalid notification cursor".into()))?;
    Ok(core_db::notifications::NotificationCursor { created_at, id })
}

/// GET /api/notifications — the caller's notifications, newest first,
/// keyset-paginated.
#[utoipa::path(operation_id = "notifications_list", get, path = "/api/notifications", tag = "notifications", security(("bearer" = [])),
    params(ListQuery),
    responses((status = 200, body = NotificationPage)))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<NotificationPage>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let before = match &q.cursor {
        Some(raw) => Some(decode_cursor(raw)?),
        None => None,
    };
    // RLS-scoped read: notifications policies demand app.org_id and the
    // pool carries no GUC of its own — ride an org-bound tx.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let (items, next) = core_db::notifications::list_page(
        &mut *tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
        q.unread_only,
        before,
        q.limit.clamp(1, 200),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(NotificationPage {
        items,
        next_cursor: next.map(|c| encode_cursor(c.created_at, c.id)),
    }))
}

/// GET /api/notifications/unread-count
#[utoipa::path(operation_id = "notifications_unread_count", get, path = "/api/notifications/unread-count", tag = "notifications",
    security(("bearer" = [])), responses((status = 200, body = UnreadCount)))]
pub async fn unread_count(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<UnreadCount>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let count = core_db::notifications::unread_count(
        &mut *tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(UnreadCount { count }))
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct UnreadCount {
    pub count: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MarkReadBulkRequest {
    pub ids: Vec<Uuid>,
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct MarkReadResult {
    /// Rows flipped from unread to read (idempotent under retry).
    pub marked: i64,
}

/// POST /api/notifications/{id}/read — marks one notification read and
/// emits `notification.read` for cross-session badge sync.
#[utoipa::path(operation_id = "notifications_mark_read", post, path = "/api/notifications/{id}/read", tag = "notifications",
    security(("bearer" = [])), params(("id" = Uuid, Path)), responses((status = 204)))]
pub async fn mark_read(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::notifications::mark_read(&mut *ctx.tx, org_id, user_id, id).await?;
        ctx.emit(
            core_domain::events::types::NOTIFICATION_READ,
            "notification",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "user_id": user_id, "ids": [id], "count": 1 }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// POST /api/notifications/mark-read — bulk variant; unknown and
/// already-read ids are skipped (idempotent).
#[utoipa::path(operation_id = "notifications_mark_read_bulk", post, path = "/api/notifications/mark-read", tag = "notifications",
    security(("bearer" = [])), request_body = MarkReadBulkRequest,
    responses((status = 200, body = MarkReadResult),
              (status = 422, description = "empty or oversized id list")))]
pub async fn mark_read_bulk(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<MarkReadBulkRequest>,
) -> ApiResult<Json<MarkReadResult>> {
    if req.ids.is_empty() {
        return Err(ApiError::Validation("ids must not be empty".into()));
    }
    if req.ids.len() > 500 {
        return Err(ApiError::Validation("too many ids (max 500)".into()));
    }
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let marked = crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let marked = core_db::notifications::mark_read_many(&mut *ctx.tx, org_id, user_id, &req.ids)
            .await? as i64;
        ctx.emit(
            core_domain::events::types::NOTIFICATION_READ,
            "notification",
            None,
            &serde_json::json!({ "user_id": user_id, "ids": req.ids, "count": marked }),
        )
        .await?;
        Ok((ctx, marked))
    })
    .await?;
    Ok(Json(MarkReadResult { marked }))
}

/// POST /api/notifications/read-all — empties the unread badge.
#[utoipa::path(operation_id = "notifications_mark_all_read", post, path = "/api/notifications/read-all", tag = "notifications",
    security(("bearer" = [])), responses((status = 200, body = MarkReadResult)))]
pub async fn mark_all_read(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<MarkReadResult>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let marked = crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let marked =
            core_db::notifications::mark_all_read(&mut *ctx.tx, org_id, user_id).await? as i64;
        ctx.emit(
            core_domain::events::types::NOTIFICATION_READ,
            "notification",
            None,
            &serde_json::json!({ "user_id": user_id, "ids": [], "count": marked }),
        )
        .await?;
        Ok((ctx, marked))
    })
    .await?;
    Ok(Json(MarkReadResult { marked }))
}

// --- per-channel preferences -------------------------------------------------

/// One resolved preference row: which chain link supplied it (`user`,
/// `org_default`, or `builtin`), the channel toggles, and the quiet-hours
/// window with its timezone.
#[derive(Debug, Serialize, ToSchema)]
pub struct PreferenceView {
    pub kind: NotificationKind,
    pub source: String,
    pub in_app: bool,
    pub email: bool,
    pub quiet_from: Option<String>,
    pub quiet_until: Option<String>,
    pub timezone: String,
}

/// GET /api/notification-preferences — the caller's effective preference
/// for every kind (defaults-first resolution applied).
#[utoipa::path(operation_id = "notification_prefs_get", get, path = "/api/notification-preferences", tag = "notifications",
    security(("bearer" = [])), responses((status = 200, body = [PreferenceView])))]
pub async fn prefs_get(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<PreferenceView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    // RLS-scoped read (pool carries no app.org_id GUC): one tx for the
    // whole effective-preference sweep.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let mut views = Vec::with_capacity(NotificationKind::ALL.len());
    for kind in NotificationKind::ALL {
        let row =
            core_db::notifications::effective_preference(&mut *tx, org_id, user_id, kind.as_str())
                .await?;
        views.push(PreferenceView {
            kind,
            source: row.source.to_string(),
            in_app: row.in_app,
            email: row.email,
            quiet_from: row.quiet_from,
            quiet_until: row.quiet_until,
            timezone: row.timezone,
        });
    }
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(views))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePreferenceRequest {
    pub kind: NotificationKind,
    pub in_app: bool,
    pub email: bool,
    /// `HH:MM` pair; both bounds together or neither.
    pub quiet_from: Option<String>,
    pub quiet_until: Option<String>,
    /// IANA timezone for the window; defaults to UTC.
    #[serde(default)]
    pub timezone: Option<String>,
}

/// PUT /api/notification-preferences — upserts the caller's override row
/// for one kind and emits `notification_prefs.updated`. Quiet hours
/// validate as a paired `HH:MM` window in a real IANA timezone;
/// validation errors return 422.
#[utoipa::path(operation_id = "notification_prefs_update", put, path = "/api/notification-preferences", tag = "notifications",
    security(("bearer" = [])), request_body = UpdatePreferenceRequest,
    responses((status = 200, body = PreferenceView),
              (status = 422, description = "invalid quiet-hours window or timezone")))]
pub async fn prefs_update(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<UpdatePreferenceRequest>,
) -> ApiResult<Json<PreferenceView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let tz = req.timezone.as_deref().unwrap_or("UTC");
    // Validate the full window up front — parse errors map to 422.
    QuietHours::parse(req.quiet_from.as_deref(), req.quiet_until.as_deref(), tz)?;

    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let view = crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let pref = core_domain::entities::NotificationPreference {
            user_id,
            kind: req.kind,
            in_app: req.in_app,
            email: req.email,
            quiet_from: req.quiet_from.clone(),
            quiet_until: req.quiet_until.clone(),
            timezone: tz.to_string(),
        };
        core_db::notifications::upsert_preference(&mut *ctx.tx, &pref).await?;
        ctx.emit(
            core_domain::events::types::NOTIFICATION_PREFS_UPDATED,
            "notification_preferences",
            Some(user_id.to_string().as_str()),
            &serde_json::to_value(&pref).unwrap_or_default(),
        )
        .await?;
        Ok((
            ctx,
            PreferenceView {
                kind: pref.kind,
                source: "user".to_string(),
                in_app: pref.in_app,
                email: pref.email,
                quiet_from: pref.quiet_from,
                quiet_until: pref.quiet_until,
                timezone: pref.timezone,
            },
        ))
    })
    .await?;
    Ok(Json(view))
}

// --- org defaults (Wave 34 lane 8: admin overrides over per-user prefs) -----

#[derive(Debug, Serialize, ToSchema)]
pub struct OrgDefaultView {
    pub kind: NotificationKind,
    pub in_app: bool,
    pub email: bool,
    pub quiet_from: Option<String>,
    pub quiet_until: Option<String>,
    pub timezone: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OrgDefaultsResponse {
    pub defaults: Vec<OrgDefaultView>,
}

/// PUT /api/notification-org-defaults — replaces the org default ROW for
/// one kind; the chain is user override → org default → built-in, so an
/// untouched user's next email follows this policy. Quiet hours validate as
/// a paired `HH:MM` window in a real IANA timezone (422 otherwise).
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateOrgDefaultRequest {
    pub kind: NotificationKind,
    pub in_app: bool,
    pub email: bool,
    /// `HH:MM` pair; both bounds together or neither.
    pub quiet_from: Option<String>,
    pub quiet_until: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
}

/// GET /api/notification-org-defaults — the org's default rows (admin).
#[utoipa::path(operation_id = "notification_org_defaults_get", get,
    path = "/api/notification-org-defaults", tag = "notifications",
    security(("bearer" = [])),
    responses((status = 200, body = OrgDefaultsResponse)))]
pub async fn org_defaults_get(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<OrgDefaultsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::notifications::list_org_defaults(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(OrgDefaultsResponse {
        defaults: rows
            .into_iter()
            .map(|row| OrgDefaultView {
                kind: row.kind.parse().unwrap_or(NotificationKind::TicketUpdate),
                in_app: row.in_app,
                email: row.email,
                quiet_from: row.quiet_from,
                quiet_until: row.quiet_until,
                timezone: row.timezone,
            })
            .collect(),
    }))
}

/// PUT /api/notification-org-defaults — upserts one kind's org default.
#[utoipa::path(operation_id = "notification_org_defaults_put", put,
    path = "/api/notification-org-defaults", tag = "notifications",
    security(("bearer" = [])), request_body = UpdateOrgDefaultRequest,
    responses((status = 200, body = OrgDefaultView),
              (status = 422, description = "invalid quiet-hours window or timezone")))]
pub async fn org_defaults_put(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<UpdateOrgDefaultRequest>,
) -> ApiResult<Json<OrgDefaultView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let tz = req.timezone.as_deref().unwrap_or("UTC");
    QuietHours::parse(req.quiet_from.as_deref(), req.quiet_until.as_deref(), tz)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let view = crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let row = core_db::notifications::OrgDefaultRow {
            org_id,
            kind: req.kind.as_str().to_string(),
            in_app: req.in_app,
            email: req.email,
            quiet_from: req.quiet_from.clone(),
            quiet_until: req.quiet_until.clone(),
            timezone: tz.to_string(),
        };
        core_db::notifications::upsert_org_default(&mut *ctx.tx, &row).await?;
        ctx.emit(
            core_domain::events::types::NOTIFICATION_PREFS_UPDATED,
            "notification_org_defaults",
            Some(req.kind.as_str()),
            &serde_json::json!({
                "scope": "org_default",
                "kind": req.kind.as_str(),
                "in_app": req.in_app,
                "email": req.email,
            }),
        )
        .await?;
        Ok((
            ctx,
            OrgDefaultView {
                kind: req.kind,
                in_app: req.in_app,
                email: req.email,
                quiet_from: req.quiet_from,
                quiet_until: req.quiet_until,
                timezone: tz.to_string(),
            },
        ))
    })
    .await?;
    Ok(Json(view))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The preference surface is driven by `NotificationKind::ALL` on GET and
    /// by the wire string a row sends back on PUT. Both request types must
    /// accept the kinds the notifier writes: `stock_low` was missing from
    /// `ALL`, so the screen skipped it silently while the migration admits it.
    #[test]
    fn the_preference_surface_accepts_every_kind_the_notifier_writes() {
        let user_row: UpdatePreferenceRequest = serde_json::from_value(serde_json::json!({
            "kind": "stock_low",
            "in_app": false,
            "email": false,
            "quiet_from": null,
            "quiet_until": null,
        }))
        .expect("stock_low is a user preference row");
        assert_eq!(user_row.kind, NotificationKind::StockLow);

        let org_row: UpdateOrgDefaultRequest = serde_json::from_value(serde_json::json!({
            "kind": "stock_low",
            "in_app": true,
            "email": true,
            "quiet_from": null,
            "quiet_until": null,
        }))
        .expect("stock_low is an org default row");
        assert_eq!(org_row.kind, NotificationKind::StockLow);

        // Every kind GET returns must also parse on the way back, and the
        // response must carry the same snake_case string the request used.
        for kind in NotificationKind::ALL {
            let req: UpdatePreferenceRequest = serde_json::from_value(serde_json::json!({
                "kind": kind.as_str(),
                "in_app": true,
                "email": false,
                "quiet_from": null,
                "quiet_until": null,
            }))
            .expect("kind parses");
            assert_eq!(req.kind, kind);

            let view = PreferenceView {
                kind,
                source: "builtin".into(),
                in_app: true,
                email: false,
                quiet_from: None,
                quiet_until: None,
                timezone: "UTC".into(),
            };
            let value = serde_json::to_value(&view).expect("view serializes");
            assert_eq!(value["kind"], kind.as_str());
        }
    }
}
