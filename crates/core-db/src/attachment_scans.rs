//! Attachment scan verdicts (Wave 52 lane P6a, migration 0139).
//!
//! Purpose: the record of what a scanner said about one stored object —
//! keyed by `(org_id, sha256)` because `attachments` rows are per-OWNER
//! while the verdict is a property of the BYTES, and several rows may share
//! one content-addressed object.
//!
//! The two write paths differ on purpose:
//!   * [`record_admitted`] inserts a `clean`/`skipped` verdict and keeps the
//!     one already there — the first admission of a digest is what happened,
//!     and a re-upload must not be able to launder a record by paying for a
//!     second scan.
//!   * [`record_quarantine`] inserts an `infected` verdict, or ESCALATES an
//!     existing admitted row when a scanner now finds a signature in bytes
//!     that passed before (scanner databases move). Escalation points the row
//!     at the quarantine copy the caller just wrote, which is why a second
//!     copy exists.
//!
//! The two ADMIN write paths (Wave 52 lane P6c) keep the row as evidence:
//!   * [`mark_released`] records an override and points `storage_key` at the
//!     canonical key the caller has already populated. It is conditional on
//!     `released_at IS NULL`, so a retried release cannot rewrite the first
//!     operator's reason.
//!   * [`delete_quarantined`] removes a still-held row after its bytes are
//!     gone. The audit event written in the same transaction is where the
//!     fact that malware was uploaded survives; a released row is never
//!     deletable here, because its bytes are admitted and its trail is the
//!     point.
//!
//! Non-purpose: no byte movement (core-store), no admission policy or
//! scanner calls (core-api), no release authorization (the P6c admin
//! surface owns who may override a quarantine).

use crate::DbResult;
use core_domain::entities::{AttachmentScan, ScanStatus};
use sqlx::PgConnection;
use uuid::Uuid;

const COLS: &str = "org_id, sha256, status, detail, scanner, size_bytes, content_type, \
                    filename, storage_key, ticket_id, uploaded_by_user_id, \
                    uploaded_by_portal_user_id, scanned_at, quarantined_at, released_at, \
                    released_by_user_id, release_reason";

fn row_to_scan(row: sqlx::postgres::PgRow) -> Result<AttachmentScan, sqlx::Error> {
    use sqlx::Row;
    let status: String = row.get("status");
    Ok(AttachmentScan {
        org_id: row.get("org_id"),
        sha256: row.get("sha256"),
        status: status.parse().map_err(|e: core_domain::DomainError| {
            sqlx::Error::ColumnDecode {
                index: "status".into(),
                source: Box::new(e),
            }
        })?,
        detail: row.get("detail"),
        scanner: row.get("scanner"),
        size_bytes: row.get("size_bytes"),
        content_type: row.get("content_type"),
        filename: row.get("filename"),
        storage_key: row.get("storage_key"),
        ticket_id: row.get("ticket_id"),
        uploaded_by_user_id: row.get("uploaded_by_user_id"),
        uploaded_by_portal_user_id: row.get("uploaded_by_portal_user_id"),
        scanned_at: row.get("scanned_at"),
        quarantined_at: row.get("quarantined_at"),
        released_at: row.get("released_at"),
        released_by_user_id: row.get("released_by_user_id"),
        release_reason: row.get("release_reason"),
    })
}

/// Fields for one verdict row. `status` must be `clean` or `skipped` for
/// [`record_admitted`] and `infected` for [`record_quarantine`]; the database
/// coherence CHECK rejects the mismatch either way.
pub struct NewScan<'a> {
    pub org_id: Uuid,
    pub sha256: &'a str,
    pub status: ScanStatus,
    pub detail: Option<&'a str>,
    pub scanner: &'a str,
    pub size_bytes: i64,
    pub content_type: &'a str,
    pub filename: &'a str,
    pub storage_key: &'a str,
    pub ticket_id: Option<Uuid>,
    pub uploaded_by_user_id: Option<Uuid>,
    pub uploaded_by_portal_user_id: Option<Uuid>,
}

/// Whether the digest was already on record, and what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmitOutcome {
    /// No verdict existed; this upload's scan is the record.
    Recorded(AttachmentScan),
    /// A verdict already existed and stands untouched.
    AlreadyRecorded(AttachmentScan),
}

/// Records a `clean` or `skipped` verdict without disturbing an existing row.
pub async fn record_admitted(conn: &mut PgConnection, s: NewScan<'_>) -> DbResult<AdmitOutcome> {
    let inserted = sqlx::query(&format!(
        r#"INSERT INTO attachment_scans (org_id, sha256, status, detail, scanner,
                                         size_bytes, content_type, filename, storage_key,
                                         ticket_id, uploaded_by_user_id,
                                         uploaded_by_portal_user_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
           ON CONFLICT (org_id, sha256) DO NOTHING
           RETURNING {COLS}"#
    ))
    .bind(s.org_id)
    .bind(s.sha256)
    .bind(s.status.as_str())
    .bind(s.detail)
    .bind(s.scanner)
    .bind(s.size_bytes)
    .bind(s.content_type)
    .bind(s.filename)
    .bind(s.storage_key)
    .bind(s.ticket_id)
    .bind(s.uploaded_by_user_id)
    .bind(s.uploaded_by_portal_user_id)
    .try_map(row_to_scan)
    .fetch_optional(&mut *conn)
    .await?;

    match inserted {
        Some(row) => Ok(AdmitOutcome::Recorded(row)),
        None => {
            let existing = get(&mut *conn, s.org_id, s.sha256).await?;
            Ok(AdmitOutcome::AlreadyRecorded(
                existing.expect("conflict implies a row"),
            ))
        }
    }
}

/// Records an `infected` verdict, escalating an admitted row when the bytes
/// on file are now known-bad. `storage_key` must be the quarantine key whose
/// copy the caller has written.
pub async fn record_quarantine(
    conn: &mut PgConnection,
    s: NewScan<'_>,
) -> DbResult<AttachmentScan> {
    let row = sqlx::query(&format!(
        r#"INSERT INTO attachment_scans (org_id, sha256, status, detail, scanner,
                                         size_bytes, content_type, filename, storage_key,
                                         ticket_id, uploaded_by_user_id,
                                         uploaded_by_portal_user_id, quarantined_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, now())
           ON CONFLICT (org_id, sha256) DO UPDATE SET
             status = EXCLUDED.status,
             detail = EXCLUDED.detail,
             scanner = EXCLUDED.scanner,
             size_bytes = EXCLUDED.size_bytes,
             content_type = EXCLUDED.content_type,
             filename = EXCLUDED.filename,
             storage_key = EXCLUDED.storage_key,
             ticket_id = EXCLUDED.ticket_id,
             uploaded_by_user_id = EXCLUDED.uploaded_by_user_id,
             uploaded_by_portal_user_id = EXCLUDED.uploaded_by_portal_user_id,
             scanned_at = now(),
             quarantined_at = EXCLUDED.quarantined_at
           WHERE attachment_scans.status <> 'infected'
           RETURNING {COLS}"#
    ))
    .bind(s.org_id)
    .bind(s.sha256)
    .bind(s.status.as_str())
    .bind(s.detail)
    .bind(s.scanner)
    .bind(s.size_bytes)
    .bind(s.content_type)
    .bind(s.filename)
    .bind(s.storage_key)
    .bind(s.ticket_id)
    .bind(s.uploaded_by_user_id)
    .bind(s.uploaded_by_portal_user_id)
    .try_map(row_to_scan)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row)
}

/// Reads the verdict on record for one object, if any.
pub async fn get(
    conn: &mut PgConnection,
    org_id: Uuid,
    sha256: &str,
) -> DbResult<Option<AttachmentScan>> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM attachment_scans WHERE org_id = $1 AND sha256 = $2"
    ))
    .bind(org_id)
    .bind(sha256)
    .try_map(row_to_scan)
    .fetch_optional(conn)
    .await?;
    Ok(row)
}

/// Every quarantined object for the org, newest first — the P6c admin list's
/// read. Released rows stay in the list (the audit trail is the point); the
/// caller renders them as released rather than filtering them away.
///
/// Unbounded on purpose: this is the shape the scan gate's own test and any
/// future sweep want. The admin PAGE reads [`list_quarantined_page`] instead,
/// because an org can accumulate one row per rejected digest and the operator
/// surface must not answer with an unbounded body.
pub async fn list_quarantined(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<AttachmentScan>> {
    list_quarantined_page(conn, org_id, i64::MAX, 0).await
}

/// One bounded page of the quarantine list, newest first. `next_offset` is
/// derived by the caller from the row count; this read does no counting.
pub async fn list_quarantined_page(
    conn: &mut PgConnection,
    org_id: Uuid,
    limit: i64,
    offset: i64,
) -> DbResult<Vec<AttachmentScan>> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM attachment_scans \
         WHERE org_id = $1 AND status = 'infected' \
         ORDER BY scanned_at DESC, sha256 \
         LIMIT $2 OFFSET $3"
    ))
    .bind(org_id)
    .bind(limit)
    .bind(offset)
    .try_map(row_to_scan)
    .fetch_all(conn)
    .await?;
    Ok(rows)
}

/// How many quarantined objects are still held, and how many were released
/// anyway. Both counts describe the whole org, not the current page.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QuarantineCounts {
    pub pending: i64,
    pub released: i64,
}

pub async fn quarantine_counts(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<QuarantineCounts> {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT count(*) FILTER (WHERE released_at IS NULL) AS pending, \
                count(*) FILTER (WHERE released_at IS NOT NULL) AS released \
         FROM attachment_scans WHERE org_id = $1 AND status = 'infected'",
    )
    .bind(org_id)
    .fetch_one(conn)
    .await?;
    Ok(QuarantineCounts {
        pending: row.get("pending"),
        released: row.get("released"),
    })
}

/// Records an operator's release of one quarantined object.
///
/// `storage_key` must be the canonical admission key whose copy the caller
/// has written: after this row commits, the column is what the operator
/// surface and any later read use to find the bytes.
///
/// Returns `None` when the row is missing or was already released. The
/// condition lives in SQL so two concurrent releases cannot both write an
/// audit reason: the first wins, the second reads the winner.
pub async fn mark_released(
    conn: &mut PgConnection,
    org_id: Uuid,
    sha256: &str,
    released_by_user_id: Uuid,
    release_reason: &str,
    storage_key: &str,
) -> DbResult<Option<AttachmentScan>> {
    let row = sqlx::query(&format!(
        "UPDATE attachment_scans \
            SET released_at = now(), released_by_user_id = $3, \
                release_reason = $4, storage_key = $5 \
          WHERE org_id = $1 AND sha256 = $2 \
            AND status = 'infected' AND released_at IS NULL \
          RETURNING {COLS}"
    ))
    .bind(org_id)
    .bind(sha256)
    .bind(released_by_user_id)
    .bind(release_reason)
    .bind(storage_key)
    .try_map(row_to_scan)
    .fetch_optional(conn)
    .await?;
    Ok(row)
}

/// Removes a still-held quarantine row once its bytes are gone.
///
/// The `released_at IS NULL` guard is deliberate and duplicated from the
/// route on purpose: a released row's bytes live at the canonical key and
/// its release trail is evidence, so no caller may delete it through this
/// path. Returns whether a row was removed.
pub async fn delete_quarantined(
    conn: &mut PgConnection,
    org_id: Uuid,
    sha256: &str,
) -> DbResult<bool> {
    let done = sqlx::query(
        "DELETE FROM attachment_scans \
          WHERE org_id = $1 AND sha256 = $2 \
            AND status = 'infected' AND released_at IS NULL",
    )
    .bind(org_id)
    .bind(sha256)
    .execute(conn)
    .await?;
    Ok(done.rows_affected() > 0)
}
