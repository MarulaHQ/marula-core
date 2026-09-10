//! Document layout queries (docs-platform v1, migration 0053): org-managed
//! document/asset types with custom field schemas. System layouts (the
//! seeded standard MSP set) are editable but never deletable — imports,
//! integrations, and the external-DMS seam rely on their presence.

use crate::{DbError, DbResult};
use core_domain::entities::{DocumentLayout, LayoutField};
use sqlx::Postgres;
use uuid::Uuid;

fn row(r: sqlx::postgres::PgRow) -> DocumentLayout {
    use sqlx::Row;
    DocumentLayout {
        id: r.get("id"),
        org_id: r.get("org_id"),
        slug: r.get("slug"),
        name: r.get("name"),
        description: r.get("description"),
        icon: r.get("icon"),
        fields: serde_json::from_value(r.get::<serde_json::Value, _>("fields")).unwrap_or_default(),
        is_system: r.get("is_system"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

const COLS: &str =
    "id, org_id, slug, name, description, icon, fields, is_system, created_at, updated_at";

pub async fn list_layouts<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<DocumentLayout>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM document_layouts WHERE org_id = $1 ORDER BY is_system DESC, name"
    ))
    .bind(org_id)
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(row).collect())
}

pub async fn get_layout<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<DocumentLayout>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(&format!(
        "SELECT {COLS} FROM document_layouts WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .map(row)
    .ok_or(DbError::NotFound)
}

/// Validates a layout's field schema into typed [`LayoutField`]s; the
/// route layer maps errors to typed validation failures.
pub fn parse_fields(v: &serde_json::Value) -> Result<Vec<LayoutField>, String> {
    serde_json::from_value::<Vec<LayoutField>>(v.clone())
        .map_err(|e| format!("fields must be a schema array: {e}"))
}

#[allow(clippy::too_many_arguments)]
pub async fn create_layout<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    slug: &str,
    name: &str,
    description: &str,
    icon: &str,
    fields: &serde_json::Value,
    is_system: bool,
) -> DbResult<DocumentLayout>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let r = sqlx::query(&format!(
        "INSERT INTO document_layouts (id, org_id, slug, name, description, icon, fields, is_system)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING {COLS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(slug)
    .bind(name)
    .bind(description)
    .bind(icon)
    .bind(fields)
    .bind(is_system)
    .fetch_one(executor)
    .await?;
    Ok(row(r))
}

#[allow(clippy::too_many_arguments)]
pub async fn update_layout<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    name: &str,
    description: &str,
    icon: &str,
    fields: &serde_json::Value,
) -> DbResult<DocumentLayout>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let r = sqlx::query(&format!(
        "UPDATE document_layouts SET name = $3, description = $4, icon = $5, fields = $6, updated_at = now()
         WHERE org_id = $1 AND id = $2
         RETURNING {COLS}"
    ))
    .bind(org_id)
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(icon)
    .bind(fields)
    .fetch_optional(executor)
    .await?
    .map(row)
    .ok_or(DbError::NotFound)?;
    Ok(r)
}

/// Delete is refused for system layouts by the CALLER (typed 409 with the
/// is_system flag); this query simply refuses to touch them too.
pub async fn delete_layout<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let r =
        sqlx::query("DELETE FROM document_layouts WHERE org_id = $1 AND id = $2 AND NOT is_system")
            .bind(org_id)
            .bind(id)
            .execute(executor)
            .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Idempotent standard-layout seed for NEW orgs (registration + demo
/// seed); existing orgs were seeded by migration 0053 itself.
pub async fn seed_default_document_layouts(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
) -> DbResult<()> {
    for (slug, name, description, icon, fields) in standard_layouts() {
        sqlx::query(
            "INSERT INTO document_layouts (id, org_id, slug, name, description, icon, fields, is_system)
             VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $6, true)
             ON CONFLICT (org_id, slug) DO NOTHING",
        )
        .bind(org_id)
        .bind(slug)
        .bind(name)
        .bind(description)
        .bind(icon)
        .bind(serde_json::from_str::<serde_json::Value>(&fields).unwrap_or_default())
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// The single source of truth for the standard MSP layout set, shared
/// with migration 0053 (kept in Rust so the seed fn and any future
/// doc-site listing can't drift apart).
pub fn standard_layouts() -> Vec<(
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    String,
)> {
    vec![
        ("server", "Server", "Physical or virtual server", "server", r#"[
            {"key":"hostname","label":"Hostname","type":"text","required":true,"position":0},
            {"key":"ip_address","label":"IP address","type":"text","position":1},
            {"key":"os","label":"Operating system","type":"text","position":2},
            {"key":"make_model","label":"Make / model","type":"text","position":3},
            {"key":"serial","label":"Serial number","type":"text","position":4},
            {"key":"warranty_end","label":"Warranty end","type":"date","position":5},
            {"key":"eol_date","label":"EOL date","type":"date","position":6},
            {"key":"virtualization_host","label":"Virtualization host","type":"bool","position":7}]"#.into()),
        ("workstation", "Workstation", "Desktop or laptop endpoint", "laptop", r#"[
            {"key":"hostname","label":"Hostname","type":"text","required":true,"position":0},
            {"key":"assigned_user","label":"Assigned user","type":"text","position":1},
            {"key":"os","label":"Operating system","type":"text","position":2},
            {"key":"make_model","label":"Make / model","type":"text","position":3},
            {"key":"serial","label":"Serial number","type":"text","position":4},
            {"key":"warranty_end","label":"Warranty end","type":"date","position":5}]"#.into()),
        ("firewall", "Firewall", "Perimeter firewall / UTM", "shield", r#"[
            {"key":"hostname","label":"Hostname","type":"text","required":true,"position":0},
            {"key":"wan_ip","label":"WAN address","type":"text","position":1},
            {"key":"lan_ip","label":"LAN address","type":"text","position":2},
            {"key":"firmware","label":"Firmware","type":"text","position":3},
            {"key":"support_end","label":"Support end","type":"date","position":4}]"#.into()),
        ("network-device", "Network device", "Switch, AP, or other LAN gear", "network", r#"[
            {"key":"hostname","label":"Hostname","type":"text","required":true,"position":0},
            {"key":"ip_address","label":"IP address","type":"text","position":1},
            {"key":"location","label":"Location","type":"text","position":2},
            {"key":"make_model","label":"Make / model","type":"text","position":3},
            {"key":"ports","label":"Ports","type":"number","position":4}]"#.into()),
        ("application", "Application", "SaaS or line-of-business application", "app-window", r#"[
            {"key":"vendor","label":"Vendor","type":"text","position":0},
            {"key":"url","label":"URL","type":"url","position":1},
            {"key":"license_count","label":"License count","type":"number","position":2},
            {"key":"renewal_date","label":"Renewal date","type":"date","position":3},
            {"key":"admin_contact","label":"Admin contact","type":"email","position":4}]"#.into()),
        ("location", "Location", "Client site or office", "map-pin", r#"[
            {"key":"address","label":"Address","type":"text","required":true,"position":0},
            {"key":"primary_contact","label":"Primary contact","type":"text","position":1},
            {"key":"access_hours","label":"Access hours","type":"text","position":2},
            {"key":"gate_code_ref","label":"Access code (see vault)","type":"text","position":3}]"#.into()),
        ("contact", "Contact record", "Third-party contact (vendor, carrier, landlord)", "user", r#"[
            {"key":"role","label":"Role","type":"text","required":true,"position":0},
            {"key":"phone","label":"Phone","type":"text","position":1},
            {"key":"email","label":"Email","type":"email","position":2},
            {"key":"company","label":"Company","type":"text","position":3}]"#.into()),
        ("sop", "SOP", "Standard operating procedure / runbook", "book", r#"[
            {"key":"applies_to","label":"Applies to","type":"text","position":0},
            {"key":"owner","label":"Owner","type":"text","position":1},
            {"key":"last_reviewed","label":"Last reviewed","type":"date","position":2}]"#.into()),
        ("certificate-domain", "Certificate / domain", "TLS certificate or domain registration", "certificate", r#"[
            {"key":"domain","label":"Domain","type":"text","required":true,"position":0},
            {"key":"registrar","label":"Registrar","type":"text","position":1},
            {"key":"expires","label":"Expires","type":"date","position":2},
            {"key":"auto_renew","label":"Auto-renew","type":"bool","position":3}]"#.into()),
        ("backup-job", "Backup job", "Backup configuration for a system or site", "database", r#"[
            {"key":"target","label":"What is backed up","type":"text","required":true,"position":0},
            {"key":"schedule","label":"Schedule","type":"text","position":1},
            {"key":"retention","label":"Retention","type":"text","position":2},
            {"key":"destination","label":"Destination","type":"text","position":3},
            {"key":"last_tested","label":"Restore last tested","type":"date","position":4}]"#.into()),
    ]
}
