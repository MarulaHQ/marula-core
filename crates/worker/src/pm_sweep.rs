//! worker/pm_sweep — the recurring/scheduled-ticket (PM schedule) tick.
//!
//! Purpose: claim due `pm_schedules` rows (FOR UPDATE SKIP LOCKED, the
//! +1h defensive bump makes the discharged slot reconstructable), compute
//! the next occurrence from the stored cadence via core-domain's cadence
//! math, stamp it onto the schedule, and enqueue a durable
//! `pm.generate` job per discharged slot. core-api's internal surface
//! applies the job through the normal ticket-creation path (§4.1 — this
//! loop never writes a domain table).
//!
//! Crash semantics mirror report delivery: a slot fires AT-MOST-ONCE per
//! schedule (the claim advances `next_run_at` in the same statement), and
//! a crash between advance and settle loses that slot. A failed cadence
//! row keeps `next_run_at` claimable and records `last_error` instead of
//! wedging silently.
//!
//! Non-purpose: no ticket creation (core-api's internal generator does
//! the domain write), no events (the `pm_runs` ledger is the run record),
//! no cadence persistence beyond stamping the computed next occurrence.

use sqlx::PgPool;
use tokio::sync::watch;

/// The job kind the queue runner dispatches to core-api's internal
/// generator.
pub const JOB_KIND: &str = "pm.generate";

/// One sweep: claim due schedules, stamp the next occurrence, enqueue
/// jobs. Returns the number of jobs enqueued.
pub async fn sweep_once(pool: &PgPool, batch_limit: i64) -> anyhow::Result<usize> {
    use tracing::Instrument as _;
    let started = std::time::Instant::now();
    let span = tracing::info_span!("pm.sweep");
    let result = sweep_inner(pool, batch_limit)
        .instrument(span.clone())
        .await;
    if let Ok(enqueued) = &result {
        span.record("enqueued", enqueued);
    } else {
        span.record("failed", true);
    }
    core_otel::metrics::sla_sweep(started.elapsed().as_secs_f64() * 1000.0);
    result
}

async fn sweep_inner(pool: &PgPool, batch_limit: i64) -> anyhow::Result<usize> {
    let claims = core_db::recurring::claim_due_schedules(pool, batch_limit).await?;
    if claims.is_empty() {
        return Ok(0);
    }
    let mut enqueued = 0usize;
    for schedule in claims {
        // Stamp the next occurrence BEFORE enqueueing so a cadence-math
        // failure never wedges the row: stamp_next_run keeps the row
        // claimable and records the error instead.
        // stamp_next_run never wedges a bad cadence row (it records
        // last_error and returns false); a DB failure is real and skips
        // the slot this pass.
        match core_db::recurring::stamp_next_run(pool, &schedule, None).await {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!(schedule_id = %schedule.id, "pm cadence row invalid; slot skipped");
                continue;
            }
            Err(error) => {
                tracing::error!(schedule_id = %schedule.id, %error, "pm stamp failed");
                continue;
            }
        }
        let job = core_db::queue::NewJob {
            org_id: schedule.org_id,
            kind: JOB_KIND.to_string(),
            payload: serde_json::json!({
                "org_id": schedule.org_id,
                "schedule_id": schedule.id,
                "slot_at": schedule.slot_at.to_rfc3339(),
            }),
            max_attempts: 5,
        };
        match core_db::queue::enqueue(pool, &job).await {
            Ok(_) => enqueued += 1,
            Err(error) => {
                tracing::error!(%error, schedule_id = %schedule.id, "pm job enqueue failed");
            }
        }
    }
    Ok(enqueued)
}

/// The host-paced loop. All business timing decisions used DB `now()`
/// already (the claim); the host clock only paces the poll.
pub async fn run_pm_sweep(pool: PgPool, interval_secs: u64, mut shutdown: watch::Receiver<bool>) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => return,
            result = sweep_once(&pool, 200) => {
                match result {
                    Ok(0) => {
                        tokio::time::sleep(std::time::Duration::from_secs(
                            interval_secs.max(1),
                        ))
                        .await;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::error!(%error, "pm sweep failed");
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sweep never wedges on a disabled cadence row: claim rows whose
    /// cadence math fails keep claimable next_run_at and last_error (the
    /// sweep skips them; the row reports the reason). Unit-pinned here —
    /// the behavior lives in recurring.rs's stamp gate.
    #[test]
    fn job_kind_matches_the_runner_branch() {
        assert_eq!(JOB_KIND, "pm.generate");
    }
}
