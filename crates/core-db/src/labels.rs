//! Label templates persistence + scan-resolution lookups (Wave 32 lane 5,
//! `p32-labels-scanning`; migration 0105).
//!
//! Two seams live here:
//!
//!   * LABEL TEMPLATES — org-scoped CRUD for typed print recipes. One
//!     active default per (org, purpose) is a partial unique index; the
//!     write path clears the previous default in the same transaction so a
//!     "make default" request never trips the index.
//!   * SCAN LOOKUPS — the deterministic code → entity resolution behind
//!     `POST /api/scan/resolve`. A scanned string is matched, in this
//!     priority order, against: a serial number, a stock item SKU/part
//!     number, a location code, then a device asset tag (all
//!     case-insensitive, all org-scoped). The first kind with a hit wins;
//!     multiple rows of that kind come back as candidates and the caller
//!     must pick — a scan never guesses between two physical items.
//!
//! Every read/write carries an explicit `org_id` predicate on top of the
//! migration's RLS policy (same posture as the rest of the crate).

use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use core_domain::entities::LabelTemplate;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Label templates
// ---------------------------------------------------------------------------

struct TemplateRow {
    id: Uuid,
    org_id: Uuid,
    name: String,
    purpose: String,
    size_preset: String,
    width_tenths_mm: Option<i32>,
    height_tenths_mm: Option<i32>,
    dpi: i32,
    barcode_token: Option<String>,
    include_qr: bool,
    fields: serde_json::Value,
    is_default: bool,
    active: bool,
    created_by: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn template_row(r: TemplateRow) -> LabelTemplate {
    LabelTemplate {
        id: r.id,
        org_id: r.org_id,
        name: r.name,
        purpose: r.purpose,
        size_preset: r.size_preset,
        width_tenths_mm: r.width_tenths_mm,
        height_tenths_mm: r.height_tenths_mm,
        dpi: r.dpi,
        barcode_token: r.barcode_token,
        include_qr: r.include_qr,
        fields: r
            .fields
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        is_default: r.is_default,
        active: r.active,
        created_by: r.created_by,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

fn fields_json(template: &LabelTemplate) -> serde_json::Value {
    serde_json::Value::Array(
        template
            .fields
            .iter()
            .map(|t| serde_json::Value::String(t.clone()))
            .collect(),
    )
}

pub async fn insert_template(
    conn: &mut sqlx::PgConnection,
    template: &LabelTemplate,
) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO label_templates
             (id, org_id, name, purpose, size_preset, width_tenths_mm, height_tenths_mm, dpi,
              barcode_token, include_qr, fields, is_default, active, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)"#,
        template.id,
        template.org_id,
        template.name.trim(),
        template.purpose,
        template.size_preset,
        template.width_tenths_mm,
        template.height_tenths_mm,
        template.dpi,
        template.barcode_token.as_deref(),
        template.include_qr,
        fields_json(template),
        template.is_default,
        template.active,
        template.created_by,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub async fn get_template(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<LabelTemplate> {
    let row = sqlx::query_as!(
        TemplateRow,
        r#"SELECT id, org_id, name, purpose, size_preset, width_tenths_mm, height_tenths_mm, dpi,
                  barcode_token, include_qr, fields, is_default, active, created_by,
                  created_at, updated_at
           FROM label_templates WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(template_row(row))
}

pub async fn list_templates(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    purpose: Option<&str>,
    only_active: bool,
    limit: i64,
) -> DbResult<Vec<LabelTemplate>> {
    let rows = sqlx::query_as!(
        TemplateRow,
        r#"SELECT id, org_id, name, purpose, size_preset, width_tenths_mm, height_tenths_mm, dpi,
                  barcode_token, include_qr, fields, is_default, active, created_by,
                  created_at, updated_at
           FROM label_templates
           WHERE org_id = $1
             AND ($2::text IS NULL OR purpose = $2)
             AND (NOT $3::boolean OR active)
           ORDER BY purpose, is_default DESC, name
           LIMIT $4"#,
        org_id,
        purpose,
        only_active,
        limit
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(template_row).collect())
}

/// Full-row update (the API layer decides which fields change; the DB read
/// gates the transaction so concurrent edits converge on last-write-wins at
/// row granularity, matching every other lane-1..3 entity).
pub async fn update_template(
    conn: &mut sqlx::PgConnection,
    template: &LabelTemplate,
) -> DbResult<()> {
    let rows = sqlx::query!(
        r#"UPDATE label_templates
           SET name = $3, purpose = $4, size_preset = $5, width_tenths_mm = $6,
               height_tenths_mm = $7, dpi = $8, barcode_token = $9, include_qr = $10,
               fields = $11, is_default = $12, active = $13, updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        template.org_id,
        template.id,
        template.name.trim(),
        template.purpose,
        template.size_preset,
        template.width_tenths_mm,
        template.height_tenths_mm,
        template.dpi,
        template.barcode_token.as_deref(),
        template.include_qr,
        fields_json(template),
        template.is_default,
        template.active,
    )
    .execute(&mut *conn)
    .await?;
    if rows.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Clears the default flag on every OTHER active template of a purpose —
/// called in the same transaction as a default write so the partial unique
/// index can never trip.
pub async fn clear_default(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    purpose: &str,
    keep_id: Uuid,
) -> DbResult<()> {
    sqlx::query!(
        r#"UPDATE label_templates SET is_default = false, updated_at = now()
           WHERE org_id = $1 AND purpose = $2 AND id <> $3 AND is_default"#,
        org_id,
        purpose,
        keep_id
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The active default template for a purpose, when one exists. The printer
/// surfaces use this to answer "print this entity" with zero configuration.
pub async fn default_template(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    purpose: &str,
) -> DbResult<Option<LabelTemplate>> {
    let row = sqlx::query_as!(
        TemplateRow,
        r#"SELECT id, org_id, name, purpose, size_preset, width_tenths_mm, height_tenths_mm, dpi,
                  barcode_token, include_qr, fields, is_default, active, created_by,
                  created_at, updated_at
           FROM label_templates
           WHERE org_id = $1 AND purpose = $2 AND is_default AND active"#,
        org_id,
        purpose
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(template_row))
}

// ---------------------------------------------------------------------------
// Scan resolution lookups
// ---------------------------------------------------------------------------

/// A serialized unit hit: the unit + its item's display facts.
pub struct SerialHit {
    pub serial_id: Uuid,
    pub stock_item_id: Uuid,
    pub serial_number: String,
    pub status: String,
    pub location_id: Option<Uuid>,
    pub item_name: String,
    pub item_sku: Option<String>,
    pub item_kind: String,
    pub is_tool: bool,
}

pub async fn find_serials_by_number(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    text: &str,
    limit: i64,
) -> DbResult<Vec<SerialHit>> {
    let rows = sqlx::query_as!(
        SerialHit,
        r#"SELECT s.id AS serial_id, s.stock_item_id, s.serial_number, s.status, s.location_id,
                  i.name AS item_name, i.sku AS item_sku, i.kind AS item_kind, i.is_tool
           FROM stock_serials s
           JOIN stock_items i ON i.id = s.stock_item_id AND i.org_id = s.org_id
           WHERE s.org_id = $1 AND lower(s.serial_number) = lower($2)
           ORDER BY s.serial_number
           LIMIT $3"#,
        org_id,
        text,
        limit
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows)
}

pub async fn get_serial_by_id(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    serial_id: Uuid,
) -> DbResult<Option<SerialHit>> {
    let row = sqlx::query_as!(
        SerialHit,
        r#"SELECT s.id AS serial_id, s.stock_item_id, s.serial_number, s.status, s.location_id,
                  i.name AS item_name, i.sku AS item_sku, i.kind AS item_kind, i.is_tool
           FROM stock_serials s
           JOIN stock_items i ON i.id = s.stock_item_id AND i.org_id = s.org_id
           WHERE s.org_id = $1 AND s.id = $2"#,
        org_id,
        serial_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

/// A stock item hit (matched on SKU or part number).
pub struct ItemHit {
    pub id: Uuid,
    pub name: String,
    pub sku: Option<String>,
    pub part_number: Option<String>,
    pub kind: String,
    pub is_tool: bool,
}

pub async fn find_items_by_code(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    text: &str,
    limit: i64,
) -> DbResult<Vec<ItemHit>> {
    let rows = sqlx::query_as!(
        ItemHit,
        r#"SELECT id, name, sku, part_number, kind, is_tool
           FROM stock_items
           WHERE org_id = $1
             AND (lower(sku) = lower($2) OR lower(part_number) = lower($2))
           ORDER BY name
           LIMIT $3"#,
        org_id,
        text,
        limit
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows)
}

pub async fn get_item_by_id(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<Option<ItemHit>> {
    let row = sqlx::query_as!(
        ItemHit,
        r#"SELECT id, name, sku, part_number, kind, is_tool
           FROM stock_items WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

/// A stock location hit (matched on the location code).
pub struct LocationHit {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub code: Option<String>,
}

pub async fn find_locations_by_code(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    text: &str,
    limit: i64,
) -> DbResult<Vec<LocationHit>> {
    let rows = sqlx::query_as!(
        LocationHit,
        r#"SELECT id, name, kind, code
           FROM stock_locations
           WHERE org_id = $1 AND lower(code) = lower($2)
           ORDER BY name
           LIMIT $3"#,
        org_id,
        text,
        limit
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows)
}

pub async fn get_location_by_id(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<Option<LocationHit>> {
    let row = sqlx::query_as!(
        LocationHit,
        r#"SELECT id, name, kind, code
           FROM stock_locations WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

/// A device asset hit (matched on the human asset tag).
pub struct AssetHit {
    pub id: Uuid,
    pub name: String,
    pub tag: String,
    pub kind: String,
}

pub async fn find_assets_by_tag(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    text: &str,
    limit: i64,
) -> DbResult<Vec<AssetHit>> {
    let rows = sqlx::query_as!(
        AssetHit,
        r#"SELECT id, name, tag, kind
           FROM device_assets
           WHERE org_id = $1 AND lower(tag) = lower($2)
           ORDER BY name
           LIMIT $3"#,
        org_id,
        text,
        limit
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows)
}

pub async fn get_asset_by_id(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<Option<AssetHit>> {
    let row = sqlx::query_as!(
        AssetHit,
        r#"SELECT id, name, tag, kind
           FROM device_assets WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

/// The facts a found sheet prints for a device asset. The asset row alone
/// cannot answer "whose is it, where does it sit, is it under warranty": the
/// client and site names live on their own tables, and the warranty horizon
/// comes from the row's own columns through the same domain rule every other
/// asset read uses (`row_to_asset`), so the two cannot disagree.
pub struct AssetFacts {
    /// NULL for an internal-org asset (ADR 0012: the MSP's own fleet carries
    /// no client) and when the row's client was deleted.
    pub client_name: Option<String>,
    /// The site the asset sits at.
    pub location_name: Option<String>,
    pub warranty: Option<core_domain::entities::client::WarrantyWindow>,
}

impl AssetFacts {
    /// An asset row that vanished between the two reads: every fact unknown.
    fn empty() -> Self {
        Self {
            client_name: None,
            location_name: None,
            warranty: None,
        }
    }
}

/// One row per asset: the client and site names by LEFT JOIN (so an asset
/// with no client or no site still resolves), plus the row's warranty inputs.
pub async fn get_asset_facts(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    asset_id: Uuid,
) -> DbResult<AssetFacts> {
    let row = sqlx::query_as!(
        AssetFactsRow,
        // The `?` suffix is a nullable override, not decoration. Postgres
        // reports `clients.name` as NOT NULL — which it is — and sqlx takes
        // that at face value for a LEFT JOIN, so an asset with no client or
        // no site decodes as a 500 ("unexpected null") instead of NULL.
        r#"SELECT c.name AS "client_name?",
                  s.name AS "location_name?",
                  a.fields,
                  a.purchase_date,
                  a.useful_life_months
           FROM device_assets a
           LEFT JOIN clients c ON c.id = a.client_id AND c.org_id = a.org_id
           LEFT JOIN sites s ON s.id = a.site_id AND s.org_id = a.org_id
           WHERE a.org_id = $1 AND a.id = $2"#,
        org_id,
        asset_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(Into::into).unwrap_or_else(AssetFacts::empty))
}

/// The raw join, mapped to the facts by the domain warranty rule.
struct AssetFactsRow {
    client_name: Option<String>,
    location_name: Option<String>,
    fields: serde_json::Value,
    purchase_date: Option<chrono::NaiveDate>,
    useful_life_months: Option<i32>,
}

impl From<AssetFactsRow> for AssetFacts {
    fn from(row: AssetFactsRow) -> Self {
        Self {
            client_name: row.client_name,
            location_name: row.location_name,
            warranty: core_domain::entities::client::asset_warranty_from_fields(
                &row.fields,
                row.purchase_date,
                row.useful_life_months,
                chrono::Utc::now().date_naive(),
            ),
        }
    }
}
