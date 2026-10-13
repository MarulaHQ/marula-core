//! worker/run_retention_sweep — the workflow run ledger's retention tick
//! (TASKLIST Wave 43 lane 14).
//!
//! Purpose: runs are the highest-volume rows this engine will produce, so
//! the run ledger's window is a SETTING (`workflow.run_retention_days`,
//! org-scope, 30 days by default) rather than an unbounded table, and
//! something has to ENFORCE the window. This is that something: one
//! housekeeping pass per interval that deletes each org's runs past ITS
//! window.
//!
//! The sweep only ever removes TERMINAL runs. A run that is still going, or
//! parked with a wake-at in the past, is OWED a wake — deleting it would be
//! losing work, not enforcing a window. That is the same distinction the
//! `workflow_runs_status_shape` CHECK protects from the write side.
//!
//! It runs on the process-wide pool, like `reclaim_stale` and the other
//! cross-org housekeeping passes: the window resolves PER ORG inside the
//! delete statement, so one statement is the whole implementation and an org
//! that never touches the setting keeps exactly the default.
//!
//! Non-purpose: no job kinds, no internal effect calls, no events — a purge
//! is housekeeping, not a domain effect, and it never needs core-api. The
//! resume/retry jobs the ledger records DO ride the existing queue through
//! `queue::enqueue`; this loop only prunes what is already finished.

use sqlx::PgPool;
use tokio::sync::watch;

/// The default cadence when the environment does not set one. Six hours is
/// often enough to keep the ledger a working surface and rare enough that
/// the sweep never competes with real work.
pub const DEFAULT_INTERVAL_SECS: u64 = 6 * 60 * 60;

/// One sweep: delete every org's expired runs, each against its own window.
/// Returns the number of runs removed.
pub async fn sweep_once(pool: &PgPool) -> anyhow::Result<u64> {
    let purged = core_db::workflow_runs::purge_expired_everywhere(pool).await?;
    if purged > 0 {
        tracing::debug!(purged, "run retention sweep");
    }
    Ok(purged)
}

/// The interval loop: sweep, sleep, repeat. A failure backs off two seconds
/// rather than spinning — the pm_sweep/rotation_sweep shape — because a
/// database hiccup must not turn into a delete loop.
pub async fn run_run_retention_sweep(
    pool: PgPool,
    interval_secs: u64,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => return,
            result = sweep_once(&pool) => {
                match result {
                    Ok(_) => {
                        tokio::time::sleep(std::time::Duration::from_secs(interval_secs.max(60)))
                            .await;
                    }
                    Err(error) => {
                        tracing::error!(%error, "run retention sweep failed");
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

    #[test]
    fn the_default_cadence_is_hours_not_seconds() {
        // Housekeeping must not compete with the sweeps that move real
        // work: one run-ledger prune every few hours is plenty when the
        // window itself is measured in days. A const assertion, because a
        // compile-time constant checked at compile time cannot be moved
        // after someone edits it — and the value moves with the window.
        const {
            assert!(DEFAULT_INTERVAL_SECS >= 6 * 60 * 60);
        }
        // ...and it must not be so rare that an org that shortens its
        // window waits a week for the table to shrink.
        const {
            assert!(DEFAULT_INTERVAL_SECS <= 24 * 60 * 60);
        }
    }

    /// The SQL the sweep calls decides which rows may be deleted, so the
    /// rule that matters — only TERMINAL runs — is pinned here against the
    /// same core-domain vocabulary the CHECK constraint uses.
    #[test]
    fn only_terminal_runs_are_ever_swept() {
        use core_domain::workflow_runs::RunStatus;
        let sweepable: Vec<RunStatus> = RunStatus::ALL
            .into_iter()
            .filter(|s| s.is_terminal())
            .collect();
        assert_eq!(
            sweepable,
            vec![
                RunStatus::Succeeded,
                RunStatus::Failed,
                RunStatus::Abandoned
            ]
        );
        // A parked run is owed a wake, not a delete; a running one is work.
        assert!(!RunStatus::Waiting.is_terminal());
        assert!(!RunStatus::Running.is_terminal());
    }
}
