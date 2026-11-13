//! Generic M2M webhook receiver (Wave 43 lane 1): HMAC-signed ingest that
//! emits domain-event envelopes onto the outbox stream — NOT alert rows.
//!
//! Surface layout (one module, two mounts — kept in a single file to
//! minimize merge surface on mod.rs):
//!
//! * `POST /api/webhooks/ingest/{source_ref}` — the machine-to-machine
//!   ingest endpoint. Mounted OUTSIDE the JWT middleware (like the alert
//!   ingestor and email inbound webhook): the `source_ref` path segment
//!   resolves org + sealed secret through `alert_source_auth_lookup`, then
//!   `X-Marula-Signature` (hex HMAC-SHA256 over the exact request body) is
//!   verified constant-time.
//!
//! The generic path reuses the SAME source registry as the alert ingestor
//! (`alert_sources`): a source with `event_type IS NOT NULL` is a generic
//! M2M receiver. This avoids a second source table — no fork.
//!
//! After HMAC verification the handler:
//! 1. Enforces a per-source rate ceiling (count over a sliding window).
//! 2. Gates replays via a UNIQUE (org, source, dedup_key) fence.
//! 3. Emits the envelope into the outbox — `event_type` comes from the
//!    source declaration, `board_id` is carried in the payload so
//!    ticket-less triggers (lane 3) can scope their rules.

use crate::crypto;
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::routes::ingest_log;
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use core_db::alerting::{self, AlertSourceAuth};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

pub const SIGNATURE_HEADER: &str = crate::routes::alerts::SIGNATURE_HEADER;

/// Per-source rate ceiling: most ingress traffic bursts within seconds;
/// a 1-minute window with a generous ceiling catches runaway loops without
/// needing Redis or a token bucket.
const RATE_WINDOW_SECS: i32 = 60;
const RATE_MAX_PER_WINDOW: i64 = 300;

// --- wire types ---------------------------------------------------------------

/// POST /api/webhooks/ingest — the generic machine envelope. `dedup_key`
/// is the replay gate (UNIQUE per source); `payload` is the provider's
/// arbitrary data; `entity_id` optionally names the entity the event
/// refers to (defaults to the dedup_key).
#[derive(Debug, Deserialize)]
pub struct WebhookIngestRequest {
    /// Client-provided idempotency key. A repeat within the source's
    /// replay window returns `duplicate` and writes nothing.
    pub dedup_key: String,
    /// Optional entity id stamped on the envelope. Defaults to dedup_key.
    pub entity_id: Option<String>,
    /// The provider's arbitrary payload. Must be a JSON object.
    #[serde(default = "empty_object")]
    pub payload: Value,
    /// When the event occurred (defaults to receipt time).
    pub occurred_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn empty_object() -> Value {
    json!({})
}

/// POST /api/webhooks/ingest — outcome of one signed delivery.
#[derive(Debug, Serialize)]
pub struct WebhookIngestResponse {
    /// `emitted` (first time), `duplicate` (replay caught by the fence).
    pub status: String,
    /// The event_id of the emitted envelope (present only for `emitted`).
    pub event_id: Option<Uuid>,
    /// The board this envelope is scoped to.
    pub board_id: Uuid,
}

// --- ingest endpoint ----------------------------------------------------------

/// POST /api/webhooks/ingest/{source_ref} — HMAC-verified generic webhook
/// ingest.
///
/// Responses: 200 with a terminal `status`; 401 missing/bad signature; 403
/// disabled source or rate-limited; 404 unknown source_ref or not a generic
/// source; 422 malformed payload.
pub async fn webhook_ingest(
    State(state): State<AppState>,
    Path(source_ref): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (source, event_type) =
        match authorized_webhook_source(&state, source_ref.trim(), &headers, &body).await {
            Ok(source) => source,
            Err(err) => return err.into_response(),
        };

    // Per-org ingest volume (Wave 52 lane R12). Counted HERE, not on the
    // response: the signature just proved which org this request belongs to,
    // and every later exit — rate limit, replay fence, a malformed envelope —
    // is still that org's traffic, which is what a metering consumer wants to
    // see. A request with a bad signature is deliberately not counted: nothing
    // about it names a tenant, so counting it would let a stranger add to any
    // org's number by guessing a source ref.
    core_otel::metrics::org_ingest(&source.r_org_id.to_string(), "webhook");

    // The durable half of that counter (Wave 67 lane `w67-ingest-store`): one
    // row per counted request, written once the answer exists so the status in
    // the row is the status the caller got.
    let mut logged = ingest_log::IngestRecord::new();
    let mut outcome = None;
    let response: Response =
        match ingest_counted(&state, &source, &event_type, &body, &mut logged).await {
            Ok(payload) => {
                outcome = Some(payload.0.status.clone());
                payload.into_response()
            }
            Err(err) => err.into_response(),
        };
    ingest_log::record(
        &state,
        &source,
        ingest_log::Endpoint::Webhook,
        body.len(),
        &logged,
        response.status(),
        outcome.as_deref(),
    )
    .await;
    response
}

/// The pre-count half of the generic door: resolve the source, refuse a
/// disabled one, refuse an alert-profile ref, and verify the signature.
///
/// Returns the declared `event_type` beside the source. Nothing here is counted
/// or logged: until the signature proves the tenant there is no org to attribute
/// the request to, and a 404 on an alert-source ref cannot attribute one either.
async fn authorized_webhook_source(
    state: &AppState,
    source_ref: &str,
    headers: &HeaderMap,
    body: &Bytes,
) -> ApiResult<(AlertSourceAuth, String)> {
    // 2. Resolve org + sealed secret (pre-auth lookup; the ref is routing
    //    info, not a credential). Same seam as the alert ingestor.
    let source = alerting::auth_lookup_source(&state.pool, source_ref)
        .await?
        .ok_or(ApiError::NotFound)?;
    if !source.r_enabled {
        return Err(ApiError::Forbidden("webhook source is disabled".into()));
    }

    // 3. Only generic M2M sources land here. An alert-source ref returns
    //    404 so the boundary is explicit — you must use /alerts/ingest.
    let event_type = source.r_event_type.clone().ok_or(ApiError::NotFound)?;

    // 4. HMAC over the RAW body — tampering invalidates even a stale
    //    valid signature. Constant-time via core_mail::hmac.
    let presented = headers
        .get(SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if presented.is_empty() {
        return Err(ApiError::Unauthorized("missing webhook signature".into()));
    }
    let secret = crypto::open(&state.config.secret_key, &source.r_secret_encrypted)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("webhook source secret: {e}")))?;
    if !core_mail::hmac::verify(&secret, body, presented) {
        return Err(ApiError::Unauthorized("invalid webhook signature".into()));
    }
    Ok((source, event_type))
}

/// The counted half of the generic door: parse, rate-limit, replay-fence, and
/// emit the envelope. Every exit from here is one logged row.
async fn ingest_counted(
    state: &AppState,
    source: &AlertSourceAuth,
    event_type: &str,
    body: &Bytes,
    logged: &mut ingest_log::IngestRecord,
) -> ApiResult<Json<WebhookIngestResponse>> {
    // 5. Parse the generic envelope document.
    let req: WebhookIngestRequest = serde_json::from_slice(body).map_err(|e| {
        tracing::debug!(error = %e, "webhook payload failed to parse");
        ApiError::Validation("webhook payload is malformed or has the wrong shape".into())
    })?;
    if req.dedup_key.trim().is_empty() {
        return Err(ApiError::Validation("dedup_key is required".into()));
    }
    if !req.payload.is_object() {
        return Err(ApiError::Validation("payload must be a JSON object".into()));
    }
    logged.dedup_key = Some(req.dedup_key.trim().to_string());

    let entity_id = req
        .entity_id
        .as_ref()
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| req.dedup_key.trim().to_string());
    logged.entity_id = Some(entity_id.clone());
    let now = chrono::Utc::now();

    // 6. All DB writes happen inside one org-bound transaction. `source`
    //    is Clone — clone the fields the closure needs, keep `source`
    //    for the post-transaction response.
    let source_ref_owned = source.r_source_ref.clone();
    let source_id = source.r_id;

    let response = org_tx::with_org_tx(
        state,
        source.r_org_id,
        source.r_org_id,
        |mut ctx| async move {
            // 7. Rate limit — reject before burning the replay fence.
            let recent = alerting::count_recent_ingests(
                &mut *ctx.tx,
                source.r_org_id,
                source.r_id,
                RATE_WINDOW_SECS,
            )
            .await?;
            if recent >= RATE_MAX_PER_WINDOW {
                return Err(ApiError::RateLimited {
                    retry_after_secs: RATE_WINDOW_SECS as i64,
                });
            }

            // 8. Replay fence — UNIQUE (org, source, dedup_key) makes a
            //    replayed delivery a no-op.
            let was_new = alerting::insert_webhook_dedup(
                &mut ctx.tx,
                source.r_org_id,
                source.r_id,
                &req.dedup_key,
                now,
            )
            .await?;
            if !was_new {
                return Ok((
                    ctx,
                    WebhookIngestResponse {
                        status: "duplicate".into(),
                        event_id: None,
                        board_id: source.r_board_id,
                    },
                ));
            }

            // 9. Build the envelope payload. `board_id` is carried at the
            //    top level so automation matching (automations.rs) and
            //    ticket-less triggers (lane 3) can scope their rules. The
            //    provider's data rides under `data`.
            let envelope_payload = json!({
                "board_id": source.r_board_id,
                "source_ref": source.r_source_ref,
                "source_id": source.r_id,
                "entity_id": entity_id,
                "data": req.payload,
            });

            // 10. Emit the domain event envelope into the outbox.
            let event_id = Uuid::now_v7();
            core_db::outbox::insert(
                &mut ctx.tx,
                event_id,
                source.r_org_id,
                event_type,
                "webhook",
                Some(entity_id.as_str()),
                &envelope_payload,
            )
            .await?;

            // 11. Audit trail — `webhook.ingested` with the connector actor.
            core_db::audit::insert(
                &mut *ctx.tx,
                &AuditEvent {
                    id: Uuid::now_v7(),
                    org_id: source.r_org_id,
                    actor: Actor::Connector(format!("alert-source:{source_ref_owned}")),
                    action: types::WEBHOOK_INGESTED.to_string(),
                    entity_type: Some("webhook".into()),
                    entity_id: Some(entity_id.clone()),
                    detail: json!({
                        "source_ref": source_ref_owned,
                        "source_id": source_id,
                        "dedup_key": req.dedup_key,
                    }),
                    ip: None,
                    occurred_at: chrono::Utc::now(),
                },
            )
            .await?;

            Ok((
                ctx,
                WebhookIngestResponse {
                    status: "emitted".into(),
                    event_id: Some(event_id),
                    board_id: source.r_board_id,
                },
            ))
        },
    )
    .await?;

    Ok(Json(response))
}

// --- router builders ----------------------------------------------------------

/// The generic M2M ingest route: mounted OUTSIDE the JWT middleware on
/// both generations (`/api/v1` canonical + `/api` legacy alias).
pub fn public_routes(gen: crate::routes::ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new().route(
        &format!("{base}/webhooks/ingest/{{source_ref}}"),
        post(webhook_ingest),
    )
}
