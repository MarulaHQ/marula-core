//! Catalog persistence (TASKLIST Phase 3, Wave 12 lane 2; migration
//! 0060): org-scoped product/service items.
//!
//! Every read and write carries an explicit `org_id` predicate on top of
//! the migration's RLS policies (same posture as the rest of the crate).
//! SKU uniqueness is DB-enforced (partial unique index on lower(sku));
//! the API layer maps the unique-violation to a typed 409.

use crate::{DbError, DbResult};
use core_domain::entities::CatalogItem;
use sqlx::Postgres;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
struct CatalogItemRow {
    id: Uuid,
    org_id: Uuid,
    name: String,
    sku: Option<String>,
    description: Option<String>,
    unit: String,
    currency: String,
    unit_price_minor: i64,
    cost_minor: i64,
    taxable: bool,
    category: Option<String>,
    group_name: Option<String>,
    active: bool,
    created_by: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn item_row(r: CatalogItemRow) -> CatalogItem {
    CatalogItem {
        id: r.id,
        org_id: r.org_id,
        name: r.name,
        sku: r.sku.filter(|s| !s.trim().is_empty()),
        description: r.description,
        unit: r.unit,
        currency: r.currency,
        unit_price_minor: r.unit_price_minor,
        cost_minor: r.cost_minor,
        taxable: r.taxable,
        category: r.category,
        group_name: r.group_name,
        active: r.active,
        created_by: r.created_by,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

const COLUMNS: &str = "id, org_id, name, sku, description, unit, currency, unit_price_minor,
                        cost_minor, taxable, category, group_name, active, created_by,
                        created_at, updated_at";

pub async fn insert_catalog_item<'e, E>(executor: E, item: &CatalogItem) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"INSERT INTO catalog_items (id, org_id, name, sku, description, unit, currency,
                                      unit_price_minor, cost_minor, taxable, category,
                                      group_name, active, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)"#,
    )
    .bind(item.id)
    .bind(item.org_id)
    .bind(item.name.trim())
    .bind(item.sku.as_deref().map(str::trim))
    .bind(item.description.as_deref())
    .bind(item.unit.trim())
    .bind(&item.currency)
    .bind(item.unit_price_minor)
    .bind(item.cost_minor)
    .bind(item.taxable)
    .bind(item.category.as_deref())
    .bind(item.group_name.as_deref())
    .bind(item.active)
    .bind(item.created_by)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_catalog_item<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<CatalogItem>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {COLUMNS} FROM catalog_items WHERE org_id = $1 AND id = $2");
    let row = sqlx::query_as::<_, CatalogItemRow>(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(item_row(row))
}

pub struct CatalogItemFilters {
    /// `None` = all items (active and archived).
    pub active: Option<bool>,
    pub category: Option<String>,
    pub group_name: Option<String>,
    /// Substring match over name/sku/category/group.
    pub search: Option<String>,
}

pub async fn list_catalog_items<'e, E>(
    executor: E,
    org_id: Uuid,
    filters: &CatalogItemFilters,
    limit: i64,
) -> DbResult<Vec<CatalogItem>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {COLUMNS} FROM catalog_items
         WHERE org_id = $1
           AND ($2::boolean IS NULL OR active = $2)
           AND ($3::text IS NULL OR category = $3)
           AND ($4::text IS NULL OR group_name = $4)
           AND ($5::text IS NULL OR
                name ILIKE '%' || $5 || '%' OR
                coalesce(sku, '') ILIKE '%' || $5 || '%' OR
                coalesce(category, '') ILIKE '%' || $5 || '%' OR
                coalesce(group_name, '') ILIKE '%' || $5 || '%')
         ORDER BY active DESC, name, id
         LIMIT $6"
    );
    let rows = sqlx::query_as::<_, CatalogItemRow>(&sql)
        .bind(org_id)
        .bind(filters.active)
        .bind(filters.category.as_deref())
        .bind(filters.group_name.as_deref())
        .bind(filters.search.as_deref())
        .bind(limit)
        .fetch_all(executor)
        .await?;
    Ok(rows.into_iter().map(item_row).collect())
}

/// Full-row update (id + org scoped). `None` = the item does not exist.
pub async fn update_catalog_item<'e, E>(
    executor: E,
    item: &CatalogItem,
) -> DbResult<Option<CatalogItem>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE catalog_items SET name = $3, sku = $4, description = $5, unit = $6,
                                  currency = $7, unit_price_minor = $8, cost_minor = $9,
                                  taxable = $10, category = $11, group_name = $12,
                                  active = $13, updated_at = now()
         WHERE org_id = $1 AND id = $2
         RETURNING {COLUMNS}"
    );
    let row = sqlx::query_as::<_, CatalogItemRow>(&sql)
        .bind(item.org_id)
        .bind(item.id)
        .bind(item.name.trim())
        .bind(item.sku.as_deref().map(str::trim))
        .bind(item.description.as_deref())
        .bind(item.unit.trim())
        .bind(&item.currency)
        .bind(item.unit_price_minor)
        .bind(item.cost_minor)
        .bind(item.taxable)
        .bind(item.category.as_deref())
        .bind(item.group_name.as_deref())
        .bind(item.active)
        .fetch_optional(executor)
        .await?;
    Ok(row.map(item_row))
}

/// Flips `active` off. `None` = the item does not exist or is already
/// archived (the API layer distinguishes via a prior read).
pub async fn archive_catalog_item<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<Option<CatalogItem>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE catalog_items SET active = false, updated_at = now()
         WHERE org_id = $1 AND id = $2 AND active
         RETURNING {COLUMNS}"
    );
    let row = sqlx::query_as::<_, CatalogItemRow>(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?;
    Ok(row.map(item_row))
}

/// Resolves a catalog item by SKU within one org (quote-line snapshots
/// key on it). Empty-string/None SKUs never match.
pub async fn find_by_sku<'e, E>(
    executor: E,
    org_id: Uuid,
    sku: &str,
) -> DbResult<Option<CatalogItem>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {COLUMNS} FROM catalog_items
         WHERE org_id = $1 AND lower(sku) = lower($2) LIMIT 1"
    );
    let row = sqlx::query_as::<_, CatalogItemRow>(&sql)
        .bind(org_id)
        .bind(sku)
        .fetch_optional(executor)
        .await?;
    Ok(row.map(item_row))
}

/// Maps a Postgres unique-violation on a 0060 unique constraint to a
/// typed conflict (SKU, quote numbers, stage names). The API layer maps
/// this to a 409.
pub fn is_unique_conflict(e: &sqlx::Error) -> bool {
    match e.as_database_error().and_then(|d| d.constraint()) {
        Some(constraint) => matches!(
            constraint,
            "uq_catalog_items_sku" | "quotes_number_key" | "pipeline_stages_name_key"
        ),
        None => false,
    }
}
