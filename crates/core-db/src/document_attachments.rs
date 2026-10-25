//! Documents attachments (Wave 17 lane 13, migration 0089): file
//! attachments on documents over the core-store ObjectStore seam (2.14).
//!
//! Purpose: metadata rows for binary objects persisted OUTSIDE Postgres,
//! bound to exactly one document. Content-addressed dedup doctrine is the
//! shared `attachments` table's, restated here: storage keys are
//! `org/<org>/attachments/<sha256>`, several rows may reference one key,
//! there is deliberately NO unique index on (org_id, sha256) — reusing an
//! object IS the point; deletion reclaims the object only when no sibling
//! row still references its key (refcount checked by the caller).
//!
//! Non-purpose: a SEPARATE table from `attachments` (that table's
//! structural CHECKs encode exactly-one-ticket-or-email ownership and stay
//! load-bearing for ticket/email semantics — widening them would be a
//! breaking edit), no portal uploads (documents are staff-surfaced only),
//! no versioning ties — attachments are independent of the document's
//! version history.

use crate::DbResult;
use sqlx::PgConnection;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DocumentAttachmentRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub document_id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub storage_key: String,
    pub uploaded_by: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

const COLS: &str = "id, org_id, document_id, filename, content_type, size_bytes, \
     sha256, storage_key, uploaded_by, created_at";
const SELECT_COLS: &str = "SELECT id, org_id, document_id, filename, content_type, size_bytes, \
     sha256, storage_key, uploaded_by, created_at";

pub struct NewDocumentAttachment<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub document_id: Uuid,
    pub filename: &'a str,
    pub content_type: &'a str,
    pub size_bytes: i64,
    pub sha256: &'a str,
    pub storage_key: &'a str,
    pub uploaded_by: Option<Uuid>,
}

/// Inserts one metadata row.
pub async fn insert(
    conn: &mut PgConnection,
    a: NewDocumentAttachment<'_>,
) -> DbResult<DocumentAttachmentRow> {
    let row = sqlx::query_as::<_, DocumentAttachmentRow>(&format!(
        r#"INSERT INTO documents_attachments (id, org_id, document_id, filename, content_type,
                                              size_bytes, sha256, storage_key, uploaded_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           RETURNING {COLS}"#
    ))
    .bind(a.id)
    .bind(a.org_id)
    .bind(a.document_id)
    .bind(a.filename)
    .bind(a.content_type)
    .bind(a.size_bytes)
    .bind(a.sha256)
    .bind(a.storage_key)
    .bind(a.uploaded_by)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row)
}

/// Fetches one attachment within the org + its document.
pub async fn get(
    conn: &mut PgConnection,
    org_id: Uuid,
    document_id: Uuid,
    attachment_id: Uuid,
) -> DbResult<DocumentAttachmentRow> {
    sqlx::query_as::<_, DocumentAttachmentRow>(&format!(
        "{SELECT_COLS} FROM documents_attachments \
         WHERE org_id = $1 AND document_id = $2 AND id = $3"
    ))
    .bind(org_id)
    .bind(document_id)
    .bind(attachment_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(crate::DbError::NotFound)
}

/// Newest-first list for one document, filtered by the same admission rule
/// the document download door applies ([`core_db::attachments::admitted_verdict_filter`]):
/// an object the door would refuse is not offered. `scanner_installed` is the
/// door's flag; false (a none-mode deployment) leaves the list as it was.
pub async fn list_for_document(
    conn: &mut PgConnection,
    org_id: Uuid,
    document_id: Uuid,
    scanner_installed: bool,
) -> DbResult<Vec<DocumentAttachmentRow>> {
    let rows = sqlx::query_as::<_, DocumentAttachmentRow>(&format!(
        "{SELECT_COLS} FROM documents_attachments a \
         WHERE a.org_id = $1 AND a.document_id = $2{} ORDER BY a.created_at, a.id",
        crate::attachments::admitted_verdict_filter(3)
    ))
    .bind(org_id)
    .bind(document_id)
    .bind(scanner_installed)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows)
}

/// An already-stored object with the same bytes in THIS org (dedup: the
/// upload skips the object PUT and reuses the key).
pub async fn find_existing_object_key(
    conn: &mut PgConnection,
    org_id: Uuid,
    sha256_hex: &str,
) -> DbResult<Option<String>> {
    let key: Option<String> = sqlx::query_scalar(
        "SELECT storage_key FROM documents_attachments \
         WHERE org_id = $1 AND sha256 = $2 LIMIT 1",
    )
    .bind(org_id)
    .bind(sha256_hex)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(key)
}

/// Deletes one row; returns it so the caller reclaims the object only when
/// [`count_other_refs`] (across BOTH tables sharing a key space) reports
/// zero remaining references. The shared table's refs count FIRST — the
/// content-addressed key space is per org across both tables, so the
/// sibling-table count must be included to avoid dropping bytes that are
/// still referenced.
pub async fn delete(
    conn: &mut PgConnection,
    org_id: Uuid,
    document_id: Uuid,
    attachment_id: Uuid,
) -> DbResult<Option<DocumentAttachmentRow>> {
    let row = sqlx::query_as::<_, DocumentAttachmentRow>(&format!(
        r#"DELETE FROM documents_attachments
           WHERE org_id = $1 AND document_id = $2 AND id = $3
           RETURNING {COLS}"#
    ))
    .bind(org_id)
    .bind(document_id)
    .bind(attachment_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

/// How many OTHER rows across BOTH attachments tables still reference this
/// storage key after a delete — the object is reclaimed only on zero.
pub async fn count_other_refs(
    conn: &mut PgConnection,
    org_id: Uuid,
    storage_key: &str,
    excluding_id: Uuid,
) -> DbResult<i64> {
    let own: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM documents_attachments \
         WHERE org_id = $1 AND storage_key = $2 AND id <> $3",
    )
    .bind(org_id)
    .bind(storage_key)
    .bind(excluding_id)
    .fetch_one(&mut *conn)
    .await?;
    let shared: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM attachments \
         WHERE org_id = $1 AND storage_key = $2",
    )
    .bind(org_id)
    .bind(storage_key)
    .fetch_one(&mut *conn)
    .await?;
    Ok(own + shared)
}
