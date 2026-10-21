//! core-db/retention — one matrix, one pair of enforcement mechanisms.
//!
//! Purpose: every table that grows with time says how long it keeps a row, and
//! something enforces that. Before this module the retention story was five
//! comments and two implementations: `outbox_events` and `workflow_runs` had
//! real sweeps, `workflow_runs` even had a per-org window setting, and the
//! tables that grow fastest — `audit_events`, `notifications`, the email and
//! webhook ledgers, the auth attempt log — had nothing but a promise. Two of
//! those promises were already false: migration 0088 said the attempt log was
//! "purged opportunistically" and it was, by a global delete inside the
//! login-failure function, which is a write on the unauthenticated path that a
//! brute-force run controls.
//!
//! [`MATRIX`] is the single place a window is decided. [`RetentionPlan`] turns
//! it into numbers for this process, from a named profile and optional
//! per-table overrides, and every number is clamped by a floor so a typo
//! cannot delete a year of audit evidence.
//!
//! Non-purpose: no per-tenant windows (a window is a platform policy — an org
//! setting would let one tenant's retention decision leak into another's
//! storage bill and into the erasure contract), no archive tier (a row past
//! its window is deleted, not moved), and no scheduling (`worker` owns the
//! clock; this module owns the statements).

use sqlx::PgPool;

use crate::DbResult;

/// How a window is enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mechanism {
    /// Age-predicated delete in bounded batches, oldest first. Right for a
    /// table whose rows are read, updated or keyed by something other than
    /// time.
    BatchedDelete,
    /// A monthly range partition per window, dropped whole once it expires.
    /// Right for an append-only table ordered by one timestamp with no
    /// dependent foreign key, view or unique index that would have to carry
    /// the partition key.
    ///
    /// The window is then enforced at PARTITION granularity, not by the day: a
    /// partition goes once its last day is past the window, so the oldest
    /// surviving row can be up to one partition period older than the number
    /// says. `docs/reference/data-lifecycle.md` states that bound; the number in
    /// this matrix is the target, not a promise of the exact day.
    PartitionDrop,
}

impl Mechanism {
    /// The token the docs and the sweep report use.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BatchedDelete => "batched_delete",
            Self::PartitionDrop => "partition_drop",
        }
    }
}

/// One row of the matrix: a table, three profile windows, and the floor no
/// configuration may go below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableWindow {
    /// Table name as Postgres spells it.
    pub table: &'static str,
    /// `compact` profile: the smallest install, the least disk.
    pub compact_days: i32,
    /// `standard` profile: the default, and what ships.
    pub standard_days: i32,
    /// `extended` profile: regulated installs, where a seven-year audit
    /// obligation is normal for an MSP.
    pub extended_days: i32,
    /// Rejected below this, whatever the profile or the override says.
    pub floor_days: i32,
    /// How the window is enforced.
    pub mechanism: Mechanism,
}

/// The retention matrix. Nine tables, three profiles, two mechanisms.
///
/// The numbers are a policy decision, not a derivation, so each row carries
/// the reason in the source of truth for prose:
/// `docs/reference/data-lifecycle.md`. The short version:
///
/// * `jobs` — 30 days. The dead set is the operator's work queue (R3's
///   `ops.dead_work`), so a month is the review cycle it has to survive. The
///   cancellation ledger (`dead_work_cancellations`) has no foreign key to
///   this table precisely so a decision outlives the row it was made about.
/// * `dead_events` — 90 days. Holds the raw payload of a stream entry that
///   never parsed, which is the only copy of that work.
/// * `consumer_deliveries` — 30 days, settled or dead only. A row awaiting
///   retry is never eligible, whatever its age.
/// * `notifications` — 180 days. The in-app inbox; long enough for a quarter
///   follow-up, short enough that the list stays a list.
/// * `webhook_deliveries` — 30 days. An attempt log; the retrying surface is
///   the `jobs` row, not this table.
/// * `email_outbound` — 60 days, terminal only. Tiny rows, and a provider's
///   error text is worth a couple of months.
/// * `email_messages` — one year. Thread history. Deleting a message cascades
///   its attachment ROWS, which is why the object store needs the integrity
///   sweep in the same release as this window.
/// * `auth_attempt_log` — 30 days. Security telemetry; 0088 shipped one day,
///   decided in a comment and enforced inside the login path. Thirty days is
///   what a lockout investigation actually needs, and the enforcement is now a
///   partition drop instead of a delete every attacker can trigger.
/// * `audit_events` — two years standard, seven years extended. The floor is
///   one year and the reasoning is in the doc; the mechanism is a batched
///   delete because the conversion costs an exclusive rewrite plus a recreate
///   of the `org_usage` read model, and the trigger to revisit is written
///   down.
pub const MATRIX: [TableWindow; 9] = [
    TableWindow {
        table: "jobs",
        compact_days: 7,
        standard_days: 30,
        extended_days: 90,
        floor_days: 7,
        mechanism: Mechanism::BatchedDelete,
    },
    TableWindow {
        table: "dead_events",
        compact_days: 30,
        standard_days: 90,
        extended_days: 180,
        floor_days: 30,
        mechanism: Mechanism::BatchedDelete,
    },
    TableWindow {
        table: "consumer_deliveries",
        compact_days: 14,
        standard_days: 30,
        extended_days: 60,
        floor_days: 7,
        mechanism: Mechanism::BatchedDelete,
    },
    TableWindow {
        table: "notifications",
        compact_days: 60,
        standard_days: 180,
        extended_days: 365,
        floor_days: 30,
        mechanism: Mechanism::BatchedDelete,
    },
    TableWindow {
        table: "webhook_deliveries",
        compact_days: 14,
        standard_days: 30,
        extended_days: 90,
        floor_days: 7,
        mechanism: Mechanism::BatchedDelete,
    },
    TableWindow {
        table: "email_outbound",
        compact_days: 30,
        standard_days: 60,
        extended_days: 180,
        floor_days: 14,
        mechanism: Mechanism::BatchedDelete,
    },
    TableWindow {
        table: "email_messages",
        compact_days: 180,
        standard_days: 365,
        extended_days: 730,
        floor_days: 90,
        mechanism: Mechanism::BatchedDelete,
    },
    TableWindow {
        table: "auth_attempt_log",
        compact_days: 14,
        standard_days: 30,
        extended_days: 90,
        floor_days: 7,
        mechanism: Mechanism::PartitionDrop,
    },
    TableWindow {
        table: "audit_events",
        compact_days: 400,
        standard_days: 730,
        extended_days: 2555,
        floor_days: 365,
        mechanism: Mechanism::BatchedDelete,
    },
];

/// Environment variable naming the profile.
pub const PROFILE_ENV: &str = "MARULA_RETENTION_PROFILE";

/// The named set of windows. One dial instead of nine, because an operator
/// choosing "keep more" should not have to find every table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RetentionProfile {
    /// The smallest install.
    Compact,
    /// What ships.
    #[default]
    Standard,
    /// Regulated installs (a seven-year audit window).
    Extended,
}

impl RetentionProfile {
    /// Every profile, in declaration order.
    pub const ALL: [Self; 3] = [Self::Compact, Self::Standard, Self::Extended];

    /// Token used in configuration and reports.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Standard => "standard",
            Self::Extended => "extended",
        }
    }

    /// Parses a profile token, case-insensitive.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let lower = value.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|p| p.as_str() == lower)
    }

    /// The window this profile picks for one matrix row.
    #[must_use]
    pub const fn days(self, row: &TableWindow) -> i32 {
        match self {
            Self::Compact => row.compact_days,
            Self::Standard => row.standard_days,
            Self::Extended => row.extended_days,
        }
    }
}

/// One resolved window for this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedWindow {
    pub table: &'static str,
    pub days: i32,
    pub mechanism: Mechanism,
}

/// The matrix as numbers, after the profile and the overrides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionPlan {
    pub profile: RetentionProfile,
    pub windows: Vec<ResolvedWindow>,
    /// Refused overrides and profile typos, one line each, for the boot log.
    /// A warning is never a silent adjustment: the operator sees the value
    /// that was asked for and the value that took effect.
    pub warnings: Vec<String>,
}

impl RetentionPlan {
    /// Resolves the plan from a key lookup (`MARULA_RETENTION_PROFILE` and
    /// `MARULA_RETENTION_<TABLE>_DAYS`).
    ///
    /// Floors win over every other input, including an explicit override:
    /// that is the whole point of a floor. A value below it is refused and
    /// reported, not clamped quietly, because "the operator set 30 days of
    /// audit retention and got 365" has to be visible on the first line of
    /// the boot log rather than discovered a year later.
    #[must_use]
    pub fn resolve(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let mut warnings = Vec::new();
        let profile = match lookup(PROFILE_ENV) {
            None => RetentionProfile::default(),
            Some(raw) => match RetentionProfile::parse(&raw) {
                Some(profile) => profile,
                None => {
                    warnings.push(format!(
                        "{PROFILE_ENV}={raw:?} is not a profile ({}); using standard",
                        RetentionProfile::ALL
                            .iter()
                            .map(|p| p.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                    RetentionProfile::default()
                }
            },
        };
        let mut windows = Vec::with_capacity(MATRIX.len());
        for row in &MATRIX {
            let env_name = override_env(row.table);
            let mut days = profile.days(row);
            if let Some(raw) = lookup(&env_name) {
                match raw.trim().parse::<i32>() {
                    Ok(value) if value >= row.floor_days => days = value,
                    Ok(value) => warnings.push(format!(
                        "{env_name}={value} is below the {} floor of {} days; keeping {days}",
                        row.table, row.floor_days
                    )),
                    Err(_) => warnings.push(format!(
                        "{env_name}={raw:?} is not a whole number of days; keeping {days}"
                    )),
                }
            }
            windows.push(ResolvedWindow {
                table: row.table,
                days,
                mechanism: row.mechanism,
            });
        }
        Self {
            profile,
            windows,
            warnings,
        }
    }

    /// The window for one table, if the matrix has it.
    #[must_use]
    pub fn window(&self, table: &str) -> Option<&ResolvedWindow> {
        self.windows.iter().find(|w| w.table == table)
    }
}

/// The per-table override variable: `MARULA_RETENTION_JOB...` from the table
/// name, upper-cased (`MARULA_RETENTION_AUDIT_EVENTS_DAYS`).
#[must_use]
pub fn override_env(table: &str) -> String {
    format!(
        "MARULA_RETENTION_{}_DAYS",
        table.to_ascii_uppercase().replace('-', "_")
    )
}

/// Outcome for one table in one pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableSweep {
    pub table: &'static str,
    /// Rows removed.
    pub deleted: u64,
    /// True when the pass stopped at its batch ceiling with rows still
    /// eligible — the caller shortens its pause so a backlog drains instead of
    /// trickling.
    pub saturated: bool,
}

/// Batches one table is allowed per pass before the sweeper yields.
///
/// Ten batches at the default 1,000 rows is ten thousand rows per table per
/// pass. A table that has never been pruned needs many passes, and the point of
/// the cap is that it does them in short statements that autovacuum can keep up
/// with, not one statement that holds locks for minutes.
pub const MAX_BATCHES_PER_PASS: u32 = 10;

/// One full pass over every table the plan resolves to a batched delete.
///
/// `PartitionDrop` rows are skipped: their window is enforced by the partition
/// maintainer, which has to run whether or not a row is deletable.
///
/// Runs cross-org on the owner pool, like the other housekeeping passes. On a
/// constrained role the predicates match nothing and the pass reports zero —
/// which is why the worker logs the deleted count rather than "sweep ok".
pub async fn sweep_once(
    pool: &PgPool,
    plan: &RetentionPlan,
    batch: i64,
) -> DbResult<Vec<TableSweep>> {
    let batch = batch.max(1);
    let mut out = Vec::new();
    for window in &plan.windows {
        if window.mechanism != Mechanism::BatchedDelete {
            continue;
        }
        let sql = delete_statement(window.table).expect("matrix table has SQL");
        let mut deleted = 0_u64;
        let mut saturated = false;
        for _ in 0..MAX_BATCHES_PER_PASS {
            let removed = sqlx::query(sql)
                .bind(window.days)
                .bind(batch)
                .execute(pool)
                .await?
                .rows_affected();
            deleted += removed;
            if removed < batch as u64 {
                saturated = false;
                break;
            }
            saturated = true;
        }
        out.push(TableSweep {
            table: window.table,
            deleted,
            saturated,
        });
    }
    Ok(out)
}

/// The delete for one table: a `ctid`-keyed batch so no table needs a
/// single-column key, oldest first through the age index the migration adds.
///
/// `ctid` rather than a key column because two of these tables have a composite
/// primary key and one has no single-column unique key at all. A row that moves
/// between the sub-select and the delete (a concurrent update) is simply missed
/// by this pass and caught by the next one; nothing is deleted that did not
/// satisfy the predicate.
///
/// The window is compared against `now()` inside the statement, never a
/// host-clock cutoff: the module rule is that every retention decision reads
/// the database clock.
///
/// Every predicate keeps rows that are still OWED work, whatever their age: a
/// job that has not finished, an email nobody sent yet, a delivery attempt
/// still waiting for its retry, a consumer row awaiting another attempt. Age is
/// only ever one half of the predicate.
fn delete_statement(table: &str) -> Option<&'static str> {
    let sql = match table {
        "jobs" => {
            r#"DELETE FROM jobs WHERE ctid IN (
                 SELECT ctid FROM jobs
                  WHERE status IN ('done', 'failed', 'dead')
                    AND updated_at < now() - make_interval(days => $1)
                  ORDER BY updated_at
                  LIMIT $2
               )"#
        }
        "dead_events" => {
            r#"DELETE FROM dead_events WHERE ctid IN (
                 SELECT ctid FROM dead_events
                  WHERE dead_at < now() - make_interval(days => $1)
                  ORDER BY dead_at
                  LIMIT $2
               )"#
        }
        "consumer_deliveries" => {
            r#"DELETE FROM consumer_deliveries WHERE ctid IN (
                 SELECT ctid FROM consumer_deliveries
                  WHERE (settled_at IS NOT NULL AND settled_at < now() - make_interval(days => $1))
                     OR (dead_at IS NOT NULL AND dead_at < now() - make_interval(days => $1))
                  ORDER BY coalesce(settled_at, dead_at)
                  LIMIT $2
               )"#
        }
        "notifications" => {
            r#"DELETE FROM notifications WHERE ctid IN (
                 SELECT ctid FROM notifications
                  WHERE created_at < now() - make_interval(days => $1)
                  ORDER BY created_at
                  LIMIT $2
               )"#
        }
        "webhook_deliveries" => {
            r#"DELETE FROM webhook_deliveries WHERE ctid IN (
                 SELECT ctid FROM webhook_deliveries
                  WHERE created_at < now() - make_interval(days => $1)
                  ORDER BY created_at
                  LIMIT $2
               )"#
        }
        "email_outbound" => {
            r#"DELETE FROM email_outbound WHERE ctid IN (
                 SELECT ctid FROM email_outbound
                  WHERE status IN ('sent', 'dead')
                    AND updated_at < now() - make_interval(days => $1)
                  ORDER BY updated_at
                  LIMIT $2
               )"#
        }
        "email_messages" => {
            r#"DELETE FROM email_messages WHERE ctid IN (
                 SELECT ctid FROM email_messages
                  WHERE created_at < now() - make_interval(days => $1)
                  ORDER BY created_at
                  LIMIT $2
               )"#
        }
        "audit_events" => {
            r#"DELETE FROM audit_events WHERE ctid IN (
                 SELECT ctid FROM audit_events
                  WHERE occurred_at < now() - make_interval(days => $1)
                  ORDER BY occurred_at
                  LIMIT $2
               )"#
        }
        _ => return None,
    };
    Some(sql)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn plan(vars: &[(&str, &str)]) -> RetentionPlan {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        RetentionPlan::resolve(|key| map.get(key).cloned())
    }

    /// The matrix and the delete statements have to stay in step: a row with a
    /// mechanism and no SQL deletes nothing while reporting success, which is
    /// the failure mode this whole lane exists to end.
    #[test]
    fn every_batched_row_has_a_statement_and_every_partitioned_row_does_not() {
        for row in &MATRIX {
            match row.mechanism {
                Mechanism::BatchedDelete => assert!(
                    delete_statement(row.table).is_some(),
                    "{} has a batched window but no delete statement",
                    row.table
                ),
                Mechanism::PartitionDrop => assert!(
                    delete_statement(row.table).is_none(),
                    "{} is partition-managed; a delete would fight the drop",
                    row.table
                ),
            }
        }
    }

    /// The row says nine tables have no window. Nine rows, nine names, no
    /// duplicates: the matrix IS the answer to "what does this instance keep".
    #[test]
    fn the_matrix_covers_the_nine_tables_with_no_duplicates() {
        let mut names: Vec<&str> = MATRIX.iter().map(|r| r.table).collect();
        assert_eq!(names.len(), 9);
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 9);
        for expected in [
            "jobs",
            "audit_events",
            "notifications",
            "webhook_deliveries",
            "email_outbound",
            "email_messages",
            "auth_attempt_log",
            "dead_events",
            "consumer_deliveries",
        ] {
            assert!(
                MATRIX.iter().any(|r| r.table == expected),
                "{expected} missing from the retention matrix"
            );
        }
    }

    #[test]
    fn an_unset_environment_is_the_standard_profile() {
        let plan = plan(&[]);
        assert_eq!(plan.profile, RetentionProfile::Standard);
        assert!(plan.warnings.is_empty());
        assert_eq!(plan.window("audit_events").unwrap().days, 730);
        assert_eq!(plan.window("jobs").unwrap().days, 30);
        assert_eq!(
            plan.window("auth_attempt_log").unwrap().mechanism,
            Mechanism::PartitionDrop
        );
    }

    #[test]
    fn a_profile_moves_every_window_at_once() {
        let extended = plan(&[(PROFILE_ENV, "extended")]);
        assert_eq!(extended.profile, RetentionProfile::Extended);
        assert_eq!(extended.window("audit_events").unwrap().days, 2555);
        assert_eq!(extended.window("jobs").unwrap().days, 90);
        let compact = plan(&[(PROFILE_ENV, "compact")]);
        assert_eq!(compact.window("email_messages").unwrap().days, 180);
    }

    /// The floor is the safety rail, and a refusal has to be VISIBLE: the
    /// operator asked for thirty days of audit retention and this process keeps
    /// a year, so the difference cannot be silent.
    #[test]
    fn an_override_below_the_floor_is_refused_loudly() {
        let plan = plan(&[("MARULA_RETENTION_AUDIT_EVENTS_DAYS", "30")]);
        assert_eq!(plan.window("audit_events").unwrap().days, 730);
        assert_eq!(plan.warnings.len(), 1);
        assert!(plan.warnings[0].contains("below the audit_events floor of 365"));
    }

    #[test]
    fn an_override_above_the_floor_wins_and_has_no_ceiling() {
        let plan = plan(&[("MARULA_RETENTION_AUDIT_EVENTS_DAYS", "3650")]);
        assert_eq!(plan.window("audit_events").unwrap().days, 3650);
        assert!(plan.warnings.is_empty());
    }

    #[test]
    fn a_typo_in_the_profile_name_keeps_standard_and_says_so() {
        let plan = plan(&[(PROFILE_ENV, "regulatory")]);
        assert_eq!(plan.profile, RetentionProfile::Standard);
        assert_eq!(plan.warnings.len(), 1);
        assert!(plan.warnings[0].contains("not a profile"));
    }

    #[test]
    fn a_non_numeric_override_is_reported_not_applied() {
        let plan = plan(&[("MARULA_RETENTION_JOBS_DAYS", "thirty")]);
        assert_eq!(plan.window("jobs").unwrap().days, 30);
        assert!(plan.warnings[0].contains("not a whole number of days"));
    }

    #[test]
    fn override_variable_names_follow_the_table() {
        assert_eq!(
            override_env("audit_events"),
            "MARULA_RETENTION_AUDIT_EVENTS_DAYS"
        );
        assert_eq!(override_env("jobs"), "MARULA_RETENTION_JOBS_DAYS");
    }
}
