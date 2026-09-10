//! DocumentVersion queries (TASKLIST D4, agent-platform §6): documents
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

/// Shared documents-row shape (0053 added client scope + layout fields).
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
    let doc = sqlx::query!(
        r#"INSERT INTO documents (id, org_id, client_id, layout_id, fields, title, current_version, created_by, updated_by)
           VALUES ($1, $2, $3, $4, $5, $6, 1, $7, $7)
           RETURNING id, org_id, client_id, layout_id, fields, title, current_version, created_by, updated_by, created_at, updated_at"#,
        id,
        org_id,
        client_id,
        layout_id,
        fields,
        title,
        author_id
    )
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
    Ok((
        DocRow {
            id: doc.id,
            org_id: doc.org_id,
            client_id: doc.client_id,
            layout_id: doc.layout_id,
            fields: doc.fields,
            title: doc.title,
            current_version: doc.current_version,
            created_by: doc.created_by,
            updated_by: doc.updated_by,
            created_at: doc.created_at,
            updated_at: doc.updated_at,
        }
        .into_document(),
        version,
    ))
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
    let row = sqlx::query!(
        r#"SELECT id, org_id, client_id, layout_id, fields, title, current_version, created_by, updated_by, created_at, updated_at
           FROM documents WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(DocRow {
        id: row.id,
        org_id: row.org_id,
        client_id: row.client_id,
        layout_id: row.layout_id,
        fields: row.fields,
        title: row.title,
        current_version: row.current_version,
        created_by: row.created_by,
        updated_by: row.updated_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
    .into_document())
}

#[derive(Debug, Default, Clone)]
pub struct DocumentFilter {
    /// Case-insensitive substring match over title.
    pub q: Option<String>,
    /// Scope narrow: a client id, or `global` for org-wide documents.
    /// None = all scopes (docs-platform v1, 0053).
    pub client_id: Option<Option<Uuid>>,
    pub layout_id: Option<Uuid>,
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
    let rows = sqlx::query!(
        r#"SELECT id, org_id, client_id, layout_id, fields, title, current_version, created_by, updated_by, created_at, updated_at
           FROM documents
           WHERE org_id = $1
             AND ($2::text IS NULL OR title ILIKE '%' || $2 || '%')
             AND ($3::uuid IS NULL OR client_id = $3)
             AND (NOT $4::bool OR client_id IS NULL)
             AND ($5::uuid IS NULL OR layout_id = $5)
           ORDER BY updated_at DESC, id"#,
        org_id,
        q,
        client_id,
        matches!(filter.client_id, Some(None)),
        layout_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            DocRow {
                id: r.id,
                org_id: r.org_id,
                client_id: r.client_id,
                layout_id: r.layout_id,
                fields: r.fields,
                title: r.title,
                current_version: r.current_version,
                created_by: r.created_by,
                updated_by: r.updated_by,
                created_at: r.created_at,
                updated_at: r.updated_at,
            }
            .into_document()
        })
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
    let row = sqlx::query!(
        r#"UPDATE documents SET
             title = $3, client_id = $4, layout_id = $5, fields = $6, updated_at = now()
           WHERE org_id = $1 AND id = $2
           RETURNING id, org_id, client_id, layout_id, fields, title, current_version,
                     created_by, updated_by, created_at, updated_at"#,
        org_id,
        id,
        title,
        client_id,
        layout_id,
        fields
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(DocRow {
        id: row.id,
        org_id: row.org_id,
        client_id: row.client_id,
        layout_id: row.layout_id,
        fields: row.fields,
        title: row.title,
        current_version: row.current_version,
        created_by: row.created_by,
        updated_by: row.updated_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
    .into_document())
}
