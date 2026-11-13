//! The M2M ingest log (Wave 67 lane `w67-read-surfaces`).
//!
//! Purpose: `/settings/integrations` shows one inbound source's accepted
//! envelopes. Until this route existed it read them off `/api/v1/audit-events`,
//! which requires `ViewAuditLog` — so a staff member holding `ManagePlugins`
//! (the grant the source surface itself needs) got the typed refusal copy
//! instead of the log, and the page had to walk the whole org-wide ledger and
//! filter client-side to find one source's rows.
//!
//! # What it reads
//!
//! One slice of the audit trail: `action = 'webhook.ingested'` and
//! `detail->>'source_ref'` equal to the source's own ref. The receiver writes
//! exactly one such event per ACCEPTED envelope (a duplicate hits the replay
//! fence and writes nothing), so this is the ingest log, not an approximation.
//!
//! # Why this is a narrowing, not a bypass
//!
//! The caller must hold `ManagePlugins` AND name a source that exists in their
//! org — the source row is read under the same org-bound transaction before any
//! audit row is touched, and a source in another org is a 404, the same answer a
//! nonexistent id gets. What comes back is one source's ingest events: no other
//! action, no other source, no write. The full trail still needs
//! `ViewAuditLog`; nothing here widens that.
//!
//! Non-purpose: no replay (the replay fence is the ingest path's), no delete, no
//! cross-source query, and no new write into the ledger.

use crate::auth::middleware::{authed, require_permission};
use crate::error::ApiResult;
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use core_domain::entities::AuditEvent;
use core_domain::policy::Permission;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// One accepted envelope, as the source's log shows it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IngestLogRowView {
    /// The audit event's id — stable across pages.
    pub id: Uuid,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    /// The provider's per-event identity: the replay fence's key.
    pub dedup_key: Option<String>,
    /// The entity the envelope carried, when the source set one.
    pub entity_id: Option<String>,
}

impl From<AuditEvent> for IngestLogRowView {
    fn from(e: AuditEvent) -> Self {
        let detail = e.detail;
        Self {
            id: e.id,
            occurred_at: e.occurred_at,
            dedup_key: detail
                .get("dedup_key")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            entity_id: e.entity_id,
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

/// GET /api/v1/alert-sources/{id}/ingest-log — the envelopes one inbound source
/// accepted, newest first.
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
    // makes "the caller may see this source" true before the audit read, and a
    // source in another org answers 404 rather than an empty log.
    let source = core_db::alerting::get_alert_source(&mut *tx.tx, org_id, id).await?;
    let rows = core_db::audit::list_ingest_for_source(
        &mut *tx.tx,
        org_id,
        &source.source_ref,
        cursor,
        limit + 1,
    )
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
