//! core-db/partitions — the maintenance a partitioned retention window needs.
//!
//! Purpose: a range partition expires by `DROP TABLE`, which is metadata work
//! instead of a delete that leaves dead tuples for autovacuum. That is only
//! true when two things hold, and nothing in Postgres provides either of them:
//!
//! 1. **A partition for the month being written must exist.** A range-partitioned
//!    table with no matching partition and no DEFAULT partition REFUSES the
//!    insert. For `auth_attempt_log` that refusal is a failed login the
//!    operator never asked for, so the table ships with a DEFAULT partition as
//!    the floor under it, and this module creates the real partition ahead of
//!    time.
//! 2. **An expired partition must be dropped.** Nothing expires on its own.
//!
//! Why a worker loop and not `pg_cron`: the wave's extension lane owns `pg_cron`
//! and it is the one extension managed Postgres most often forbids, so the
//! retention window cannot depend on it. The maintainer is a housekeeping pass
//! in the worker, which already runs on the table-owner role the DDL needs.
//!
//! Why the DEFAULT partition must be drained, not ignored: `CREATE TABLE ...
//! PARTITION OF` fails when the DEFAULT partition already holds a row in the new
//! range ("partition constraint is violated by some row"). In steady state the
//! DEFAULT is empty because partitions are created months ahead; after an
//! outage, an upgrade, or a clock that jumped, it is not. The pass therefore
//! moves rows out of DEFAULT into the partition it just created — inside one
//! transaction, so a reader never sees a row twice or zero times.
//!
//! Non-purpose: no scheduling, no cross-table coordination, and no DDL for any
//! table that is not in [`MANAGED`]. A database where the parent is not
//! partitioned at all is reported and skipped rather than repaired: rebuilding
//! a table is a migration's job, not a housekeeping pass's.

use chrono::{DateTime, Datelike, Duration, Months, NaiveDate, Utc};
use sqlx::PgPool;

use crate::{DbError, DbResult};

/// A table whose retention window is enforced by partition drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManagedPartitioned {
    /// The partitioned parent.
    pub table: &'static str,
    /// The range column every partition is keyed by.
    pub column: &'static str,
    /// Child-name prefix; the suffix is `YYYY_MM`.
    pub prefix: &'static str,
}

/// Every table the maintainer manages. Adding a row here without converting the
/// table in a migration is a no-op (the parent is not partitioned, and the pass
/// says so) rather than an error, because a half-rolled-out conversion must not
/// take the retention pass down with it.
pub const MANAGED: [ManagedPartitioned; 1] = [ManagedPartitioned {
    table: "auth_attempt_log",
    column: "occurred_at",
    prefix: "auth_attempt_log_p",
}];

/// Months of partitions to keep created ahead of the current one.
///
/// Three is a deliberate multiple of the pass's own cadence (it runs hourly by
/// default) and of any plausible operator vacation: the maintainer would have
/// to fail for a quarter before an insert had nowhere to go, and the DEFAULT
/// partition catches even that.
pub const AHEAD_MONTHS: u32 = 3;

/// One partition removed by a pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedPartition {
    pub name: String,
    /// Row estimate from `pg_class.reltuples`, not a count: counting a month of
    /// attempt rows before dropping it would cost exactly what the drop saves.
    pub estimated_rows: i64,
}

/// What one pass did to one managed table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartitionOutcome {
    pub table: &'static str,
    /// Partitions created, in calendar order.
    pub created: Vec<String>,
    /// Rows moved out of the DEFAULT partition into a real one.
    pub moved_rows: u64,
    /// Partitions dropped, oldest first.
    pub dropped: Vec<DroppedPartition>,
    /// Set when the table could not be maintained at all; the caller logs it.
    pub skipped: Option<String>,
}

/// Ensures every managed table has partitions through `AHEAD_MONTHS` and drops
/// the ones whose whole range is older than its retention window.
///
/// The window comes from the same [`crate::retention::RetentionPlan`] the
/// batched deletes read, so a table cannot expire at one age in one mechanism
/// and a different age in the other.
pub async fn maintain(
    pool: &PgPool,
    plan: &crate::retention::RetentionPlan,
) -> DbResult<Vec<PartitionOutcome>> {
    let (now, this_month): (DateTime<Utc>, NaiveDate) =
        sqlx::query_as("SELECT now(), date_trunc('month', now())::date")
            .fetch_one(pool)
            .await?;

    let mut out = Vec::with_capacity(MANAGED.len());
    for target in &MANAGED {
        let Some(window) = plan.window(target.table) else {
            out.push(PartitionOutcome {
                table: target.table,
                skipped: Some("no retention window resolved for this table".into()),
                ..PartitionOutcome::default()
            });
            continue;
        };
        out.push(maintain_one(pool, target, now, this_month, window.days).await?);
    }
    Ok(out)
}

async fn maintain_one(
    pool: &PgPool,
    target: &ManagedPartitioned,
    now: DateTime<Utc>,
    this_month: NaiveDate,
    window_days: i32,
) -> DbResult<PartitionOutcome> {
    // Is this actually a partitioned parent? A database restored from a dump
    // that predates the conversion, or a maintainer pointed at the wrong
    // table, must be reported rather than "maintained" into an error every
    // hour.
    let relkind: Option<String> =
        sqlx::query_scalar("SELECT relkind::text FROM pg_class WHERE relname = $1")
            .bind(target.table)
            .fetch_optional(pool)
            .await?;
    match relkind.as_deref() {
        Some("p") => {}
        Some(other) => {
            return Ok(PartitionOutcome {
                table: target.table,
                skipped: Some(format!("relation is {other}, not a partitioned table")),
                ..PartitionOutcome::default()
            });
        }
        None => {
            return Ok(PartitionOutcome {
                table: target.table,
                skipped: Some("relation does not exist".into()),
                ..PartitionOutcome::default()
            });
        }
    }

    let children = children(pool, target).await?;
    let cutoff = (now - Duration::days(i64::from(window_days.max(1)))).date_naive();
    let mut outcome = PartitionOutcome {
        table: target.table,
        ..PartitionOutcome::default()
    };

    // 1. Drop what has fully expired. `upper` is exclusive, so a partition whose
    //    upper bound is at or before the cutoff holds nothing younger than it.
    for child in &children {
        let Some((_, upper)) = child.month else {
            continue;
        };
        if upper <= cutoff {
            sqlx::query(&format!("DROP TABLE IF EXISTS {}", ident(&child.name)?))
                .execute(pool)
                .await?;
            outcome.dropped.push(DroppedPartition {
                name: child.name.clone(),
                estimated_rows: child.estimated_rows,
            });
        }
    }

    // 2. Ensure the months we are about to write into exist.
    for month_index in 0..=AHEAD_MONTHS {
        let Some(month) = this_month.checked_add_months(Months::new(month_index)) else {
            continue;
        };
        let name = format!("{}{:04}_{:02}", target.prefix, month.year(), month.month());
        if children.iter().any(|c| c.name == name) {
            continue;
        }
        let Some(upper) = month.checked_add_months(Months::new(1)) else {
            continue;
        };
        // A month whose range the retention window already covers (an install
        // whose clock jumped backwards, or a first run with `--ahead` larger
        // than the window) is created and immediately expired; creating it
        // first keeps the DEFAULT drain honest.
        outcome.moved_rows += create_partition(pool, target, &name, month, upper).await?;
        outcome.created.push(name);
    }

    Ok(outcome)
}

/// A child partition: its name, its month range when the name carries one, and
/// the planner's row estimate.
struct Child {
    name: String,
    month: Option<(NaiveDate, NaiveDate)>,
    estimated_rows: i64,
}

async fn children(pool: &PgPool, target: &ManagedPartitioned) -> DbResult<Vec<Child>> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        r#"SELECT child.relname, GREATEST(child.reltuples, 0)::bigint
             FROM pg_inherits i
             JOIN pg_class child ON child.oid = i.inhrelid
             JOIN pg_class parent ON parent.oid = i.inhparent
            WHERE parent.relname = $1"#,
    )
    .bind(target.table)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, estimated_rows)| Child {
            month: month_from_name(&name, target.prefix),
            name,
            estimated_rows,
        })
        .collect())
}

/// Parses `…_pYYYY_MM` into its month range. A name that does not match is not
/// ours to drop: it is either the DEFAULT partition or something an operator
/// created, and neither is safe to infer a window from.
fn month_from_name(name: &str, prefix: &str) -> Option<(NaiveDate, NaiveDate)> {
    let suffix = name.strip_prefix(prefix)?;
    let (year, month) = suffix.split_once('_')?;
    let year: i32 = year.parse().ok()?;
    let month: u32 = month.parse().ok()?;
    let start = NaiveDate::from_ymd_opt(year, month, 1)?;
    let end = start.checked_add_months(Months::new(1))?;
    Some((start, end))
}

/// Creates one partition, draining the DEFAULT partition of any rows in the new
/// range first (see the module note).
///
/// Returns the number of rows moved out of DEFAULT.
async fn create_partition(
    pool: &PgPool,
    target: &ManagedPartitioned,
    name: &str,
    lower: NaiveDate,
    upper: NaiveDate,
) -> DbResult<u64> {
    let child = ident(name)?;
    let parent = ident(target.table)?;
    let column = ident(target.column)?;
    let default_name = format!("{}_default", target.table);
    let has_default: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
             SELECT 1 FROM pg_inherits i
               JOIN pg_class child ON child.oid = i.inhrelid
               JOIN pg_class parent ON parent.oid = i.inhparent
              WHERE parent.relname = $1
                AND pg_get_expr(child.relpartbound, child.oid) = 'DEFAULT'
           )"#,
    )
    .bind(target.table)
    .fetch_one(pool)
    .await?;

    let default_ident = ident(&default_name)?;
    let lower_lit = format!("'{lower} 00:00:00+00'");
    let upper_lit = format!("'{upper} 00:00:00+00'");
    let straddles = if has_default {
        sqlx::query_scalar::<_, bool>(&format!(
            "SELECT EXISTS (SELECT 1 FROM {default_ident} WHERE {column} >= {lower_lit}::timestamptz AND {column} < {upper_lit}::timestamptz)"
        ))
        .fetch_one(pool)
        .await?
    } else {
        false
    };

    let mut tx = pool.begin().await?;
    let mut moved = 0_u64;
    if straddles {
        let tmp = "_partition_move";
        sqlx::query(&format!("DROP TABLE IF EXISTS {tmp}"))
            .execute(&mut *tx)
            .await
            .ok();
        sqlx::query(&format!(
            "CREATE TEMP TABLE {tmp} ON COMMIT DROP AS SELECT * FROM {default_ident} \
             WHERE {column} >= {lower_lit}::timestamptz AND {column} < {upper_lit}::timestamptz"
        ))
        .execute(&mut *tx)
        .await?;
        moved = sqlx::query(&format!(
            "DELETE FROM {default_ident} WHERE {column} >= {lower_lit}::timestamptz AND {column} < {upper_lit}::timestamptz"
        ))
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    sqlx::query(&format!(
        "CREATE TABLE {child} PARTITION OF {parent} \
         FOR VALUES FROM ({lower_lit}) TO ({upper_lit})"
    ))
    .execute(&mut *tx)
    .await?;
    // RLS on the child as well as the parent: a direct query against a
    // partition does not consult the parent's policy, and the RLS-coverage gate
    // reads every relation of kind 'r'. The predicate is the parent's, verbatim.
    sqlx::query(&format!("ALTER TABLE {child} ENABLE ROW LEVEL SECURITY"))
        .execute(&mut *tx)
        .await?;
    sqlx::query(&format!(
        "CREATE POLICY {child}_org ON {child} USING (org_id IS NULL OR org_id = NULLIF(current_setting('app.org_id', true), '')::uuid)"
    ))
    .execute(&mut *tx)
    .await?;
    if straddles {
        // Back through the parent, so the rows land in the partition that just
        // made room for them.
        sqlx::query(&format!(
            "INSERT INTO {parent} SELECT * FROM _partition_move"
        ))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(moved)
}

/// Guards every interpolated identifier: these names are generated from a
/// constant prefix and a calendar, never from input, but a `format!` that can
/// reach SQL earns a check that says why it is safe.
fn ident(name: &str) -> DbResult<String> {
    let ok = !name.is_empty()
        && name.len() <= 63
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if ok {
        Ok(name.to_string())
    } else {
        Err(DbError::Sql(sqlx::Error::Configuration(
            format!("refusing to interpolate identifier {name:?}").into(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_names_round_trip_to_their_month() {
        let (start, end) =
            month_from_name("auth_attempt_log_p2026_10", MANAGED[0].prefix).expect("parsed");
        assert_eq!(start, NaiveDate::from_ymd_opt(2026, 10, 1).unwrap());
        assert_eq!(end, NaiveDate::from_ymd_opt(2026, 11, 1).unwrap());
    }

    /// The DEFAULT partition and an operator's own table must never be read as
    /// expired: a name this module cannot parse is a name it does not drop.
    #[test]
    fn unparsable_names_are_not_attributed_a_month() {
        for name in [
            "auth_attempt_log_default",
            "auth_attempt_log_p2026",
            "auth_attempt_log_p2026_13",
            "other_table_p2026_10",
        ] {
            assert!(
                month_from_name(name, MANAGED[0].prefix).is_none(),
                "{name} must not yield a month"
            );
        }
    }

    /// A December partition has to roll to the next year; `month + 1` arithmetic
    /// that only knew 1..=12 would silently fail here and refuse January inserts.
    #[test]
    fn december_rolls_into_the_next_year() {
        let (start, end) =
            month_from_name("auth_attempt_log_p2026_12", MANAGED[0].prefix).expect("parsed");
        assert_eq!(start, NaiveDate::from_ymd_opt(2026, 12, 1).unwrap());
        assert_eq!(end, NaiveDate::from_ymd_opt(2027, 1, 1).unwrap());
    }

    #[test]
    fn identifiers_that_could_carry_sql_are_refused() {
        assert!(ident("auth_attempt_log_p2026_10").is_ok());
        for bad in ["", "Auth", "a;drop", "a b", "a\"b"] {
            assert!(ident(bad).is_err(), "{bad:?} must be refused");
        }
    }
}
