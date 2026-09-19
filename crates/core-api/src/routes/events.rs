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
//!
//! ## Resume (Wave 13 lane 10)
//!
//! Every frame carries its event uuid as the SSE `id:` field (contract
//! events: the outbox row's uuid; presence frames: a per-frame uuid).
//! On reconnect EventSource sends `Last-Event-ID` automatically; the
//! server then REPLAYS the events committed after that cursor from the
//! outbox (bounded by `realtime::REPLAY_BUFFER`, default 256), before the
//! live stream resumes — the ledger dedupes by seq, so replayed and live
//! frames interleave safely. Cutoff: a cursor further back than the
//! replay buffer (or an unknown/foreign-org/presence cursor id) gets NO
//! replay — the client falls back to its full snapshot rehydration, the
//! pre-resume behavior.

use crate::auth::middleware::authed;
use crate::error::{ApiError, ApiResult};
use crate::realtime;
use axum::extract::Query;
use axum::http::{Extensions, HeaderMap};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::IntoResponse;
use futures_util::StreamExt;
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
/// JSON documented above, each carrying its event uuid as the SSE `id:`
/// field; comment heartbeats every 15s keep intermediaries from closing
/// an idle connection. A per-user connection cap (8) guards fan-out
/// memory; over the cap the request is a typed 429. A `Last-Event-ID`
/// header triggers a bounded outbox replay of the events missed while
/// disconnected (see the resume section above); past the replay buffer
/// the client falls back to full snapshot rehydration.
#[utoipa::path(
    operation_id = "events_stream",
    get,
    path = "/api/events/stream",
    tag = "events",
    security(("bearer" = [])),
    params(StreamQuery),
    responses(
        (status = 200, description = "text/event-stream of StreamFrame JSON data lines with id: fields and comment heartbeats; honors Last-Event-ID for bounded resume replay", content_type = "text/event-stream"),
        (status = 401, description = "missing or invalid bearer token"),
        (status = 429, description = "per-user connection cap reached; reconnect later"),
    )
)]
pub async fn stream(
    extensions: Extensions,
    headers: HeaderMap,
    Query(q): Query<StreamQuery>,
    axum::extract::State(state): axum::extract::State<crate::state::AppState>,
) -> ApiResult<axum::response::Response> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let scope = q.ticket.map(|id| ("ticket".to_string(), id.to_string()));
    let Some(rx) = realtime::hub().subscribe(org_id, authed.principal.user.id, scope.clone())
    else {
        return Err(ApiError::RateLimited {
            retry_after_secs: 30,
        });
    };

    // Resume: replay committed-but-missed events BEFORE the live stream
    // resumes. The subscriber is registered first, so anything committed
    // during the replay query is already queued in the live buffer — no
    // gap, only harmless at-least-once duplication (ledger dedupes by
    // seq). A past-the-buffer or unknown cursor replays nothing. The
    // replay rides an ORG-BOUND tx (Wave 18 lane 2b): outbox_events /
    // entity_write_seqs are RLS-gated and the raw pool carries no GUC.
    let mut replay = Vec::new();
    let mut cutoff = false;
    if let Some(last_event_id) = headers.get("last-event-id").and_then(|v| v.to_str().ok()) {
        let mut tx = match state.pool.begin().await {
            Ok(tx) => tx,
            Err(e) => return Err(ApiError::from(e)),
        };
        if let Err(e) = core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id).await {
            return Err(ApiError::from(e));
        }
        let (frames, past_buffer) =
            realtime::replay_after(&mut tx, org_id, scope.clone(), last_event_id).await;
        replay = frames;
        cutoff = past_buffer;
        // Frames are already materialized; the tx only scopes the reads.
        let _ = tx.rollback().await;
    }

    let replay_stream = futures_util::stream::iter(replay.into_iter().map(|frame| {
        let id = frame.id.map(|u| u.to_string());
        let data = serde_json::to_string(&frame).unwrap_or_else(|_| "{}".to_string());
        Ok::<_, Infallible>(sse_frame(data, id))
    }));
    let cutoff_stream = if cutoff {
        futures_util::stream::once(async {
            Ok::<_, Infallible>(
                SseEvent::default()
                    .comment("resume cursor past the replay buffer; rehydrate from snapshots"),
            )
        })
        .boxed()
    } else {
        futures_util::stream::empty::<Result<SseEvent, Infallible>>().boxed()
    };

    let frames = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|frame| {
            let id = frame.id.map(|u| u.to_string());
            let data = serde_json::to_string(&*frame).unwrap_or_else(|_| "{}".to_string());
            (Ok::<_, Infallible>(sse_frame(data, id)), rx)
        })
    });
    let body = Sse::new(replay_stream.chain(cutoff_stream).chain(frames)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    );
    Ok(body.into_response())
}

fn sse_frame(data: String, id: Option<String>) -> SseEvent {
    let event = SseEvent::default().data(data);
    match id {
        Some(id) => event.id(id),
        None => event,
    }
}
