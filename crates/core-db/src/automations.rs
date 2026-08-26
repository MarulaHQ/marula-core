//! Automation rule persistence (`automation_rules`, 0015).
//!
//! Purpose: CRUD for event-triggered rules plus the trigger-indexed lookup
//! the worker's stream consumer uses. Documents stay JSONB here; parsing
//! into typed conditions/actions is core-domain's fallible job so a
//! malformed stored rule can be skipped, never crash evaluation.
//!
//! Non-purpose: no matching logic (core-domain `automations`), no effect
//! application (core-api), no stream consumption (worker).

use crate::{DbError, DbResult};
use serde_json::Value;
use sqlx::Postgres;
use uuid::Uuid;

/// Raw storage row — documents unparsed by design (see module note).
#[derive(Debug, Clone)]
pub struct AutomationRuleRecord {
    pub id: Uuid,
    pub org_id: Uuid,
    pub board_id: Option<Uuid>,
    pub name: String,
    pub trigger_event: String,
    pub conditions: Value,
    pub actions: Value,
    pub enabled: bool,
}

#[allow(clippy::too_many_arguments)]
pub async fn insert<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    board_id: Option<Uuid>,
    name: &str,
    trigger_event: &str,
    conditions: Value,
    actions: Value,
) -> DbResult<AutomationRuleRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO automation_rules
             (id, org_id, board_id, name, trigger_event, conditions, actions)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING id, org_id, board_id, name, trigger_event,
                     conditions, actions, enabled"#,
        id,
        org_id,
        board_id,
        name,
        trigger_event,
        conditions,
        actions,
    )
    .fetch_one(executor)
    .await?;
    Ok(record_from(
        row.id,
        row.org_id,
        row.board_id,
        row.name,
        row.trigger_event,
        row.conditions,
        row.actions,
        row.enabled,
    ))
}

pub async fn list<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<AutomationRuleRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, board_id, name, trigger_event, conditions, actions, enabled
           FROM automation_rules WHERE org_id = $1 ORDER BY name"#,
        org_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            record_from(
                row.id,
                row.org_id,
                row.board_id,
                row.name,
                row.trigger_event,
                row.conditions,
                row.actions,
                row.enabled,
            )
        })
        .collect())
}

pub async fn get<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<AutomationRuleRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, board_id, name, trigger_event, conditions, actions, enabled
           FROM automation_rules WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(record_from(
        row.id,
        row.org_id,
        row.board_id,
        row.name,
        row.trigger_event,
        row.conditions,
        row.actions,
        row.enabled,
    ))
}

/// Enabled rules listening for one event type in one org — the hot path of
/// the stream consumer. The partial index from 0015 serves this.
pub async fn list_enabled_by_trigger<'e, E>(
    executor: E,
    org_id: Uuid,
    trigger_event: &str,
) -> DbResult<Vec<AutomationRuleRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, board_id, name, trigger_event, conditions, actions, enabled
           FROM automation_rules
           WHERE org_id = $1 AND trigger_event = $2 AND enabled"#,
        org_id,
        trigger_event,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            record_from(
                row.id,
                row.org_id,
                row.board_id,
                row.name,
                row.trigger_event,
                row.conditions,
                row.actions,
                row.enabled,
            )
        })
        .collect())
}

pub struct RulePatch {
    pub name: Option<String>,
    pub board_id: Option<Option<Uuid>>,
    pub trigger_event: Option<String>,
    pub conditions: Option<Value>,
    pub actions: Option<Value>,
    pub enabled: Option<bool>,
}

/// COALESCE-style partial update; the double-Option board_id follows the
/// ticket-assignee convention (Some(None) clears the scope).
pub async fn update<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    patch: &RulePatch,
) -> DbResult<AutomationRuleRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"UPDATE automation_rules SET
             name = COALESCE($3, name),
             board_id = CASE WHEN $4::bool THEN $5 ELSE board_id END,
             trigger_event = COALESCE($6, trigger_event),
             conditions = COALESCE($7, conditions),
             actions = COALESCE($8, actions),
             enabled = COALESCE($9, enabled),
             updated_at = now()
           WHERE org_id = $1 AND id = $2
           RETURNING id, org_id, board_id, name, trigger_event,
                     conditions, actions, enabled"#,
        org_id,
        id,
        patch.name,
        patch.board_id.is_some(),
        patch.board_id.clone().flatten(),
        patch.trigger_event,
        patch.conditions,
        patch.actions,
        patch.enabled,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(record_from(
        row.id,
        row.org_id,
        row.board_id,
        row.name,
        row.trigger_event,
        row.conditions,
        row.actions,
        row.enabled,
    ))
}

pub async fn delete<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        "DELETE FROM automation_rules WHERE org_id = $1 AND id = $2",
        org_id,
        id
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn record_from(
    id: Uuid,
    org_id: Uuid,
    board_id: Option<Uuid>,
    name: String,
    trigger_event: String,
    conditions: Value,
    actions: Value,
    enabled: bool,
) -> AutomationRuleRecord {
    AutomationRuleRecord {
        id,
        org_id,
        board_id,
        name,
        trigger_event,
        conditions,
        actions,
        enabled,
    }
}
