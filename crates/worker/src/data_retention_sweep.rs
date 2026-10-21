//! worker/data_retention_sweep — the retention matrix's clock
//! (Wave 52 lane R6, `p52-data-lifecycle-dr`).
//!
//! Purpose: nine tables grew forever because their window lived in a comment.
//! `core_db::retention` holds the matrix (table → window → mechanism) and the
//! delete statements; `core_db::partitions` holds the DDL a partition-drop
//! window needs. This loop is the clock that runs both, once per interval, on
//! the housekeeping role.
//!
//! Order inside a pass matters and is not incidental:
//!
//! 1. **Partitions first.** Creating the month a row is about to be written into
//!    is the only step whose failure hurts a user (a range-partitioned table
//!    with nowhere to put a row refuses the INSERT). Doing it first means a
//!    slow batched delete later in the pass never delays it.
//! 2. **Then the batched deletes**, oldest first, bounded per table. A table that
//!    has never been pruned is pruned in short statements autovacuum can keep up
//!    with, not one statement holding locks for minutes.
//!
//! Non-purpose: no archive tier (a row past its window is deleted, not moved), no
//! object-store work (`object_gc_sweep` owns that, and it needs a store), and no
//! events — a purge is housekeeping, never a domain effect.

use sqlx::PgPool;
use tokio::sync::watch;

use core_db::retention::{RetentionPlan, TableSweep};

/// Default cadence: hourly. Retention is housekeeping, not user-visible latency,
/// but the partition-maintenance half is what keeps INSERTs working, so this is
/// deliberately faster than the six-hourly run-ledger and outbox sweeps: a
/// quarter of the interval is cheap, and three months of headroom means the
/// maintainer can fail for a long time before anyone notices.
pub const DEFAULT_INTERVAL_SECS: u64 = 60 * 60;

/// Rows per delete statement. Small enough that one statement is a range scan
/// and not a lock, large enough that a normal interval clears its backlog.
pub const DEFAULT_BATCH: i64 = 1_000;

/// Pause after a pass that hit a table's batch ceiling. The same shape P1's
/// outbox sweep uses, and for the same reason: without a catch-up pause a
/// backlog of ten million rows drains only as fast as the interval allows,
/// while every log line says the sweep is working.
pub const CATCH_UP_PAUSE_SECS: u64 = 30;

/// Settings for the loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRetentionConfig {
    /// Seconds between passes.
    pub interval_secs: u64,
    /// Rows per delete statement.
    pub batch: i64,
    /// The resolved matrix.
    pub plan: RetentionPlan,
}

impl DataRetentionConfig {
    /// Resolves the matrix from a key lookup plus the two cadence knobs.
    ///
    /// A zero interval is a broken setting, not "sweep constantly": it falls
    /// back to the default, the same way the outbox sweep treats it.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            interval_secs: lookup("MARULA_RETENTION_SWEEP_SECS")
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|secs| *secs > 0)
                .unwrap_or(DEFAULT_INTERVAL_SECS),
            batch: lookup("MARULA_RETENTION_BATCH")
                .and_then(|v| v.parse::<i64>().ok())
                .filter(|batch| *batch > 0)
                .unwrap_or(DEFAULT_BATCH),
            plan: RetentionPlan::resolve(lookup),
        }
    }
}

/// What one pass did, for the log line and for tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassReport {
    pub tables: Vec<TableSweep>,
    /// Partitions created and dropped, per managed table.
    pub partitions: Vec<core_db::partitions::PartitionOutcome>,
    /// True when any table stopped at its batch ceiling with work waiting.
    pub backlog: bool,
    /// Tables whose partition maintenance could not run at all (not
    /// partitioned, missing, or DDL refused). Reported, never fatal: the
    /// batched deletes for other tables must still run.
    pub skipped: Vec<(&'static str, String)>,
}

impl PassReport {
    /// Total rows removed in the pass.
    #[must_use]
    pub fn deleted(&self) -> u64 {
        self.tables.iter().map(|t| t.deleted).sum()
    }
}

/// One pass: maintain partitions, then delete past-window rows table by table.
pub async fn sweep_once(pool: &PgPool, config: &DataRetentionConfig) -> anyhow::Result<PassReport> {
    let mut report = PassReport::default();

    // A partition-maintenance failure is logged and the pass continues. It is a
    // serious fault in one table (INSERTs still land in DEFAULT, so the product
    // works), but it must not stop the eight batched windows from running.
    match core_db::partitions::maintain(pool, &config.plan).await {
        Ok(outcomes) => {
            for outcome in &outcomes {
                if let Some(reason) = &outcome.skipped {
                    report.skipped.push((outcome.table, reason.clone()));
                }
                if !outcome.created.is_empty() || !outcome.dropped.is_empty() {
                    tracing::info!(
                        table = outcome.table,
                        created = ?outcome.created,
                        moved_rows = outcome.moved_rows,
                        dropped = ?outcome.dropped,
                        "partition maintenance"
                    );
                }
            }
            report.partitions = outcomes;
        }
        Err(error) => {
            tracing::error!(%error, "partition maintenance failed; batched retention continues");
        }
    }

    let tables = core_db::retention::sweep_once(pool, &config.plan, config.batch).await?;
    report.backlog = tables.iter().any(|t| t.saturated);
    // Log only tables that did something: nine "0 rows" lines an hour is noise
    // that trains an operator to stop reading the line that matters.
    for table in tables.iter().filter(|t| t.deleted > 0) {
        tracing::info!(
            table = table.table,
            deleted = table.deleted,
            "retention sweep"
        );
    }
    report.tables = tables;
    Ok(report)
}

/// The loop. A failure backs off two seconds rather than spinning, the shape the
/// other sweeps use.
pub async fn run_data_retention_sweep(
    pool: PgPool,
    config: DataRetentionConfig,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => return,
            result = sweep_once(&pool, &config) => {
                match result {
                    Ok(report) => {
                        let pause = if report.backlog {
                            CATCH_UP_PAUSE_SECS
                        } else {
                            config.interval_secs.max(60)
                        };
                        tokio::time::sleep(std::time::Duration::from_secs(pause)).await;
                    }
                    Err(error) => {
                        tracing::error!(%error, "data retention sweep failed");
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
    use std::collections::HashMap;

    fn config(vars: &[(&str, &str)]) -> DataRetentionConfig {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        DataRetentionConfig::from_lookup(|key| map.get(key).cloned())
    }

    #[test]
    fn defaults_are_the_documented_ones() {
        let cfg = config(&[]);
        assert_eq!(cfg.interval_secs, DEFAULT_INTERVAL_SECS);
        assert_eq!(cfg.batch, DEFAULT_BATCH);
        assert_eq!(
            cfg.plan.profile,
            core_db::retention::RetentionProfile::Standard
        );
    }

    /// A zero from the environment is a broken knob, not an instruction to sweep
    /// in a tight loop or to delete nothing at all.
    #[test]
    fn zero_values_fall_back_instead_of_being_honoured() {
        let cfg = config(&[
            ("MARULA_RETENTION_SWEEP_SECS", "0"),
            ("MARULA_RETENTION_BATCH", "0"),
        ]);
        assert_eq!(cfg.interval_secs, DEFAULT_INTERVAL_SECS);
        assert_eq!(cfg.batch, DEFAULT_BATCH);
    }

    /// Catch-up has to be faster than the normal cadence, or the shorter pause
    /// after a capped pass is a no-op and a backlog never drains.
    #[test]
    fn catch_up_is_faster_than_the_interval() {
        const _: () = assert!(CATCH_UP_PAUSE_SECS < DEFAULT_INTERVAL_SECS);
    }

    #[test]
    fn the_report_sums_what_each_table_deleted() {
        let report = PassReport {
            tables: vec![
                TableSweep {
                    table: "jobs",
                    deleted: 3,
                    saturated: false,
                },
                TableSweep {
                    table: "audit_events",
                    deleted: 4,
                    saturated: true,
                },
            ],
            ..PassReport::default()
        };
        assert_eq!(report.deleted(), 7);
    }
}
