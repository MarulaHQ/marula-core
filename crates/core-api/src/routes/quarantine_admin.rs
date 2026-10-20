//! The quarantine admin surface (Wave 52 lane P6c): what the scanner held,
//! and the two decisions an operator can make about it.
//!
//! Purpose: read the quarantined set, inspect one object, RELEASE an object
//! the scanner rejected, or delete the held bytes. Reads and writes
//! `attachment_scans` only; the upload path is untouched, which is why this
//! lane can run beside the uploader-notification lane.
//!
//! # Release is an override, and it is the product's most consequential one
//!
//! Every other upload decision is the scanner's. Release admits bytes a
//! scanner named infected into the canonical attachment key, so it is gated
//! on the Admin matrix (`Action::Edit`, which owner and admin hold), carries
//! a REQUIRED reason, and writes an `audit_events` row naming the actor, the
//! object and the reason in the same transaction that records the release.
//! A release without a reason is refused rather than defaulted, because the
//! only thing that makes an override reviewable later is why it happened.
//!
//! # Bytes move copy-then-delete, and a retry finishes the job
//!
//! The canonical copy is written BEFORE the quarantine copy is removed, so a
//! crash between the two leaves both (harmless, content-addressed, identical
//! bytes) rather than neither. The row is updated after the move; a crash
//! between the move and the update leaves a row pointing at a quarantine key
//! that no longer exists, and the retry sees the canonical copy and finishes
//! the update. `released_at IS NULL` in the UPDATE makes the second release
//! return the first one instead of rewriting its reason.
//!
//! # Deleting removes the row, not the fact
//!
//! The record that malware was uploaded is evidence, so it does not leave
//! silently. Deleting a held object removes its bytes and its row, and writes
//! `attachment_scan.deleted` with the digest, filename, scanner and signature
//! into `audit_events`, which is append-only and admin-readable. A released
//! row is NOT deletable here: its bytes are admitted and its release trail is
//! exactly what this surface exists to preserve.
//!
//! Non-purpose: no re-scanning, no bulk actions, no retention sweep, no
//! upload-path change.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use bytes::Bytes;
use chrono::Utc;
use core_domain::entities::{Actor, AttachmentScan, AuditEvent};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// The admin matrix, the same shape the SLA, automation and workflow-ledger
/// admin routes use. Reads take `View`, release takes `Edit`, delete takes
/// `Delete`; owner and admin hold all three, and an auditor holds `View`.
fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

/// Where one object sits in the quarantine: still held, or released anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuarantineState {
    /// The bytes are held under the quarantine key; nothing an end user can
    /// reach points at them.
    Pending,
    /// An operator overrode the verdict. The bytes are at the canonical
    /// attachment key, and the reason and actor are on the row.
    Released,
}

impl From<&AttachmentScan> for QuarantineState {
    fn from(scan: &AttachmentScan) -> Self {
        if scan.released_at.is_some() {
            QuarantineState::Released
        } else {
            QuarantineState::Pending
        }
    }
}

/// One quarantined object as the operator surface reads it.
///
/// The digest is the identity: `attachment_scans` is keyed by
/// `(org_id, sha256)` because the verdict belongs to the BYTES, not to the
/// upload that brought them in.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct QuarantineItemView {
    pub sha256: String,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    /// Adapter that produced the verdict (`clamd`, `none`).
    pub scanner: String,
    /// The scanner's own words — usually a signature name.
    pub detail: Option<String>,
    pub state: QuarantineState,
    pub quarantined_at: chrono::DateTime<Utc>,
    pub scanned_at: chrono::DateTime<Utc>,
    /// The ticket the upload was aimed at, when there was one. The row is
    /// `ON DELETE SET NULL`, so a deleted ticket leaves the record intact.
    pub ticket_id: Option<Uuid>,
    pub uploaded_by_user_id: Option<Uuid>,
    pub uploaded_by_portal_user_id: Option<Uuid>,
    /// Where the bytes live now: the quarantine key while held, the canonical
    /// key after a release.
    pub storage_key: String,
    pub released_at: Option<chrono::DateTime<Utc>>,
    pub released_by_user_id: Option<Uuid>,
    pub release_reason: Option<String>,
}

impl From<AttachmentScan> for QuarantineItemView {
    fn from(s: AttachmentScan) -> Self {
        let state = QuarantineState::from(&s);
        Self {
            sha256: s.sha256,
            filename: s.filename,
            content_type: s.content_type,
            size_bytes: s.size_bytes,
            scanner: s.scanner,
            detail: s.detail,
            state,
            quarantined_at: s.quarantined_at.unwrap_or(s.scanned_at),
            scanned_at: s.scanned_at,
            ticket_id: s.ticket_id,
            uploaded_by_user_id: s.uploaded_by_user_id,
            uploaded_by_portal_user_id: s.uploaded_by_portal_user_id,
            storage_key: s.storage_key,
            released_at: s.released_at,
            released_by_user_id: s.released_by_user_id,
            release_reason: s.release_reason,
        }
    }
}

/// One page of the quarantine list, with the org-wide counts beside it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct QuarantinePage {
    /// Newest first. Released rows stay listed: the trail is the point.
    pub items: Vec<QuarantineItemView>,
    /// Objects still held, across the whole org.
    pub pending: i64,
    /// Objects released anyway, across the whole org.
    pub released: i64,
    /// Offset for the next page; absent when this page is the last.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<i64>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct QuarantineQuery {
    /// Page size. Clamped to 1..=200.
    pub limit: Option<i64>,
    /// Rows to skip, from a previous page's `next_offset`.
    pub offset: Option<i64>,
}

/// The reason is the whole point of the override, so it is a required field
/// rather than an optional one with a default.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ReleaseQuarantineRequest {
    /// Why these bytes are being admitted despite the scanner's verdict.
    /// Recorded verbatim on the row and in the audit trail.
    pub reason: String,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct DeleteQuarantineQuery {
    /// Optional note for the audit trail. The default sentence names the
    /// action when the operator does not add one.
    pub reason: Option<String>,
}

/// Longest reason accepted. The column is `text`, so the limit is a judgement
/// about what an operator can usefully re-read later, not a storage cap.
const MAX_REASON_CHARS: usize = 500;

fn check_reason(raw: &str) -> ApiResult<String> {
    let reason = raw.trim();
    if reason.is_empty() {
        return Err(ApiError::Validation(
            "a reason is required to release a quarantined file".into(),
        ));
    }
    if reason.chars().count() > MAX_REASON_CHARS {
        return Err(ApiError::Validation(format!(
            "release reason is longer than {MAX_REASON_CHARS} characters"
        )));
    }
    Ok(reason.to_string())
}

/// Validates the path digest before it reaches a query. The column CHECK is
/// the backstop; this turns a typo into a typed refusal instead of a 404
/// that looks like "nothing quarantined".
fn check_digest(raw: &str) -> ApiResult<&str> {
    let ok = raw.len() == 64
        && raw
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'));
    if ok {
        Ok(raw)
    } else {
        Err(ApiError::Validation(
            "sha256 must be 64 lowercase hex characters".into(),
        ))
    }
}

/// Streams a held object into memory for the canonical write.
///
/// `ObjectStore` has no stream-to-stream copy, and adding one is a
/// `core-store` trait change that belongs to that crate rather than to an
/// admin route. The bound is the live upload cap: these bytes got through the
/// pre-store gate under that cap, so a larger object means the deployment's
/// cap moved after the upload, and refusing is safer than buffering.
async fn read_held(obj: core_store::GetObject) -> ApiResult<Bytes> {
    use futures_util::TryStreamExt;
    let mut body = Vec::with_capacity(obj.size_bytes as usize);
    let mut stream = obj.stream;
    while let Some(chunk) = stream.try_next().await.map_err(store_error)? {
        body.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(body))
}

/// Store failures reach the client as the shared attachment error shape, so a
/// disk or S3 outage reads the same here as it does on the upload path.
fn store_error(e: core_store::StoreError) -> ApiError {
    match e {
        core_store::StoreError::NotFound(_) => ApiError::NotFound,
        other => {
            tracing::error!(error = %other, "object store failure on the quarantine surface");
            ApiError::Internal(anyhow::anyhow!(other.to_string()))
        }
    }
}

fn is_missing(e: &core_store::StoreError) -> bool {
    matches!(e, core_store::StoreError::NotFound(_))
}

/// GET /api/admin/quarantine — the org's quarantined objects, newest first.
///
/// `pending` are still held; `released` were admitted by an operator and keep
/// their row so the override stays visible.
#[utoipa::path(operation_id = "quarantine_admin_list", get, path = "/api/admin/quarantine",
    tag = "quarantine", security(("bearer" = [])),
    params(QuarantineQuery),
    responses((status = 200, body = QuarantinePage)))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<QuarantineQuery>,
) -> ApiResult<Json<QuarantinePage>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let limit = q.limit.unwrap_or(100).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);

    let mut tx = org_tx::begin(&state, org_id, authed.principal.user.id).await?;
    let counts = core_db::attachment_scans::quarantine_counts(&mut tx.tx, org_id).await?;
    let rows =
        core_db::attachment_scans::list_quarantined_page(&mut tx.tx, org_id, limit, offset).await?;
    tx.commit().await?;

    let seen = offset + rows.len() as i64;
    let total = counts.pending + counts.released;
    Ok(Json(QuarantinePage {
        items: rows.into_iter().map(QuarantineItemView::from).collect(),
        pending: counts.pending,
        released: counts.released,
        next_offset: (seen < total).then_some(seen),
    }))
}

/// GET /api/admin/quarantine/{sha256} — one held or released object.
#[utoipa::path(operation_id = "quarantine_admin_get", get,
    path = "/api/admin/quarantine/{sha256}", tag = "quarantine",
    security(("bearer" = [])),
    params(("sha256" = String, Path)),
    responses((status = 200, body = QuarantineItemView)))]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(sha256): Path<String>,
) -> ApiResult<Json<QuarantineItemView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let digest = check_digest(&sha256)?;

    let mut tx = org_tx::begin(&state, org_id, authed.principal.user.id).await?;
    let scan = core_db::attachment_scans::get(&mut tx.tx, org_id, digest)
        .await?
        .filter(AttachmentScan::is_quarantined)
        .ok_or(ApiError::NotFound)?;
    tx.commit().await?;
    Ok(Json(QuarantineItemView::from(scan)))
}

/// POST /api/admin/quarantine/{sha256}/release — admit the object anyway.
///
/// The bytes move from the quarantine key to the canonical attachment key,
/// the row records actor + reason + timestamp, and an `audit_events` row is
/// written in the same transaction. Repeating the call returns the first
/// release unchanged and writes no second audit row.
#[utoipa::path(operation_id = "quarantine_admin_release", post,
    path = "/api/admin/quarantine/{sha256}/release", tag = "quarantine",
    security(("bearer" = [])),
    params(("sha256" = String, Path)),
    request_body = ReleaseQuarantineRequest,
    responses((status = 200, body = QuarantineItemView)))]
pub async fn release(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(sha256): Path<String>,
    Json(req): Json<ReleaseQuarantineRequest>,
) -> ApiResult<Json<QuarantineItemView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let digest = check_digest(&sha256)?;
    let reason = check_reason(&req.reason)?;

    let mut tx = org_tx::begin(&state, org_id, user_id).await?;
    let scan = core_db::attachment_scans::get(&mut tx.tx, org_id, digest)
        .await?
        .filter(AttachmentScan::is_quarantined)
        .ok_or(ApiError::NotFound)?;

    // Idempotent repeat: the first operator's reason is the one on record,
    // and a second audit row would say an override happened twice.
    if scan.released_at.is_some() {
        tx.commit().await?;
        return Ok(Json(QuarantineItemView::from(scan)));
    }

    let canonical = core_store::attachment_key(org_id, digest);
    let quarantine = core_store::quarantine_key(org_id, digest);
    let mut bytes_moved = false;
    match state.store.get(&quarantine).await {
        Ok(obj) => {
            if obj.size_bytes > state.config.max_upload_bytes as u64 {
                return Err(ApiError::Conflict(format!(
                    "the held object is {} bytes, above the current {} byte upload cap",
                    obj.size_bytes, state.config.max_upload_bytes
                )));
            }
            // Copy first, delete second: a crash between them leaves both
            // copies, which is harmless, instead of neither.
            let body = read_held(obj).await?;
            state
                .store
                .put(&canonical, &scan.content_type, body)
                .await
                .map_err(store_error)?;
            state.store.delete(&quarantine).await.map_err(store_error)?;
            bytes_moved = true;
        }
        Err(e) if is_missing(&e) => {
            // A previous attempt copied the bytes and stopped before the row
            // update. The canonical copy is the proof it got that far.
            if state.store.get(&canonical).await.is_err() {
                return Err(ApiError::Conflict(
                    "the quarantined bytes are no longer in the store, so there is \
                     nothing to release; delete the record instead"
                        .into(),
                ));
            }
        }
        Err(e) => return Err(store_error(e)),
    }

    let updated = core_db::attachment_scans::mark_released(
        &mut tx.tx, org_id, digest, user_id, &reason, &canonical,
    )
    .await?;

    let Some(updated) = updated else {
        // Another request released it between our read and our UPDATE. Its
        // reason and audit row stand; this one writes nothing.
        let current = core_db::attachment_scans::get(&mut tx.tx, org_id, digest)
            .await?
            .ok_or(ApiError::NotFound)?;
        tx.commit().await?;
        return Ok(Json(QuarantineItemView::from(current)));
    };

    core_db::audit::insert(
        &mut *tx.tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(user_id),
            action: "attachment_scan.released".into(),
            entity_type: Some("attachment_scan".into()),
            entity_id: Some(digest.to_string()),
            detail: serde_json::json!({
                "sha256": digest,
                "filename": &updated.filename,
                "content_type": &updated.content_type,
                "size_bytes": updated.size_bytes,
                "scanner": &updated.scanner,
                "scanner_detail": &updated.detail,
                "quarantined_at": updated.quarantined_at,
                "reason": &reason,
                "bytes_moved": bytes_moved,
                "storage_key": &canonical,
            }),
            ip: None,
            occurred_at: Utc::now(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Json(QuarantineItemView::from(updated)))
}

/// DELETE /api/admin/quarantine/{sha256} — remove the held bytes.
///
/// The row goes with them and the fact stays in the audit trail. A released
/// object is refused (409): its bytes are admitted and its trail is evidence.
#[utoipa::path(operation_id = "quarantine_admin_delete", delete,
    path = "/api/admin/quarantine/{sha256}", tag = "quarantine",
    security(("bearer" = [])),
    params(("sha256" = String, Path), DeleteQuarantineQuery),
    responses((status = 204)))]
pub async fn remove(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(sha256): Path<String>,
    Query(q): Query<DeleteQuarantineQuery>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let digest = check_digest(&sha256)?;
    let note = q
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(MAX_REASON_CHARS).collect::<String>());

    let mut tx = org_tx::begin(&state, org_id, user_id).await?;
    let scan = core_db::attachment_scans::get(&mut tx.tx, org_id, digest)
        .await?
        .filter(AttachmentScan::is_quarantined)
        .ok_or(ApiError::NotFound)?;
    if scan.released_at.is_some() {
        return Err(ApiError::Conflict(
            "this object was released, so its bytes are admitted and its release \
             trail is evidence; delete is for objects still held"
                .into(),
        ));
    }

    // Bytes first: a crash before the row delete leaves a row pointing at a
    // missing key, and the retry finishes it because store deletes are
    // idempotent.
    state
        .store
        .delete(&scan.storage_key)
        .await
        .map_err(store_error)?;
    let removed = core_db::attachment_scans::delete_quarantined(&mut tx.tx, org_id, digest).await?;
    if !removed {
        return Err(ApiError::NotFound);
    }

    core_db::audit::insert(
        &mut *tx.tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(user_id),
            action: "attachment_scan.deleted".into(),
            entity_type: Some("attachment_scan".into()),
            entity_id: Some(digest.to_string()),
            detail: serde_json::json!({
                "sha256": digest,
                "filename": scan.filename,
                "content_type": scan.content_type,
                "size_bytes": scan.size_bytes,
                "scanner": scan.scanner,
                "scanner_detail": scan.detail,
                "quarantined_at": scan.quarantined_at,
                "reason": note,
                "storage_key": scan.storage_key,
            }),
            ip: None,
            occurred_at: Utc::now(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
