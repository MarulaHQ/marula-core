//! worker/queue_runner — the general job queue processor.
//!
//! Purpose: claim jobs from the `jobs` table (SKIP LOCKED, DB-clock due
//! checks), dispatch by kind to core-api's internal surface, and settle
//! each job as done / retry-with-backoff / dead-letter. Backoff comes from
//! the queue's [`RetryPolicy`](core_domain::retry::RetryPolicy) and is
//! computed in Postgres time, so a skewed host clock can neither delay nor
//! fast-forward anyone else's retries.
//!
//! Non-purpose: no job semantics here (payloads are opaque to this loop),
//! no scheduling of future jobs (producers do that), no direct writes to
//! domain tables.

use crate::effects::{EffectApi, EffectOutcome};
use crate::webhook_out::{self, DeliverySink};
use core_db::queue::ClaimedJob;
use core_domain::retry::{self, Subsystem};
use sqlx::PgPool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::watch;
use tracing::Instrument as _;

/// Delay before the attempt that follows `attempts_made` failures, in
/// seconds, from the queue's retry policy with a fresh jitter sample.
/// `attempts_made` is 1-based: the first failure waits the policy's base.
///
/// The numbers live in `core_domain::retry`, not here: the constant pair this
/// function used to carry clamped its exponent, so the cap its own docstring
/// promised was unreachable.
#[must_use]
pub fn backoff_secs(attempts_made: i32) -> i64 {
    backoff_secs_with(attempts_made, retry::sample())
}

/// [`backoff_secs`] with the jitter sample supplied, so a test can pin the
/// value a real worker would sleep between two known bounds.
#[must_use]
pub fn backoff_secs_with(attempts_made: i32, sample: f64) -> i64 {
    let attempts = u32::try_from(attempts_made.max(1)).unwrap_or(1);
    retry::policy(Subsystem::Queue).delay_secs(attempts, sample)
}

/// The queue's attempt budget as a job row stores it. Producers take their
/// `max_attempts` from here so a sweep and the queue cannot disagree about how
/// many tries a job has.
#[must_use]
pub fn queue_max_attempts() -> i32 {
    retry::attempt_limit_i32(Subsystem::Queue)
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
    idle_ms: u64,
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
        // The in-flight tick is NOT abandoned on shutdown. Dropping it mid-pass
        // released nothing: the claim stayed in the table until `reclaim_stale`
        // picked it up five minutes later, while the drain reported "drained".
        // So shutdown stops STARTING ticks and lets the current one settle.
        let in_flight = tick(
            &pool,
            &effects,
            &sink,
            webhook_secret_key,
            batch,
            stale_after_secs,
            &counters,
            &worker,
        );
        tokio::pin!(in_flight);
        tokio::select! {
            _ = shutdown.changed() => {
                if let Err(error) = (&mut in_flight).await {
                    tracing::error!(%error, "in-flight queue tick failed during drain");
                }
                return;
            }
            result = &mut in_flight => {
                match result {
                    Ok(claimed) => {
                        if claimed == 0 {
                            // Idle: modest sleep by default; every timing
                            // decision in SQL already used DB now(). A small
                            // install can stretch this via QUEUE_IDLE_MS.
                            tokio::time::sleep(std::time::Duration::from_millis(idle_ms.max(1)))
                                .await;
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
    // Queue-depth gauges (the private build tracker 2.18): one aggregate COUNT per tick —
    // the `jobs` table is small compared to the domain tables.
    if let Ok(stats) = core_db::queue::stats(pool).await {
        core_otel::metrics::queue_depth(
            stats.pending.max(0) as u64,
            stats.running.max(0) as u64,
            stats.dead.max(0) as u64,
        );
        // Dead-letter AGE beside the depth (the private build tracker R7a): depth alone says
        // how many jobs nobody will retry, not whether they have been rotting
        // there for an hour or a week. Guarded on `dead > 0` because that
        // statement has no partial index to lean on — an install with an
        // empty dead set pays nothing, and the series stays absent instead of
        // reporting a zero age for a set that is empty.
        if stats.dead > 0 {
            if let Ok(age) = core_db::queue::oldest_dead_age_secs(pool).await {
                core_otel::metrics::dead_letter_age(age.map(|secs| secs.max(0) as u64));
            }
        }
    }
    // Parked workflow runs (the private build tracker R7a): a run waiting on an event that
    // never arrives is invisible today — it is not in the jobs table and no
    // consumer holds it. Count plus the oldest idle age makes the stall
    // visible from the metric side. `status = 'waiting'` is index-served by
    // `idx_workflow_runs_status`.
    if let Ok(parked) = core_db::workflow_runs::parked_stats(pool).await {
        core_otel::metrics::workflow_parked(
            parked.parked.max(0) as u64,
            parked.oldest_age_secs.map(|secs| secs.max(0) as u64),
        );
    }
    // Backup age (the private build tracker R7a): reads the stamp the backup runner writes.
    // Opt-in by path; unset means no series, never a zero.
    crate::backup_health::sample();
    let jobs = core_db::queue::claim_batch(pool, batch, worker).await?;
    let count = jobs.len();

    // Per-tick span with batch size; per-job spans (kind + attempts) carry
    // the claim→apply latency histogram sample (the private build tracker 2.18).
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
    // ledger write + settle (it needs more than one effect call). It
    // reports the settle it actually made, so the counters and the otel
    // outcome reflect retries and deads for this kind too — not a
    // uniform Done.
    if job.kind == webhook_out::JOB_KIND {
        return match webhook_secret_key {
            Some(key) => {
                let settle =
                    webhook_out::process_delivery_job(pool, &key, sink, effects, job).await;
                match settle {
                    Err(error) => {
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
                    }
                    Ok(webhook_out::DeliverySettle::Done) => {
                        counters.done.fetch_add(1, Ordering::Relaxed);
                        Ok(SettleOutcome::Done)
                    }
                    Ok(webhook_out::DeliverySettle::Retried) => {
                        counters.retried.fetch_add(1, Ordering::Relaxed);
                        Ok(SettleOutcome::Retried)
                    }
                    Ok(webhook_out::DeliverySettle::Dead) => {
                        counters.dead.fetch_add(1, Ordering::Relaxed);
                        Ok(SettleOutcome::Dead)
                    }
                }
            }
            None => {
                // Fail-closed none mode: no at-rest key ⇒ never deliver.
                core_db::queue::mark_dead(pool, job.id, "MARULA_SECRET_KEY unset").await?;
                counters.dead.fetch_add(1, Ordering::Relaxed);
                Ok(SettleOutcome::Dead)
            }
        };
    }
    // Workflow resume (Wave 43 lane 15): a parked run's wake, or a fresh
    // event-triggered run, is executed IN-PROCESS rather than POSTed to a
    // core-api path — the executor is the worker's own loop, and node
    // effects still cross the org-scoped seam via `effects`. The executor
    // classifies its own outcome so this loop stays the single settle
    // authority.
    if job.kind == core_domain::workflow_runs::WORKFLOW_RESUME_JOB {
        let result = crate::workflow::process_resume_job(pool, effects, job).await;
        return match result.outcome {
            crate::workflow::JobOutcome::Done => {
                core_db::queue::mark_done(pool, job.id).await?;
                counters.done.fetch_add(1, Ordering::Relaxed);
                Ok(SettleOutcome::Done)
            }
            crate::workflow::JobOutcome::Retry => {
                core_db::queue::schedule_retry(
                    pool,
                    job.id,
                    backoff_secs(job.attempts),
                    &result.error,
                )
                .await?;
                counters.retried.fetch_add(1, Ordering::Relaxed);
                Ok(SettleOutcome::Retried)
            }
            crate::workflow::JobOutcome::Dead => {
                core_db::queue::mark_dead(pool, job.id, &result.error).await?;
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
        // Staged import batches (the private build tracker D2 remainder): the runner replays
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

    /// The old constant was `5·2^n` with the shift clamped at 8, which capped
    /// at 1280 s while the docstring promised an hour. These assertions are
    /// bounds, not values: the exact number is the policy's business, and the
    /// loop jitters it.
    #[test]
    fn backoff_grows_exponentially_within_the_policys_bounds() {
        let policy = retry::policy(Subsystem::Queue);
        // No jitter at all (sample 1.0 for full jitter, 0.5 for equal) gives
        // the computed delay back minus the spread; compare the ceilings.
        for attempts in 1..12 {
            let upper = backoff_secs_with(attempts, 1.0);
            let computed = policy
                .base_delay(u32::try_from(attempts).expect("positive"))
                .as_secs() as i64;
            assert!(
                upper <= computed,
                "attempt {attempts}: jitter produced {upper}s above the computed {computed}s"
            );
            assert!(upper >= 1, "a retry must wait at least a second");
            assert!(
                i64::try_from(policy.cap.as_secs()).unwrap() >= upper,
                "a delay above the cap escaped"
            );
        }
        // The curve reaches the cap rather than flattening short of it.
        assert_eq!(
            backoff_secs_with(12, 1.0),
            i64::try_from(policy.cap.as_secs()).unwrap()
        );
        // A nonsensical attempt count clamps to the first delay, not an
        // enormous one.
        assert!(backoff_secs_with(-3, 1.0) <= 5);
        assert!(backoff_secs_with(0, 1.0) <= 5);
    }

    #[test]
    fn jitter_shortens_the_wait_but_never_removes_both_bounds() {
        let policy = retry::policy(Subsystem::Queue);
        let computed = policy.base_delay(4).as_secs() as i64;
        // Full jitter spans [0, computed]; the seconds conversion floors at 1.
        assert_eq!(backoff_secs_with(4, 0.0), 1);
        assert_eq!(backoff_secs_with(4, 1.0), computed);
        let samples: Vec<i64> = (0..16).map(|_| backoff_secs(4)).collect();
        assert!(
            samples.iter().any(|s| *s < computed),
            "the queue's waits are not being spread: {samples:?}"
        );
        assert!(samples.iter().all(|s| *s >= 1 && *s <= computed));
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
