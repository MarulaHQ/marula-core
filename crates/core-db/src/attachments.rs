//! Attachment metadata queries (TASKLIST 2.14).
//!
//! Purpose: persistence for `attachments` rows — insert, org-scoped
//! reads, comment linking, deletion with content-address refcounting, and
//! the dedup lookup that lets N rows share one stored object.
//!
//! Non-purpose: no byte movement (callers go through
//! `core_store::ObjectStore`), no admission rules (core-domain), no
//! authorization.

use crate::{DbError, DbResult};
use core_domain::entities::{Attachment, AttachmentKind};
use sqlx::PgConnection;
use uuid::Uuid;

const COLS: &str = "id, org_id, kind::text AS kind, ticket_id, ticket_comment_id, \
                    email_message_id, filename, content_type, size_bytes, sha256, \
                    storage_key, uploaded_by_user_id, uploaded_by_portal_user_id, created_at";

fn row_to_attachment(row: sqlx::postgres::PgRow) -> Result<Attachment, sqlx::Error> {
    use sqlx::Row;
    let kind: String = row.get("kind");
    Ok(Attachment {
        id: row.get("id"),
        org_id: row.get("org_id"),
        kind: kind
            .parse()
            .map_err(|e: core_domain::DomainError| sqlx::Error::ColumnDecode {
                index: "kind".into(),
                source: Box::new(e),
            })?,
        ticket_id: row.get("ticket_id"),
        ticket_comment_id: row.get("ticket_comment_id"),
        email_message_id: row.get("email_message_id"),
        filename: row.get("filename"),
        content_type: row.get("content_type"),
        size_bytes: row.get("size_bytes"),
        sha256: row.get("sha256"),
        storage_key: row.get("storage_key"),
        uploaded_by_user_id: row.get("uploaded_by_user_id"),
        uploaded_by_portal_user_id: row.get("uploaded_by_portal_user_id"),
        created_at: row.get("created_at"),
    })
}

/// A new attachment row's fields (bytes already stored by the caller).
pub struct NewAttachment<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub kind: AttachmentKind,
    pub ticket_id: Option<Uuid>,
    pub ticket_comment_id: Option<Uuid>,
    pub email_message_id: Option<Uuid>,
    pub filename: &'a str,
    pub content_type: &'a str,
    pub size_bytes: i64,
    pub sha256: &'a str,
    pub storage_key: &'a str,
    pub uploaded_by_user_id: Option<Uuid>,
    pub uploaded_by_portal_user_id: Option<Uuid>,
}

/// Inserts one attachment metadata row.
pub async fn insert(conn: &mut PgConnection, a: NewAttachment<'_>) -> DbResult<Attachment> {
    let row = sqlx::query(&format!(
        r#"INSERT INTO attachments (id, org_id, kind, ticket_id, ticket_comment_id,
                                   email_message_id, filename, content_type, size_bytes,
                                   sha256, storage_key, uploaded_by_user_id,
                                   uploaded_by_portal_user_id)
           VALUES ($1, $2, $3::text::attachment_kind, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
           RETURNING {COLS}"#
    ))
    .bind(a.id)
    .bind(a.org_id)
    .bind(a.kind.as_str())
    .bind(a.ticket_id)
    .bind(a.ticket_comment_id)
    .bind(a.email_message_id)
    .bind(a.filename)
    .bind(a.content_type)
    .bind(a.size_bytes)
    .bind(a.sha256)
    .bind(a.storage_key)
    .bind(a.uploaded_by_user_id)
    .bind(a.uploaded_by_portal_user_id)
    .try_map(row_to_attachment)
    .fetch_one(conn)
    .await?;
    Ok(row)
}

/// Fetches one attachment by id within the org.
pub async fn get(conn: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<Attachment> {
    let row = sqlx::query(&format!(
        r#"SELECT {COLS} FROM attachments WHERE org_id = $1 AND id = $2"#
    ))
    .bind(org_id)
    .bind(id)
    .try_map(row_to_attachment)
    .fetch_optional(conn)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row)
}

/// Every attachment hanging off one ticket (any kind).
pub async fn list_for_ticket(
    conn: &mut PgConnection,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<Vec<Attachment>> {
    let rows = sqlx::query(&format!(
        r#"SELECT {COLS} FROM attachments WHERE org_id = $1 AND ticket_id = $2 ORDER BY created_at, id"#
    ))
    .bind(org_id)
    .bind(ticket_id)
    .try_map(row_to_attachment)
    .fetch_all(conn)
    .await?;
    Ok(rows)
}

/// Attachments bound to one specific comment.
pub async fn list_for_comment(
    conn: &mut PgConnection,
    org_id: Uuid,
    comment_id: Uuid,
) -> DbResult<Vec<Attachment>> {
    let rows = sqlx::query(&format!(
        r#"SELECT {COLS} FROM attachments WHERE org_id = $1 AND ticket_comment_id = $2 ORDER BY created_at, id"#
    ))
    .bind(org_id)
    .bind(comment_id)
    .try_map(row_to_attachment)
    .fetch_all(conn)
    .await?;
    Ok(rows)
}

/// Attachments extracted from one inbound email message.
pub async fn list_for_email(
    conn: &mut PgConnection,
    org_id: Uuid,
    email_message_id: Uuid,
) -> DbResult<Vec<Attachment>> {
    let rows = sqlx::query(&format!(
        r#"SELECT {COLS} FROM attachments WHERE org_id = $1 AND email_message_id = $2 ORDER BY created_at, id"#
    ))
    .bind(org_id)
    .bind(email_message_id)
    .try_map(row_to_attachment)
    .fetch_all(conn)
    .await?;
    Ok(rows)
}

/// Dedup lookup: the storage key this org ALREADY holds for these exact
/// bytes, when any prior attachment references them.
pub async fn find_existing_object_key(
    conn: &mut PgConnection,
    org_id: Uuid,
    sha256_hex: &str,
) -> DbResult<Option<String>> {
    let key: Option<String> = sqlx::query_scalar(
        "SELECT storage_key FROM attachments WHERE org_id = $1 AND sha256 = $2 LIMIT 1",
    )
    .bind(org_id)
    .bind(sha256_hex)
    .fetch_optional(conn)
    .await?;
    Ok(key)
}

/// Binds an unattached upload to its comment. Succeeds only when the row
/// is still unlinked and belongs to the same ticket — double-link races
/// surface as `None`.
pub async fn link_to_comment(
    conn: &mut PgConnection,
    org_id: Uuid,
    attachment_id: Uuid,
    ticket_id: Uuid,
    comment_id: Uuid,
) -> DbResult<Option<Attachment>> {
    let row = sqlx::query(&format!(
        r#"UPDATE attachments SET ticket_comment_id = $3
           WHERE org_id = $1 AND id = $2 AND ticket_id = $4
             AND kind = 'ticket_comment' AND ticket_comment_id IS NULL
           RETURNING {COLS}"#
    ))
    .bind(org_id)
    .bind(attachment_id)
    .bind(comment_id)
    .bind(ticket_id)
    .try_map(row_to_attachment)
    .fetch_optional(conn)
    .await?;
    Ok(row)
}

/// Deletes one row; returns it so the caller can reclaim the object only
/// when [`count_other_refs`] reports zero remaining references.
pub async fn delete(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<Option<Attachment>> {
    let row = sqlx::query(&format!(
        r#"DELETE FROM attachments WHERE org_id = $1 AND id = $2 RETURNING {COLS}"#
    ))
    .bind(org_id)
    .bind(id)
    .try_map(row_to_attachment)
    .fetch_optional(conn)
    .await?;
    Ok(row)
}

/// How many OTHER rows still reference this storage key after a delete —
/// the object is reclaimed only on zero.
pub async fn count_other_refs(
    conn: &mut PgConnection,
    org_id: Uuid,
    storage_key: &str,
    excluding_id: Uuid,
) -> DbResult<i64> {
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM attachments WHERE org_id = $1 AND storage_key = $2 AND id <> $3",
    )
    .bind(org_id)
    .bind(storage_key)
    .bind(excluding_id)
    .fetch_one(conn)
    .await?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cols_cover_every_entity_field() {
        // Guard against silent column drift between COLS and the entity.
        for col in [
            "id",
            "org_id",
            "kind",
            "ticket_id",
            "ticket_comment_id",
            "email_message_id",
            "filename",
            "content_type",
            "size_bytes",
            "sha256",
            "storage_key",
            "uploaded_by_user_id",
            "uploaded_by_portal_user_id",
            "created_at",
        ] {
            assert!(COLS.contains(col), "COLS missing {col}");
        }
    }
}
