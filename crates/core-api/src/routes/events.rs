//! `GET /api/events/stream` — the server→client push channel (Wave 12
//! lane 12): SSE frames carrying typed contract events as they commit,
//! plus ephemeral presence frames.
//!
//! ## Frame taxonomy (binding distinction)
//!
//! * **Contract-event frames** — `type` is a registered contract event
//!   type (`ticket.updated`, `ticket.comment_added`, ...; registry in
//!   `core-events` contract.rs, goldens in `fixtures/events/v1`). The
//!   frame is the notify payload's identity:
//!   `{type, org_id, entity, entity_id, seq, at}`. seq is the per-entity
//!   monotonic ledger value: reconciliation is idempotent by seq
//!   (at-least-once delivery dedupes in the ledger). These frames mirror
//!   outbox rows — the trail stays authoritative.
//! * **Presence frames** — `type: "presence"` plus a `viewers` roster.
//!   EPHEMERAL by design: presence is never written to the database, never
//!   enters the event trail, and is NOT registered in the contract
//!   registry (no goldens, no U5 manifest). It is transport state, not
//!   contract.
//!
//! Transport posture: SSE over WS (one-way server→client is all the
//! reconcile path needs; trivially proxy-friendly; EventSource reconnects
//! natively). Heartbeat comments (`: ping`) keep proxies from idling the
//! connection out. Delivery is at-least-once in spirit: events missed
//! while disconnected are recovered by the client's snapshot rehydration
//! on reconnect, and duplicate frames are harmless under seq dedupe.
//! Auth is the existing bearer/session middleware (`require_auth`) — the
//! principal's org scopes every frame at fan-out.

use crate::auth::middleware::authed;
use crate::error::{ApiError, ApiResult};
use crate::realtime;
use axum::extract::Query;
use axum::http::Extensions;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::IntoResponse;
use serde::Deserialize;
use std::convert::Infallible;
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct StreamQuery {
    /// Optional per-entity scope: only frames for this ticket (its events
    /// AND its presence) are delivered. Omit for the org-wide stream.
    #[serde(default)]
    pub ticket: Option<Uuid>,
}

/// GET /api/events/stream — live contract-event + presence frames.
///
/// Response is `text/event-stream`: `data:` frames are the StreamFrame
/// JSON documented above; comment heartbeats every 15s keep intermediaries
/// from closing an idle connection. A per-user connection cap (8) guards
/// fan-out memory; over the cap the request is a typed 429.
#[utoipa::path(
    operation_id = "events_stream",
    get,
    path = "/api/events/stream",
    tag = "events",
    security(("bearer" = [])),
    params(StreamQuery),
    responses(
        (status = 200, description = "text/event-stream of StreamFrame JSON data lines with comment heartbeats", content_type = "text/event-stream"),
        (status = 401, description = "missing or invalid bearer token"),
        (status = 429, description = "per-user connection cap reached; reconnect later"),
    )
)]
pub async fn stream(
    extensions: Extensions,
    Query(q): Query<StreamQuery>,
) -> ApiResult<axum::response::Response> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let scope = q.ticket.map(|id| ("ticket".to_string(), id.to_string()));
    let Some(rx) = realtime::hub().subscribe(org_id, authed.principal.user.id, scope) else {
        return Err(ApiError::RateLimited {
            retry_after_secs: 30,
        });
    };

    let frames = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|frame| {
            let data = serde_json::to_string(&*frame).unwrap_or_else(|_| "{}".to_string());
            (Ok::<_, Infallible>(SseEvent::default().data(data)), rx)
        })
    });
    let body = Sse::new(frames).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    );
    Ok(body.into_response())
}
