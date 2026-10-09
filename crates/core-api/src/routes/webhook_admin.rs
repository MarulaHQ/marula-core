//! Webhook subscription management + delivery log (TASKLIST 2.15).
//!
//! Purpose: admin CRUD for signed OUTBOUND webhooks, the cursor-paged
//! per-webhook delivery ledger, and a test-fire endpoint. Secrets are
//! generated server-side (`whsec_…`), sealed at rest before insert, shown
//! EXACTLY ONCE in the creation response, and never selected back or
//! logged (§6). The test event enters the normal outbox (`webhook.test`),
//! so what admins prove here is the REAL pipeline: relay → stream match →
//! durable job → signed POST → attempt ledger.
//!
//! Non-purpose: no inbound webhooks (email owns its receiver), no
//! connector installation consent flows beyond storage, no retry logic
//! (the worker owns attempts).

use crate::auth::middleware::{authed, require_permission};
use crate::crypto;
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use chrono::Utc;
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use core_domain::policy::Permission;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

// --- wire shapes ---------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct WebhookView {
    pub id: Uuid,
    pub url: String,
    /// Empty array = wildcard (all event types).
    pub event_types: Vec<String>,
    pub enabled: bool,
    pub created_by: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<core_db::webhooks::WebhookRow> for WebhookView {
    fn from(r: core_db::webhooks::WebhookRow) -> Self {
        Self {
            id: r.id,
            url: r.url,
            event_types: r.event_types,
            enabled: r.enabled,
            created_by: r.created_by,
            created_at: r.created_at,
        }
    }
}

/// Returned by create ONLY — carries the signing secret in plaintext one
/// final time. There is deliberately no read-back path.
#[derive(Debug, Serialize, ToSchema)]
pub struct WebhookSecretResponse {
    pub webhook: WebhookView,
    pub secret: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateWebhookRequest {
    /// HTTPS endpoint receiving `POST`s. (http:// allowed for local dev.)
    pub url: String,
    /// Exact event types to subscribe; empty subscribes to everything.
    #[serde(default)]
    pub event_types: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateWebhookRequest {
    pub url: Option<String>,
    pub event_types: Option<Vec<String>>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeliveryAttemptView {
    pub id: Uuid,
    pub event_id: Uuid,
    pub event_type: String,
    pub attempt: i32,
    /// HTTP status when a response arrived; null for transport failures.
    pub response_status: Option<i32>,
    pub next_retry_at: Option<chrono::DateTime<chrono::Utc>>,
    pub delivered_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<core_db::webhooks::DeliveryRow> for DeliveryAttemptView {
    fn from(r: core_db::webhooks::DeliveryRow) -> Self {
        Self {
            id: r.id,
            event_id: r.event_id,
            event_type: r.event_type,
            attempt: r.attempt,
            response_status: r.response_status,
            next_retry_at: r.next_retry_at,
            delivered_at: r.delivered_at,
            last_error: r.last_error,
            created_at: r.created_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeliveryPage {
    pub deliveries: Vec<DeliveryAttemptView>,
    /// Opaque keyset cursor for the next page; absent when exhausted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct DeliveryQuery {
    pub cursor: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    50
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

// --- validation ------------------------------------------------------------------

fn validate_url(url: &str) -> ApiResult<()> {
    let parsed = url::Url::parse(url)
        .map_err(|_| ApiError::Validation("url is not a valid absolute URL".into()))?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err(ApiError::Validation("url scheme must be http(s)".into()));
    }
    if parsed.host_str().is_none() {
        return Err(ApiError::Validation("url needs a host".into()));
    }
    Ok(())
}

/// Validates that every event type in `event_types` is either a registered
/// platform type (source='platform') or has an org-level grant. Empty
/// vec = wildcard (no validation needed). Platform types need no grant;
/// all others (plugin-contributed, external, or truly unknown) require
/// a row in `event_type_grants`.
async fn validate_event_types(
    pool: &sqlx::Pool<sqlx::Postgres>,
    org_id: Uuid,
    event_types: &[String],
) -> ApiResult<()> {
    let ungranted = core_db::event_types::check_event_types(pool, org_id, event_types)
        .await
        .map_err(ApiError::from)?;
    if !ungranted.is_empty() {
        return Err(ApiError::Validation(format!(
            "unknown or ungranted event types {:?}; either subscribe only to \
             platform types, or obtain a grant for custom types",
            ungranted
        )));
    }
    Ok(())
}

// --- handlers ---------------------------------------------------------------------

/// GET /api/webhooks — the org's subscriptions (never includes secrets).
#[utoipa::path(operation_id = "webhooks_list", get, path = "/api/webhooks", tag = "integrations", security(("bearer" = [])),
    responses((status = 200, body = WebhookList)))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<WebhookList>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManagePlugins)?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, authed.principal.user.org_id).await?;
    let rows = core_db::webhooks::list_webhooks(&mut *tx, authed.principal.user.org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(WebhookList {
        webhooks: rows.into_iter().map(WebhookView::from).collect(),
    }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WebhookList {
    pub webhooks: Vec<WebhookView>,
}

/// POST /api/webhooks — create a subscription; responds with the signing
/// secret once. Emits an audit record in the same transaction.
#[utoipa::path(operation_id = "webhooks_create", post, path = "/api/webhooks", tag = "integrations", security(("bearer" = [])),
    request_body = CreateWebhookRequest,
    responses((status = 201, body = WebhookSecretResponse)))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateWebhookRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<WebhookSecretResponse>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManagePlugins)?;
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    validate_url(&req.url)?;
    validate_event_types(&state.pool, org_id, &req.event_types).await?;

    // Fresh 256-bit secret; sealed immediately, returned once.
    let raw_secret = format!("whsec_{}", uuid::Uuid::now_v7().simple());
    let sealed = crypto::seal(&state.config.secret_key, raw_secret.as_bytes())
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))?;

    let id = Uuid::now_v7();
    let mut tx = org_tx::begin(&state, org_id, user_id).await?;
    let row = core_db::webhooks::insert_webhook(
        &mut *tx.tx,
        id,
        org_id,
        req.url.trim(),
        &sealed,
        &req.event_types,
        user_id,
        None,
    )
    .await?;
    core_db::audit::insert(
        &mut *tx.tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(user_id),
            action: "webhook.created".into(),
            entity_type: Some("webhook".into()),
            entity_id: Some(id.to_string()),
            detail: serde_json::json!({ "url": row.url, "event_types": row.event_types }),
            ip: None,
            occurred_at: Utc::now(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(WebhookSecretResponse {
            webhook: row.into(),
            secret: raw_secret,
        }),
    ))
}

/// PATCH /api/webhooks/{id} — mutate URL / filter / enabled flag.
#[utoipa::path(operation_id = "webhooks_update", patch, path = "/api/webhooks/{id}", tag = "integrations", security(("bearer" = [])),
    request_body = UpdateWebhookRequest,
    params(("id" = Uuid, Path)),
    responses((status = 200, body = WebhookView)))]
pub async fn update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateWebhookRequest>,
) -> ApiResult<Json<WebhookView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManagePlugins)?;
    let org_id = authed.principal.user.org_id;
    if let Some(url) = &req.url {
        validate_url(url)?;
    }
    if let Some(types_) = &req.event_types {
        validate_event_types(&state.pool, org_id, types_).await?;
    }
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let row = core_db::webhooks::update_webhook(
        &mut *tx,
        org_id,
        id,
        req.url.as_deref().map(str::trim),
        req.event_types.as_deref(),
        req.enabled,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(row.into()))
}

/// DELETE /api/webhooks/{id} — removes the subscription; deliveries
/// cascade (history is intentionally scoped to the subscription's life).
#[utoipa::path(operation_id = "webhooks_delete", delete, path = "/api/webhooks/{id}", tag = "integrations", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 204)))]
pub async fn remove(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManagePlugins)?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    core_db::webhooks::delete_webhook(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// GET /api/webhooks/{id}/deliveries — the in-app delivery log, newest
/// first, keyset-cursor paged.
#[utoipa::path(operation_id = "webhooks_deliveries_list", get, path = "/api/webhooks/{id}/deliveries", tag = "integrations", security(("bearer" = [])),
    params(("id" = Uuid, Path), DeliveryQuery),
    responses((status = 200, body = DeliveryPage)))]
pub async fn deliveries(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Query(q): Query<DeliveryQuery>,
) -> ApiResult<Json<DeliveryPage>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManagePlugins)?;
    let org_id = authed.principal.user.org_id;
    let limit = q.limit.clamp(1, 200);
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    // Existence check gives a clean 404 instead of an empty page.
    core_db::webhooks::get_webhook(&mut *tx, org_id, id).await?;
    let cursor = q.cursor.as_deref().and_then(decode_cursor);
    let rows = core_db::webhooks::list_deliveries(&mut tx, org_id, id, cursor, limit).await?;
    tx.commit().await.map_err(ApiError::from)?;
    let next_cursor = if rows.len() as i64 >= limit {
        rows.last().map(|r| encode_cursor(r.created_at, r.id))
    } else {
        None
    };
    Ok(Json(DeliveryPage {
        deliveries: rows.into_iter().map(DeliveryAttemptView::from).collect(),
        next_cursor,
    }))
}

/// POST /api/webhooks/{id}/test — fires one `webhook.test` envelope into
/// the real pipeline (outbox → relay → stream → signed delivery), then
/// returns 202; poll the deliveries endpoint to watch attempts land.
#[utoipa::path(operation_id = "webhooks_test_fire", post, path = "/api/webhooks/{id}/test", tag = "integrations", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 202, body = TestFireResponse)))]
pub async fn test_fire(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<(axum::http::StatusCode, Json<TestFireResponse>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManagePlugins)?;
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;

    let mut tx = org_tx::begin(&state, org_id, user_id).await?;
    core_db::webhooks::get_webhook(&mut *tx.tx, org_id, id).await?;
    let event_id = Uuid::now_v7();
    let payload = serde_json::json!({
        "source": "test_fire",
        "fired_by": user_id,
        "note": "Manual test fire from the integrations settings page.",
    });
    core_db::outbox::insert(
        &mut tx.tx,
        event_id,
        org_id,
        types::WEBHOOK_TEST,
        "webhook",
        Some(id.to_string().as_str()),
        &payload,
    )
    .await?;
    core_db::audit::insert(
        &mut *tx.tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(user_id),
            action: "webhook.test_fired".into(),
            entity_type: Some("webhook".into()),
            entity_id: Some(id.to_string()),
            detail: serde_json::json!({ "event_id": event_id }),
            ip: None,
            occurred_at: Utc::now(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(TestFireResponse { event_id }),
    ))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TestFireResponse {
    /// The envelope id to look for in the delivery log.
    pub event_id: Uuid,
}
