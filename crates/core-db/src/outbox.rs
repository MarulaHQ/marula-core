//! Transactional outbox access. `insert` runs inside the caller's mutation
//! transaction; the worker polls unpublished rows and marks them relayed.

use crate::DbResult;
use serde_json::Value;
use uuid::Uuid;

pub struct OutboxRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub event_type: String,
    pub entity: String,
    pub entity_id: Option<String>,
    pub payload: Value,
    pub payload_schema_ref: String,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
}

/// Called within the same transaction as the mutation it describes.
pub async fn insert(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    org_id: Uuid,
    event_type: &str,
    entity: &str,
    entity_id: Option<&str>,
    payload: &Value,
) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO outbox_events (id, org_id, event_type, entity, entity_id, payload)
           VALUES ($1, $2, $3, $4, $5, $6)"#,
        id,
        org_id,
        event_type,
        entity,
        entity_id,
        payload
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Fetches up to `limit` unpublished events in occurrence order.
/// `FOR UPDATE SKIP LOCKED` lets multiple workers relay safely — call this
/// inside an explicit transaction and hold it until `mark_published`.
pub async fn fetch_unpublished<'e, E>(executor: E, limit: i64) -> DbResult<Vec<OutboxRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, event_type, entity, entity_id, payload, payload_schema_ref, occurred_at
           FROM outbox_events
           WHERE published_at IS NULL
           ORDER BY occurred_at
           LIMIT $1
           FOR UPDATE SKIP LOCKED"#,
        limit
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| OutboxRow {
            id: row.id,
            org_id: row.org_id,
            event_type: row.event_type,
            entity: row.entity,
            entity_id: row.entity_id,
            payload: row.payload,
            payload_schema_ref: row.payload_schema_ref,
            occurred_at: row.occurred_at,
        })
        .collect())
}

pub async fn mark_published<'e, E>(executor: E, ids: &[Uuid]) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    if ids.is_empty() {
        return Ok(());
    }
    sqlx::query!(
        r#"UPDATE outbox_events SET published_at = now() WHERE id = ANY($1)"#,
        ids
    )
    .execute(executor)
    .await?;
    Ok(())
}

#[allow(dead_code)]
fn _assert_send() {
    fn is_send<T: Send>() {}
    is_send::<OutboxRow>();
}
