//! The ingest log (Wave 67 lane `w67-ingest-store`) — the write half both
//! ingest doors share, and the read surface that answers from it.
//!
//! Purpose: `marula.org.ingest_requests` was a counter with no durable
//! record. A number says how much traffic an org's sources produce; it cannot
//! say which source sent what and when, or which source went quiet, and those
//! are the two questions a connector owner asks. `ingest_requests` (migration
//! 0184) holds one row per counted request, and this module is the only path
//! in and the one path out.
//!
//! # The write
//!
//! [`record`] runs after the request has been handled, on its own org-bound
//! transaction, for the same population the counter counts: a request whose
//! HMAC signature verified. A request that never proved a tenant is not
//! counted and not logged, for the reason the counter's own comment gives —
//! nothing about it names an org, so a stranger could otherwise add rows to
//! any org by guessing a source ref.
//!
//! The two answer columns are taken from what the caller actually received:
//! the status is the response's own status, and the refusal token comes from
//! that status. There is no second opinion to drift from.
//!
//! # The read
//!
//! `GET /api/v1/alert-sources/{id}/ingest-log` — one source's log, newest
//! first, keyset-paged. It reads `ingest_requests` and nothing else: until
//! this lane it walked `/api/v1/audit-events` for `webhook.ingested` rows,
//! which required `ViewAuditLog` on top of `ManagePlugins`, so a staff member
//! holding the grant the source surface itself needs got a typed refusal
//! instead of the log — and a REFUSED delivery was invisible, because a
//! refusal writes no audit row.
//!
//! # Why this is a narrowing, not a bypass
//!
//! The caller must hold `ManagePlugins` AND name a source that exists in their
//! org: the source row is read under the same org-bound transaction before any
//! log row, and a source in another org is a 404, the same answer a
//! nonexistent id gets. Both predicates are in the SQL, not applied afterwards.
//!
//! Non-purpose: no replay (the replay fence belongs to the ingest path), no
//! delete (retention owns that), no cross-source query, and no write into the
//! audit trail.

use crate::auth::middleware::{authed, require_permission};
use crate::error::ApiResult;
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{Extensions, StatusCode},
    Json,
};
use core_db::alerting::{AlertSourceAuth, IngestRequestRow, NewIngestRequest};
use core_domain::policy::Permission;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// Which ingest door served a request: the same closed pair as the counter's
/// `source` attribute (`core_otel::metrics::INGEST_SOURCES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    Alert,
    Webhook,
}

impl Endpoint {
    /// The token the column and the counter both use.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Alert => "alert",
            Self::Webhook => "webhook",
        }
    }
}

/// The row one request will be logged as, as far as it is known so far.
///
/// Created at the counter, which is the moment the request becomes countable
/// and the moment the correlation id exists; the fields after it are filled in
/// as the handler learns them, so a delivery that never parsed still logs with
/// its id and its status rather than not at all.
#[derive(Debug, Clone)]
pub struct IngestRecord {
    /// The row's primary key and the value an operator greps the logs for.
    pub request_id: Uuid,
    /// When the endpoint received the request.
    pub received_at: chrono::DateTime<chrono::Utc>,
    /// The provider's idempotency key, once the document parsed.
    pub dedup_key: Option<String>,
    /// The entity the document named, when it named one.
    pub entity_id: Option<String>,
}

impl IngestRecord {
    /// A fresh record for one counted request.
    #[must_use]
    pub fn new() -> Self {
        Self {
            request_id: Uuid::now_v7(),
            received_at: chrono::Utc::now(),
            dedup_key: None,
            entity_id: None,
        }
    }
}

impl Default for IngestRecord {
    fn default() -> Self {
        Self::new()
    }
}

/// The refusal token a status maps to.
///
/// One status carries one reason, and this is that mapping — the same tokens
/// the error envelope already uses, so a row and a reply to the caller read
/// alike. The status itself is never derived from this: it is the response's
/// own status, and this function only names it.
#[must_use]
fn refusal_reason(status: StatusCode) -> Option<&'static str> {
    if status.as_u16() < 400 {
        return None;
    }
    Some(match status.as_u16() {
        401 => "unauthorized",
        403 => "forbidden",
        404 => "not_found",
        409 => "conflict",
        422 => "validation_failed",
        429 => "rate_limited",
        _ => "internal",
    })
}

/// Records one handled ingest request.
///
/// Never fails the request it describes. The ingest answer has already been
/// decided by the time this runs, and the row is evidence about that answer;
/// a write that does not land is a missing log line, while a 5xx would turn a
/// database hiccup into an ingest outage and invite the provider to retry a
/// delivery the endpoint already handled. The failure is logged with the
/// request id, so the gap is discoverable rather than silent.
///
/// How long the row lives is the retention matrix's decision, not this
/// caller's: `core_db::retention::MATRIX` keeps it 7 days compact, 30 standard,
/// 90 extended (floor 7), and the worker's hourly sweep deletes it on the
/// housekeeping role. Nothing in this module deletes.
pub async fn record(
    state: &AppState,
    source: &AlertSourceAuth,
    endpoint: Endpoint,
    byte_count: usize,
    record: &IngestRecord,
    status: StatusCode,
    outcome: Option<&str>,
) {
    let http_status = i16::try_from(status.as_u16()).unwrap_or(500);
    let byte_count = i32::try_from(byte_count).unwrap_or(i32::MAX);
    let new = NewIngestRequest {
        request_id: record.request_id,
        org_id: source.r_org_id,
        alert_source_id: source.r_id,
        source_ref: source.r_source_ref.as_str(),
        endpoint: endpoint.as_str(),
        received_at: record.received_at,
        http_status,
        outcome,
        refusal_reason: refusal_reason(status),
        byte_count,
        dedup_key: record.dedup_key.as_deref(),
        entity_id: record.entity_id.as_deref(),
    };
    let written = org_tx::with_org_tx(
        state,
        source.r_org_id,
        source.r_org_id,
        |mut ctx| async move {
            core_db::alerting::insert_ingest_request(&mut *ctx.tx, new).await?;
            Ok((ctx, ()))
        },
    )
    .await;
    if let Err(err) = written {
        tracing::warn!(
            error = %err,
            request_id = %record.request_id,
            source_ref = %source.r_source_ref,
            "ingest request was not logged"
        );
    }
}

/// One row of a source's log.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IngestLogRowView {
    /// The request id — the correlation id, stable across pages.
    pub id: Uuid,
    /// When the endpoint received the request.
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    /// The door that served it: `alert` or `webhook`.
    pub endpoint: String,
    /// The status the endpoint returned to the caller.
    pub http_status: i16,
    /// True when the endpoint accepted the delivery.
    pub accepted: bool,
    /// What an accepted delivery did (`emitted`, `duplicate`, `collapsed`,
    /// `incident_created`); absent on a refusal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Why a delivery was refused; absent on an acceptance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal_reason: Option<String>,
    /// The request body's length.
    pub byte_count: i32,
    /// The provider's per-event identity: the replay fence's key.
    pub dedup_key: Option<String>,
    /// The entity the envelope carried, when the source set one.
    pub entity_id: Option<String>,
}

impl From<IngestRequestRow> for IngestLogRowView {
    fn from(r: IngestRequestRow) -> Self {
        Self {
            id: r.request_id,
            occurred_at: r.received_at,
            endpoint: r.endpoint,
            http_status: r.http_status,
            accepted: r.http_status < 400,
            outcome: r.outcome,
            refusal_reason: r.refusal_reason,
            byte_count: r.byte_count,
            dedup_key: r.dedup_key,
            entity_id: r.entity_id,
        }
    }
}

/// One page of the log, newest first.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IngestLogPage {
    pub items: Vec<IngestLogRowView>,
    /// Opaque cursor for the next page; absent when this page is the last.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct IngestLogQuery {
    /// Opaque cursor from a previous page's `next_cursor`.
    pub cursor: Option<String>,
    /// Page size, clamped to 1..=200.
    pub limit: Option<i64>,
}

/// GET /api/v1/alert-sources/{id}/ingest-log — what one inbound source sent,
/// newest first.
///
/// `ManagePlugins`, the same grant the source list takes: the log is the read
/// half of a surface only that grant can configure.
#[utoipa::path(operation_id = "alert_source_ingest_log", get,
    path = "/api/v1/alert-sources/{id}/ingest-log", tag = "alerts",
    security(("bearer" = [])),
    params(("id" = Uuid, Path), IngestLogQuery),
    responses((status = 200, body = IngestLogPage)))]
pub async fn ingest_log(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Query(q): Query<IngestLogQuery>,
) -> ApiResult<Json<IngestLogPage>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManagePlugins)?;
    let org_id = authed.principal.user.org_id;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);

    // A cursor is opaque to the caller but versioned inside, so a token from a
    // newer build is refused rather than mis-anchoring the walk.
    let cursor = match q.cursor.as_deref() {
        Some(raw) => Some(super::audit::decode_cursor(raw).ok_or_else(|| {
            crate::error::ApiError::Validation(
                "`cursor` did not decode; pass the `next_cursor` from a previous page".into(),
            )
        })?),
        None => None,
    };

    let mut tx = org_tx::begin(&state, org_id, authed.principal.user.id).await?;
    // Resolve the source first, under the same org-bound transaction: it is what
    // makes "the caller may see this source" true before the log read, and a
    // source in another org answers 404 rather than an empty log.
    let source = core_db::alerting::get_alert_source(&mut *tx.tx, org_id, id).await?;
    let rows =
        core_db::alerting::list_ingest_requests(&mut *tx.tx, org_id, source.id, cursor, limit + 1)
            .await?;
    tx.commit().await?;

    // The `+1` probe: one extra row means there is another page.
    let has_more = rows.len() as i64 > limit;
    let items: Vec<IngestLogRowView> = rows
        .into_iter()
        .take(limit as usize)
        .map(IngestLogRowView::from)
        .collect();
    let next_cursor = if has_more {
        items
            .last()
            .map(|row| super::audit::encode_cursor(row.occurred_at, row.id))
    } else {
        None
    };
    Ok(Json(IngestLogPage { items, next_cursor }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The column's vocabulary and the counter's `source` attribute are one
    /// closed pair: a new endpoint that counts itself must log itself too, and
    /// this test is where a divergence fails.
    #[test]
    fn the_endpoint_tokens_are_the_counters_sources() {
        for (endpoint, token) in [(Endpoint::Alert, "alert"), (Endpoint::Webhook, "webhook")] {
            assert_eq!(endpoint.as_str(), token);
            assert!(
                core_otel::metrics::INGEST_SOURCES.contains(&token),
                "{token} is not a counter source"
            );
        }
    }

    /// A refusal names its status, an acceptance names nothing as a refusal,
    /// and every status either owns one or owns none — never both.
    #[test]
    fn a_status_maps_to_at_most_one_refusal_reason() {
        assert_eq!(refusal_reason(StatusCode::OK), None);
        assert_eq!(
            refusal_reason(StatusCode::TOO_MANY_REQUESTS),
            Some("rate_limited")
        );
        assert_eq!(
            refusal_reason(StatusCode::UNPROCESSABLE_ENTITY),
            Some("validation_failed")
        );
        assert_eq!(
            refusal_reason(StatusCode::INTERNAL_SERVER_ERROR),
            Some("internal")
        );
    }
}
