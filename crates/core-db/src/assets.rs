//! Asset instances (Wave 11 lane 5, migration 0054): client-scoped rows in
//! `device_assets` (0002) gaining typed classification via
//! `document_layouts` — `layout_id` + schema-validated `fields`, mirroring
//! the documents shape. The free-text `kind`/`status` columns are kept for
//! connector compatibility; typed behavior binds to the layout.

use crate::{DbError, DbResult};
use core_domain::entities::DeviceAsset;
use sqlx::{Postgres, Row};
use uuid::Uuid;

pub(crate) const ASSET_COLUMNS: &str = "id, org_id, client_id, site_id, tag, name, kind, status, \
     layout_id, fields, ownership, rental_vendor, rental_start, rental_end, rental_cost_minor, \
     rental_currency, rental_cycle, purchase_date, purchase_cost_minor, purchase_currency, \
     depreciation_method, useful_life_months, salvage_value_minor, depreciation_rate_bps, \
     identity_keys, observed_fields, curated_fields, field_sources, merged_into_id, \
     created_at, updated_at";

pub(crate) fn row_to_asset(row: sqlx::postgres::PgRow) -> DeviceAsset {
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
        ownership: row.get("ownership"),
        rental_vendor: row.get("rental_vendor"),
        rental_start: row.get("rental_start"),
        rental_end: row.get("rental_end"),
        rental_cost_minor: row.get("rental_cost_minor"),
        rental_currency: row.get("rental_currency"),
        rental_cycle: row.get("rental_cycle"),
        purchase_date: row.get("purchase_date"),
        purchase_cost_minor: row.get("purchase_cost_minor"),
        purchase_currency: row.get("purchase_currency"),
        depreciation_method: row.get("depreciation_method"),
        useful_life_months: row.get("useful_life_months"),
        salvage_value_minor: row.get("salvage_value_minor"),
        depreciation_rate_bps: row.get("depreciation_rate_bps"),
        identity_keys: row.get("identity_keys"),
        observed_fields: row.get("observed_fields"),
        curated_fields: row.get("curated_fields"),
        field_sources: row.get("field_sources"),
        merged_into_id: row.get("merged_into_id"),
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

// ---- inventory lifecycle (migration 0061) ----------------------------------

/// Rental tracking + depreciation inputs, patch semantics: absent = keep.
#[derive(Debug, Clone, Default)]
pub struct AssetLifecyclePatch {
    pub ownership: Option<String>,
    pub rental_vendor: Option<String>,
    pub rental_start: Option<chrono::NaiveDate>,
    pub rental_end: Option<chrono::NaiveDate>,
    pub rental_cost_minor: Option<i64>,
    pub rental_currency: Option<String>,
    pub rental_cycle: Option<String>,
    pub purchase_date: Option<chrono::NaiveDate>,
    pub purchase_cost_minor: Option<i64>,
    pub purchase_currency: Option<String>,
    pub depreciation_method: Option<String>,
    pub useful_life_months: Option<i32>,
    pub salvage_value_minor: Option<i64>,
    pub depreciation_rate_bps: Option<i32>,
}

impl AssetLifecyclePatch {
    pub fn is_empty(&self) -> bool {
        self.ownership.is_none()
            && self.rental_vendor.is_none()
            && self.rental_start.is_none()
            && self.rental_end.is_none()
            && self.rental_cost_minor.is_none()
            && self.rental_currency.is_none()
            && self.rental_cycle.is_none()
            && self.purchase_date.is_none()
            && self.purchase_cost_minor.is_none()
            && self.purchase_currency.is_none()
            && self.depreciation_method.is_none()
            && self.useful_life_months.is_none()
            && self.salvage_value_minor.is_none()
            && self.depreciation_rate_bps.is_none()
    }
}

pub async fn update_asset_lifecycle<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    p: &AssetLifecyclePatch,
) -> DbResult<DeviceAsset>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        r#"UPDATE device_assets SET
             ownership = COALESCE($3, ownership),
             rental_vendor = COALESCE($4, rental_vendor),
             rental_start = COALESCE($5, rental_start),
             rental_end = COALESCE($6, rental_end),
             rental_cost_minor = COALESCE($7, rental_cost_minor),
             rental_currency = COALESCE($8, rental_currency),
             rental_cycle = COALESCE($9, rental_cycle),
             purchase_date = COALESCE($10, purchase_date),
             purchase_cost_minor = COALESCE($11, purchase_cost_minor),
             purchase_currency = COALESCE($12, purchase_currency),
             depreciation_method = COALESCE($13, depreciation_method),
             useful_life_months = COALESCE($14, useful_life_months),
             salvage_value_minor = COALESCE($15, salvage_value_minor),
             depreciation_rate_bps = COALESCE($16, depreciation_rate_bps),
             updated_at = now()
           WHERE org_id = $1 AND id = $2
           RETURNING {ASSET_COLUMNS}"#
    ))
    .bind(org_id)
    .bind(id)
    .bind(p.ownership.as_deref())
    .bind(p.rental_vendor.as_deref())
    .bind(p.rental_start)
    .bind(p.rental_end)
    .bind(p.rental_cost_minor)
    .bind(p.rental_currency.as_deref())
    .bind(p.rental_cycle.as_deref())
    .bind(p.purchase_date)
    .bind(p.purchase_cost_minor)
    .bind(p.purchase_currency.as_deref())
    .bind(p.depreciation_method.as_deref())
    .bind(p.useful_life_months)
    .bind(p.salvage_value_minor)
    .bind(p.depreciation_rate_bps)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row_to_asset(row))
}
