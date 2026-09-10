//! Asset instances (Wave 11 lane 5, migration 0054): client-scoped rows in
//! `device_assets` (0002) gaining typed classification via
//! `document_layouts` — `layout_id` + schema-validated `fields`, mirroring
//! the documents shape. The free-text `kind`/`status` columns are kept for
//! connector compatibility; typed behavior binds to the layout.

use crate::{DbError, DbResult};
use core_domain::entities::DeviceAsset;
use sqlx::{Postgres, Row};
use uuid::Uuid;

const ASSET_COLUMNS: &str = "id, org_id, client_id, site_id, tag, name, kind, status, \
     layout_id, fields, created_at, updated_at";

fn row_to_asset(row: sqlx::postgres::PgRow) -> DeviceAsset {
    DeviceAsset {
        id: row.get("id"),
        org_id: row.get("org_id"),
        client_id: row.get("client_id"),
        site_id: row.get("site_id"),
        tag: row.get("tag"),
        name: row.get("name"),
        kind: row.get("kind"),
        status: row.get("status"),
        layout_id: row.get("layout_id"),
        fields: row.get("fields"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

/// Org-wide asset list with filters — the resource-REST surface for
/// external-DMS parity (IT Glue/Hudu exports, generic REST). Filters:
/// optional client, optional layout, optional `q` substring over
/// tag/name/hostname (hostname comes from the custom `fields`).
pub async fn list_assets<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Option<Uuid>,
    layout_id: Option<Uuid>,
    q: Option<&str>,
) -> DbResult<Vec<DeviceAsset>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(&format!(
        r#"SELECT {ASSET_COLUMNS}
           FROM device_assets
           WHERE org_id = $1
             AND ($2::uuid IS NULL OR client_id = $2)
             AND ($3::uuid IS NULL OR layout_id = $3)
             AND ($4::text IS NULL OR
                  tag ILIKE '%' || $4 || '%' OR name ILIKE '%' || $4 || '%' OR
                  fields::text ILIKE '%' || $4 || '%')
           ORDER BY tag, name"#
    ))
    .bind(org_id)
    .bind(client_id)
    .bind(layout_id)
    .bind(q.map(|s| s.trim()).filter(|s| !s.is_empty()))
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(row_to_asset).collect())
}

pub async fn get_asset<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<DeviceAsset>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(&format!(
        r#"SELECT {ASSET_COLUMNS} FROM device_assets WHERE org_id = $1 AND id = $2"#
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .map(row_to_asset)
    .ok_or(DbError::NotFound)
}

/// Creates a typed asset instance. `fields` is validated against the
/// layout's schema by the API layer (outside the org tx).
#[allow(clippy::too_many_arguments)]
pub async fn create_asset<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    client_id: Uuid,
    site_id: Option<Uuid>,
    tag: &str,
    name: &str,
    kind: &str,
    status: &str,
    layout_id: Option<Uuid>,
    fields: &serde_json::Value,
) -> DbResult<DeviceAsset>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        r#"INSERT INTO device_assets
             (id, org_id, client_id, site_id, tag, name, kind, status, layout_id, fields)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING {ASSET_COLUMNS}"#
    ))
    .bind(id)
    .bind(org_id)
    .bind(client_id)
    .bind(site_id)
    .bind(tag)
    .bind(name)
    .bind(kind)
    .bind(status)
    .bind(layout_id)
    .bind(fields)
    .fetch_one(executor)
    .await?;
    Ok(row_to_asset(row))
}

/// Patch semantics: absent = keep (no un-set in v1). A layout change
/// re-validates the effective fields (kept fields vs the NEW schema) in
/// the API layer.
#[allow(clippy::too_many_arguments)]
pub async fn update_asset<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    name: Option<&str>,
    status: Option<&str>,
    site_id: Option<Option<Uuid>>,
    layout_id: Option<Option<Uuid>>,
    fields: Option<&serde_json::Value>,
) -> DbResult<DeviceAsset>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        r#"UPDATE device_assets SET
             name = COALESCE($3, name),
             status = COALESCE($4, status),
             site_id = COALESCE($5, site_id),
             layout_id = COALESCE($6, layout_id),
             fields = COALESCE($7, fields),
             updated_at = now()
           WHERE org_id = $1 AND id = $2
           RETURNING {ASSET_COLUMNS}"#
    ))
    .bind(org_id)
    .bind(id)
    .bind(name)
    .bind(status)
    .bind(site_id)
    .bind(layout_id)
    .bind(fields)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row_to_asset(row))
}

pub async fn delete_asset<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let r = sqlx::query("DELETE FROM device_assets WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(executor)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}
