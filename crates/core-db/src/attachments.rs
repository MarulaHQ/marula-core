//! Attachment metadata queries (the private build tracker 2.14).
//!
//! Purpose: persistence for `attachments` rows — insert, org-scoped
//! reads, comment linking, deletion with content-address refcounting,
//! portal visibility filtering, and the dedup lookup that lets N rows
//! share one stored object.
//!
//! Non-purpose: no byte movement (callers go through
//! `core_store::ObjectStore`), no admission rules (core-domain), no
//! authorization.

use crate::{DbError, DbResult};
use core_domain::entities::{Attachment, AttachmentKind};
use sqlx::{PgConnection, Postgres};
use uuid::Uuid;

const COLS: &str = "id, org_id, kind::text AS kind, ticket_id, ticket_comment_id, \
                    email_message_id, filename, content_type, size_bytes, sha256, \
                    storage_key, uploaded_by_user_id, uploaded_by_portal_user_id, created_at";

/// Same projection, qualified for queries that JOIN ticket_comments
/// (unqualified columns are ambiguous once both tables are in scope).
const QUALIFIED_COLS: &str = "a.id, a.org_id, a.kind::text AS kind, a.ticket_id, \
                    a.ticket_comment_id, a.email_message_id, a.filename, a.content_type, \
                    a.size_bytes, a.sha256, a.storage_key, a.uploaded_by_user_id, \
                    a.uploaded_by_portal_user_id, a.created_at";

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
pub async fn get<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Attachment>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        r#"SELECT {COLS} FROM attachments WHERE org_id = $1 AND id = $2"#
    ))
    .bind(org_id)
    .bind(id)
    .try_map(row_to_attachment)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row)
}

/// Every attachment hanging off one ticket (any kind).
pub async fn list_for_ticket<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<Vec<Attachment>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(&format!(
        r#"SELECT {COLS} FROM attachments WHERE org_id = $1 AND ticket_id = $2 ORDER BY created_at, id"#
    ))
    .bind(org_id)
    .bind(ticket_id)
    .try_map(row_to_attachment)
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// Attachments bound to one specific comment.
pub async fn list_for_comment<'e, E>(
    executor: E,
    org_id: Uuid,
    comment_id: Uuid,
) -> DbResult<Vec<Attachment>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(&format!(
        r#"SELECT {COLS} FROM attachments WHERE org_id = $1 AND ticket_comment_id = $2 ORDER BY created_at, id"#
    ))
    .bind(org_id)
    .bind(comment_id)
    .try_map(row_to_attachment)
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// Attachments extracted from one inbound email message.
pub async fn list_for_email<'e, E>(
    executor: E,
    org_id: Uuid,
    email_message_id: Uuid,
) -> DbResult<Vec<Attachment>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(&format!(
        r#"SELECT {COLS} FROM attachments WHERE org_id = $1 AND email_message_id = $2 ORDER BY created_at, id"#
    ))
    .bind(org_id)
    .bind(email_message_id)
    .try_map(row_to_attachment)
    .fetch_all(executor)
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
             AND kind::text = 'ticket_comment' AND ticket_comment_id IS NULL
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

/// Routing targets of an inbound message (for authz on its attachments).
pub async fn email_targets(
    conn: &mut PgConnection,
    org_id: Uuid,
    email_message_id: Uuid,
) -> DbResult<Option<(Option<Uuid>, Option<Uuid>)>> {
    let row: Option<(Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
        "SELECT ticket_id, comment_id FROM email_messages WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(email_message_id)
    .fetch_optional(conn)
    .await?;
    Ok(row)
}

// --- portal visibility (the private build tracker 2.13 exposure × the private build tracker 2.14 files) -------

/// Portal-visible attachments for one exposed ticket: kind=ticket rows
/// always, comment rows when their note kind is in the visible set, and
/// email rows always (inbound mail creates client-visible notes only).
/// Invisible rows are indistinguishable from absent ones (no existence
/// leak through 404s).
pub async fn list_portal_visible<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
    visible_comment_kinds: &[String],
) -> DbResult<Vec<Attachment>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(&format!(
        r#"SELECT {QUALIFIED_COLS} FROM attachments a
           LEFT JOIN ticket_comments c ON c.id = a.ticket_comment_id
           WHERE a.org_id = $1 AND a.ticket_id = $2
             AND (a.kind::text = 'ticket'
                  OR c.kind = ANY($3)
                  OR a.kind::text = 'email_message')
           ORDER BY a.created_at, a.id"#
    ))
    .bind(org_id)
    .bind(ticket_id)
    .bind(visible_comment_kinds.to_vec())
    .try_map(row_to_attachment)
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// One portal-visible attachment by id under the same visibility rule as
/// [`list_portal_visible`]; `None` covers both missing and hidden.
pub async fn find_portal_visible<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
    attachment_id: Uuid,
    visible_comment_kinds: &[String],
) -> DbResult<Option<Attachment>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        r#"SELECT {QUALIFIED_COLS} FROM attachments a
           LEFT JOIN ticket_comments c ON c.id = a.ticket_comment_id
           WHERE a.org_id = $1 AND a.ticket_id = $2 AND a.id = $3
             AND (a.kind::text = 'ticket'
                  OR c.kind = ANY($4)
                  OR a.kind::text = 'email_message')"#
    ))
    .bind(org_id)
    .bind(ticket_id)
    .bind(attachment_id)
    .bind(visible_comment_kinds.to_vec())
    .try_map(row_to_attachment)
    .fetch_optional(executor)
    .await?;
    Ok(row)
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

    #[test]
    fn qualified_cols_qualify_every_column() {
        for col in COLS.split(',') {
            let bare = col.trim().split(' ').next().unwrap_or("");
            assert!(
                QUALIFIED_COLS.contains(&format!("a.{bare}")),
                "QUALIFIED_COLS missing a.{bare}"
            );
        }
    }
}
