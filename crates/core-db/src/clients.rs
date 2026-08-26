//! Client → site → contact hierarchy queries.

use crate::{DbError, DbResult};
use core_domain::entities::{Client, Contact, DeviceAsset, Site};
use sqlx::Postgres;
use uuid::Uuid;

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

pub async fn create_site<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    client_id: Uuid,
    name: &str,
) -> DbResult<Site>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO sites (id, org_id, client_id, name)
           VALUES ($1, $2, $3, $4)
           RETURNING id, org_id, client_id, name, address, city, region, postal_code,
                     country, latitude, longitude, timezone, created_at"#,
        id,
        org_id,
        client_id,
        name
    )
    .fetch_one(executor)
    .await?;
    Ok(Site {
        id: row.id,
        org_id: row.org_id,
        client_id: row.client_id,
        name: row.name,
        address: row.address,
        city: row.city,
        region: row.region,
        postal_code: row.postal_code,
        country: row.country,
        latitude: row.latitude,
        longitude: row.longitude,
        timezone: row.timezone,
        created_at: row.created_at,
    })
}

pub async fn get_site<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Site>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, client_id, name, address, city, region, postal_code,
                  country, latitude, longitude, timezone, created_at
           FROM sites WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(Site {
        id: row.id,
        org_id: row.org_id,
        client_id: row.client_id,
        name: row.name,
        address: row.address,
        city: row.city,
        region: row.region,
        postal_code: row.postal_code,
        country: row.country,
        latitude: row.latitude,
        longitude: row.longitude,
        timezone: row.timezone,
        created_at: row.created_at,
    })
}

pub async fn list_sites<'e, E>(executor: E, org_id: Uuid, client_id: Uuid) -> DbResult<Vec<Site>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, client_id, name, address, city, region, postal_code,
                  country, latitude, longitude, timezone, created_at
           FROM sites WHERE org_id = $1 AND client_id = $2 ORDER BY name"#,
        org_id,
        client_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Site {
            id: row.id,
            org_id: row.org_id,
            client_id: row.client_id,
            name: row.name,
            address: row.address,
            city: row.city,
            region: row.region,
            postal_code: row.postal_code,
            country: row.country,
            latitude: row.latitude,
            longitude: row.longitude,
            timezone: row.timezone,
            created_at: row.created_at,
        })
        .collect())
}

pub async fn create_contact<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    client_id: Uuid,
    name: &str,
    email: Option<&str>,
    phone: Option<&str>,
) -> DbResult<Contact>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO contacts (id, org_id, client_id, name, email, phone)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING id, org_id, client_id, site_id, name, email, phone, title,
                     is_primary, created_at"#,
        id,
        org_id,
        client_id,
        name,
        email,
        phone
    )
    .fetch_one(executor)
    .await?;
    Ok(Contact {
        id: row.id,
        org_id: row.org_id,
        client_id: row.client_id,
        site_id: row.site_id,
        name: row.name,
        email: row.email,
        phone: row.phone,
        title: row.title,
        is_primary: row.is_primary,
        created_at: row.created_at,
    })
}

pub async fn get_contact<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Contact>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, client_id, site_id, name, email, phone, title,
                  is_primary, created_at
           FROM contacts WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(Contact {
        id: row.id,
        org_id: row.org_id,
        client_id: row.client_id,
        site_id: row.site_id,
        name: row.name,
        email: row.email,
        phone: row.phone,
        title: row.title,
        is_primary: row.is_primary,
        created_at: row.created_at,
    })
}

pub async fn list_contacts<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<Vec<Contact>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, client_id, site_id, name, email, phone, title,
                  is_primary, created_at
           FROM contacts WHERE org_id = $1 AND client_id = $2 ORDER BY name"#,
        org_id,
        client_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Contact {
            id: row.id,
            org_id: row.org_id,
            client_id: row.client_id,
            site_id: row.site_id,
            name: row.name,
            email: row.email,
            phone: row.phone,
            title: row.title,
            is_primary: row.is_primary,
            created_at: row.created_at,
        })
        .collect())
}

/// Finds a contact by email across the org's clients — requester
/// resolution for inbound email (email-in/out v1).
pub async fn find_contact_by_email<'e, E>(
    executor: E,
    org_id: Uuid,
    email: &str,
) -> DbResult<Option<Contact>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, client_id, site_id, name, email, phone, title,
                  is_primary, created_at
           FROM contacts WHERE org_id = $1 AND lower(email) = lower($2)
           ORDER BY is_primary DESC, created_at LIMIT 1"#,
        org_id,
        email.trim()
    )
    .fetch_optional(executor)
    .await?;
    Ok(rows.map(|row| Contact {
        id: row.id,
        org_id: row.org_id,
        client_id: row.client_id,
        site_id: row.site_id,
        name: row.name,
        email: row.email,
        phone: row.phone,
        title: row.title,
        is_primary: row.is_primary,
        created_at: row.created_at,
    }))
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
    let row = sqlx::query!(
        r#"INSERT INTO device_assets (id, org_id, client_id, tag, name, kind)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING id, org_id, client_id, site_id, tag, name, kind, status,
                     created_at, updated_at"#,
        id,
        org_id,
        client_id,
        tag,
        name,
        kind
    )
    .fetch_one(executor)
    .await?;
    Ok(DeviceAsset {
        id: row.id,
        org_id: row.org_id,
        client_id: row.client_id,
        site_id: row.site_id,
        tag: row.tag,
        name: row.name,
        kind: row.kind,
        status: row.status,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

pub async fn list_assets<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<Vec<DeviceAsset>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, client_id, site_id, tag, name, kind, status,
                   created_at, updated_at
           FROM device_assets WHERE org_id = $1 AND client_id = $2 ORDER BY tag"#,
        org_id,
        client_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| DeviceAsset {
            id: row.id,
            org_id: row.org_id,
            client_id: row.client_id,
            site_id: row.site_id,
            tag: row.tag,
            name: row.name,
            kind: row.kind,
            status: row.status,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
        .collect())
}
