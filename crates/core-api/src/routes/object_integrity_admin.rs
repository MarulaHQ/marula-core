//! The object-integrity read surface (Wave 67 lane `w67-read-surfaces`).
//!
//! Purpose: the worker's object-GC pass classifies every finding
//! `orphan_deleted` or `bytes_missing` and writes it to
//! `object_integrity_findings`, but until this module nothing read those rows
//! back: the only query over the table selected `storage_key` for
//! de-duplication. An operator asking "what did the pass find, and is anything
//! still broken" had to open SQL.
//!
//! # Read-only, on purpose
//!
//! The pass is the writer, and it runs in the worker's owner pool because it
//! spans every org (a key's namespace carries the org, but the deleted object
//! and the store listing do not). Reclaiming bytes or re-uploading them is not
//! this surface's job: a missing-bytes finding is repaired by uploading the
//! object again, and an orphan is already gone. So this module has one verb, a
//! GET, and no audit event — a read that changes nothing is not a decision.
//!
//! # Scope
//!
//! Findings are per org when the key carries an org prefix, and instance-wide
//! (NULL) when it predates that convention; migration 0146's policy admits both
//! to an org-scoped reader, which is exactly what a page needs — an operator
//! cannot repair a key they are not allowed to see, and a NULL key belongs to no
//! org to hide it from.
//!
//! Non-purpose: no fix action, no bulk repair, no retention, and no second
//! writer.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Query, State},
    http::Extensions,
    Json,
};
use core_db::object_gc::{FindingFilter, FindingKind, IntegrityFinding, OpenFindingCounts};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// The Admin matrix, the same gate the quarantine and dead-work surfaces use:
/// reads take `View`, which owner, admin and an auditor all hold.
fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

/// One finding as the operator surface reads it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IntegrityFindingView {
    pub id: Uuid,
    /// Groups every finding of one pass.
    pub pass_id: Uuid,
    pub found_at: chrono::DateTime<chrono::Utc>,
    /// `orphan_deleted` — bytes with no row referencing them.
    /// `bytes_missing` — a row whose bytes are gone.
    pub kind: String,
    pub storage_key: String,
    /// Bytes reclaimed (orphan) or bytes that should have been there (missing).
    pub size_bytes: Option<i64>,
    /// Which table claimed the key, for a missing-bytes finding.
    pub source_table: Option<String>,
    /// Set when a later pass saw the same key healthy again.
    pub resolved_at: Option<chrono::DateTime<chrono::Utc>>,
    pub note: Option<String>,
}

impl From<IntegrityFinding> for IntegrityFindingView {
    fn from(f: IntegrityFinding) -> Self {
        Self {
            id: f.id,
            pass_id: f.pass_id,
            found_at: f.found_at,
            kind: f.kind.as_str().to_string(),
            storage_key: f.storage_key,
            size_bytes: f.size_bytes,
            source_table: f.source_table,
            resolved_at: f.resolved_at,
            note: f.note,
        }
    }
}

/// One page of the ledger, with the two open counts beside it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IntegrityFindingPage {
    /// Newest finding first.
    pub items: Vec<IntegrityFindingView>,
    /// Offset for the next page; absent when this page is the last.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<i64>,
    /// Open findings by direction, over the whole org — not just this page.
    pub open_orphans: i64,
    pub open_missing: i64,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct IntegrityFindingQuery {
    /// Restrict to one direction: `orphan_deleted` or `bytes_missing`.
    pub kind: Option<String>,
    /// Include findings a later pass resolved. Off by default: the list exists
    /// to show what is still broken.
    pub include_resolved: Option<bool>,
    /// Page size, clamped to 1..=200.
    pub limit: Option<i64>,
    /// Rows to skip, from a previous page's `next_offset`.
    pub offset: Option<i64>,
}

/// Parses the kind filter. A typo is a 422, never an empty page that reads as
/// "nothing was found".
fn check_kind(raw: &str) -> ApiResult<FindingKind> {
    FindingKind::parse(raw).ok_or_else(|| {
        ApiError::Validation(format!(
            "unknown finding kind {raw:?}; expected one of orphan_deleted, bytes_missing"
        ))
    })
}

/// GET /api/v1/admin/object-integrity/findings — what the object-integrity
/// pass found, newest first, with the open counts per direction.
#[utoipa::path(operation_id = "object_integrity_findings", get,
    path = "/api/v1/admin/object-integrity/findings", tag = "object_integrity",
    security(("bearer" = [])),
    params(IntegrityFindingQuery),
    responses((status = 200, body = IntegrityFindingPage)))]
pub async fn list_findings(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<IntegrityFindingQuery>,
) -> ApiResult<Json<IntegrityFindingPage>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let limit = q.limit.unwrap_or(100).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let kind = match q.kind.as_deref() {
        Some(raw) => Some(check_kind(raw)?),
        None => None,
    };

    let mut tx = org_tx::begin(&state, org_id, authed.principal.user.id).await?;
    let rows = core_db::object_gc::list_findings(
        &mut *tx.tx,
        org_id,
        FindingFilter {
            kind,
            include_resolved: q.include_resolved.unwrap_or(false),
            // The `+1` is the page probe: one extra row means there is a next
            // page, without a second COUNT over the ledger.
            limit: limit + 1,
            offset,
        },
    )
    .await?;
    let counts: OpenFindingCounts =
        core_db::object_gc::open_finding_counts(&mut *tx.tx, org_id).await?;
    tx.commit().await?;

    let has_more = rows.len() as i64 > limit;
    let items: Vec<IntegrityFindingView> = rows
        .into_iter()
        .take(limit as usize)
        .map(IntegrityFindingView::from)
        .collect();
    let seen = offset + items.len() as i64;
    Ok(Json(IntegrityFindingPage {
        items,
        next_offset: has_more.then_some(seen),
        open_orphans: counts.orphans,
        open_missing: counts.missing,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_the_view_emits_parses_back() {
        for kind in [FindingKind::OrphanDeleted, FindingKind::BytesMissing] {
            assert_eq!(check_kind(kind.as_str()).expect("known kind"), kind);
        }
    }

    #[test]
    fn a_mistyped_kind_is_a_validation_error_not_an_empty_page() {
        for bad in ["orphan", "missing", "bytesMissing", ""] {
            assert!(matches!(check_kind(bad), Err(ApiError::Validation(_))));
        }
    }
}
