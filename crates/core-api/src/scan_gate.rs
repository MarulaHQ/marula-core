//! The upload scan gate (Wave 52 lane P6a): the one place bytes arriving
//! from outside are checked by the configured scanner before a surface
//! stores them.
//!
//! Purpose: turn a scanner verdict into the two things a caller needs — a
//! [`ScanStatus`] it can record, and a scanner label it can name — and turn
//! a scanner OUTAGE into a refusal. Every store-writing upload surface calls
//! [`gate`] before its `ObjectStore::put`.
//!
//! Why the gate fails closed: `core-mail::ScanError::Unavailable` means the
//! scanner did not answer, so "is this file dangerous?" has no answer.
//! Storing the bytes and recording a verdict afterwards would be recording a
//! guess; the documented none mode ([`core_mail::NoopScan`]) exists so a
//! deployment with no scanner says `skipped` out loud instead of failing.
//! The distinction is the whole point: unreachable ≠ disabled.
//!
//! Non-purpose: no store access, no transaction, no policy about what to do
//! with a verdict (each surface owns its rows, its event, and its refusal
//! shape — P6b gives every one of them a user-facing outcome).

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use core_domain::entities::ScanStatus;

/// What the scanner said, in the terms a caller records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateVerdict {
    /// `clean`, `skipped`, or `infected` — never absent, and never a guess.
    pub status: ScanStatus,
    /// Adapter that decided (`clamd`, `none`, a test double).
    pub scanner: &'static str,
    /// Scanner detail string (a signature name, or the none-mode note).
    pub detail: Option<String>,
}

impl GateVerdict {
    /// True when the caller must refuse the upload.
    #[must_use]
    pub fn is_rejected(&self) -> bool {
        self.status == ScanStatus::Infected
    }
}

/// Scans an upload's octets, or refuses the request because no scanner
/// answered.
///
/// # Errors
/// [`ApiError::Unavailable`] (503) when the scanner cannot be reached. The
/// caller must write NOTHING in that case — that is what closing the gate
/// means.
pub async fn gate(state: &AppState, bytes: &[u8]) -> ApiResult<GateVerdict> {
    let scanner = std::sync::Arc::clone(&state.mail.scanner);
    // The BYTES leg: an upload is binary, and a text view of it is how a
    // signature in the binary region stops matching.
    match scanner.scan_bytes(bytes).await {
        Ok(verdict) => Ok(GateVerdict {
            status: if verdict.infected {
                ScanStatus::Infected
            } else if verdict.skipped {
                ScanStatus::Skipped
            } else {
                ScanStatus::Clean
            },
            scanner: scanner.name(),
            detail: verdict.detail,
        }),
        Err(core_mail::ScanError::Unavailable(reason)) => {
            tracing::error!(
                scanner = scanner.name(),
                reason = %reason,
                "upload scan unavailable; refusing the upload and storing nothing (fail closed)"
            );
            Err(ApiError::Unavailable(format!(
                "the malware scanner is unavailable, so the upload was refused \
                 and nothing was stored: {reason}"
            )))
        }
    }
}

/// What an upload surface did with one gated upload. Both arms COMMIT: the
/// quarantine record is a fact on disk, and the typed refusal a caller
/// returns for the second arm describes it rather than undoing it.
pub enum Gated<T> {
    /// Admitted: the surface's own row exists.
    Admitted(T),
    /// Refused by the scan: bytes held under the quarantine key, no row.
    Quarantined { detail: Option<String> },
}

/// One rejected upload, as the quarantine record needs to describe it.
pub struct Rejection<'a> {
    pub org_id: uuid::Uuid,
    /// Lowercase hex sha256 — the object's identity.
    pub sha256: &'a str,
    pub filename: &'a str,
    pub content_type: &'a str,
    pub size_bytes: i64,
    pub bytes: bytes::Bytes,
    /// The ticket this upload was aimed at, when there was one. Non-ticket
    /// surfaces (documents, drawn marks) record no target here.
    pub ticket_id: Option<uuid::Uuid>,
    pub uploaded_by_user_id: Option<uuid::Uuid>,
    pub verdict: &'a GateVerdict,
}

/// Holds rejected bytes and writes their verdict row, inside the caller's
/// transaction. The bytes keep their digest under the quarantine namespace:
/// a later release can admit them at their canonical key without re-hashing,
/// and a key alone still says which side of the gate the object is on.
///
/// Re-uploading an object already quarantined does not copy it again.
pub async fn quarantine(
    conn: &mut sqlx::PgConnection,
    store: &dyn core_store::ObjectStore,
    r: Rejection<'_>,
) -> ApiResult<()> {
    let key = core_store::quarantine_key(r.org_id, r.sha256);
    let already = core_db::attachment_scans::get(&mut *conn, r.org_id, r.sha256)
        .await?
        .is_some_and(|scan| scan.is_quarantined());
    if !already {
        store
            .put(&key, r.content_type, r.bytes)
            .await
            .map_err(crate::routes::attachments::store_error)?;
    }
    core_db::attachment_scans::record_quarantine(
        conn,
        core_db::attachment_scans::NewScan {
            org_id: r.org_id,
            sha256: r.sha256,
            status: ScanStatus::Infected,
            detail: r.verdict.detail.as_deref(),
            scanner: r.verdict.scanner,
            size_bytes: r.size_bytes,
            content_type: r.content_type,
            filename: r.filename,
            storage_key: &key,
            ticket_id: r.ticket_id,
            uploaded_by_user_id: r.uploaded_by_user_id,
            uploaded_by_portal_user_id: None,
        },
    )
    .await?;
    Ok(())
}
