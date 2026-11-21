//! Asset instances (Wave 11 lane 5, migration 0054): client-scoped rows in
//! `device_assets` (0002) gaining typed classification via
//! `document_layouts` — `layout_id` + schema-validated `fields`, mirroring
//! the documents shape. The free-text `kind`/`status` columns are kept for
//! connector compatibility; typed behavior binds to the layout.

use crate::{DbError, DbResult};
use chrono::Utc;
use core_domain::entities::DeviceAsset;
use sqlx::{Postgres, Row};
use uuid::Uuid;

pub(crate) const ASSET_COLUMNS: &str = "id, org_id, client_id, scope, site_id, tag, name, kind, status, \
     layout_id, fields, ownership, rental_vendor, rental_start, rental_end, rental_cost_minor, \
     rental_currency, rental_cycle, purchase_date, purchase_cost_minor, purchase_currency, \
     purchase_fx_rate_used::text AS purchase_fx_rate_used, purchase_fx_rate_date, purchase_fx_source, \
     base_purchase_cost_minor, \
     depreciation_method, useful_life_months, salvage_value_minor, depreciation_rate_bps, \
     identity_keys, observed_fields, curated_fields, field_sources, merged_into_id, \
     created_at, updated_at";

pub(crate) fn row_to_asset(row: sqlx::postgres::PgRow) -> DeviceAsset {
    DeviceAsset {
        id: row.get("id"),
        org_id: row.get("org_id"),
        client_id: row.get("client_id"),
        scope: row.get("scope"),
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
        purchase_fx_rate_used: row.get("purchase_fx_rate_used"),
        purchase_fx_rate_date: row.get("purchase_fx_rate_date"),
        purchase_fx_source: row.get("purchase_fx_source"),
        base_purchase_cost_minor: row.get("base_purchase_cost_minor"),
        depreciation_method: row.get("depreciation_method"),
        useful_life_months: row.get("useful_life_months"),
        salvage_value_minor: row.get("salvage_value_minor"),
        depreciation_rate_bps: row.get("depreciation_rate_bps"),
        // The one place the warranty horizon is derived. Every read of an
        // asset row lands here, so the ledger, the record page and the scan
        // resolver answer from the same rule rather than each inventing one.
        warranty: {
            let fields: serde_json::Value = row.get("fields");
            core_domain::entities::client::asset_warranty_from_fields(
                &fields,
                row.get("purchase_date"),
                row.get("useful_life_months"),
                Utc::now().date_naive(),
            )
        },
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
/// optional client, optional scope (`client` | `internal_org`, migration
/// 0103), optional layout, optional `q` substring over tag/name/hostname
/// (hostname comes from the custom `fields`).
pub async fn list_assets<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Option<Uuid>,
    scope: Option<&str>,
    layout_id: Option<Uuid>,
    q: Option<&str>,
    visibility: Option<crate::tickets::ClientVisibility>,
) -> DbResult<Vec<DeviceAsset>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    // A client-restricted row is filtered before the result set, never after:
    // this list is not paginated today, but an unqualified read here becomes a
    // short page the moment it grows one.
    let visibility_clause = if visibility.is_some() {
        r#"AND (client_id IS NULL OR NOT EXISTS (
                 SELECT 1 FROM clients c
                  WHERE c.id = device_assets.client_id
                    AND c.org_id = device_assets.org_id
                    AND jsonb_array_length(c.restricted_to) > 0
                    AND NOT (c.restricted_to ? $6::text)
                    AND NOT $7))"#
    } else {
        ""
    };
    let rows = {
        let sql = format!(
            r#"SELECT {ASSET_COLUMNS}
               FROM device_assets
               WHERE org_id = $1
                 AND ($2::uuid IS NULL OR client_id = $2)
                 AND ($3::text IS NULL OR scope = $3)
                 AND ($4::uuid IS NULL OR layout_id = $4)
                 AND ($5::text IS NULL OR
                      tag ILIKE '%' || $5 || '%' OR name ILIKE '%' || $5 || '%' OR
                      fields::text ILIKE '%' || $5 || '%')
                 {visibility_clause}
               ORDER BY tag, name"#
        );
        let mut query = sqlx::query(&sql)
            .bind(org_id)
            .bind(client_id)
            .bind(scope)
            .bind(layout_id)
            .bind(q.map(|s| s.trim()).filter(|s| !s.is_empty()));
        if let Some(visibility) = visibility {
            query = query
                .bind(visibility.user_id.to_string())
                .bind(visibility.is_admin);
        }
        query.fetch_all(executor).await?
    };
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
/// layout's schema by the API layer (outside the org tx). `scope`
/// (migration 0103) is 'client' or 'internal_org'; internal-org rows
/// carry `client_id` NULL (the DB CHECK ties the pair together).
///
/// Ownership is left to the column default (`owned`); callers that already
/// know the final ownership must use [`create_asset_with_ownership`], because
/// the custody class is derived at insert and a post-insert patch would be a
/// silent class change.
#[allow(clippy::too_many_arguments)]
pub async fn create_asset<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    client_id: Option<Uuid>,
    scope: &str,
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
    create_asset_with_ownership(
        executor, org_id, id, client_id, scope, site_id, tag, name, kind, status, layout_id,
        fields, None,
    )
    .await
}

/// Creates an asset with the ownership the row is actually bought under. The
/// migration 0232 insert trigger derives `custody_class` from `scope` and
/// `ownership`, so writing the final ownership in this statement is what makes
/// a rented-in asset land in `rental_in` instead of the owned default. There is
/// no second unit and no post-insert class update.
#[allow(clippy::too_many_arguments)]
pub async fn create_asset_with_ownership<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    client_id: Option<Uuid>,
    scope: &str,
    site_id: Option<Uuid>,
    tag: &str,
    name: &str,
    kind: &str,
    status: &str,
    layout_id: Option<Uuid>,
    fields: &serde_json::Value,
    ownership: Option<&str>,
) -> DbResult<DeviceAsset>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        r#"INSERT INTO device_assets
             (id, org_id, client_id, scope, site_id, tag, name, kind, status, layout_id, fields,
              ownership)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, COALESCE($12, 'owned'))
           RETURNING {ASSET_COLUMNS}"#
    ))
    .bind(id)
    .bind(org_id)
    .bind(client_id)
    .bind(scope)
    .bind(site_id)
    .bind(tag)
    .bind(name)
    .bind(kind)
    .bind(status)
    .bind(layout_id)
    .bind(fields)
    .bind(ownership)
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

pub async fn delete_asset(conn: &mut sqlx::PgConnection, org_id: Uuid, id: Uuid) -> DbResult<()> {
    let r = sqlx::query("DELETE FROM device_assets WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    // The edge names the asset by kind and id, so nothing cascades it.
    crate::knowledge_home::delete_links_for(conn, org_id, "asset", id).await
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
    /// FX snapshot (ADR 0002, inventory receipt lock moment) — computed by
    /// the API layer when the purchase is written in a non-base currency.
    pub purchase_fx_rate_used: Option<String>,
    pub purchase_fx_rate_date: Option<chrono::NaiveDate>,
    pub purchase_fx_source: Option<String>,
    pub base_purchase_cost_minor: Option<i64>,
    pub depreciation_method: Option<String>,
    pub useful_life_months: Option<i32>,
    pub salvage_value_minor: Option<i64>,
    pub depreciation_rate_bps: Option<i32>,
    /// When true, an ownership change that moves the derived custody class
    /// writes a class movement before the row is patched. The assets route sets
    /// this; callers that record the class transition themselves (procurement)
    /// leave it false so the movement is written once, with its own source and
    /// reason. A false here never permits a silent in-place class update: the
    /// column still cannot change except through the movement trigger.
    pub record_class_movement: bool,
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
            && self.purchase_fx_rate_used.is_none()
            && self.purchase_fx_rate_date.is_none()
            && self.purchase_fx_source.is_none()
            && self.base_purchase_cost_minor.is_none()
            && self.depreciation_method.is_none()
            && self.useful_life_months.is_none()
            && self.salvage_value_minor.is_none()
            && self.depreciation_rate_bps.is_none()
    }
}

pub async fn update_asset_lifecycle(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    p: &AssetLifecyclePatch,
) -> DbResult<DeviceAsset> {
    // One lock for the read and the write: two concurrent patches must not both
    // read the same class and each write a movement from it. The lock is also
    // what lets the ownership write and the class movement run in the order the
    // `rental_in` shape constraint needs.
    let current = sqlx::query(
        "SELECT scope, ownership, custody_class FROM device_assets \
          WHERE org_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    let scope: String = current.try_get("scope")?;
    let ownership: String = current.try_get("ownership")?;
    let custody_class: String = current.try_get("custody_class")?;

    // An ownership change moves the derived class. That is a movement row, not
    // a column edit: the 0232 trigger applies it and its open-agreement guard
    // refuses a unit that is out.
    let next_class = match p.ownership.as_deref() {
        Some(next_ownership) if p.record_class_movement && next_ownership != ownership => {
            let from = core_domain::entities::CustodyClass::parse(&custody_class)
                .map_err(|e| DbError::Corrupt(e.to_string()))?;
            if from.is_terminal() {
                return Err(DbError::Conflict(format!(
                    "an asset in {} does not change custody class",
                    from.as_str()
                )));
            }
            let to = core_domain::entities::custody_class_for_asset(&scope, next_ownership)
                .map_err(|e| DbError::Conflict(e.to_string()))?;
            (to != from).then_some((from, to, ownership.clone(), next_ownership.to_string()))
        }
        _ => None,
    };

    // The intermediate row has to satisfy `device_assets_rented_in_shape` too,
    // and that constraint ties `rental_in` to a non-owned ownership. So the two
    // writes are ordered by direction:
    //
    //   * into `rental_in`: ownership first (a fleet row owned by us may become
    //     `rented` while it is still fleet), then the movement that makes it a
    //     vendor rental;
    //   * out of `rental_in`: the movement first (the class leaves `rental_in`,
    //     so the still-`rented` ownership is no longer constrained), then the
    //     ownership.
    //
    // One movement either way; the open-agreement refusal is the trigger's, and
    // it still sees the class it must.
    if let Some((from, to, from_ownership, next_ownership)) = &next_class {
        let into_rental_in = *to == core_domain::entities::CustodyClass::RentalIn;
        if into_rental_in {
            apply_lifecycle_patch(conn, org_id, id, p).await?;
        }
        sqlx::query(
            r#"INSERT INTO inventory_class_movements
                 (id, org_id, unit_kind, device_asset_id, from_class, to_class, reason,
                  source_kind, source_id)
               VALUES ($1, $2, 'asset', $3, $4, $5, $6, 'device_asset', $3)"#,
        )
        .bind(Uuid::now_v7())
        .bind(org_id)
        .bind(id)
        .bind(from.as_str())
        .bind(to.as_str())
        .bind(format!(
            "ownership changed from {from_ownership} to {next_ownership}"
        ))
        .execute(&mut *conn)
        .await
        .map_err(map_class_guard)?;
        if into_rental_in {
            // The patch already ran; the movement is what changed the class,
            // so the final row is a read, not a second write.
            return get_asset(conn, org_id, id).await;
        }
    }

    apply_lifecycle_patch(conn, org_id, id, p).await
}

/// The lifecycle columns, patch semantics: absent = keep. Split out of
/// [`update_asset_lifecycle`] because the `rental_in` shape constraint makes the
/// order of this write and the class movement matter.
async fn apply_lifecycle_patch(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    p: &AssetLifecyclePatch,
) -> DbResult<DeviceAsset> {
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
             purchase_fx_rate_used = COALESCE($13::numeric, purchase_fx_rate_used),
             purchase_fx_rate_date = COALESCE($14, purchase_fx_rate_date),
             purchase_fx_source = COALESCE($15::text, purchase_fx_source),
             base_purchase_cost_minor = COALESCE($16, base_purchase_cost_minor),
             depreciation_method = COALESCE($17, depreciation_method),
             useful_life_months = COALESCE($18, useful_life_months),
             salvage_value_minor = COALESCE($19, salvage_value_minor),
             depreciation_rate_bps = COALESCE($20, depreciation_rate_bps),
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
    .bind(p.purchase_fx_rate_used.clone())
    .bind(p.purchase_fx_rate_date)
    .bind(p.purchase_fx_source.clone())
    .bind(p.base_purchase_cost_minor)
    .bind(p.depreciation_method.as_deref())
    .bind(p.useful_life_months)
    .bind(p.salvage_value_minor)
    .bind(p.depreciation_rate_bps)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row_to_asset(row))
}

/// The class-movement trigger raises `P0001` with a sentence naming the unit
/// and the reason it cannot move (an open loan, a checkout, a ticket). The
/// asset is the caller's own, so the sentence is safe to hand back as a typed
/// 409 instead of a generic database failure.
fn map_class_guard(e: sqlx::Error) -> DbError {
    if let sqlx::Error::Database(db) = &e {
        if db.code().as_deref() == Some("P0001") {
            return DbError::Conflict(db.message().to_string());
        }
    }
    DbError::Sql(e)
}
