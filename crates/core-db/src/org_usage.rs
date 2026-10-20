//! Org resource usage: the materialized row, its refresh, and the read
//! that serves it (the private build tracker Wave 47 lane A6, migration 0131).
//!
//! Purpose: four narrow seams.
//!   * [`get_usage`] reads the MATERIALIZED row the API serves — one
//!     indexed primary-key lookup per request, never an aggregate over
//!     the org's history.
//!   * [`refresh_orgs`] recomputes a SET of orgs' rows from the read-model
//!     view (`ops.v1_org_usage`, whose definition lives in the migration)
//!     into the table, one round-trip per batch. It is an upsert keyed on
//!     `org_usage.org_id`, so a replay lands on the same rows and updates
//!     them — the idempotency is the primary key's, not a guard's. It is the
//!     only write into the table, and the sweep calls it directly; tests
//!     call it with a one-org slice so they exercise the identical statement.
//!   * [`claim_stale_orgs`] is the sweep's work list: the orgs whose row is
//!     stale, oldest first, so a refresh pass costs proportional to what
//!     changed rather than to the org count.
//!
//! The row carries `refreshed_at` on purpose: the consumer reads it
//! alongside the counts and can see staleness instead of assuming
//! freshness. Nothing here enforces anything — there is no code path in
//! this module that refuses a write or throttles a caller because a count
//! is high. Configured limits are read (via [`list_configured_limits`]),
//! reported against, and never acted on.
//!
//! Non-purpose: no per-org API-call counter and no per-org database byte
//! estimate. Neither can be computed honestly in a shared database, so
//! neither exists here — see the header of migration 0131 for the reasoning
//! and the API for how the gap is reported.

use crate::settings;
use crate::DbResult;
use core_domain::usage::{UsageMetric, QUOTA_SETTING_PREFIX};
use sqlx::Postgres;
use sqlx::Row;
use uuid::Uuid;
/// One org's materialized usage. The column set mirrors the read-model
/// view exactly — the table is a copy of the view's row plus the time the
/// copy was taken, never a second definition.
#[derive(Debug, Clone)]
pub struct OrgUsageRow {
    pub org_id: Uuid,
    pub active_users: i64,
    pub tickets_open: i64,
    pub tickets_in_progress: i64,
    pub tickets_waiting: i64,
    pub tickets_closed: i64,
    pub managed_devices: i64,
    pub object_store_bytes: i64,
    pub automation_runs: i64,
    pub automation_runs_24h: i64,
    pub webhook_deliveries: i64,
    pub webhook_deliveries_24h: i64,
    pub audit_events_24h: i64,
    /// When this row was computed. Surfaced verbatim by the API.
    pub refreshed_at: chrono::DateTime<chrono::Utc>,
}

impl OrgUsageRow {
    /// Tickets across every category. The inventory-facing total a
    /// `usage_quota.tickets` limit is measured against.
    pub fn tickets_total(&self) -> i64 {
        self.tickets_open + self.tickets_in_progress + self.tickets_waiting + self.tickets_closed
    }

    /// The usage value this metric reports, from the materialized row.
    /// The single place a metric name meets a column: the pairing the
    /// quota surface needs to render one row per limit.
    pub fn metric_value(&self, metric: UsageMetric) -> i64 {
        match metric {
            UsageMetric::ActiveUsers => self.active_users,
            UsageMetric::Tickets => self.tickets_total(),
            UsageMetric::ManagedDevices => self.managed_devices,
            UsageMetric::ObjectStoreBytes => self.object_store_bytes,
            UsageMetric::AutomationRuns => self.automation_runs,
            UsageMetric::WebhookDeliveries => self.webhook_deliveries,
        }
    }
}

const USAGE_COLUMNS: &str = "org_id, active_users, tickets_open, tickets_in_progress, \
     tickets_waiting, tickets_closed, managed_devices, object_store_bytes, \
     automation_runs, automation_runs_24h, webhook_deliveries, \
     webhook_deliveries_24h, audit_events_24h, refreshed_at";

/// The one org's materialized usage row, or `None` when the refresh sweep
/// has never run for it (an org created a minute ago, or a worker that has
/// not ticked yet). The caller decides how to render absence.
pub async fn get_usage<'e, E>(executor: E, org_id: Uuid) -> DbResult<Option<OrgUsageRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {USAGE_COLUMNS} FROM org_usage WHERE org_id = $1");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .fetch_optional(executor)
        .await?;
    row.map(|r| row_to_usage(&r)).transpose()
}

/// Recomputes a SET of orgs' usage in one round-trip: the sweep's refresh
/// path and the only write into `org_usage`. Idempotent — `org_id` is the
/// table's primary key and the conflict target, so a replay of the same
/// batch updates the same rows rather than forking them.
///
/// Returns only the rows that were actually written: an org id in the input
/// that does not exist (the view has no row for it) simply does not come
/// back, so a batch spanning a deletion returns fewer rows than it was
/// given without failing.
///
/// One statement per batch rather than one per org: a per-org form costs
/// `N` parse+bind+execute round-trips on a single pool connection, and the
/// sweep refreshes up to 50 orgs per batch — so the batch form is a
/// 50-fold reduction in sweep wall time for the same upserts.
pub async fn refresh_orgs<'e, E>(executor: E, org_ids: &[Uuid]) -> DbResult<Vec<OrgUsageRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    if org_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        r#"INSERT INTO org_usage (
             org_id, active_users, tickets_open, tickets_in_progress,
             tickets_waiting, tickets_closed, managed_devices,
             object_store_bytes, automation_runs, automation_runs_24h,
             webhook_deliveries, webhook_deliveries_24h, audit_events_24h,
             refreshed_at)
           SELECT v.org_id, v.active_users, v.tickets_open,
                  v.tickets_in_progress, v.tickets_waiting, v.tickets_closed,
                  v.managed_devices, v.object_store_bytes, v.automation_runs,
                  v.automation_runs_24h, v.webhook_deliveries,
                  v.webhook_deliveries_24h, v.audit_events_24h, now()
             FROM ops.v1_org_usage v
            WHERE v.org_id = ANY($1)
           ON CONFLICT (org_id) DO UPDATE SET
             active_users = EXCLUDED.active_users,
             tickets_open = EXCLUDED.tickets_open,
             tickets_in_progress = EXCLUDED.tickets_in_progress,
             tickets_waiting = EXCLUDED.tickets_waiting,
             tickets_closed = EXCLUDED.tickets_closed,
             managed_devices = EXCLUDED.managed_devices,
             object_store_bytes = EXCLUDED.object_store_bytes,
             automation_runs = EXCLUDED.automation_runs,
             automation_runs_24h = EXCLUDED.automation_runs_24h,
             webhook_deliveries = EXCLUDED.webhook_deliveries,
             webhook_deliveries_24h = EXCLUDED.webhook_deliveries_24h,
             audit_events_24h = EXCLUDED.audit_events_24h,
             refreshed_at = now()
           RETURNING {USAGE_COLUMNS}"#
    );
    let rows = sqlx::query(&sql).bind(org_ids).fetch_all(executor).await?;
    rows.iter().map(row_to_usage).collect()
}

/// Up to `limit` orgs whose usage row is stale, OLDEST FIRST, and `None` for
/// an org with no row at all (`-infinity` sorts before every real
/// timestamp, so a never-measured org is by definition the stalest).
///
/// This is the whole refresh policy in one statement: the sweep decides
/// WHAT to refresh, and [`refresh_orgs`] does the work. An org that was
/// just refreshed is not in the list, so a short interval does not mean
/// repeated work on a quiet tenant.
///
/// The two arms are separate on purpose, because they need different access
/// paths and a single `LEFT JOIN … WHERE u.org_id IS NULL OR u.refreshed_at
/// < …` used NEITHER:
///   * the stale arm is a range scan on `idx_org_usage_refreshed`
///     (`refreshed_at, org_id`), which serves both the predicate and the
///     ordering;
///   * the missing arm is an anti-join over `orgs`, and it is the only
///     arm that is non-empty before the first sweep has ran (and whenever
///     a new org appears). Its cost is one PK probe per org on a table that
///     holds one row per tenant.
///
/// An `OR` across the two would block the index scan and force an ordered
/// `orgs` walk with a probe per org on EVERY sweep, including the steady
/// state where nothing is stale.
///
/// Ordering by staleness rather than by `org.id` matters when more than one
/// batch is due: a permanently-failing org is then the first thing the next
/// pass retries, instead of being served after fresher orgs because a UUID
/// happened to sort high.
pub async fn claim_stale_orgs<'e, E>(
    executor: E,
    max_age_secs: i64,
    limit: i64,
) -> DbResult<Vec<Uuid>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT org_id
             FROM (
               SELECT u.org_id, u.refreshed_at
                 FROM org_usage u
                WHERE u.refreshed_at < now() - make_interval(secs => $1::float8)
               UNION ALL
               SELECT o.id, '-infinity'::timestamptz
                 FROM orgs o
                WHERE NOT EXISTS (SELECT 1 FROM org_usage x WHERE x.org_id = o.id)
             ) due
            ORDER BY refreshed_at, org_id
            LIMIT $2"#,
    )
    .bind(max_age_secs)
    .bind(limit)
    .fetch_all(executor)
    .await?;
    Ok(rows.iter().map(|r| r.get::<Uuid, _>("org_id")).collect())
}

/// The org's configured quota limits: the metrics whose
/// `usage_quota.*` org-scope settings hold a number.
///
/// A key outside the closed family configures nothing, and a value that is
/// not an integer is skipped — a limit is a count of a real thing, so a
/// non-integer one is a provisioning error and not a limit. An org that
/// has configured nothing gets an empty list and the quota surface says
/// so outright rather than showing invented defaults.
pub async fn list_configured_limits<'e, E>(
    executor: E,
    org_id: Uuid,
) -> DbResult<Vec<(UsageMetric, i64)>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let overrides =
        settings::list_org_values_with_prefix(executor, org_id, QUOTA_SETTING_PREFIX).await?;
    let mut out: Vec<(UsageMetric, i64)> = overrides
        .iter()
        .filter_map(|o| {
            let metric = UsageMetric::from_limit_setting_key(&o.key)?;
            let limit = o.value.as_i64()?;
            Some((metric, limit))
        })
        .collect();
    // Deterministic order for a rendered table: the metric's own wire
    // order, not the order rows happen to come back from the table.
    out.sort_by_key(|(metric, _)| {
        UsageMetric::ALL
            .iter()
            .position(|m| *m == *metric)
            .unwrap_or(usize::MAX)
    });
    Ok(out)
}

fn row_to_usage(row: &sqlx::postgres::PgRow) -> DbResult<OrgUsageRow> {
    Ok(OrgUsageRow {
        org_id: row.try_get("org_id")?,
        active_users: row.try_get("active_users")?,
        tickets_open: row.try_get("tickets_open")?,
        tickets_in_progress: row.try_get("tickets_in_progress")?,
        tickets_waiting: row.try_get("tickets_waiting")?,
        tickets_closed: row.try_get("tickets_closed")?,
        managed_devices: row.try_get("managed_devices")?,
        object_store_bytes: row.try_get("object_store_bytes")?,
        automation_runs: row.try_get("automation_runs")?,
        automation_runs_24h: row.try_get("automation_runs_24h")?,
        webhook_deliveries: row.try_get("webhook_deliveries")?,
        webhook_deliveries_24h: row.try_get("webhook_deliveries_24h")?,
        audit_events_24h: row.try_get("audit_events_24h")?,
        refreshed_at: row.try_get("refreshed_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The materialized row pairs each metric with exactly one column, and
    /// the pairings the quota surface depends on are the ones that could
    /// silently drift: `tickets` is the sum of the four category columns
    /// (an inventory limit constrains the total, not one bucket).
    #[test]
    fn metric_values_pair_with_the_right_columns() {
        let row = OrgUsageRow {
            org_id: Uuid::nil(),
            active_users: 3,
            tickets_open: 10,
            tickets_in_progress: 4,
            tickets_waiting: 2,
            tickets_closed: 100,
            managed_devices: 7,
            object_store_bytes: 1024,
            automation_runs: 50,
            automation_runs_24h: 5,
            webhook_deliveries: 200,
            webhook_deliveries_24h: 20,
            audit_events_24h: 30,
            refreshed_at: chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
        };
        assert_eq!(row.tickets_total(), 116);
        assert_eq!(row.metric_value(UsageMetric::Tickets), 116);
        assert_eq!(row.metric_value(UsageMetric::ActiveUsers), 3);
        assert_eq!(row.metric_value(UsageMetric::ManagedDevices), 7);
        assert_eq!(row.metric_value(UsageMetric::ObjectStoreBytes), 1024);
        assert_eq!(row.metric_value(UsageMetric::AutomationRuns), 50);
        assert_eq!(row.metric_value(UsageMetric::WebhookDeliveries), 200);
    }
}
