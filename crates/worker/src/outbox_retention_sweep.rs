//! worker/outbox_retention_sweep — the event outbox's retention tick
//! (Wave 52 lane P1, `p52-event-stream-bounds`).
//!
//! Purpose: `outbox_events` is the contract-event delivery trail. Every
//! mutation writes a row inside its own transaction, the relay publishes it
//! and stamps `published_at`, and from that moment the only reader that cares
//! is SSE resume (`core-api`'s `realtime::replay_after`), which is bounded by
//! its own replay buffer. So a published row past the window is dead weight
//! that nothing reads — but nothing deleted it either, and the table grew for
//! the life of the deployment. This is the something: one housekeeping pass per
//! interval that removes published rows past the window.
//!
//! Non-purpose, and the important half: this never touches an UNPUBLISHED row.
//! An unpublished row is an event the relay still owes a consumer, and deleting
//! one would drop it silently. The sweep also writes no events of its own and
//! calls nothing on the internal effect seam — a purge is housekeeping, not a
//! domain effect, exactly like the workflow run-ledger sweep next to it.
//!
//! Window: a deployment setting (`OUTBOX_RETENTION_DAYS`, default 7), not an
//! org setting. The run ledger's window is per-org because a run is a tenant's
//! own record; the outbox window is about how long a client may be disconnected
//! and still resume, and the answer is the same for every tenant on an
//! instance. The audit trail lives in a SEPARATE table (`audit_events`, from
//! migration 0001) and is not what this prunes. A short window here is not a
//! short audit window, and changing this number cannot lose audit history.

use sqlx::PgPool;
use tokio::sync::watch;

/// Rows removed per statement. Small enough that a pass is a handful of short
/// range scans on the `(published_at)` partial index rather than one long
/// delete, large enough that a normal interval finishes in one or two.
pub const PRUNE_BATCH: i64 = 5_000;

/// Batches per pass. Caps how long one sweep can run, and therefore how long it
/// holds a connection, on the first pass after the upgrade, when the backlog
/// may be the entire history of the install.
pub const MAX_BATCHES_PER_PASS: usize = 20;

/// Pause before the next pass when one pass hits [`MAX_BATCHES_PER_PASS`] with
/// work still waiting. Short enough that a large backlog is worked through in
/// under an hour, long enough to leave the database alone between passes.
pub const CATCH_UP_PAUSE_SECS: u64 = 30;

/// The default cadence when the environment does not set one. Retention is
/// housekeeping, not user-visible latency, so this is hours, not seconds — and
/// it deliberately matches the run-ledger sweep's six hours so an operator has
/// one mental model for both.
pub const DEFAULT_INTERVAL_SECS: u64 = 6 * 60 * 60;

/// Catch-up has to be faster than the normal cadence, or the shorter pause after
/// a capped pass is a no-op and a large backlog never clears. Compile-time, like
/// the stream bound's floor check: neither number is an input.
const _: () = {
    assert!(CATCH_UP_PAUSE_SECS < DEFAULT_INTERVAL_SECS);
    // 30s into 6h: the loop returns to a capped backlog 720 times more often.
    assert!(DEFAULT_INTERVAL_SECS / CATCH_UP_PAUSE_SECS == 720);
};

/// Default window, in days.
pub const DEFAULT_RETENTION_DAYS: u64 = 7;

/// Settings for the outbox retention loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutboxRetentionConfig {
    /// Seconds between sweeps.
    pub interval_secs: u64,
    /// How long a published row is kept.
    pub retention_days: u64,
}

impl OutboxRetentionConfig {
    /// Resolves both knobs, keeping the window inside the range
    /// `make_interval(days => …)` accepts and at least one day so a
    /// misconfiguration cannot delete a row the relay is still publishing.
    pub fn from_values(interval_secs: Option<u64>, retention_days: Option<u64>) -> Self {
        Self {
            interval_secs: interval_secs
                .filter(|secs| *secs > 0)
                .unwrap_or(DEFAULT_INTERVAL_SECS),
            retention_days: retention_days
                .map(|days| days.clamp(1, 3650))
                .unwrap_or(DEFAULT_RETENTION_DAYS),
        }
    }
}

/// One sweep: delete published rows past the window, oldest first, until a
/// batch comes back short or the pass cap is reached.
///
/// Returns the rows removed and whether the pass cap was reached with the last
/// batch still full, which means more rows are past the window and the caller
/// should come back sooner than its interval.
pub async fn sweep_once(pool: &PgPool, retention_days: u64) -> anyhow::Result<(u64, bool)> {
    let days = i32::try_from(retention_days).unwrap_or(i32::MAX);
    let mut removed = 0_u64;
    for _ in 0..MAX_BATCHES_PER_PASS {
        let batch = core_db::outbox::prune_published(pool, days, PRUNE_BATCH).await?;
        removed += batch;
        if batch < PRUNE_BATCH as u64 {
            // A short batch means the range is exhausted — the common case.
            if removed > 0 {
                tracing::debug!(removed, retention_days, "outbox retention sweep");
            }
            return Ok((removed, false));
        }
    }
    // The cap ran out with the last batch still full, so more is waiting. Say
    // so: a sweep that reports a number without the flag reads as "done", and
    // the operator would never know the window is still behind. The caller uses
    // the flag to come back sooner than its interval.
    tracing::warn!(
        removed,
        retention_days,
        batches = MAX_BATCHES_PER_PASS,
        "outbox retention sweep hit its batch cap; more rows are past the window"
    );
    Ok((removed, true))
}

/// The loop: sweep, sleep, repeat. A failure backs off two seconds rather than
/// spinning — the shape the run-ledger sweep uses — because a database hiccup
/// must not turn into a delete loop.
///
/// The pause after a successful pass is the configured interval, except when
/// the pass hit its batch cap: then it is [`CATCH_UP_PAUSE_SECS`]. Without that
/// shorter pause a backlog of ten million rows would take twenty-five days to
/// work through at one hundred thousand rows every six hours, and the sweep
/// would look like it was doing its job the whole time.
pub async fn run_outbox_retention_sweep(
    pool: PgPool,
    config: OutboxRetentionConfig,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => return,
            result = sweep_once(&pool, config.retention_days) => {
                match result {
                    Ok((_, more)) => {
                        let pause = if more {
                            CATCH_UP_PAUSE_SECS
                        } else {
                            config.interval_secs.max(60)
                        };
                        tokio::time::sleep(std::time::Duration::from_secs(pause)).await;
                    }
                    Err(error) => {
                        tracing::error!(%error, "outbox retention sweep failed");
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

    /// The defaults are the documented ones, and a missing knob is not a
    /// zero-day window: unset must land on the conservative default rather
    /// than on "delete everything published".
    #[test]
    fn defaults_are_conservative_and_a_missing_knob_is_not_zero() {
        let cfg = OutboxRetentionConfig::from_values(None, None);
        assert_eq!(cfg.interval_secs, DEFAULT_INTERVAL_SECS);
        assert_eq!(cfg.retention_days, DEFAULT_RETENTION_DAYS);

        // A zero from the environment is a broken setting, not "retain
        // nothing" — both knobs fall back rather than honouring it.
        let cfg = OutboxRetentionConfig::from_values(Some(0), Some(0));
        assert_eq!(cfg.interval_secs, DEFAULT_INTERVAL_SECS);
        assert_eq!(cfg.retention_days, 1);
    }

    #[test]
    fn the_window_is_kept_inside_what_make_interval_accepts() {
        // Below one day clamps up: a sub-day window would race the relay on a
        // row it has fetched but not yet stamped.
        assert_eq!(
            OutboxRetentionConfig::from_values(None, Some(0)).retention_days,
            1
        );
        assert_eq!(
            OutboxRetentionConfig::from_values(None, Some(30)).retention_days,
            30
        );
        // Above the upper bound clamps down instead of overflowing i32 when
        // the sweep converts it.
        assert_eq!(
            OutboxRetentionConfig::from_values(None, Some(u64::MAX)).retention_days,
            3650
        );
    }
}
