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
    let id = Uuid::now_v7();
    sqlx::query!(
        r#"INSERT INTO jobs (id, org_id, kind, payload, max_attempts)
           VALUES ($1, $2, $3, $4, $5)"#,
        id,
        job.org_id,
        job.kind,
        job.payload,
        job.max_attempts,
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
    let rows = sqlx::query!(
        r#"UPDATE jobs
           SET status = 'running', claimed_at = now(), claimed_by = $1,
               attempts = attempts + 1, updated_at = now()
           WHERE id IN (
             SELECT id FROM jobs
             WHERE status = 'pending' AND run_after <= now()
             ORDER BY id
             FOR UPDATE SKIP LOCKED
             LIMIT $2
           )
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
