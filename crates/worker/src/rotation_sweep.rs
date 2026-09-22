//! worker/rotation_sweep — the secret-rotation reminder tick (Wave 19
//! lane 3).
//!
//! Purpose: claim due rotation rows across BOTH vault subjects
//! (`secret_records` + `passwords` — the additive 0096 columns share the
//! sweep) via core-db's transactional claim (the `rotation_reminded_at`
//! stamp IS the claim, at-most-once per schedule), and enqueue a durable
//! `secrets.rotation` job per claimed row. core-api's internal surface
//! applies the job: the in-app `secret_rotation` notification and the
//! `secret.rotation_due` domain event (the automation/webhook hook).
//!
//! Crash semantics mirror pm_sweep: a row fires AT-MOST-ONCE per
//! rotation schedule (the claim flips the marker in the same statement
//! that returns the row), a crash between claim and settle loses that
//! slot, and a re-scheduled row (`rotate_at` moved forward) re-arms the
//! reminder because the marker comparison uses the NEW deadline.
//!
//! Non-purpose: no domain writes here (the internal surface owns the
//! notification + event), no secret material anywhere (payloads carry
//! ids/titles only — the sweep never unseals anything).

use sqlx::PgPool;
use tokio::sync::watch;

/// The job kind the queue runner dispatches to core-api's internal
/// rotation surface.
pub const JOB_KIND: &str = "secrets.rotation";

/// One sweep: claim due rows, enqueue jobs. Returns the number of jobs
/// enqueued.
pub async fn sweep_once(pool: &PgPool, batch_limit: i64) -> anyhow::Result<usize> {
    let started = std::time::Instant::now();
    let result = sweep_inner(pool, batch_limit).await;
    if let Ok(enqueued) = &result {
        if *enqueued > 0 {
            tracing::debug!(
                enqueued,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "rotation sweep"
            );
        }
    }
    result
}

async fn sweep_inner(pool: &PgPool, batch_limit: i64) -> anyhow::Result<usize> {
    let claims = core_db::share_links::claim_due_rotation(pool, batch_limit).await?;
    let mut enqueued = 0usize;
    for claim in &claims {
        let job = core_db::queue::NewJob {
            org_id: claim.org_id,
            kind: JOB_KIND.to_string(),
            payload: serde_json::json!({
                "org_id": claim.org_id,
                "subject_kind": claim.subject_kind.as_str(),
                "subject_id": claim.subject_id,
                "title": claim.title,
                "rotate_at": claim.rotate_at.to_rfc3339(),
            }),
            max_attempts: 5,
        };
        match core_db::queue::enqueue(pool, &job).await {
            Ok(_) => enqueued += 1,
            Err(error) => {
                // The reminder marker is already flipped; a lost enqueue
                // means THAT slot reminds never (at-most-once for the
                // job leg — the operator-visible effect is the same
                // silent-lose posture pm_sweep documents per slot).
                tracing::error!(
                    subject = %claim.subject_id,
                    %error,
                    "rotation job enqueue failed; slot lost"
                );
            }
        }
    }
    Ok(enqueued)
}

/// The interval loop: sweeps with the shared eval interval; an empty
/// sweep sleeps one interval, a failure backs off 2 s (pm_sweep shape).
pub async fn run_rotation_sweep(
    pool: PgPool,
    interval_secs: u64,
    mut shutdown: watch::Receiver<bool>,
) {
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
                        tracing::error!(%error, "rotation sweep failed");
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::JOB_KIND;
    #[test]
    fn job_kind_matches_the_runner_branch() {
        assert_eq!(JOB_KIND, "secrets.rotation");
    }
}
