use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{set_org, DbResult};

pub async fn check_event_types(
    pool: &PgPool,
    org_id: Uuid,
    event_types: &[String],
) -> DbResult<Vec<String>> {
    if event_types.is_empty() {
        return Ok(Vec::new());
    }

    let mut tx = pool.begin().await?;
    set_org(&mut tx, org_id).await?;

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

    tx.commit().await?;
    Ok(ungranted)
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

pub async fn find_unknown_types(
    pool: &PgPool,
    event_types: &[String],
) -> DbResult<Vec<String>> {
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
pub async fn list_org_plugin_event_types<'e, E>(
    executor: E,
    org_id: Uuid,
) -> DbResult<Vec<String>>
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
