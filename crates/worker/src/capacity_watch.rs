//! worker/capacity_watch — emits the two signals behind the `docs/perf.md` F4
//! trigger.
//!
//! F4 measured the candidate ticket-list index, did not ship it, and wrote down
//! what would reopen the decision: page-1 p95 past about two thirds of the
//! budget, or the SQL sort plan regressing. This module is the emitter for both
//! arms, so the alert rule in `deploy/observability` reads a series instead of a
//! sentence somebody has to remember.
//!
//! The measurement itself lives in [`core_db::capacity`], next to the statement
//! it measures; this file turns one sample into metric points:
//!
//!   * `marula.db.page1_p95` (ms) always, when the sample succeeded;
//!   * `marula.db.page1_sort_index_scan` (1/0) only when a plan was read — an
//!     unreadable plan is absence, not `0`, because `0` asserts the planner
//!     materialized the sort and nobody looked.
//!
//! No loop, no interval, no cache. `docs/perf.md` F4's trigger is evaluated by a
//! scheduled job, and that job belongs to the lane that schedules work; the
//! call this module needs from it is `capacity_watch::sample(&pool)`.
//!
//! Why the sample is not on a hot path: it issues [`core_db::capacity::PAGE1_TIMED_RUNS`]
//! timed page-1 queries plus their warmups, which is a measurement, not a probe.
//! That cost is why it belongs to a slow schedule and why nothing here runs it
//! on a request.

use core_db::capacity::{self, Page1Sample};
use core_db::DbResult;
use sqlx::PgPool;

/// Takes one sample and emits both signals. `None` means there was nothing to
/// measure (no org with tickets); nothing is emitted in that case.
pub async fn sample(pool: &PgPool) -> DbResult<Option<Page1Sample>> {
    let Some(sample) = capacity::sample_page1(pool).await? else {
        return Ok(None);
    };
    core_otel::metrics::capacity_page1_p95(sample.p95_ms);
    if let Some(plan) = &sample.plan {
        core_otel::metrics::capacity_page1_sort_index_scan(plan.index_scan);
    }
    // One line per sample, with the plan's node chain: the metric says which
    // arm fired, and this says what the planner actually did about it.
    tracing::info!(
        org = %sample.org_id,
        p95_ms = sample.p95_ms,
        runs = sample.runs,
        rows = sample.rows,
        index_scan = sample.plan.as_ref().map(|p| p.index_scan),
        sort_method = sample.plan.as_ref().and_then(|p| p.sort_method.as_deref()),
        plan = sample
            .plan
            .as_ref()
            .map(|p| p.chain.join(" > "))
            .unwrap_or_default(),
        "page-1 capacity sample"
    );
    Ok(Some(sample))
}
