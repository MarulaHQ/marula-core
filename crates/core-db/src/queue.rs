//! General worker job queue over the `jobs` table (0015).
//!
//! Purpose: durable, retrying, dead-lettering work items for background
//! processors. Claims are atomic single-statement CTEs using
//! `FOR UPDATE SKIP LOCKED` so concurrent workers never share a job, and
//! stale `running` rows (crashed claimants) are re-armed in the same pass.
//! Every timestamp decision is made by Postgres `now()` — host clocks are
//! never consulted for ordering or backoff.
//!
//! Non-purpose: no job semantics here (kinds + handlers live in worker),
//! no event emission, no HTTP.

use crate::{DbError, DbResult};
use serde_json::Value;
use sqlx::Postgres;
use uuid::Uuid;

/// A work item handed to a handler after a successful claim.
#[derive(Debug, Clone)]
pub struct ClaimedJob {
    pub id: Uuid,
    pub org_id: Uuid,
    pub kind: String,
    pub payload: Value,
    /// Attempts INCLUDING this one (first claim ⇒ 1).
    pub attempts: i32,
    pub max_attempts: i32,
    /// Claimant recorded by THIS attempt (lease diagnostics).
    pub claimed_by: String,
}

#[derive(Debug, Clone)]
pub struct NewJob {
    pub org_id: Uuid,
    pub kind: String,
    pub payload: Value,
    pub max_attempts: i32,
}

/// Enqueues one job, due immediately.
pub async fn enqueue<'e, E>(executor: E, job: &NewJob) -> DbResult<Uuid>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    enqueue_after(executor, job, chrono::Utc::now()).await
}

/// Enqueues one job unless a job carrying the same `dedupe_key` exists.
///
/// Why this exists: some producers are at-least-once at the source. The
/// webhooks-out consumer re-reads an event when a pending stream entry is
/// replayed after a crash or claimed from a consumer that died mid-handling;
/// without a guard the second read inserts a second `webhook.deliver` row and
/// the receiver gets the same signed POST twice. `Ok(None)` therefore is not a
/// failure — it means this delivery is already queued, in whatever state, and
/// the caller's work for that key is done.
///
/// The key is text and caller-formatted (`<kind>:<subject>`). Three producers
/// fill it in today: the webhooks consumer (`webhook.deliver:<webhook>:<event>`,
/// `crates/worker/src/webhook_out.rs`), the automations consumer
/// (`automation.effects:<rule>:<event>`) and the workflow consumer
/// (`workflow.resume:<op>:<run-or-rule>:<event>`, `crates/worker/src/workflow.rs`).
/// All three are stream consumers, so all three can read one entry twice. The
/// partial unique index from migration 0142 makes the insert a no-op on a repeat
/// and leaves every job kind that passes no key untouched. The row is never
/// removed by the queue:
/// a settled or dead-lettered job keeps its key, so a late replay of the same
/// event cannot resurrect a delivery the retry policy already finished.
///
/// Runtime query rather than `sqlx::query!` on purpose: the new column has no
/// entry in the committed `.sqlx` cache, and re-preparing it belongs to a lane
/// that can run `cargo sqlx prepare`. The integration test in
/// `crates/worker/tests/webhook_restart_replay.rs` executes this statement
/// against a migrated database, which is what proves it parses and binds.
pub async fn enqueue_deduped<'e, E>(
    executor: E,
    job: &NewJob,
    dedupe_key: &str,
) -> DbResult<Option<Uuid>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let id = Uuid::now_v7();
    let inserted: Option<(Uuid,)> = sqlx::query_as(
        r#"INSERT INTO jobs (id, org_id, kind, payload, max_attempts, run_after, dedupe_key)
           VALUES ($1, $2, $3, $4, $5, now(), $6)
           ON CONFLICT (dedupe_key) WHERE dedupe_key IS NOT NULL DO NOTHING
           RETURNING id"#,
    )
    .bind(id)
    .bind(job.org_id)
    .bind(&job.kind)
    .bind(&job.payload)
    .bind(job.max_attempts)
    .bind(dedupe_key)
    .fetch_optional(executor)
    .await?;
    Ok(inserted.map(|(id,)| id))
}

/// Enqueues one job due at a specific time (the private build tracker Wave 43 lane 14).
///
/// The run ledger's parked-workflow wakes need this: a run parked until a
/// `flow.delay` deadline or a `flow.waitUntil` date is woken by a
/// `workflow.resume` job that must NOT be claimed before its wake-at,
/// which is what the `jobs.run_after` column already models. Enqueuing it
/// due immediately and letting the claim path re-park would poll; forking
/// a second queue for workflow wakes would be a second retry/dead-letter
/// history to keep honest. This is the existing queue, used as designed.
pub async fn enqueue_after<'e, E>(
    executor: E,
    job: &NewJob,
    run_after: chrono::DateTime<chrono::Utc>,
) -> DbResult<Uuid>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let id = Uuid::now_v7();
    sqlx::query!(
        r#"INSERT INTO jobs (id, org_id, kind, payload, max_attempts, run_after)
           VALUES ($1, $2, $3, $4, $5, $6)"#,
        id,
        job.org_id,
        job.kind,
        job.payload,
        job.max_attempts,
        run_after,
    )
    .execute(executor)
    .await?;
    Ok(id)
}

/// Re-arms rows whose claiming worker died mid-flight (`claimed_at` older
/// than `stale_after_secs`). Separate from [`claim_batch`] ON PURPOSE: a
/// data-modifying CTE's writes are invisible to its own statement (single
/// snapshot), so re-arming and claiming must be sequential statements.
pub async fn reclaim_stale<'e, E>(executor: E, stale_after_secs: i64) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"UPDATE jobs
           SET status = 'pending', claimed_at = NULL, claimed_by = NULL,
               updated_at = now()
           WHERE status = 'running'
             AND claimed_at < now() - make_interval(secs => $1)"#,
        stale_after_secs as f64,
    )
    .execute(executor)
    .await?;
    Ok(res.rows_affected())
}

/// Claims up to `batch` pending jobs atomically:
/// `FOR UPDATE SKIP LOCKED` keeps concurrent workers disjoint, and every
/// time decision is Postgres `now()` — host clocks are never consulted.
pub async fn claim_batch<'e, E>(
    executor: E,
    batch: i64,
    worker_name: &str,
) -> DbResult<Vec<ClaimedJob>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    // The pick is a MATERIALIZED CTE on purpose, not `WHERE id IN (SELECT ... LIMIT n)`.
    //
    // Under the RLS role the planner turns that `IN` into a `Nested Loop Semi
    // Join` with the `Limit` on the inner side, so the limit is re-executed per
    // outer row and every candidate finds itself: a batch of 4 claimed 6 of 6
    // in `job_queue_concurrent_claims_are_disjoint` once the plan chose a seq
    // scan. A materialized CTE is an optimization fence, so the pick runs
    // exactly once and `batch` stays a real bound.
    let rows = sqlx::query!(
        r#"WITH picked AS MATERIALIZED (
             SELECT id FROM jobs
             WHERE status = 'pending' AND run_after <= now()
             ORDER BY id
             FOR UPDATE SKIP LOCKED
             LIMIT $2
           )
           UPDATE jobs
           SET status = 'running', claimed_at = now(), claimed_by = $1,
               attempts = attempts + 1, updated_at = now()
           WHERE id IN (SELECT id FROM picked)
           RETURNING id, org_id, kind, payload, attempts, max_attempts, claimed_by"#,
        worker_name,
        batch,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ClaimedJob {
            id: r.id,
            org_id: r.org_id,
            kind: r.kind,
            payload: r.payload,
            attempts: r.attempts,
            max_attempts: r.max_attempts,
            claimed_by: r.claimed_by.unwrap_or_default(),
        })
        .collect())
}

/// Marks a processed job complete.
pub async fn mark_done<'e, E>(executor: E, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"UPDATE jobs SET status = 'done', last_error = NULL,
                  claimed_at = NULL, claimed_by = NULL, updated_at = now()
           WHERE id = $1 AND status = 'running'"#,
        id,
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Puts a failed job back for another attempt after `backoff_secs`
/// (Postgres-side arithmetic — skew between workers cannot reorder it).
pub async fn schedule_retry<'e, E>(
    executor: E,
    id: Uuid,
    backoff_secs: i64,
    error: &str,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"UPDATE jobs SET status = 'pending',
                  run_after = now() + make_interval(secs => $2),
                  last_error = $3, claimed_at = NULL, claimed_by = NULL,
                  updated_at = now()
           WHERE id = $1 AND status = 'running'"#,
        id,
        backoff_secs as f64,
        error,
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Dead-letters an exhausted job (kept for inspection, never retried).
pub async fn mark_dead<'e, E>(executor: E, id: Uuid, error: &str) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"UPDATE jobs SET status = 'dead', last_error = $2,
                  claimed_at = NULL, claimed_by = NULL, updated_at = now()
           WHERE id = $1 AND status = 'running'"#,
        id,
        error,
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Queue health snapshot for /healthz-style surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueStats {
    pub pending: i64,
    pub running: i64,
    pub dead: i64,
}

pub async fn stats<'e, E>(executor: E) -> DbResult<QueueStats>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT
             COUNT(*) FILTER (WHERE status = 'pending') AS pending,
             COUNT(*) FILTER (WHERE status = 'running') AS running,
             COUNT(*) FILTER (WHERE status = 'dead')    AS dead
           FROM jobs"#
    )
    .fetch_one(executor)
    .await?;
    Ok(QueueStats {
        pending: row.pending.unwrap_or(0),
        running: row.running.unwrap_or(0),
        dead: row.dead.unwrap_or(0),
    })
}

/// Age in seconds of the OLDEST dead-lettered job, or `None` when nothing is
/// dead.
///
/// `jobs.updated_at` is set by the same UPDATE that flips a row to `dead`
/// (see [`mark_dead`]), so it is the moment the job was abandoned — not when it
/// was enqueued. That is the number an operator cares about: "the oldest
/// thing nobody is going to retry has been sitting there for four days".
///
/// Runtime query rather than `sqlx::query!` on purpose: this statement is new,
/// it has no entry in the committed `.sqlx` cache, and re-preparing that cache
/// belongs to a lane that can run `cargo sqlx prepare`. The gate that keeps it
/// honest is the integration test in `tests/`, which runs it against a
/// migrated database.
pub async fn oldest_dead_age_secs<'e, E>(executor: E) -> DbResult<Option<i64>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let age: Option<i64> = sqlx::query_scalar(
        r#"SELECT EXTRACT(EPOCH FROM (now() - MIN(updated_at)))::bigint
             FROM jobs
            WHERE status = 'dead'"#,
    )
    .fetch_one(executor)
    .await?;
    Ok(age)
}
