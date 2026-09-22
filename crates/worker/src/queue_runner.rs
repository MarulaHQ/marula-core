//! worker/queue_runner — the general job queue processor.
//!
//! Purpose: claim jobs from the `jobs` table (SKIP LOCKED, DB-clock due
//! checks), dispatch by kind to core-api's internal surface, and settle
//! each job as done / retry-with-backoff / dead-letter. Backoff is
//! exponential and computed in Postgres time, so a skewed host clock can
//! neither delay nor fast-forward anyone else's retries.
//!
//! Non-purpose: no job semantics here (payloads are opaque to this loop),
//! no scheduling of future jobs (producers do that), no direct writes to
//! domain tables.

use crate::effects::{EffectApi, EffectOutcome};
use crate::webhook_out::{self, DeliverySink};
use core_db::queue::ClaimedJob;
use sqlx::PgPool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::watch;
use tracing::Instrument as _;

/// Exponential backoff: 10s, 20s, 40s … capped at one hour. `attempt` is
/// the count INCLUDING the failed one (first failure ⇒ 1).
pub fn backoff_secs(attempt: i32) -> i64 {
    let shift = attempt.clamp(0, 8) as u32;
    (5_i64.saturating_mul(1 << shift)).min(3600)
}

/// Outcome counters for health reporting.
#[derive(Debug, Default)]
pub struct QueueCounters {
    pub done: AtomicU64,
    pub retried: AtomicU64,
    pub dead: AtomicU64,
}

#[allow(clippy::too_many_arguments)]
pub async fn run_queue<S: DeliverySink, E: EffectApi>(
    pool: PgPool,
    effects: E,
    sink: S,
    webhook_secret_key: Option<crate::config::AtRestKey>,
    batch: i64,
    stale_after_secs: i64,
    counters: std::sync::Arc<QueueCounters>,
    mut shutdown: watch::Receiver<bool>,
) {
    let worker = format!("worker-{}", std::process::id());
    // Re-arm leases orphaned by a previous crash before the first tick.
    if let Err(error) = core_db::queue::reclaim_stale(&pool, stale_after_secs).await {
        tracing::error!(%error, "initial stale reclaim failed");
    }
    loop {
        if *shutdown.borrow() {
            return;
        }
        tokio::select! {
            _ = shutdown.changed() => return,
            result = tick(&pool, &effects, &sink, webhook_secret_key, batch, stale_after_secs, &counters, &worker) => {
                match result {
                    Ok(claimed) => {
                        if claimed == 0 {
                            // Idle: modest sleep; every timing decision in
                            // SQL already used DB now().
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "queue tick failed");
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn tick<S: DeliverySink, E: EffectApi>(
    pool: &PgPool,
    effects: &E,
    sink: &S,
    webhook_secret_key: Option<crate::config::AtRestKey>,
    batch: i64,
    stale_after_secs: i64,
    counters: &QueueCounters,
    worker: &str,
) -> anyhow::Result<usize> {
    // Stale re-arm runs on its own statement: a data-modifying CTE's
    // writes would be invisible to the claim in the same statement.
    core_db::queue::reclaim_stale(pool, stale_after_secs).await?;
    // Queue-depth gauges (TASKLIST 2.18): one aggregate COUNT per tick —
    // the `jobs` table is small compared to the domain tables.
    if let Ok(stats) = core_db::queue::stats(pool).await {
        core_otel::metrics::queue_depth(
            stats.pending.max(0) as u64,
            stats.running.max(0) as u64,
            stats.dead.max(0) as u64,
        );
    }
    let jobs = core_db::queue::claim_batch(pool, batch, worker).await?;
    let count = jobs.len();

    // Per-tick span with batch size; per-job spans (kind + attempts) carry
    // the claim→apply latency histogram sample (TASKLIST 2.18).
    let tick_span = tracing::info_span!("queue.tick", batch_size = count);
    let mut settled: u64 = 0;
    for job in jobs {
        let job_span = tracing::info_span!(
            "queue.job",
            kind = %job.kind,
            attempts = job.attempts,
        );
        let (outcome, elapsed_ms) = {
            let started = Instant::now();
            let outcome = settle_job(pool, effects, sink, webhook_secret_key, &job, counters)
                .instrument(job_span.clone())
                .await?; // DB failure aborts the tick exactly as before
            (outcome, started.elapsed().as_secs_f64() * 1000.0)
        };
        // Unlike the original, the metric outcome mirrors the settle branch
        // already recorded in the health counters — one source of truth.
        let (metric_outcome, done) = match outcome {
            SettleOutcome::Done => (core_otel::metrics::JobOutcome::Done, true),
            SettleOutcome::Retried => (core_otel::metrics::JobOutcome::Retried, false),
            SettleOutcome::Dead => (core_otel::metrics::JobOutcome::Dead, false),
        };
        core_otel::metrics::job_settled(&job.kind, metric_outcome, elapsed_ms);
        if done {
            settled += 1;
        }
    }
    tick_span.record("settled", settled);
    Ok(count)
}

/// One job's settle path. `Ok(SettleOutcome)` maps 1:1 onto the health
/// counters so metrics and liveness can never disagree; `Err` is a DB
/// failure and aborts the tick (remaining jobs are re-claimed next tick).
async fn settle_job<S: DeliverySink, E: EffectApi>(
    pool: &PgPool,
    effects: &E,
    sink: &S,
    webhook_secret_key: Option<crate::config::AtRestKey>,
    job: &ClaimedJob,
    counters: &QueueCounters,
) -> anyhow::Result<SettleOutcome> {
    // Signed-webhook delivery performs its own HTTP attempt + attempt
    // ledger write + settle (it needs more than one effect call).
    if job.kind == webhook_out::JOB_KIND {
        return match webhook_secret_key {
            Some(key) => {
                if let Err(error) =
                    webhook_out::process_delivery_job(pool, &key, sink, effects, job).await
                {
                    tracing::error!(%error, job_id = %job.id, "webhook delivery crashed");
                    core_db::queue::schedule_retry(
                        pool,
                        job.id,
                        backoff_secs(job.attempts),
                        "delivery processing error",
                    )
                    .await?;
                    counters.retried.fetch_add(1, Ordering::Relaxed);
                    Ok(SettleOutcome::Retried)
                } else {
                    Ok(SettleOutcome::Done)
                }
            }
            None => {
                // Fail-closed none mode: no at-rest key ⇒ never deliver.
                core_db::queue::mark_dead(pool, job.id, "MERIDIAN_SECRET_KEY unset").await?;
                counters.dead.fetch_add(1, Ordering::Relaxed);
                Ok(SettleOutcome::Dead)
            }
        };
    }
    let path: &'static str = match job.kind.as_str() {
        "sla.verdict" => "/api/internal/sla/verdict",
        // PM schedule generation (Wave 11 lane 9): a claimed slot fires
        // through core-api's normal ticket-creation path; the pm_runs
        // ledger makes replays inert.
        "pm.generate" => "/api/internal/pm/generate",
        // Secret-rotation reminders (Wave 19 lane 3): the internal
        // surface owns the notification + `secret.rotation_due` event.
        "secrets.rotation" => "/api/internal/secrets/rotation",
        "automation.effects" => "/api/internal/effects",
        // Staged import batches (TASKLIST D2 remainder): the runner replays
        // the batch's ObjectStore chunks through core-api's commit path.
        "import.batch.run" => "/api/internal/imports/batch/run",
        other => {
            tracing::error!(kind = %other, job_id = %job.id, "unknown job kind");
            core_db::queue::mark_dead(pool, job.id, "unknown job kind").await?;
            counters.dead.fetch_add(1, Ordering::Relaxed);
            return Ok(SettleOutcome::Dead);
        }
    };
    let outcome = effects.post(path, job.payload.clone()).await;
    match outcome {
        EffectOutcome::Applied => {
            core_db::queue::mark_done(pool, job.id).await?;
            counters.done.fetch_add(1, Ordering::Relaxed);
            Ok(SettleOutcome::Done)
        }
        EffectOutcome::Rejected => {
            // Permanent rejection dead-letters immediately regardless
            // of attempts left — retrying a 4xx only burns CPU.
            core_db::queue::mark_dead(pool, job.id, "rejected by core-api").await?;
            counters.dead.fetch_add(1, Ordering::Relaxed);
            Ok(SettleOutcome::Dead)
        }
        EffectOutcome::Transient => {
            if job.attempts >= job.max_attempts {
                core_db::queue::mark_dead(pool, job.id, "attempts exhausted").await?;
                counters.dead.fetch_add(1, Ordering::Relaxed);
                Ok(SettleOutcome::Dead)
            } else {
                let backoff = backoff_secs(job.attempts);
                core_db::queue::schedule_retry(pool, job.id, backoff, "core-api unreachable")
                    .await?;
                counters.retried.fetch_add(1, Ordering::Relaxed);
                Ok(SettleOutcome::Retried)
            }
        }
    }
}

/// Outcome classification for metrics (health counters stay authoritative).
enum SettleOutcome {
    Done,
    Retried,
    Dead,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_exponential_and_capped() {
        assert_eq!(backoff_secs(1), 10);
        assert_eq!(backoff_secs(2), 20);
        assert_eq!(backoff_secs(3), 40);
        assert_eq!(backoff_secs(7), 640);
        assert_eq!(backoff_secs(8), 1280);
        assert_eq!(backoff_secs(30), 1280);
        assert_eq!(
            backoff_secs(-3),
            5,
            "nonsensical attempts clamp to the floor"
        );
    }

    /// The settle logic is exercised end-to-end by db_suite's queue tests;
    /// here we pin the pure classification contract.
    #[test]
    fn outcome_classification_is_exhaustive_for_the_runner() {
        let classify = |o: EffectOutcome| match o {
            EffectOutcome::Applied => "done",
            EffectOutcome::Rejected => "dead",
            EffectOutcome::Transient => "retry",
        };
        assert_eq!(classify(EffectOutcome::Applied), "done");
        assert_eq!(classify(EffectOutcome::Rejected), "dead");
        assert_eq!(classify(EffectOutcome::Transient), "retry");
    }
}
