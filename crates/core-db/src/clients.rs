//! Client → site → contact hierarchy queries.

use crate::assets::ASSET_COLUMNS;
use crate::{DbError, DbResult};
use core_domain::entities::{Client, Contact, DeviceAsset, Site};
use sqlx::{Postgres, Row};
use uuid::Uuid;

/// Contact read columns — 0101 added the CI5 tiers + merge tombstone.
pub const CONTACT_COLUMNS: &str = "id, org_id, client_id, site_id, name, email, phone, \
     title, is_primary, created_at, observed_fields, curated_fields, field_sources, \
     merged_into_id";

pub async fn create_client<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    name: &str,
    notes: Option<&str>,
) -> DbResult<Client>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO clients (id, org_id, name, notes)
           VALUES ($1, $2, $3, $4)
           RETURNING id, org_id, name, restricted_to, notes, created_at, updated_at"#,
        id,
        org_id,
        name,
        notes
    )
    .fetch_one(executor)
    .await?;
    Ok(Client {
        id: row.id,
        org_id: row.org_id,
        name: row.name,
        restricted_to: serde_json::from_value(row.restricted_to).unwrap_or_default(),
        notes: row.notes,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

pub async fn list_clients<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<Client>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, name, restricted_to, notes, created_at, updated_at
           FROM clients WHERE org_id = $1 ORDER BY name"#,
        org_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Client {
            id: row.id,
            org_id: row.org_id,
            name: row.name,
            restricted_to: serde_json::from_value(row.restricted_to).unwrap_or_default(),
            notes: row.notes,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
        .collect())
}

pub async fn get_client<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Client>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, name, restricted_to, notes, created_at, updated_at
           FROM clients WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(Client {
        id: row.id,
        org_id: row.org_id,
        name: row.name,
        restricted_to: serde_json::from_value(row.restricted_to).unwrap_or_default(),
        notes: row.notes,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// The shared site projection (0228 adds `archived_at`). Kept as a const so
/// every read and write returns a row the same decoder can turn into a `Site`.
const SITE_COLUMNS: &str = "id, org_id, client_id, name, address, city, region, postal_code, \
     country, latitude, longitude, timezone, created_at, archived_at";

fn row_to_site(row: sqlx::postgres::PgRow) -> Site {
    use sqlx::Row;
    Site {
        id: row.get("id"),
        org_id: row.get("org_id"),
        client_id: row.get("client_id"),
        name: row.get("name"),
        address: row.get("address"),
        city: row.get("city"),
        region: row.get("region"),
        postal_code: row.get("postal_code"),
        country: row.get("country"),
        latitude: row.get("latitude"),
        longitude: row.get("longitude"),
        timezone: row.get("timezone"),
        created_at: row.get("created_at"),
        archived_at: row.get("archived_at"),
    }
}

/// Creates a site. The optional geography fields existed on the entity since
/// 0002; the import framework (the private build tracker D2) needed them first-class, so the
/// single canonical insert path now persists them for API callers too.
#[allow(clippy::too_many_arguments)]
pub async fn create_site<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    client_id: Uuid,
    name: &str,
    address: Option<&str>,
    city: Option<&str>,
    region: Option<&str>,
    postal_code: Option<&str>,
    country: Option<&str>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    timezone: Option<&str>,
) -> DbResult<Site>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"INSERT INTO sites (id, org_id, client_id, name, address, city, region,
                              postal_code, country, latitude, longitude, timezone)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
           RETURNING {SITE_COLUMNS}"#
    );
    let row = sqlx::query(&sql)
        .bind(id)
        .bind(org_id)
        .bind(client_id)
        .bind(name)
        .bind(address)
        .bind(city)
        .bind(region)
        .bind(postal_code)
        .bind(country)
        .bind(latitude)
        .bind(longitude)
        .bind(timezone)
        .fetch_one(executor)
        .await?;
    Ok(row_to_site(row))
}

pub async fn get_site<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Site>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {SITE_COLUMNS} FROM sites WHERE org_id = $1 AND id = $2");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_site(row))
}

/// The list statement for one client, so the archive predicate is testable
/// without a database.
fn list_sites_sql(include_archived: bool) -> String {
    let archived_clause = if include_archived {
        ""
    } else {
        "AND archived_at IS NULL"
    };
    format!(
        r#"SELECT {SITE_COLUMNS}
           FROM sites WHERE org_id = $1 AND client_id = $2 {archived_clause}
           ORDER BY name"#
    )
}

/// A client's sites, live ones only unless `include_archived` asks. Archived
/// sites stay readable by id (`get_site`) so links and ticket history resolve.
pub async fn list_sites<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
    include_archived: bool,
) -> DbResult<Vec<Site>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = list_sites_sql(include_archived);
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(client_id)
        .fetch_all(executor)
        .await?;
    Ok(rows.into_iter().map(row_to_site).collect())
}

/// Archives or restores a site. Archived sites stay readable by id so links
/// and ticket history resolve; pickers and lists hide them by default.
pub async fn set_site_archived(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    client_id: Uuid,
    id: Uuid,
    archived: bool,
) -> DbResult<()> {
    let res = sqlx::query(
        r#"UPDATE sites
              SET archived_at = CASE WHEN $4 THEN COALESCE(archived_at, now()) ELSE NULL END
            WHERE org_id = $1 AND client_id = $2 AND id = $3"#,
    )
    .bind(org_id)
    .bind(client_id)
    .bind(id)
    .bind(archived)
    .execute(&mut *conn)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Creates a contact. `site_id`, `title`, and `is_primary` round out the
/// 0002 schema (import framework the private build tracker D2 is the first writer).
#[allow(clippy::too_many_arguments)]
pub async fn create_contact<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    client_id: Uuid,
    site_id: Option<Uuid>,
    name: &str,
    email: Option<&str>,
    phone: Option<&str>,
    title: Option<&str>,
    is_primary: bool,
) -> DbResult<Contact>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(
        r#"INSERT INTO contacts (id, org_id, client_id, site_id, name, email, phone,
                                 title, is_primary)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           RETURNING id, org_id, client_id, site_id, name, email, phone, title,
                      is_primary, created_at,
                      observed_fields, curated_fields, field_sources, merged_into_id"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(client_id)
    .bind(site_id)
    .bind(name)
    .bind(email)
    .bind(phone)
    .bind(title)
    .bind(is_primary)
    .fetch_one(executor)
    .await?;
    Ok(row_to_contact(&row))
}

pub async fn get_contact<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Contact>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        "SELECT {CONTACT_COLUMNS} FROM contacts WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row_to_contact(&row))
}

pub async fn list_contacts<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<Vec<Contact>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(&format!(
        "SELECT {CONTACT_COLUMNS} FROM contacts WHERE org_id = $1 AND client_id = $2 ORDER BY name"
    ))
    .bind(org_id)
    .bind(client_id)
    .fetch_all(executor)
    .await?;
    Ok(rows.iter().map(row_to_contact).collect())
}

/// Finds a contact by email across the org's clients — requester
/// resolution for inbound email (email-in/out v1). Tombstoned (merged)
/// rows are excluded: the email resolves to the canonical contact.
pub async fn find_contact_by_email<'e, E>(
    executor: E,
    org_id: Uuid,
    email: &str,
) -> DbResult<Option<Contact>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        "SELECT {CONTACT_COLUMNS} FROM contacts
         WHERE org_id = $1 AND lower(email) = lower($2) AND merged_into_id IS NULL
         ORDER BY is_primary DESC, created_at LIMIT 1"
    ))
    .bind(org_id)
    .bind(email.trim())
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|r| row_to_contact(&r)))
}

/// The one `Contact` row mapper (0101 added the CI5 tiers + merge
/// tombstone; pre-0101 callers keep their shape via serde defaults).
pub(crate) fn row_to_contact(row: &sqlx::postgres::PgRow) -> Contact {
    Contact {
        id: row.try_get("id").unwrap_or_default(),
        org_id: row.try_get("org_id").unwrap_or_default(),
        client_id: row.try_get("client_id").unwrap_or_default(),
        site_id: row.try_get("site_id").ok(),
        name: row.try_get("name").unwrap_or_default(),
        email: row.try_get("email").ok(),
        phone: row.try_get("phone").ok(),
        title: row.try_get("title").ok(),
        is_primary: row.try_get("is_primary").unwrap_or_default(),
        created_at: row.try_get("created_at").unwrap_or_default(),
        observed_fields: row.try_get("observed_fields").unwrap_or_default(),
        curated_fields: row.try_get("curated_fields").unwrap_or_default(),
        field_sources: row.try_get("field_sources").unwrap_or_default(),
        merged_into_id: row.try_get("merged_into_id").ok(),
    }
}

pub async fn create_asset<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    client_id: Uuid,
    tag: &str,
    name: &str,
    kind: &str,
) -> DbResult<DeviceAsset>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        r#"INSERT INTO device_assets (id, org_id, client_id, tag, name, kind)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING {ASSET_COLUMNS}"#
    ))
    .bind(id)
    .bind(org_id)
    .bind(client_id)
    .bind(tag)
    .bind(name)
    .bind(kind)
    .fetch_one(executor)
    .await?;
    Ok(crate::assets::row_to_asset(row))
}

pub async fn list_assets<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<Vec<DeviceAsset>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(&format!(
        r#"SELECT {ASSET_COLUMNS}
           FROM device_assets WHERE org_id = $1 AND client_id = $2 ORDER BY tag"#
    ))
    .bind(org_id)
    .bind(client_id)
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(crate::assets::row_to_asset).collect())
}

// --- updates (import framework ConflictPolicy::Update path, the private build tracker D2) ---
// Whole-row overwrites driven by canonical import specs; staff-facing edit
// surfaces can adopt them without reshaping. NotFound = foreign org or id.

pub async fn update_client<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    name: &str,
    notes: Option<&str>,
) -> DbResult<Client>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"UPDATE clients SET name = $3, notes = $4, updated_at = now()
           WHERE org_id = $1 AND id = $2
           RETURNING id, org_id, name, restricted_to, notes, created_at, updated_at"#,
        org_id,
        id,
        name,
        notes
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(Client {
        id: row.id,
        org_id: row.org_id,
        name: row.name,
        restricted_to: serde_json::from_value(row.restricted_to).unwrap_or_default(),
        notes: row.notes,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn update_site<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    name: &str,
    address: Option<&str>,
    city: Option<&str>,
    region: Option<&str>,
    postal_code: Option<&str>,
    country: Option<&str>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    timezone: Option<&str>,
) -> DbResult<Site>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"UPDATE sites SET name = $3, address = $4, city = $5, region = $6,
                            postal_code = $7, country = $8, latitude = $9,
                            longitude = $10, timezone = $11
           WHERE org_id = $1 AND id = $2
           RETURNING {SITE_COLUMNS}"#
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .bind(name)
        .bind(address)
        .bind(city)
        .bind(region)
        .bind(postal_code)
        .bind(country)
        .bind(latitude)
        .bind(longitude)
        .bind(timezone)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_site(row))
}

#[allow(clippy::too_many_arguments)]
pub async fn update_contact<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    name: &str,
    email: Option<&str>,
    phone: Option<&str>,
    title: Option<&str>,
    is_primary: bool,
) -> DbResult<Contact>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(
        r#"UPDATE contacts SET name = $3, email = $4, phone = $5, title = $6,
                               is_primary = $7,
                               curated_fields =
                                 (CASE WHEN email IS DISTINCT FROM $4
                                       THEN (CASE WHEN $4 IS NULL
                                                  THEN curated_fields - 'email'
                                                  ELSE curated_fields
                                                        || jsonb_build_object('email', $4)
                                            END)
                                       ELSE curated_fields END)
                                 || (CASE WHEN phone IS DISTINCT FROM $5
                                          THEN (CASE WHEN $5 IS NULL
                                                     THEN curated_fields - 'phone'
                                                     ELSE jsonb_build_object('phone', $5)
                                                END)
                                          ELSE '{}'::jsonb END)
                                 || (CASE WHEN name IS DISTINCT FROM $3
                                          THEN jsonb_build_object('name', $3)
                                          ELSE '{}'::jsonb END)
                                 || (CASE WHEN title IS DISTINCT FROM $6
                                          THEN (CASE WHEN $6 IS NULL
                                                     THEN curated_fields - 'title'
                                                     ELSE jsonb_build_object('title', $6)
                                                END)
                                          ELSE '{}'::jsonb END),
                               updated_at = now()
           WHERE org_id = $1 AND id = $2
           RETURNING id, org_id, client_id, site_id, name, email, phone, title,
                      is_primary, created_at,
                      observed_fields, curated_fields, field_sources, merged_into_id"#,
    )
    .bind(org_id)
    .bind(id)
    .bind(name)
    .bind(email)
    .bind(phone)
    .bind(title)
    .bind(is_primary)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row_to_contact(&row))
}

// --- batch loaders (import framework planning, the private build tracker D2) ------------------
// One query per chunk instead of per-row getters; used to build the pure
// engine's canonical projections and to prove parent rows still exist.

fn split_uuids(ids: &[Uuid]) -> Option<Vec<Uuid>> {
    (!ids.is_empty()).then(|| ids.to_vec())
}

pub async fn get_clients_by_ids<'e, E>(
    executor: E,
    org_id: Uuid,
    ids: &[Uuid],
) -> DbResult<Vec<Client>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let Some(bind) = split_uuids(ids) else {
        return Ok(Vec::new());
    };
    let rows = sqlx::query!(
        r#"SELECT id, org_id, name, restricted_to, notes, created_at, updated_at
           FROM clients WHERE org_id = $1 AND id = ANY($2)"#,
        org_id,
        &bind
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Client {
            id: row.id,
            org_id: row.org_id,
            name: row.name,
            restricted_to: serde_json::from_value(row.restricted_to).unwrap_or_default(),
            notes: row.notes,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
        .collect())
}

pub async fn get_sites_by_ids<'e, E>(executor: E, org_id: Uuid, ids: &[Uuid]) -> DbResult<Vec<Site>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let Some(bind) = split_uuids(ids) else {
        return Ok(Vec::new());
    };
    let sql = format!("SELECT {SITE_COLUMNS} FROM sites WHERE org_id = $1 AND id = ANY($2)");
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(&bind)
        .fetch_all(executor)
        .await?;
    Ok(rows.into_iter().map(row_to_site).collect())
}

pub async fn get_contacts_by_ids<'e, E>(
    executor: E,
    org_id: Uuid,
    ids: &[Uuid],
) -> DbResult<Vec<Contact>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let Some(bind) = split_uuids(ids) else {
        return Ok(Vec::new());
    };
    let rows = sqlx::query(&format!(
        "SELECT {CONTACT_COLUMNS} FROM contacts WHERE org_id = $1 AND id = ANY($2)"
    ))
    .bind(org_id)
    .bind(&bind)
    .fetch_all(executor)
    .await?;
    Ok(rows.iter().map(row_to_contact).collect())
}

#[cfg(test)]
mod archive_tests {
    use super::*;

    #[test]
    fn site_list_hides_archived_unless_asked() {
        assert!(list_sites_sql(false).contains("AND archived_at IS NULL"));
        assert!(!list_sites_sql(true).contains("archived_at IS NULL"));
    }
}
