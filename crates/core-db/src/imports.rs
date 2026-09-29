//! Import framework persistence (TASKLIST D2): import-batch bookkeeping and
//! the external-id → internal-uuid mappings that make pushes idempotent.
//!
//! Purpose: durable state for the pure engine in `core_domain::import_core`.
//! Batches are start/finish spans with cumulative counters; `external_ids`
//! carries one stable internal uuid per (org, entity_kind, external_key) —
//! re-pushing a batch resolves through here and can never duplicate rows.
//! The D2 remainder adds the async giant-batch bookkeeping (`staged_chunks`
//! counts deterministic ObjectStore chunk keys; `queued_at` marks a batch
//! handed to the worker, once) and the saved field-mapping templates per
//! source system.
//!
//! Non-purpose: no chunk PAYLOADS here (staged rows live in the ObjectStore
//! under `org/{org}/import-batches/{batch}/chunk-NNNNN`); no conflict-policy
//! logic here — decisions come from core-domain, this module only records
//! their effects and bumps counters transactionally.

use crate::DbResult;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::Serialize;
use sqlx::Postgres;
use uuid::Uuid;

/// One import run: dry-run probe, synchronous batch, or async staged batch.
#[derive(Debug, Clone)]
pub struct ImportBatch {
    pub id: Uuid,
    pub org_id: Uuid,
    pub label: String,
    /// Free-form operator note about the source system, if provided.
    pub source_note: Option<String>,
    /// client | site | contact | board | ticket | time_entry (0033).
    pub entity_kind: String,
    /// "skip" | "update" | "fail".
    pub conflict_policy: String,
    pub dry_run: bool,
    pub rows_total: i32,
    pub rows_created: i32,
    pub rows_updated: i32,
    pub rows_skipped: i32,
    pub rows_failed: i32,
    /// Driving identity — NO foreign key by design: machine drivers are
    /// api-key ids that never exist in `users` (see 0027 comment block).
    pub created_by: Uuid,
    /// "user" | "api_key".
    pub created_by_kind: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// Number of staged chunk objects uploaded for an async batch; chunk
    /// keys are the deterministic `chunk-00000`..`chunk-{n-1}` sequence.
    pub staged_chunks: i32,
    /// Set exactly once when the batch is enqueued for the worker; a
    /// queued or finished batch accepts no further staging.
    pub queued_at: Option<DateTime<Utc>>,
}

const BATCH_COLS: &str = "id, org_id, label, source_note, entity_kind, conflict_policy, dry_run, \
     rows_total, rows_created, rows_updated, rows_skipped, rows_failed, \
     created_by, created_by_kind, started_at, finished_at, staged_chunks, queued_at";

pub async fn insert_batch<'e, E>(executor: E, batch: &ImportBatch) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO import_batches (id, org_id, label, source_note, entity_kind,
                                       conflict_policy, dry_run, created_by, created_by_kind)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#,
        batch.id,
        batch.org_id,
        batch.label,
        batch.source_note,
        batch.entity_kind,
        batch.conflict_policy,
        batch.dry_run,
        batch.created_by,
        batch.created_by_kind,
    )
    .execute(executor)
    .await?;
    Ok(())
}

fn batch_from_row(row: sqlx::postgres::PgRow) -> ImportBatch {
    use sqlx::Row;
    ImportBatch {
        id: row.get("id"),
        org_id: row.get("org_id"),
        label: row.get("label"),
        source_note: row.get("source_note"),
        entity_kind: row.get("entity_kind"),
        conflict_policy: row.get("conflict_policy"),
        dry_run: row.get("dry_run"),
        rows_total: row.get("rows_total"),
        rows_created: row.get("rows_created"),
        rows_updated: row.get("rows_updated"),
        rows_skipped: row.get("rows_skipped"),
        rows_failed: row.get("rows_failed"),
        created_by: row.get("created_by"),
        created_by_kind: row.get("created_by_kind"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        staged_chunks: row.get("staged_chunks"),
        queued_at: row.get("queued_at"),
    }
}

pub async fn get_batch<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<ImportBatch>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {BATCH_COLS} FROM import_batches WHERE org_id = $1 AND id = $2");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_one(executor)
        .await?;
    Ok(batch_from_row(row))
}

pub async fn list_batches<'e, E>(
    executor: E,
    org_id: Uuid,
    limit: i64,
) -> DbResult<Vec<ImportBatch>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {BATCH_COLS} FROM import_batches WHERE org_id = $1 ORDER BY started_at DESC LIMIT $2"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(limit.clamp(1, 200))
        .fetch_all(executor)
        .await?;
    Ok(rows.into_iter().map(batch_from_row).collect())
}

/// Adds one chunk's outcomes to the running counters. Returns false when the
/// batch is unknown OR already finished — late chunks after finish are a
/// caller-visible conflict, not silent counter drift.
#[allow(clippy::too_many_arguments)]
pub async fn add_chunk_counters<'e, E>(
    executor: E,
    org_id: Uuid,
    batch_id: Uuid,
    total: i32,
    created: i32,
    updated: i32,
    skipped: i32,
    failed: i32,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        r#"UPDATE import_batches
           SET rows_total = rows_total + $3,
               rows_created = rows_created + $4,
               rows_updated = rows_updated + $5,
               rows_skipped = rows_skipped + $6,
               rows_failed = rows_failed + $7
           WHERE org_id = $1 AND id = $2 AND finished_at IS NULL"#,
        org_id,
        batch_id,
        total,
        created,
        updated,
        skipped,
        failed,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Closes the run. Fails with NotFound when missing or double-finished.
pub async fn mark_batch_finished<'e, E>(
    executor: E,
    org_id: Uuid,
    batch_id: Uuid,
) -> DbResult<ImportBatch>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE import_batches SET finished_at = now() \
         WHERE org_id = $1 AND id = $2 AND finished_at IS NULL RETURNING {BATCH_COLS}"
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(batch_id)
        .fetch_one(executor)
        .await
        .map_err(|_| crate::DbError::NotFound)?;
    Ok(batch_from_row(row))
}

// --- async giant batches (D2 remainder) -----------------------------------------

/// Deterministic ObjectStore key for one staged chunk. Staging is append-
/// only and sequential: chunk N is uploaded only after N-1, and the count
/// in `import_batches.staged_chunks` is the runbook for the worker.
#[must_use]
pub fn staged_chunk_key(org_id: Uuid, batch_id: Uuid, index: i32) -> String {
    format!("org/{org_id}/import-batches/{batch_id}/chunk-{index:05}")
}

/// Appends one staged chunk to the batch's count and returns the NEW count
/// (the index just claimed). `None` when the batch is unknown, finished, or
/// already queued — staging closes the moment a batch is enqueued.
pub async fn bump_staged_chunks<'e, E>(
    executor: E,
    org_id: Uuid,
    batch_id: Uuid,
) -> DbResult<Option<i32>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row: Option<(i32,)> = sqlx::query_as(
        "UPDATE import_batches SET staged_chunks = staged_chunks + 1 \
         WHERE org_id = $1 AND id = $2 AND finished_at IS NULL AND queued_at IS NULL \
         RETURNING staged_chunks",
    )
    .bind(org_id)
    .bind(batch_id)
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|(count,)| count))
}

/// Marks the batch as handed to the worker. Exactly-once: a second enqueue
/// (or a stage racing the enqueue) finds `queued_at` already set and gets
/// `false`.
pub async fn mark_batch_queued<'e, E>(executor: E, org_id: Uuid, batch_id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        "UPDATE import_batches SET queued_at = now() \
         WHERE org_id = $1 AND id = $2 AND finished_at IS NULL AND queued_at IS NULL",
        org_id,
        batch_id,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

// --- saved field-mapping templates (D2 remainder) ---------------------------------

/// One saved source→canonical column mapping, keyed per source system so
/// operators reuse the same template for every run of the same exporter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportMappingTemplate {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub source_system: String,
    pub entity_kind: String,
    /// JSON object: source column → canonical Marula column.
    pub mapping: serde_json::Value,
    pub created_by: Uuid,
    pub created_by_kind: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const TEMPLATE_COLS: &str = "id, org_id, name, source_system, entity_kind, mapping, \
     created_by, created_by_kind, created_at, updated_at";

fn template_from_row(row: sqlx::postgres::PgRow) -> ImportMappingTemplate {
    use sqlx::Row;
    ImportMappingTemplate {
        id: row.get("id"),
        org_id: row.get("org_id"),
        name: row.get("name"),
        source_system: row.get("source_system"),
        entity_kind: row.get("entity_kind"),
        mapping: row.get("mapping"),
        created_by: row.get("created_by"),
        created_by_kind: row.get("created_by_kind"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_template<'e, E>(executor: E, template: &ImportMappingTemplate) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO import_mapping_templates
             (id, org_id, name, source_system, entity_kind, mapping, created_by, created_by_kind)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        template.id,
        template.org_id,
        template.name,
        template.source_system,
        template.entity_kind,
        template.mapping,
        template.created_by,
        template.created_by_kind,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_template<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<ImportMappingTemplate>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {TEMPLATE_COLS} FROM import_mapping_templates WHERE org_id = $1 AND id = $2"
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_one(executor)
        .await?;
    Ok(template_from_row(row))
}

/// Lists templates newest-first, optionally narrowed to one source system.
pub async fn list_templates<'e, E>(
    executor: E,
    org_id: Uuid,
    source_system: Option<&str>,
    limit: i64,
) -> DbResult<Vec<ImportMappingTemplate>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {TEMPLATE_COLS} FROM import_mapping_templates \
         WHERE org_id = $1 AND ($2::text IS NULL OR source_system = $2) \
         ORDER BY created_at DESC, id LIMIT $3"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(source_system)
        .bind(limit.clamp(1, 200))
        .fetch_all(executor)
        .await?;
    Ok(rows.into_iter().map(template_from_row).collect())
}

/// Content update. Name/source-system changes move the uniqueness key, so
/// a rename colliding with a sibling surfaces as a constraint conflict.
pub async fn update_template<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    name: &str,
    source_system: &str,
    mapping: &serde_json::Value,
) -> DbResult<ImportMappingTemplate>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE import_mapping_templates \
         SET name = $3, source_system = $4, mapping = $5, updated_at = now() \
         WHERE org_id = $1 AND id = $2 RETURNING {TEMPLATE_COLS}"
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .bind(name)
        .bind(source_system)
        .bind(mapping)
        .fetch_one(executor)
        .await
        .map_err(|_| crate::DbError::NotFound)?;
    Ok(template_from_row(row))
}

pub async fn delete_template<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        "DELETE FROM import_mapping_templates WHERE org_id = $1 AND id = $2",
        org_id,
        id,
    )
    .execute(executor)
    .await?;
    Ok(())
}

// --- external-id mappings -----------------------------------------------------

/// Resolves an external key to the internal uuid it owns, org-scoped by both
/// parameter and RLS.
pub async fn find_mapping<'e, E>(
    executor: E,
    org_id: Uuid,
    entity_kind: &str,
    external_key: &str,
) -> DbResult<Option<Uuid>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT internal_id FROM external_ids
           WHERE org_id = $1 AND entity_kind = $2 AND external_key = $3"#,
        org_id,
        entity_kind,
        external_key
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|r| r.internal_id))
}

/// Records (or refreshes) the ownership of an external key. The UNIQUE
/// constraint guarantees one internal id per key forever; re-seen keys only
/// move `last_seen_batch` forward.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_mapping<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    entity_kind: &str,
    external_key: &str,
    internal_id: Uuid,
    last_seen_batch: Uuid,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO external_ids (id, org_id, entity_kind, external_key,
                                     internal_id, last_seen_batch)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (org_id, entity_kind, external_key) DO UPDATE
             SET internal_id = EXCLUDED.internal_id,
                 last_seen_batch = EXCLUDED.last_seen_batch,
                 updated_at = now()"#,
        id,
        org_id,
        entity_kind,
        external_key,
        internal_id,
        last_seen_batch
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Batch resolves many external keys of ONE kind at once — a chunk's
/// pre-flight planning does a handful of these instead of thousands of
/// point queries. Returned pairs are exact matches only.
pub async fn find_mappings<'e, E>(
    executor: E,
    org_id: Uuid,
    entity_kind: &str,
    external_keys: &[String],
) -> DbResult<Vec<(String, Uuid)>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    if external_keys.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query!(
        r#"SELECT external_key, internal_id FROM external_ids
           WHERE org_id = $1 AND entity_kind = $2 AND external_key = ANY($3)"#,
        org_id,
        entity_kind,
        external_keys
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.external_key, r.internal_id))
        .collect())
}
