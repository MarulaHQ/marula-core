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
//! A rejection is not a silent drop (lane P6b). [`quarantine`] writes three
//! things in the caller's transaction: the held bytes, the verdict row, and
//! a notification for the uploader plus the org's scan-detection recipients.
//! It also returns the [`QuarantinedUpload`] the surface refuses with, so the
//! 422, the notification, and the record all describe the same object.
//!
//! One surface scans and refuses instead (lane P6d, the secret vault): the
//! vault seals user plaintext before storing, so a quarantine copy would hold
//! that plaintext unsealed, in the one namespace that is not encrypted. Its
//! door calls [`gate`] on the plaintext before sealing, records an admitted
//! verdict like every other door, and on an infection answers
//! [`GateVerdict::refusal`] with nothing written anywhere.
//!
//! Non-purpose: no store access beyond the quarantine copy, no admission
//! policy, no transaction of its own. The caller owns its rows, its
//! `attachment.quarantined` event, and the point at which it returns the
//! refusal.
//!
//! Two callers are not upload doors and still belong here (Wave 54 lane S7):
//! the inbound-email pipeline scans each DECODED attachment through [`gate`]
//! before it stores the part, and holds a rejected one through
//! [`quarantine_in_tx`] — the same record, the same notification, the same
//! namespace an uploader's file gets. Email used to be the one way bytes
//! entered the store without a verdict row, which is what made an inbound
//! attachment refuse to download later.

use crate::error::{ApiError, ApiResult};
use crate::org_tx::OrgTx;
use crate::state::AppState;
use core_domain::entities::ScanStatus;
use core_mail::ScanProvider;
use sqlx::{Postgres, Transaction};

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

    /// The typed refusal for a surface that keeps NOTHING on a rejection
    /// (Wave 52 lane P6d, the secret vault): no object, no verdict row, no
    /// quarantine copy, so no operator ever has to act on it. The refusal
    /// names the same object and the same finding the scan just produced,
    /// which is why it is built here rather than assembled by the route.
    #[must_use]
    pub fn refusal(&self, sha256: &str, filename: &str) -> ApiError {
        ApiError::UploadRefused {
            sha256: sha256.to_string(),
            filename: filename.to_string(),
            scanner: self.scanner.to_string(),
            detail: self.detail.clone(),
        }
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
    gate_with(state.mail.scanner.as_ref(), bytes).await
}

/// The same gate for a caller that holds the configured scanner but no
/// [`AppState`]: the operator verdict backfill ([`crate::scan_backfill`]) runs
/// in the CLI process, where there is no router and no server state. One
/// implementation still decides what a scanner answer MEANS, so the backfill
/// cannot record a verdict shape an upload door would not.
///
/// # Errors
/// [`ApiError::Unavailable`] (503) when the scanner cannot be reached; the
/// caller writes nothing for the object it was judging.
pub async fn gate_with(scanner: &dyn ScanProvider, bytes: &[u8]) -> ApiResult<GateVerdict> {
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
        Err(core_mail::ScanError::TooLarge {
            limit_bytes,
            len_bytes,
        }) => {
            tracing::warn!(
                scanner = scanner.name(),
                limit_bytes,
                len_bytes,
                "upload is past the scanner's stream limit; refusing it as too large to scan"
            );
            Err(ApiError::UploadTooLarge { limit_bytes })
        }
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

/// Whether this deployment has a scanner that could have recorded a verdict.
///
/// The documented none mode ([`core_mail::NoopScan`], `name() == "none"`) is
/// the answer. One definition, read by the download doors and by the verdict
/// backfill: a backfill that disagreed with the doors about whether a scanner
/// exists would either lock files out (writing slots nobody reads) or leave
/// the hole it was run to close.
#[must_use]
pub fn scanner_is_installed(scanner: &dyn ScanProvider) -> bool {
    scanner.name() != "none"
}

/// What an upload surface did with one gated upload. Both arms COMMIT: the
/// quarantine record is a fact on disk, and the typed refusal a caller
/// returns for the second arm describes it rather than undoing it.
pub enum Gated<T> {
    /// Admitted: the surface's own row exists.
    Admitted(T),
    /// Refused by the scan: bytes held under the quarantine key, no row.
    /// The payload is what the surface refuses the request with, built by
    /// [`quarantine`] from the same values it wrote down.
    Quarantined(QuarantinedUpload),
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
    /// surfaces (documents) record no target here.
    pub ticket_id: Option<uuid::Uuid>,
    pub uploaded_by_user_id: Option<uuid::Uuid>,
    pub verdict: &'a GateVerdict,
}

/// A held object, described in the terms a client renders: which object,
/// what it was called, what decided, what it found. [`Self::refusal`] turns
/// that into the typed 422, so the response and the record cannot disagree.
pub struct QuarantinedUpload {
    /// Lowercase hex sha256 of the held bytes.
    pub sha256: String,
    pub filename: String,
    /// Adapter that called it (`clamd`, `none`, a test double).
    pub scanner: String,
    /// Scanner detail (usually the signature name).
    pub detail: Option<String>,
}

impl QuarantinedUpload {
    /// The typed refusal this rejection answers with.
    #[must_use]
    pub fn refusal(self) -> ApiError {
        ApiError::UploadQuarantined {
            sha256: self.sha256,
            filename: self.filename,
            scanner: self.scanner,
            detail: self.detail,
        }
    }
}

/// Holds rejected bytes, writes their verdict row, and tells the people who
/// need to know — all inside the caller's transaction, so the notification
/// can never describe a quarantine the database does not hold.
///
/// The bytes keep their digest under the quarantine namespace: a later
/// release can admit them at their canonical key without re-hashing, and a
/// key alone still says which side of the gate the object is on.
/// Re-uploading an object already quarantined does not copy it again.
pub async fn quarantine(
    ctx: &mut OrgTx<'_>,
    store: &dyn core_store::ObjectStore,
    r: Rejection<'_>,
) -> ApiResult<QuarantinedUpload> {
    quarantine_in_tx(&mut ctx.tx, store, r).await
}

/// The same write, for a caller that owns a plain org-scoped transaction
/// rather than an [`OrgTx`]: the inbound-email pipeline routes a message to
/// its org inside a transaction it opened itself, and a held attachment needs
/// the identical record. One implementation, so the two doors cannot drift on
/// what a quarantine means.
pub async fn quarantine_in_tx(
    tx: &mut Transaction<'static, Postgres>,
    store: &dyn core_store::ObjectStore,
    r: Rejection<'_>,
) -> ApiResult<QuarantinedUpload> {
    let key = core_store::quarantine_key(r.org_id, r.sha256);
    let already = core_db::attachment_scans::get(&mut *tx, r.org_id, r.sha256)
        .await?
        .is_some_and(|scan| scan.is_quarantined());
    if !already {
        // Cheap Arc bump on `Bytes`, and it keeps `r` borrowable for the
        // notification below.
        let bytes = r.bytes.clone();
        store
            .put(&key, r.content_type, bytes)
            .await
            .map_err(crate::routes::attachments::store_error)?;
    }
    // The verdict row is first-wins on an infection: `record_quarantine`
    // refuses to rewrite a row that already reads `infected`, so a repeat
    // upload of the same held object returns no row. That is the rule
    // working, not a missing record — the refusal and the notification below
    // still describe the object that is already held, and the uploader of a
    // second attempt gets the same typed answer as the first.
    match core_db::attachment_scans::record_quarantine(
        &mut *tx,
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
    .await
    .map_err(core_db::DbError::normalized)
    {
        Ok(_) => {}
        Err(core_db::DbError::NotFound) => {
            tracing::debug!(
                sha256 = r.sha256,
                "object was already quarantined; the verdict row keeps its first record"
            );
        }
        Err(e) => return Err(e.into()),
    }

    notify_quarantine(tx, &r).await?;

    Ok(QuarantinedUpload {
        sha256: r.sha256.to_string(),
        filename: r.filename.to_string(),
        scanner: r.verdict.scanner.to_string(),
        detail: r.verdict.detail.clone(),
    })
}

/// Who hears about a held upload, and how.
///
/// TWO AUDIENCES, deduped: the uploader, whose file did not land, and the
/// org's owners/admins — a held object is an operator's problem, and the
/// admin surface that releases or deletes it is where they act on it. An
/// uploader who is also an owner/admin gets one row, not two.
///
/// The `scan_detection` preference governs both, the same chain the
/// inbound-email quarantine notice uses: a recipient with in-app switched off
/// is skipped before any row exists. The uploader is not exempt from that
/// preference, because the typed 422 already told them in-band; the row is
/// the durable copy, and honouring the switch is what keeps one preference
/// meaning one thing.
async fn notify_quarantine(
    tx: &mut Transaction<'static, Postgres>,
    r: &Rejection<'_>,
) -> ApiResult<()> {
    let kind = core_domain::entities::NotificationKind::ScanDetection;
    let mut recipients: Vec<uuid::Uuid> = Vec::new();
    if let Some(user_id) = r.uploaded_by_user_id {
        recipients.push(user_id);
    }
    for user_id in core_db::notifications::scan_detection_recipients(&mut **tx, r.org_id).await? {
        if !recipients.contains(&user_id) {
            recipients.push(user_id);
        }
    }
    if recipients.is_empty() {
        return Ok(());
    }

    // The title names the outcome; the body names the file and the finding,
    // because "an upload failed" without them is a message the uploader
    // cannot act on.
    let title = "An upload was held by the malware scan";
    let body = format!(
        "{} ({} bytes) was not attached: {} found {}. The bytes are quarantined for an \
         operator, so the same file is refused again.",
        clip(r.filename, 120),
        r.size_bytes,
        r.verdict.scanner,
        clip(
            r.verdict.detail.as_deref().unwrap_or("no scanner detail"),
            160
        ),
    );

    for user_id in recipients {
        let pref = core_db::notifications::effective_preference(
            &mut **tx,
            r.org_id,
            user_id,
            kind.as_str(),
        )
        .await?;
        if !pref.in_app {
            continue;
        }
        let notification = core_db::notifications::insert(
            &mut **tx,
            uuid::Uuid::now_v7(),
            r.org_id,
            user_id,
            kind.as_str(),
            r.ticket_id,
            title,
            Some(&body),
        )
        .await?;
        core_db::outbox::insert(
            &mut *tx,
            uuid::Uuid::now_v7(),
            r.org_id,
            core_domain::events::types::NOTIFICATION_CREATED,
            "notification",
            Some(notification.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::NotificationCreated {
                user_id,
                kind: notification.kind.as_str().to_string(),
                ticket_id: r.ticket_id,
                title: notification.title.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
    }
    Ok(())
}

/// Trims to `max_chars` on a char boundary, marking the cut, so a filename
/// or scanner string can never push the body past sensible bounds.
fn clip(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}
