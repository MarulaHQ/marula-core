//! Org-export ledger (Wave 17 lane 13, migration 0089): the read model
//! over past org exports. Every bundle created through
//! POST /api/org/exports lands a row here so /settings/export can list
//! past exports, archive them, and mint ONE fresh signed download link
//! per row at request time.
//!
//! Purpose: insert-on-create, newest-first list, archive toggle, and the
//! row facts (bundle_version / size / sha256 / storage_key) the download
//! seam re-verifies. The ledger NEVER stores an expiring URL — the signed
//! link is minted from the stored reference with the same 15-minute TTL
//! the create flow documents.
//!
//! Non-purpose: no scheduled/retention machinery (rows are what they
//! are — an archived row keeps its bytes and its audit trail), no
//! per-row RBAC (owner/admin-gated at the route, same as the create).

use crate::DbResult;
use sqlx::PgConnection;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OrgExportRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub created_by: Option<Uuid>,
    pub bundle_version: i32,
    pub size_bytes: i64,
    pub sha256: String,
    pub storage_key: String,
    pub archived: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub struct NewOrgExport<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub created_by: Option<Uuid>,
    pub bundle_version: i32,
    pub size_bytes: i64,
    pub sha256: &'a str,
    pub storage_key: &'a str,
}

/// Lands one ledger row at bundle-creation time.
pub async fn insert(conn: &mut PgConnection, e: NewOrgExport<'_>) -> DbResult<OrgExportRow> {
    let row = sqlx::query_as::<_, OrgExportRow>(
        "INSERT INTO org_exports (id, org_id, created_by, bundle_version, size_bytes, sha256, storage_key)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, org_id, created_by, bundle_version, size_bytes, sha256, storage_key, archived, created_at",
    )
    .bind(e.id)
    .bind(e.org_id)
    .bind(e.created_by)
    .bind(e.bundle_version)
    .bind(e.size_bytes)
    .bind(e.sha256)
    .bind(e.storage_key)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row)
}

/// Fetches one row (RLS org-scoped) — the 404 + signature reference for
/// the download seam.
pub async fn get(conn: &mut PgConnection, org_id: Uuid, export_id: Uuid) -> DbResult<OrgExportRow> {
    sqlx::query_as::<_, OrgExportRow>(
        "SELECT id, org_id, created_by, bundle_version, size_bytes, sha256, storage_key, archived, created_at \
         FROM org_exports WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(export_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(crate::DbError::NotFound)
}

/// Newest-first ledger page; `archived` filter is None = all.
pub async fn list(
    conn: &mut PgConnection,
    org_id: Uuid,
    archived: Option<bool>,
    limit: i64,
    offset: i64,
) -> DbResult<Vec<OrgExportRow>> {
    let rows = sqlx::query_as::<_, OrgExportRow>(
        "SELECT id, org_id, created_by, bundle_version, size_bytes, sha256, storage_key, archived, created_at \
         FROM org_exports \
         WHERE org_id = $1 AND ($2::boolean IS NULL OR archived = $2) \
         ORDER BY created_at DESC, id DESC \
         LIMIT LEAST(GREATEST($3, 1), 200) OFFSET GREATEST($4, 0)",
    )
    .bind(org_id)
    .bind(archived)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows)
}

/// Archives (or unarchives) one row. Returns `None` when the row vanished
/// between the load and the update.
pub async fn set_archived(
    conn: &mut PgConnection,
    org_id: Uuid,
    export_id: Uuid,
    archived: bool,
) -> DbResult<Option<OrgExportRow>> {
    let row = sqlx::query_as::<_, OrgExportRow>(
        "UPDATE org_exports SET archived = $3 \
         WHERE org_id = $1 AND id = $2 \
         RETURNING id, org_id, created_by, bundle_version, size_bytes, sha256, storage_key, archived, created_at",
    )
    .bind(org_id)
    .bind(export_id)
    .bind(archived)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}
