//! Decision-edge persistence (ADR 0010 §2, §5; Wave 23 lane 2,
//! migration 0090).
//!
//! Purpose: org-owned per-surface decision configs (fork-on-first-edit
//! over the versioned starter question sets), starter-set reads, and
//! the append-only `decision_answers` shadow/audit store. Query style
//! follows `ai_policy::` — plain string queries, independent of the
//! offline query-cache state.
//!
//! Non-purpose: NO admission policy or dispatch (core-api owns the
//! edge), NO config validation (core-decision owns the class rules —
//! callers must validate before persisting), no HTTP.

use crate::DbResult;
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Starter question sets (versioned seed data)
// ---------------------------------------------------------------------------

/// The stored `schema` jsonb for one starter version, or None when the
/// (surface, version) pair does not exist.
pub async fn get_question_set(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    surface_key: &str,
    version: i32,
) -> DbResult<Option<Value>> {
    let row: Option<(Value,)> = sqlx::query_as(
        r#"SELECT schema FROM ai_decision_question_sets
           WHERE surface_key = $1 AND version = $2"#,
    )
    .bind(surface_key)
    .bind(version)
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|(schema,)| schema))
}

// ---------------------------------------------------------------------------
// Org per-surface configs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct DecisionSurfaceConfig {
    pub org_id: Uuid,
    pub surface_key: String,
    pub starter_version: i32,
    pub customized: bool,
    /// The forked config (None while the row still references the
    /// starter set — ADR 0010 §2 fork-on-first-edit).
    pub config: Option<Value>,
    pub mode: String,
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub enabled: bool,
}

pub async fn get_surface_config(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    surface_key: &str,
) -> DbResult<Option<DecisionSurfaceConfig>> {
    type ConfigRow = (
        i32,
        bool,
        Option<Value>,
        String,
        Option<String>,
        Option<String>,
        bool,
    );
    let row: Option<ConfigRow> = sqlx::query_as(
        r#"SELECT starter_version, customized, config, mode,
                      model_provider, model_id, enabled
               FROM ai_decision_surface_configs
               WHERE org_id = $1 AND surface_key = $2"#,
    )
    .bind(org_id)
    .bind(surface_key)
    .fetch_optional(executor)
    .await?;
    Ok(row.map(
        |(starter_version, customized, config, mode, model_provider, model_id, enabled)| {
            DecisionSurfaceConfig {
                org_id,
                surface_key: surface_key.to_string(),
                starter_version,
                customized,
                config,
                mode,
                model_provider,
                model_id,
                enabled,
            }
        },
    ))
}

/// Insert or update the org's config row for a surface. A customized
/// write pins the fork (`customized = true`, full `config`); a starter
/// reference write passes `customized = false, config = NULL`.
pub async fn upsert_surface_config(
    tx: &mut PgConnection,
    cfg: &DecisionSurfaceConfig,
    updated_by: Uuid,
) -> DbResult<()> {
    sqlx::query(
        r#"INSERT INTO ai_decision_surface_configs
             (id, org_id, surface_key, starter_version, customized, config,
              mode, model_provider, model_id, enabled, updated_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
           ON CONFLICT (org_id, surface_key) DO UPDATE SET
             starter_version = EXCLUDED.starter_version,
             customized = EXCLUDED.customized,
             config = EXCLUDED.config,
             mode = EXCLUDED.mode,
             model_provider = EXCLUDED.model_provider,
             model_id = EXCLUDED.model_id,
             enabled = EXCLUDED.enabled,
             updated_by = EXCLUDED.updated_by,
             updated_at = now()"#,
    )
    .bind(Uuid::now_v7())
    .bind(cfg.org_id)
    .bind(&cfg.surface_key)
    .bind(cfg.starter_version)
    .bind(cfg.customized)
    .bind(&cfg.config)
    .bind(&cfg.mode)
    .bind(&cfg.model_provider)
    .bind(&cfg.model_id)
    .bind(cfg.enabled)
    .bind(updated_by)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// The decision-answers shadow/audit store
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct DecisionAnswersNew {
    pub id: Uuid,
    pub org_id: Uuid,
    pub surface: String,
    pub entity_type: String,
    pub entity_id: Option<Uuid>,
    pub mode: String,
    pub provider: String,
    pub model: String,
    /// Per-question answers with FULL normalized distributions (no
    /// prose — the class produces no rationales).
    pub answers: Value,
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub request_id: Option<String>,
    pub created_by: Option<Uuid>,
}

/// Append one audit row. The table is append-only at the database level
/// (0090 trigger); callers record shadow AND live runs alike.
pub async fn record_decision_answers(
    tx: &mut PgConnection,
    row: &DecisionAnswersNew,
) -> DbResult<()> {
    sqlx::query(
        r#"INSERT INTO decision_answers
             (id, org_id, surface, entity_type, entity_id, mode,
              provider, model, answers, prompt_tokens, completion_tokens,
              request_id, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)"#,
    )
    .bind(row.id)
    .bind(row.org_id)
    .bind(&row.surface)
    .bind(&row.entity_type)
    .bind(row.entity_id)
    .bind(&row.mode)
    .bind(&row.provider)
    .bind(&row.model)
    .bind(&row.answers)
    .bind(row.prompt_tokens)
    .bind(row.completion_tokens)
    .bind(&row.request_id)
    .bind(row.created_by)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Read model for consumers (Wave 23 lane 3): the newest recorded answers
/// for one entity on one surface. Suggestion storage rides the append-only
/// audit store itself — the newest row per (surface, entity) IS the live
/// suggestion; mode/thresholds classify act-vs-pending at read time.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct DecisionAnswersRow {
    pub id: Uuid,
    pub surface: String,
    pub entity_type: String,
    pub entity_id: Option<Uuid>,
    pub mode: String,
    pub provider: String,
    pub model: String,
    pub answers: Value,
    pub request_id: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Newest answers row for one entity on one surface (plain-string style
/// matching this module; org RLS applies via the caller's tx/pool GUC).
pub async fn latest_answers_for_entity<'e, E>(
    executor: E,
    org_id: Uuid,
    surface: &str,
    entity_type: &str,
    entity_id: Uuid,
) -> DbResult<Option<DecisionAnswersRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query_as::<_, DecisionAnswersRow>(
        r#"SELECT id, surface, entity_type, entity_id, mode, provider, model,
                  answers, request_id, created_at
             FROM decision_answers
            WHERE org_id = $1 AND surface = $2 AND entity_type = $3 AND entity_id = $4
            ORDER BY created_at DESC
            LIMIT 1"#,
    )
    .bind(org_id)
    .bind(surface)
    .bind(entity_type)
    .bind(entity_id)
    .fetch_optional(executor)
    .await
    .map_err(Into::into)
}

/// Newest answers row per surface for one ticket — the workbench
/// suggestion card's read (triage/workbench/dispatch fans).
pub async fn latest_answers_for_ticket<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<Vec<DecisionAnswersRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query_as::<_, DecisionAnswersRow>(
        r#"SELECT DISTINCT ON (surface)
                  id, surface, entity_type, entity_id, mode, provider, model,
                  answers, request_id, created_at
             FROM decision_answers
            WHERE org_id = $1 AND entity_type = 'ticket' AND entity_id = $2
            ORDER BY surface, created_at DESC"#,
    )
    .bind(org_id)
    .bind(ticket_id)
    .fetch_all(executor)
    .await
    .map_err(Into::into)
}
