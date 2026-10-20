//! Audit log endpoint (admin-only, §6): filtered listing with keyset
//! cursor pagination over the append-only trail.
//!
//! Two read directions on one path, because they answer two different
//! questions and each has its own safety property:
//!
//! * default (newest first) — "what just happened". A page may shift when new
//!   rows arrive; that is fine for a human reading a list.
//! * `order=asc` + `after=<cursor>` — the EXPORT contract (TASKLIST R7a).
//!   An external consumer stores the cursor it got back and resumes from it:
//!   rows arrive oldest-first, keyed on `(occurred_at, id)`, so a concurrently
//!   written row always lands in a later page and never between two pages
//!   that were already delivered. That is what makes an audit export
//!   resumable rather than approximately-once.
//!
//! The cursor is opaque and versioned (`v1|<rfc3339>|<uuid>`, base64url). A
//! consumer must treat it as a token, never parse it — but the version prefix
//! is there so a future format change is detectable at the boundary instead
//! of silently mis-anchoring a resuming exporter. Cursors minted before the
//! prefix existed (`<rfc3339>|<uuid>`) still decode, so a stored checkpoint
//! from an older build keeps working.

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

/// The cursor format revision. Bump when the payload inside the token
/// changes shape; the decoder accepts every revision it still understands.
pub const CURSOR_VERSION: &str = "v1";

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct AuditQuery {
    /// Legacy cursor (kept for compatibility): strictly older than this.
    pub before: Option<chrono::DateTime<chrono::Utc>>,
    /// Keyset cursor from a previous page's `next_cursor`; strictly older
    /// than this position (newest-first mode).
    pub cursor: Option<String>,
    /// EXPORT cursor: a previous page's `next_cursor` in ascending mode.
    /// Rows strictly AFTER this position, oldest first, so an external
    /// consumer can resume exactly where it stopped. Mutually exclusive with
    /// `cursor`.
    pub after: Option<String>,
    /// `desc` (default) for a human list, `asc` for an export tail. `after`
    /// implies `asc`.
    pub order: Option<String>,
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

/// Decodes a cursor produced by [`encode_cursor`].
///
/// Accepts the versioned form (`v1|ts|uuid`) and the pre-versioning form
/// (`ts|uuid`) so a checkpoint written by an older build still resumes.
fn decode_cursor(raw: &str) -> Option<(chrono::DateTime<chrono::Utc>, Uuid)> {
    let decoded = B64.decode(raw.as_bytes()).ok()?;
    let text = String::from_utf8(decoded).ok()?;
    // Split from the RIGHT: the uuid carries no '|', but the prefix may grow.
    let (head, id) = text.rsplit_once('|')?;
    let id = id.parse().ok()?;
    let ts = match head.rsplit_once('|') {
        // Versioned: `<version>|<rfc3339>`; the tag must not be read as a ts.
        Some((version, ts)) if version.starts_with('v') => ts,
        // Pre-versioning: the whole head is the timestamp.
        _ => head,
    };
    Some((ts.parse().ok()?, id))
}

fn encode_cursor(at: chrono::DateTime<chrono::Utc>, id: Uuid) -> String {
    B64.encode(format!("{CURSOR_VERSION}|{}|{}", at.to_rfc3339(), id).as_bytes())
}

/// GET /api/audit-events — append-only audit trail. Newest first by default;
/// `order=asc` + `after` turns it into a forward-only export tail.
#[utoipa::path(operation_id = "audit_list", get, path = "/api/audit-events", tag = "audit", security(("bearer" = [])),
    params(AuditQuery),
    responses((status = 200, body = AuditPage),
              (status = 400, description = "cursor and after are mutually exclusive, order is not asc|desc, or a cursor does not decode")))]
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
    // Two cursor params ask for two different walks of the same trail; taking
    // one and ignoring the other would silently re-deliver rows an exporter
    // already handled, so it is a 400 rather than a precedence rule.
    if q.cursor.is_some() && q.after.is_some() {
        return Err(ApiError::Validation(
            "pass either `cursor` (newest-first) or `after` (ascending export), not both".into(),
        ));
    }
    let ascending = q.order.as_deref() == Some("asc") || q.after.is_some();
    if let Some(order) = q.order.as_deref() {
        if order != "asc" && order != "desc" {
            return Err(ApiError::Validation(
                "`order` must be `asc` or `desc`".into(),
            ));
        }
    }
    let limit = q.limit.clamp(1, 500);
    // A cursor that does not decode is a broken checkpoint, not an empty
    // page: answering 200 with the newest rows would make an exporter
    // re-deliver everything it had already sent.
    let cursor = match q.cursor.as_deref() {
        Some(raw) => Some(decode_cursor(raw).ok_or_else(|| {
            ApiError::Validation("`cursor` is not a cursor this build issued".into())
        })?),
        None => None,
    };
    let after = match q.after.as_deref() {
        Some(raw) => Some(decode_cursor(raw).ok_or_else(|| {
            ApiError::Validation("`after` is not a cursor this build issued".into())
        })?),
        None => None,
    };
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let filter = AuditFilter {
        actor: q.actor,
        action: q.action,
        from: q.from,
        to: q.to,
        before: q.before,
        cursor,
    };
    let events = if ascending {
        core_db::audit::list_forward(&mut *tx, authed.principal.user.org_id, filter, after, limit)
            .await?
    } else {
        core_db::audit::list_filtered(&mut *tx, authed.principal.user.org_id, filter, limit).await?
    };
    tx.commit().await.map_err(ApiError::from)?;
    // A full page hints another exists; anchor it to the last row. In
    // ascending mode the same rule holds and the anchor is strictly LATER,
    // which is what makes the position resumable.
    let next_cursor = if events.len() as i64 >= limit {
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
    /// Opaque cursor for the next page; absent when exhausted. In ascending
    /// mode pass it back as `after` to resume the export exactly here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn at(secs: i64) -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn cursors_round_trip_through_the_encoder() {
        let id = Uuid::now_v7();
        let raw = encode_cursor(at(0), id);
        assert_eq!(decode_cursor(&raw), Some((at(0), id)));
    }

    /// A checkpoint written before the version prefix existed must still
    /// resume — a format change that invalidates stored checkpoints forces
    /// every exporter to re-deliver history.
    #[test]
    fn pre_versioning_cursors_still_decode() {
        let id = Uuid::now_v7();
        let legacy = B64.encode(format!("{}|{}", at(42).to_rfc3339(), id).as_bytes());
        assert_eq!(decode_cursor(&legacy), Some((at(42), id)));
    }

    #[test]
    fn garbage_cursors_are_rejected_rather_than_read_as_empty() {
        for bad in [
            "",
            "not-base64!!",
            &B64.encode("no-pipes"),
            &B64.encode(format!("v1|not-a-time|{}", Uuid::now_v7())),
            &B64.encode("v1|1700000000|not-a-uuid"),
        ] {
            assert_eq!(decode_cursor(bad), None, "{bad:?} must not decode");
        }
    }

    /// The token's payload names its version, which is what lets a future
    /// build detect — rather than silently misread — a checkpoint it does
    /// not understand.
    #[test]
    fn the_encoded_payload_carries_the_cursor_version() {
        let raw = encode_cursor(at(0), Uuid::now_v7());
        let decoded = String::from_utf8(B64.decode(raw).expect("base64")).expect("utf8");
        assert!(
            decoded.starts_with(&format!("{CURSOR_VERSION}|")),
            "payload was {decoded:?}"
        );
    }
}
