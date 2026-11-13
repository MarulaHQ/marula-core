use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{set_org, DbResult};

pub async fn check_event_types(
    pool: &PgPool,
    org_id: Uuid,
    event_types: &[String],
) -> DbResult<Vec<String>> {
    let mut tx = pool.begin().await?;
    set_org(&mut tx, org_id).await?;
    check_event_types_scoped(&mut tx, org_id, event_types).await
}

/// The admission check on a connection the caller has ALREADY org-scoped.
/// A route handler rides its own `set_org_and_user` transaction, so it calls
/// this rather than `check_event_types`: the second form opens a fresh tx on
/// the bare pool, which works only because it re-scopes itself internally,
/// and a read that has to self-scope is a read the RLS gate cannot see
/// through.
pub async fn check_event_types_scoped(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    event_types: &[String],
) -> DbResult<Vec<String>> {
    if event_types.is_empty() {
        return Ok(Vec::new());
    }

    let ungranted: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT t.type
        FROM UNNEST($1) AS t(type)
        WHERE NOT EXISTS (
            SELECT 1 FROM event_types et
            WHERE et.event_type = t.type AND et.source = 'platform'
        )
        AND NOT EXISTS (
            SELECT 1 FROM event_type_grants eg
            WHERE eg.org_id = $2 AND eg.event_type = t.type
        )
        "#,
    )
    .bind(event_types.to_vec())
    .bind(org_id)
    .fetch_all(&mut *tx)
    .await?;

    Ok(ungranted)
}

/// One row of the global catalogue, as the registry read returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryEntry {
    pub event_type: String,
    pub label: String,
    pub description: Option<String>,
}

/// One of the org's grants, with whatever the catalogue knows about the type.
///
/// A grant is valid for a type the catalogue has never heard of — that is what
/// `event_type_grants` is FOR (migration 0124) — so `label` is absent for a bare
/// string and `in_registry` says which case the caller is looking at. A surface
/// that treated a missing label as an error would hide the grants that matter
/// most: the escape hatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantedEventType {
    pub event_type: String,
    pub label: Option<String>,
    pub description: Option<String>,
    pub in_registry: bool,
    pub granted_at: chrono::DateTime<chrono::Utc>,
}

/// The platform types: the ones Marula itself emits, always subscribable
/// without a grant. Ordered by type so the surface does not jitter between
/// reads.
pub async fn list_platform_event_types<'e, E>(executor: E) -> DbResult<Vec<RegistryEntry>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        r#"
        SELECT event_type, label, description
        FROM event_types
        WHERE source = 'platform'
        ORDER BY event_type
        "#,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(event_type, label, description)| RegistryEntry {
            event_type,
            label,
            description,
        })
        .collect())
}

/// The org's grants, joined to the catalogue by a LEFT JOIN so a grant for a
/// type no install declares still reads back.
/// The `query_as` row shape behind [`list_org_grants_detailed`]. A named alias
/// because the tuple is wider than clippy's complexity budget, and the shape
/// belongs to one statement.
type GrantDetailRow = (
    String,
    Option<String>,
    Option<String>,
    bool,
    chrono::DateTime<chrono::Utc>,
);

pub async fn list_org_grants_detailed<'e, E>(
    executor: E,
    org_id: Uuid,
) -> DbResult<Vec<GrantedEventType>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows: Vec<GrantDetailRow> = sqlx::query_as(
        r#"
        SELECT g.event_type, et.label, et.description, et.event_type IS NOT NULL, g.created_at
        FROM event_type_grants g
        LEFT JOIN event_types et ON et.event_type = g.event_type
        WHERE g.org_id = $1
        ORDER BY g.event_type
        "#,
    )
    .bind(org_id)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(event_type, label, description, in_registry, granted_at)| GrantedEventType {
                event_type,
                label,
                description,
                in_registry,
                granted_at,
            },
        )
        .collect())
}

pub async fn list_event_type_grants(
    tx: &mut Transaction<'_, Postgres>,
    org_id: Uuid,
) -> DbResult<Vec<String>> {
    let rows: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT event_type
        FROM event_type_grants
        WHERE org_id = $1
        ORDER BY event_type
        "#,
    )
    .bind(org_id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows)
}

pub async fn insert_event_type_grant(
    tx: &mut Transaction<'_, Postgres>,
    org_id: Uuid,
    event_type: &str,
    granted_by: Option<Uuid>,
) -> DbResult<()> {
    sqlx::query(
        r#"
        INSERT INTO event_type_grants (org_id, event_type, granted_by)
        VALUES ($1, $2, $3)
        ON CONFLICT (org_id, event_type) DO NOTHING
        "#,
    )
    .bind(org_id)
    .bind(event_type)
    .bind(granted_by)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn delete_event_type_grant(
    tx: &mut Transaction<'_, Postgres>,
    org_id: Uuid,
    event_type: &str,
) -> DbResult<()> {
    sqlx::query(
        r#"
        DELETE FROM event_type_grants
        WHERE org_id = $1 AND event_type = $2
        "#,
    )
    .bind(org_id)
    .bind(event_type)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn is_event_type_known(pool: &PgPool, event_type: &str) -> DbResult<bool> {
    let found: Option<i32> = sqlx::query_scalar(
        r#"
        SELECT 1 FROM event_types WHERE event_type = $1
        "#,
    )
    .bind(event_type)
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

pub async fn find_unknown_types(pool: &PgPool, event_types: &[String]) -> DbResult<Vec<String>> {
    if event_types.is_empty() {
        return Ok(Vec::new());
    }
    let unknown: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT t.type
        FROM UNNEST($1) AS t(type)
        WHERE NOT EXISTS (
            SELECT 1 FROM event_types et WHERE et.event_type = t.type
        )
        "#,
    )
    .bind(event_types.to_vec())
    .fetch_all(pool)
    .await?;
    Ok(unknown)
}

/// The event types the org's plugin installs declare as triggers
/// (`source = 'plugin'`, their installs alive). These belong to the
/// TRIGGER catalogue, not to the node palette: a plugin trigger arrives
/// through lane 1's signed ingest and needs no workflow node at all
/// (Wave 43 lane 13's catalogue decision). Separating them here is what
/// stops a second trigger path being invented.
pub async fn list_org_plugin_event_types<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<String>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT DISTINCT et.event_type
        FROM event_types et
        JOIN plugin_installs i ON i.id = et.plugin_install_id
        WHERE et.source = 'plugin'
          AND i.org_id = $1
          AND i.status = 'enabled'
          AND i.revoked_at IS NULL
        ORDER BY et.event_type
        "#,
    )
    .bind(org_id)
    .fetch_all(executor)
    .await?;
    Ok(rows)
}
