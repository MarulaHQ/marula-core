//! Undo read-model queries (TASKLIST IF7, Wave 9 lane 5).
//!
//! Reads the per-field / per-record applied-state history from the
//! versioned `ops.v1_record_undo_stack` / `ops.v1_field_undo_stack` views
//! over the 0028 write trail (migration 0041). The trail is the storage of
//! record — append-only, RLS-bound to the caller's org; this module only
//! queries it and never writes (reverts are normal IF1 writes appended by
//! core-api through `instant::record`).

use crate::DbResult;
use serde_json::Value;
use uuid::Uuid;

/// One applied write on a record — a step of the undo stack.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UndoStepRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub entity: String,
    pub entity_id: Uuid,
    pub seq: i64,
    pub op: String,
    pub actor_id: Uuid,
    pub actor_kind: String,
    pub idempotency_key: Option<String>,
    /// Prior entity snapshot; NULL on creates.
    pub before: Option<Value>,
    /// Applied entity snapshot.
    pub after: Value,
    /// The value-restore revert path can step back over this write
    /// (op <> 'create' and a prior snapshot exists).
    pub restorable: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// One field of one applied write — per-field history granularity.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UndoFieldRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub entity: String,
    pub entity_id: Uuid,
    pub seq: i64,
    pub op: String,
    pub field: String,
    /// Value before the write (NULL when absent before).
    pub prior: Option<Value>,
    /// Value after the write (NULL when absent after).
    pub new_value: Option<Value>,
    /// NULL-safe prior-vs-new comparison.
    pub changed: bool,
    pub actor_id: Uuid,
    pub actor_kind: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// One record's applied-write history, newest first.
pub async fn record_stack<'e, E>(
    executor: E,
    org_id: Uuid,
    entity: &str,
    entity_id: Uuid,
    limit: i64,
) -> DbResult<Vec<UndoStepRow>>
where
    E: sqlx::PgExecutor<'e>,
{
    let rows = sqlx::query_as::<_, UndoStepRow>(
        r#"SELECT id, org_id, entity, entity_id, seq, op, actor_id, actor_kind,
                  idempotency_key, before, after, restorable, created_at
           FROM ops.v1_record_undo_stack
           WHERE org_id = $1 AND entity = $2 AND entity_id = $3
           ORDER BY seq DESC
           LIMIT $4"#,
    )
    .bind(org_id)
    .bind(entity)
    .bind(entity_id)
    .bind(limit.clamp(1, 200))
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// One record's per-field history, newest write first.
pub async fn field_stack<'e, E>(
    executor: E,
    org_id: Uuid,
    entity: &str,
    entity_id: Uuid,
    limit: i64,
) -> DbResult<Vec<UndoFieldRow>>
where
    E: sqlx::PgExecutor<'e>,
{
    let rows = sqlx::query_as::<_, UndoFieldRow>(
        r#"SELECT id, org_id, entity, entity_id, seq, op, field, prior, new_value,
                  changed, actor_id, actor_kind, created_at
           FROM ops.v1_field_undo_stack
           WHERE org_id = $1 AND entity = $2 AND entity_id = $3
           ORDER BY seq DESC, field
           LIMIT $4"#,
    )
    .bind(org_id)
    .bind(entity)
    .bind(entity_id)
    .bind(limit.clamp(1, 200))
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// One specific trail write: the revert target lookup.
pub async fn trail_step<'e, E>(
    executor: E,
    org_id: Uuid,
    entity: &str,
    entity_id: Uuid,
    seq: i64,
) -> DbResult<Option<UndoStepRow>>
where
    E: sqlx::PgExecutor<'e>,
{
    let row = sqlx::query_as::<_, UndoStepRow>(
        r#"SELECT id, org_id, entity, entity_id, seq, op, actor_id, actor_kind,
                  idempotency_key, before, after, restorable, created_at
           FROM ops.v1_record_undo_stack
           WHERE org_id = $1 AND entity = $2 AND entity_id = $3 AND seq = $4"#,
    )
    .bind(org_id)
    .bind(entity)
    .bind(entity_id)
    .bind(seq)
    .fetch_optional(executor)
    .await?;
    Ok(row)
}
