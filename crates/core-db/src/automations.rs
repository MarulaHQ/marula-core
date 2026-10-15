//! Automation rule persistence (`automation_rules`, 0015).
//!
//! Purpose: CRUD for event-triggered rules plus the trigger-indexed lookup
//! the worker's stream consumer uses. Documents stay JSONB here; parsing
//! into typed conditions/actions is core-domain's fallible job so a
//! malformed stored rule can be skipped, never crash evaluation.
//!
//! Non-purpose: no matching logic (core-domain `automations`), no effect
//! application (core-api), no stream consumption (worker).
//!
//! This module uses RUNTIME sqlx queries (`sqlx::query`/`query_as`), not the
//! compile-time `query!` macro: Wave 43 lane 3 added the nullable
//! `subject_kind` column to `automation_rules`, and a runtime query needs
//! no offline `.sqlx` regeneration for a schema change (AGENTS.md lane
//! rule).

use crate::{DbError, DbResult};
use core_domain::automations::AutomationSubjectKind;
use serde_json::Value;
use sqlx::Postgres;
use uuid::Uuid;

/// Row shape shared by every read here — the SELECT column order below.
type RuleRowTuple = (
    Uuid,
    Uuid,
    Option<Uuid>,
    Option<String>,
    String,
    String,
    Value,
    Value,
    bool,
);

/// Raw storage row — documents unparsed by design (see module note).
#[derive(Debug, Clone)]
pub struct AutomationRuleRecord {
    pub id: Uuid,
    pub org_id: Uuid,
    pub board_id: Option<Uuid>,
    /// What the rule's subject is; NULL in the column reads as `Ticket`.
    pub subject_kind: AutomationSubjectKind,
    pub name: String,
    pub trigger_event: String,
    pub conditions: Value,
    pub actions: Value,
    pub enabled: bool,
}

const SELECT_COLUMNS: &str = "id, org_id, board_id, subject_kind, name, trigger_event, \
                              conditions, actions, enabled";

fn record_from(row: RuleRowTuple) -> AutomationRuleRecord {
    let (id, org_id, board_id, subject_kind, name, trigger_event, conditions, actions, enabled) =
        row;
    AutomationRuleRecord {
        id,
        org_id,
        board_id,
        subject_kind: AutomationSubjectKind::from_db(subject_kind.as_deref()),
        name,
        trigger_event,
        conditions,
        actions,
        enabled,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn insert<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    board_id: Option<Uuid>,
    subject_kind: AutomationSubjectKind,
    name: &str,
    trigger_event: &str,
    conditions: Value,
    actions: Value,
) -> DbResult<AutomationRuleRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as::<_, RuleRowTuple>(&format!(
        "INSERT INTO automation_rules
              (id, org_id, board_id, subject_kind, name, trigger_event, conditions, actions)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING {SELECT_COLUMNS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(board_id)
    .bind(subject_kind.as_str())
    .bind(name)
    .bind(trigger_event)
    .bind(conditions)
    .bind(actions)
    .fetch_one(executor)
    .await?;
    Ok(record_from(row))
}

pub async fn list<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<AutomationRuleRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as::<_, RuleRowTuple>(&format!(
        "SELECT {SELECT_COLUMNS} FROM automation_rules WHERE org_id = $1 ORDER BY name"
    ))
    .bind(org_id)
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(record_from).collect())
}

pub async fn get<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<AutomationRuleRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as::<_, RuleRowTuple>(&format!(
        "SELECT {SELECT_COLUMNS} FROM automation_rules WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(record_from(row))
}

/// Enabled SIMPLE-tier rules listening for one event type in one org — the
/// hot path of the automations stream consumer. The partial index from 0015
/// serves this.
///
/// `definition IS NULL` is load-bearing (Wave 43 lane 15): a rule carrying a
/// `definition` is a multi-step workflow document the executor runs, and its
/// `actions` are empty. Without this filter the automations consumer would
/// also match it and enqueue a no-op `automation.effects` job; the executor's
/// own match is `definition IS NOT NULL`, so the two consumers partition the
/// table and a document is never run twice.
pub async fn list_enabled_by_trigger<'e, E>(
    executor: E,
    org_id: Uuid,
    trigger_event: &str,
) -> DbResult<Vec<AutomationRuleRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as::<_, RuleRowTuple>(&format!(
        "SELECT {SELECT_COLUMNS} FROM automation_rules
            WHERE org_id = $1 AND trigger_event = $2 AND enabled
              AND definition IS NULL"
    ))
    .bind(org_id)
    .bind(trigger_event)
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(record_from).collect())
}

/// One enabled WORKFLOW document (Wave 43 lane 15): the graph rules the
/// executor matches by trigger. `revision` is lane 14's `rule_version`; the
/// `definition` is the raw document the executor validates with
/// `core_domain::workflow_document::WorkflowDocument::parse`.
#[derive(Debug, Clone)]
pub struct WorkflowRuleRecord {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub revision: i32,
    /// The author. A workflow runs as its author, never as a system user
    /// (spec item B); the executor refuses a document with no author.
    pub created_by: Option<Uuid>,
    pub definition: Value,
}

type WorkflowRuleRowTuple = (Uuid, Uuid, String, i32, Option<Uuid>, Value);

const WORKFLOW_COLUMNS: &str = "id, org_id, name, revision, created_by, definition";

fn workflow_from(row: WorkflowRuleRowTuple) -> WorkflowRuleRecord {
    let (id, org_id, name, revision, created_by, definition) = row;
    WorkflowRuleRecord {
        id,
        org_id,
        name,
        revision,
        created_by,
        definition,
    }
}

/// Enabled workflow documents listening for one event type in one org — the
/// executor's trigger path. Served by `idx_automation_rules_workflow`.
pub async fn list_enabled_workflows_by_trigger<'e, E>(
    executor: E,
    org_id: Uuid,
    trigger_event: &str,
) -> DbResult<Vec<WorkflowRuleRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as::<_, WorkflowRuleRowTuple>(&format!(
        "SELECT {WORKFLOW_COLUMNS} FROM automation_rules
            WHERE org_id = $1 AND trigger_event = $2 AND enabled
              AND definition IS NOT NULL
            ORDER BY name"
    ))
    .bind(org_id)
    .bind(trigger_event)
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(workflow_from).collect())
}

/// One workflow document by id, the document and its revision.
pub async fn get_workflow<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<WorkflowRuleRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as::<_, WorkflowRuleRowTuple>(&format!(
        "SELECT {WORKFLOW_COLUMNS} FROM automation_rules
            WHERE org_id = $1 AND id = $2 AND definition IS NOT NULL"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(workflow_from(row))
}

pub struct RulePatch {
    pub name: Option<String>,
    pub board_id: Option<Option<Uuid>>,
    /// `Some` replaces the subject declaration (Wave 43 lane 3).
    pub subject_kind: Option<AutomationSubjectKind>,
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
    let row = sqlx::query_as::<_, RuleRowTuple>(&format!(
        "UPDATE automation_rules SET
              name = COALESCE($3, name),
              board_id = CASE WHEN $4::bool THEN $5 ELSE board_id END,
              subject_kind = COALESCE($6, subject_kind),
              trigger_event = COALESCE($7, trigger_event),
              conditions = COALESCE($8, conditions),
              actions = COALESCE($9, actions),
              enabled = COALESCE($10, enabled),
              updated_at = now()
            WHERE org_id = $1 AND id = $2
            RETURNING {SELECT_COLUMNS}"
    ))
    .bind(org_id)
    .bind(id)
    .bind(patch.name.as_deref())
    .bind(patch.board_id.is_some())
    .bind(patch.board_id.flatten())
    .bind(patch.subject_kind.map(|kind| kind.as_str()))
    .bind(patch.trigger_event.as_deref())
    .bind(patch.conditions.clone())
    .bind(patch.actions.clone())
    .bind(patch.enabled)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(record_from(row))
}

pub async fn delete<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query("DELETE FROM automation_rules WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(executor)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}
