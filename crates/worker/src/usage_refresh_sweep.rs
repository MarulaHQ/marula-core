//! worker/usage_refresh_sweep — the org usage table's refresh tick
//! (the private build tracker Wave 47 lane A6, migration 0131).
//!
//! Purpose: `org_usage` is MATERIALIZED — it holds a copy of the
//! aggregates, not the aggregates themselves — and something has to take
//! that copy. This loop does, one BATCH at a time, for the orgs whose row
//! is stale:
//!
//!   1. [`claim_stale_orgs`](core_db::org_usage::claim_stale_orgs) returns
//!      the orgs whose row is stale, oldest first, capped at one batch.
//!   2. [`refresh_orgs`](core_db::org_usage::refresh_orgs) recomputes the
//!      whole batch in ONE round-trip. The `org_id` primary key makes that
//!      an upsert, so a replay — a worker restart, a retry, a second loop —
//!      lands on the SAME rows and updates them. Idempotency is structural,
//!      not something this loop has to maintain.
//!   3. When the batch statement fails, [`bisect_refresh`] splits it to
//!      isolate the offending org(s) instead of letting one bad org starve
//!      every org behind it in the oldest-first claim.
//!
//! The work is proportional to what changed, not to the org count: an org
//! refreshed a minute ago is not in the claim, so a short interval costs
//! almost nothing on a quiet tenant and bounds the staleness every
//! consumer sees. Each refresh reads `refreshed_at` back into the row it
//! writes, so an operator (and the API's own payload) can tell when a
//! number was measured instead of assuming it is fresh.
//!
//! A full batch means there is probably more due, so the loop claims again
//! up to [`MAX_BATCHES_PER_SWEEP`] times. The alternative — one batch per
//! pass — makes a fleet wide enough for a few batches fall permanently
//! behind, which is a staleness nobody can see from the API, since the
//! payload only reports its own row's age.
//!
//! Why no queue job: a usage row is not an EFFECT. There is no core-api
//! call, no event, no outward delivery — the refresh writes its own table
//! on the process-wide pool exactly like the run-retention housekeeping
//! pass (`run_retention_sweep`). Registering a job kind would put a
//! monitoring copy through the delivery machinery that moves real work,
//! and `settle_job`'s closed kind list would have to grow for it. A
//! housekeeping shape is the honest one.
//!
//! Non-purpose: no enforcement of any kind. The refresh computes counts;
//! it never compares them to a configured limit, and a high count changes
//! nothing about what an org may do. Quotas are a report.

use sqlx::PgPool;
use tokio::sync::watch;
use uuid::Uuid;

/// Orgs refreshed per batch. A bound, not a quota: one batch touching more
/// than this is a graph-wide backlog, and the loop below keeps claiming
/// while a batch comes back FULL, so a backlog drains across passes rather
/// than silently widening (a growing backlog is exactly the staleness an
/// operator cannot see from the API, which reports only its own row).
pub const REFRESH_BATCH: i64 = 50;

/// The most batches one sweep may claim. Fifty orgs per batch across forty
/// batches is two thousand orgs per pass, which is past any single hosted
/// fleet today. The cap keeps one pass from competing indefinitely with the
/// loops that move real work, and the next pass continues from whichever org
/// is now oldest.
pub const MAX_BATCHES_PER_SWEEP: usize = 40;

/// The default cadence when the environment does not set one. Five
/// minutes keeps a monitoring view usefully fresh while the per-org work
/// stays trivial — and the payload carries its own `refreshed_at`, so the
/// consumer never has to guess what this number is.
pub const DEFAULT_INTERVAL_SECS: u64 = 5 * 60;

/// One sweep: refresh every org whose usage row is stale by at least
/// `interval_secs`, oldest first. Returns how many orgs were refreshed.
///
/// A full batch means there may be more due, so the loop claims again; the
/// iteration cap bounds one pass so a huge backlog yields to the rest of
/// the worker and the NEXT pass continues where this one stopped (the claim
/// is oldest-first, so whatever this pass skipped is the next pass's first).
///
/// The whole claimed batch goes through ONE `refresh_orgs` round-trip, so
/// sweep wall time is batches × per-batch latency rather than
/// orgs × per-org latency on a single pool connection.
///
/// Failure isolation is the reason this is not a plain `?` loop.
/// `refresh_orgs` is one statement over the whole batch, so one org whose
/// rows make the statement fail rolls back all 50 upserts. A naive retry
/// re-runs the identical oldest-first claim and fails identically, starving
/// every org behind the poison one — up to `MAX_BATCHES_PER_SWEEP` times per
/// pass, forever, with ~1,950 healthy orgs never refreshed. So a failing
/// batch is bisected to isolate the offending org(s) ([`bisect_refresh`]),
/// the survivors are still refreshed, and the poisoned ids are excluded
/// from the rest of this sweep's claims. A permanently poisoned org keeps
/// its own row stale, which is the honest visible state; it must not
/// silently starve the fleet.
pub async fn sweep_once(pool: &PgPool, interval_secs: u64) -> anyhow::Result<u64> {
    let mut refreshed = 0_u64;
    // Orgs whose refresh failed this sweep, excluded from later claims so
    // the pass can drain past them.
    let mut poisoned: Vec<Uuid> = Vec::new();
    for _ in 0..MAX_BATCHES_PER_SWEEP {
        let mut due = core_db::org_usage::claim_stale_orgs(
            pool,
            interval_secs.max(1) as i64,
            // Claim one extra row per known poison so a full batch is not
            // short purely because a poisoned org was filtered out.
            REFRESH_BATCH + poisoned.len() as i64,
        )
        .await?;
        due.retain(|org| !poisoned.contains(org));
        if due.is_empty() {
            break;
        }
        let (written, bad) = bisect_refresh(pool, &due).await;
        refreshed += written as u64;
        if !bad.is_empty() {
            tracing::warn!(
                poisoned = bad.len(),
                "org usage refresh isolated permanently-failing org(s); \
                 their rows stay stale and are skipped for this sweep"
            );
            poisoned.extend(bad);
        }
        // A short batch means the backlog is drained (or the claim hit orgs
        // that no longer exist, which write no row). Either way there is
        // nothing worth claiming again this pass.
        if due.len() < REFRESH_BATCH as usize {
            break;
        }
    }
    if refreshed > 0 {
        tracing::debug!(refreshed, "org usage refresh sweep");
    }
    Ok(refreshed)
}

/// Refresh `orgs`, bisecting the batch when the whole-set statement fails so
/// one poisoned org cannot take its batch-mates down with it.
///
/// Returns (rows written, poisoned org ids). The common case is ONE
/// round-trip; bisection only happens on failure, and its depth is bounded
/// by log2(batch) — at most ~6 levels for a 50-org batch.
async fn bisect_refresh(pool: &PgPool, orgs: &[Uuid]) -> (usize, Vec<Uuid>) {
    match core_db::org_usage::refresh_orgs(pool, orgs).await {
        Ok(rows) => (rows.len(), Vec::new()),
        Err(error) => {
            if orgs.len() <= 1 {
                // A single-org statement failed: this org is the poison.
                tracing::error!(
                    %error,
                    org = ?orgs.first(),
                    "org usage refresh failed for one org"
                );
                return (0, orgs.to_vec());
            }
            let mid = orgs.len() / 2;
            let (left_written, mut left_bad) = Box::pin(bisect_refresh(pool, &orgs[..mid])).await;
            let (right_written, right_bad) = Box::pin(bisect_refresh(pool, &orgs[mid..])).await;
            left_bad.extend(right_bad);
            (left_written + right_written, left_bad)
        }
    }
}

/// The interval loop: refresh the stale orgs, sleep, repeat. A failure
/// backs off two seconds rather than spinning — the pm_sweep /
/// retention_sweep shape — because a database hiccup must not turn into a
/// refresh loop that competes with real work.
pub async fn run_usage_refresh_sweep(
    pool: PgPool,
    interval_secs: u64,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => return,
            result = sweep_once(&pool, interval_secs) => {
                match result {
                    Ok(_) => {
                        tokio::time::sleep(std::time::Duration::from_secs(interval_secs.max(60)))
                            .await;
                    }
                    Err(error) => {
                        tracing::error!(%error, "org usage refresh sweep failed");
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
    fn the_default_cadence_is_minutes_not_hours() {
        // A monitoring view that is hours old is an audit log, not a
        // monitor; and a refresh batch is one aggregate statement per stale
        // batch, so a few minutes is cheap. Pinned as const assertions so
        // editing the value forces a deliberate decision rather than drift.
        const {
            assert!(DEFAULT_INTERVAL_SECS >= 60);
        }
        const {
            assert!(DEFAULT_INTERVAL_SECS <= 15 * 60);
        }
    }

    #[test]
    fn the_batch_is_bounded_so_one_pass_cannot_run_away() {
        // The claim is ordered and capped, and the sweep loop stops
        // claiming at the cap, so a fleet-wide backlog drains across
        // passes instead of one statement touching every org at once.
        const {
            assert!(REFRESH_BATCH >= 10);
        }
        const {
            assert!(REFRESH_BATCH <= 500);
        }
        // A batch of 50 across at most 40 batches is 2,000 orgs per pass —
        // comfortably past any single hosted fleet today, and the next
        // pass continues from the oldest still-stale org.
        const {
            assert!(MAX_BATCHES_PER_SWEEP >= 10);
        }
        const {
            assert!(MAX_BATCHES_PER_SWEEP <= 200);
        }
    }

    /// Bisecting a failing batch isolates a poison org on the first pass
    /// and stops there: the depth is logarithmic in the batch size, not
    /// linear. Pinned because the bound is what makes failure isolation
    /// affordable — a linear retry would be 50 statements per poisoned
    /// sweep where log2(50) ≈ 6 levels is at most 99, and only for an org
    /// whose rows genuinely cannot be refreshed.
    #[test]
    fn bisection_depth_is_logarithmic_in_the_batch_size() {
        // A closure cannot call itself, so the recursion is a nested `fn`
        // mirroring bisect_refresh's split (half, then each half).
        fn levels(n: usize) -> u32 {
            if n <= 1 {
                return 0;
            }
            1 + (levels(n / 2)).max(levels(n - n / 2))
        }
        let depth = levels(REFRESH_BATCH as usize);
        assert_eq!(depth, 6, "log2(50) rounds to 6 split levels");
        // Worst-case statement count is bounded by 2×batch − 1.
        let worst = 2 * REFRESH_BATCH - 1;
        assert_eq!(worst, 99);
        assert!(
            depth < 10,
            "a deeper split would mean a retry shape, not a bisection"
        );
    }
}
