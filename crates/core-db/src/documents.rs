//! DocumentVersion queries (the private build tracker D4, agent-platform §6): documents
//! with immutable append-only versions.
//!
//! Purpose: typed persistence over `documents` / `document_versions`
//! (0032). Version rows are APPEND-ONLY at the app layer (audit_events
//! posture): this module exposes insert + read paths only, never UPDATE
//! or DELETE on versions. Numbering is MAX(version)+1 under the
//! instant-first entity seq lock (serialization lives in the API's
//! transaction, where the lock is taken before these queries run).
//!
//! Non-purpose: restore planning and validation (core-documents owns the
//! policy; callers hand the plan's fields to [`insert_version`]) and
//! search (indexer-owned, kind `document`).

use crate::{DbError, DbResult};
use core_domain::entities::{Document, DocumentVersion};
use serde_json::Value;
use sqlx::Postgres;
use uuid::Uuid;

/// Shared documents-row shape (0053 added client scope + layout fields,
/// 0228 adds the archive marker).
struct DocRow {
    id: Uuid,
    org_id: Uuid,
    client_id: Option<Uuid>,
    layout_id: Option<Uuid>,
    fields: Value,
    title: String,
    current_version: i32,
    created_by: Option<Uuid>,
    updated_by: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    archived_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// The projection every document read and write returns, so one decoder turns
/// all of them into a `Document`.
const DOC_COLUMNS: &str = "id, org_id, client_id, layout_id, fields, title, current_version, \
     created_by, updated_by, created_at, updated_at, archived_at";

fn row_to_doc_row(row: sqlx::postgres::PgRow) -> DocRow {
    use sqlx::Row;
    DocRow {
        id: row.get("id"),
        org_id: row.get("org_id"),
        client_id: row.get("client_id"),
        layout_id: row.get("layout_id"),
        fields: row.get("fields"),
        title: row.get("title"),
        current_version: row.get("current_version"),
        created_by: row.get("created_by"),
        updated_by: row.get("updated_by"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        archived_at: row.get("archived_at"),
    }
}

impl DocRow {
    fn into_document(self) -> Document {
        Document {
            id: self.id,
            org_id: self.org_id,
            client_id: self.client_id,
            layout_id: self.layout_id,
            fields: self.fields,
            title: self.title,
            current_version: self.current_version,
            created_by: self.created_by,
            updated_by: self.updated_by,
            created_at: self.created_at,
            updated_at: self.updated_at,
            archived_at: self.archived_at,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn row_version(
    id: Uuid,
    org_id: Uuid,
    document_id: Uuid,
    version: i32,
    title: String,
    body: String,
    change_summary: String,
    author_id: Option<Uuid>,
    restored_from_version: Option<i32>,
    created_at: chrono::DateTime<chrono::Utc>,
) -> DocumentVersion {
    DocumentVersion {
        id,
        org_id,
        document_id,
        version,
        title,
        body,
        change_summary,
        author_id,
        restored_from_version,
        created_at,
    }
}

/// Creates the document AND its immutable version 1 in one statement pair
/// (same transaction via the connection seam — callers inside an org tx
/// pass `&mut *tx`).
#[allow(clippy::too_many_arguments)]
pub async fn create_document(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    client_id: Option<Uuid>,
    layout_id: Option<Uuid>,
    fields: &Value,
    title: &str,
    body: &str,
    change_summary: &str,
    author_id: Uuid,
) -> DbResult<(Document, DocumentVersion)> {
    let sql = format!(
        r#"INSERT INTO documents (id, org_id, client_id, layout_id, fields, title, current_version, created_by, updated_by)
           VALUES ($1, $2, $3, $4, $5, $6, 1, $7, $7)
           RETURNING {DOC_COLUMNS}"#
    );
    let doc = sqlx::query(&sql)
        .bind(id)
        .bind(org_id)
        .bind(client_id)
        .bind(layout_id)
        .bind(fields)
        .bind(title)
        .bind(author_id)
        .fetch_one(&mut *executor)
        .await?;
    let version = insert_version(
        executor,
        org_id,
        id,
        1,
        title,
        body,
        change_summary,
        author_id,
        None,
    )
    .await?;
    Ok((row_to_doc_row(doc).into_document(), version))
}

/// Appends one immutable version and bumps the document's current_version,
/// title and updated_at. This is the ONLY write into document_versions;
/// there is deliberately no update or delete counterpart.
#[allow(clippy::too_many_arguments)]
pub async fn insert_version<'e, E>(
    executor: E,
    org_id: Uuid,
    document_id: Uuid,
    version: i32,
    title: &str,
    body: &str,
    change_summary: &str,
    author_id: Uuid,
    restored_from_version: Option<i32>,
) -> DbResult<DocumentVersion>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"WITH bumped AS (
             UPDATE documents SET
               current_version = $4,
               title = $5,
               updated_by = $8,
               updated_at = now()
             WHERE org_id = $1 AND id = $2
             RETURNING 1
           )
           INSERT INTO document_versions
             (id, org_id, document_id, version, title, body, change_summary,
              author_id, restored_from_version)
           SELECT $3, $1, $2, $4, $5, $6, $7, $8, $9 FROM bumped
           RETURNING id, org_id, document_id, version, title, body, change_summary,
                     author_id, restored_from_version, created_at"#,
        org_id,
        document_id,
        Uuid::now_v7(),
        version,
        title,
        body,
        change_summary,
        author_id,
        restored_from_version
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row_version(
        row.id,
        row.org_id,
        row.document_id,
        row.version,
        row.title,
        row.body,
        row.change_summary,
        row.author_id,
        row.restored_from_version,
        row.created_at,
    ))
}

/// The document's newest version number; `None` when the document does not
/// exist (or is invisible under RLS — callers map that to their scope
/// error, mirroring the get paths).
pub async fn max_version<'e, E>(
    executor: E,
    org_id: Uuid,
    document_id: Uuid,
) -> DbResult<Option<i32>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row: Option<Option<i32>> = sqlx::query_scalar(
        "SELECT MAX(version) FROM document_versions WHERE org_id = $1 AND document_id = $2",
    )
    .bind(org_id)
    .bind(document_id)
    .fetch_optional(executor)
    .await?;
    Ok(row.flatten())
}

pub async fn get_document<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Document>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {DOC_COLUMNS} FROM documents WHERE org_id = $1 AND id = $2");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_doc_row(row).into_document())
}

/// Archives or restores a document. Archived documents stay readable by id so
/// links and version history resolve; lists and search hide them by default.
pub async fn set_document_archived(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    archived: bool,
) -> DbResult<()> {
    let res = sqlx::query(
        r#"UPDATE documents
              SET archived_at = CASE WHEN $3 THEN COALESCE(archived_at, now()) ELSE NULL END,
                  updated_at = now()
            WHERE org_id = $1 AND id = $2"#,
    )
    .bind(org_id)
    .bind(id)
    .bind(archived)
    .execute(&mut *conn)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

#[derive(Debug, Default, Clone)]
pub struct DocumentFilter {
    /// Case-insensitive substring match over title.
    pub q: Option<String>,
    /// Scope narrow: a client id, or `global` for org-wide documents.
    /// None = all scopes (docs-platform v1, 0053).
    pub client_id: Option<Option<Uuid>>,
    pub layout_id: Option<Uuid>,
    /// Archived documents are hidden by default; a staff list may ask for
    /// them (the archive view). Every other surface leaves this false.
    pub include_archived: bool,
}

/// The list statement for a filter, so the archive predicate is testable
/// without a database.
fn list_documents_sql(filter: &DocumentFilter) -> String {
    let archived_clause = if filter.include_archived {
        ""
    } else {
        "AND archived_at IS NULL"
    };
    format!(
        r#"SELECT {DOC_COLUMNS}
           FROM documents
           WHERE org_id = $1
             AND ($2::text IS NULL OR title ILIKE '%' || $2 || '%')
             AND ($3::uuid IS NULL OR client_id = $3)
             AND (NOT $4::bool OR client_id IS NULL)
             AND ($5::uuid IS NULL OR layout_id = $5)
             {archived_clause}
           ORDER BY updated_at DESC, id"#
    )
}

pub async fn list_documents<'e, E>(
    executor: E,
    org_id: Uuid,
    filter: &DocumentFilter,
) -> DbResult<Vec<Document>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let q = filter.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let client_id = filter.client_id.flatten();
    let layout_id = filter.layout_id;
    let sql = list_documents_sql(filter);
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(q)
        .bind(client_id)
        .bind(matches!(filter.client_id, Some(None)))
        .bind(layout_id)
        .fetch_all(executor)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| row_to_doc_row(row).into_document())
        .collect())
}

/// Full history, newest first — metadata plus complete bodies so any pair
/// is diffable without extra fetches.
pub async fn list_versions<'e, E>(
    executor: E,
    org_id: Uuid,
    document_id: Uuid,
) -> DbResult<Vec<DocumentVersion>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, document_id, version, title, body, change_summary,
                  author_id, restored_from_version, created_at
           FROM document_versions
           WHERE org_id = $1 AND document_id = $2
           ORDER BY version DESC"#,
        org_id,
        document_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            row_version(
                r.id,
                r.org_id,
                r.document_id,
                r.version,
                r.title,
                r.body,
                r.change_summary,
                r.author_id,
                r.restored_from_version,
                r.created_at,
            )
        })
        .collect())
}

pub async fn get_version<'e, E>(
    executor: E,
    org_id: Uuid,
    document_id: Uuid,
    version: i32,
) -> DbResult<DocumentVersion>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, document_id, version, title, body, change_summary,
                  author_id, restored_from_version, created_at
           FROM document_versions
           WHERE org_id = $1 AND document_id = $2 AND version = $3"#,
        org_id,
        document_id,
        version
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row_version(
        row.id,
        row.org_id,
        row.document_id,
        row.version,
        row.title,
        row.body,
        row.change_summary,
        row.author_id,
        row.restored_from_version,
        row.created_at,
    ))
}

/// Updates the mutable handle columns (docs-platform v1, 0053): client
/// scope, layout, custom fields, title. Content changes still go through
/// [`insert_version`] — this never touches version rows.
#[allow(clippy::too_many_arguments)]
pub async fn update_document_meta<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    title: &str,
    client_id: Option<Uuid>,
    layout_id: Option<Uuid>,
    fields: &Value,
) -> DbResult<Document>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"UPDATE documents SET
             title = $3, client_id = $4, layout_id = $5, fields = $6, updated_at = now()
           WHERE org_id = $1 AND id = $2
           RETURNING {DOC_COLUMNS}"#
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .bind(title)
        .bind(client_id)
        .bind(layout_id)
        .bind(fields)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_doc_row(row).into_document())
}

#[cfg(test)]
mod archive_tests {
    use super::*;

    #[test]
    fn document_list_hides_archived_unless_asked() {
        let live = list_documents_sql(&DocumentFilter::default());
        assert!(live.contains("AND archived_at IS NULL"));
        let all = list_documents_sql(&DocumentFilter {
            include_archived: true,
            ..Default::default()
        });
        assert!(!all.contains("archived_at IS NULL"));
    }
}
