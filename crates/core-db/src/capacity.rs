//! core-db/capacity — the two signals behind the `docs/perf.md` F4 trigger.
//!
//! F4 measured a candidate expression index on the ticket list at 0.66 ms
//! against 31 ms for page 1 and deliberately did not ship it, because page-1
//! p95 had four to five times of headroom. The decision reopens on one of two
//! conditions, both written down in F4: page-1 p95 passing about two thirds of
//! the budget, or the SQL sort plan regressing. Until this module existed, both
//! conditions lived in a sentence and reopened by memory.
//!
//! One statement, two numbers. Both read the page-1 statement the endpoint
//! itself runs — [`crate::tickets::list_tickets`] with the default filter — so
//! neither signal can describe a query the product does not issue:
//!
//!   * [`page1_p95`] times that statement over [`PAGE1_TIMED_RUNS`] runs in
//!     org-bound transactions, the shape the perf harness measures.
//!   * [`page1_plan`] asks `ops.explain_plan` (migration 0145) for the plan of
//!     the SAME statement with its binds filled in, and [`classify_plan`] turns
//!     that jsonb into the boolean the trigger asks: is the ORDER BY still
//!     served by an ordered index, or does the planner materialize it?
//!
//! The plan text is derived from `tickets::page_sql`, not copied from it. A
//! copied EXPLAIN is how a plan signal starts watching a query nobody runs.
//!
//! Which org: the one with the most tickets, read from `org_usage` with `orgs`
//! as the fallback for an org that has not been refreshed yet. The trigger is
//! about this install's worst page 1, and an org with eleven tickets has no plan
//! worth reading — Postgres will scan it, and be right.
//!
//! Fidelity, stated rather than assumed. The sample runs on the pool the caller
//! hands it. In the worker that pool is the table owner, so RLS is in neither
//! the plan nor the timing, while F4's 31 ms was measured with RLS in the path.
//! The sample therefore reads no higher than the endpoint's page 1: a sample
//! past the trigger is a real breach, and a sample below it is not yet proof of
//! headroom.
//!
//! Non-purpose: no scheduling. The trigger is evaluated by a job, and that job
//! belongs to its own lane; this module is the callable half, and
//! [`PAGE1_BUDGET_MS`]/[`PAGE1_TRIGGER_MS`] are the decision numbers it publishes
//! so the alert rule and this file cannot drift apart.

use sqlx::PgPool;
use std::time::Instant;
use uuid::Uuid;

use crate::tickets::TicketFilter;
use crate::DbResult;

/// The page-1 product budget, in milliseconds.
///
/// `docs/perf.md` §5 states it as the HTTP budget the nightly perf gate enforces
/// (`perf-harness load --concurrency 8 --budget-ms 150`), and F4 compares its
/// SQL numbers against the same number. Restated here because a threshold in a
/// rule file and a threshold in a document drift; these constants are what the
/// alert rule was written against.
pub const PAGE1_BUDGET_MS: f64 = 150.0;

/// F4's trigger point: two thirds of the budget.
///
/// F4 wrote "about two thirds of budget" as the level where the ticket-index
/// decision reopens. Kept as its own number rather than computed, because
/// `150.0 * 2.0 / 3.0` is `99.99999999999999` in binary floating point and a
/// threshold that is off by an ulp is a threshold two people will read
/// differently. [`tests::the_trigger_is_two_thirds_of_the_budget`] keeps the
/// pair honest.
pub const PAGE1_TRIGGER_MS: f64 = 100.0;

/// The ratio F4 stated, for readers who want the sentence and not the number.
pub const PAGE1_TRIGGER_RATIO: f64 = 2.0 / 3.0;

/// Timed runs per sample. F4's numbers and the Wave 52 P5 measurements used
/// forty timed runs and five warmups; matching that is what makes a sample
/// comparable with the numbers already in `docs/perf.md`.
pub const PAGE1_TIMED_RUNS: usize = 40;

/// Warmup runs, discarded. The first page pays for pool and planner caches.
pub const PAGE1_WARMUP_RUNS: usize = 5;

/// The page size the endpoint's default filter asks for (`core-api`
/// `routes::tickets::list` clamps to 1..=200 and the UI defaults to 50).
pub const PAGE1_LIMIT: i64 = 50;

/// What the page-1 plan does with the ORDER BY.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortPlan {
    /// The sort tuple is reached through an ordered index scan: no `Sort` node
    /// is in the plan. `false` means the planner materializes the sort, which
    /// is the shape F4 recorded at 50k.
    pub index_scan: bool,
    /// Every node type on the outermost path, outermost first — e.g.
    /// `["Limit", "Sort", "Seq Scan"]`. Diagnostics for the log line and the
    /// runbook; the boolean above is the signal.
    pub chain: Vec<String>,
    /// `Sort Method` from the sort node, when there is one
    /// (`top-N heapsort`, `quicksort`, `external merge Disk: 2048kB`). A disk
    /// sort is the cheapest way to recognize the sort outgrowing `work_mem`.
    pub sort_method: Option<String>,
}

/// One page-1 sample: the latency arm and the plan arm, from one statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Page1Sample {
    /// The org the sample was taken against (the install's largest).
    pub org_id: Uuid,
    /// p95 of the timed runs, in milliseconds.
    pub p95_ms: f64,
    /// Timed runs the p95 was computed over.
    pub runs: usize,
    /// Rows the page returned, per run — the page shape is part of the sample
    /// because a page that shrank is a plan change nobody needs an EXPLAIN to
    /// explain.
    pub rows: usize,
    /// The plan classification, or `None` when `ops.explain_plan` could not be
    /// read (schema not migrated yet, function not executable by this role).
    /// Absence is reported as absence: a plan signal pinned at `0` would claim
    /// the sort was materialized when nobody looked.
    pub plan: Option<SortPlan>,
}

/// The page-1 filter: no filters, first page, the UI's page size.
#[must_use]
pub fn page1_filter() -> TicketFilter {
    TicketFilter {
        board_id: None,
        assignee_id: None,
        client_id: None,
        category: None,
        priority: None,
        tag_ids: None,
        limit: PAGE1_LIMIT,
        offset: 0,
        cursor: None,
    }
}

/// The page-1 statement with its binds filled in, ready for `EXPLAIN`.
///
/// Built from [`crate::tickets::page_sql`] so it is the product's statement and
/// not a copy of it. The binds are replaced highest-first so `$1` cannot match
/// the front of `$10`, and the NULLs keep their original casts
/// (`$2::uuid` becomes `NULL::uuid`), which is what lets the planner fold the
/// page-1 filter to a constant exactly as it does for bound NULLs.
#[must_use]
pub fn page1_explain_sql(org_id: Uuid) -> String {
    let mut sql = crate::tickets::page_sql(&page1_filter());
    let org_literal = format!("'{org_id}'::uuid");
    let limit = PAGE1_LIMIT.to_string();
    for (bind, literal) in [
        ("$8", "0"),
        ("$7", limit.as_str()),
        ("$6", "NULL"),
        ("$5", "NULL"),
        ("$4", "NULL"),
        ("$3", "NULL"),
        ("$2", "NULL"),
        ("$1", org_literal.as_str()),
    ] {
        sql = sql.replace(bind, literal);
    }
    sql
}

/// Classifies one `EXPLAIN (FORMAT JSON)` plan as [`SortPlan`].
///
/// `plan` is the object `ops.explain_plan` returns, i.e. `(EXPLAIN ...) -> 0`,
/// so the first key is `Plan`. Returns `None` when the value is not a plan at
/// all — an empty object, a truncated read, a different function's output —
/// because "I could not read the plan" and "the plan sorts" must not be the
/// same series.
///
/// The walk follows the first child at each level. The page-1 statement has one
/// scan under one sort; a plan with a join under it would be a different
/// statement, and following one branch is still better than guessing at the
/// rest.
#[must_use]
pub fn classify_plan(plan: &serde_json::Value) -> Option<SortPlan> {
    // A plan nested deeper than this is not a Postgres plan for this query; the
    // cap keeps a hostile or malformed value from walking a deep tree.
    const MAX_DEPTH: usize = 64;

    let mut node = plan.get("Plan")?;
    let mut chain: Vec<String> = Vec::new();
    let mut sort_method: Option<String> = None;
    loop {
        if chain.len() >= MAX_DEPTH {
            return None;
        }
        let node_type = node.get("Node Type")?.as_str()?.to_string();
        if sort_method.is_none() {
            if let Some(method) = node.get("Sort Method").and_then(|v| v.as_str()) {
                sort_method = Some(method.to_string());
            }
        }
        chain.push(node_type);
        match node.get("Plans").and_then(|p| p.as_array()) {
            Some(children) if !children.is_empty() => node = &children[0],
            _ => break,
        }
    }
    if chain.is_empty() {
        return None;
    }
    // `Incremental Sort` is a sort: it materializes whatever the index order did
    // not already give it, so the ORDER BY is not served the way F4's index
    // would serve it.
    let sorts = chain
        .iter()
        .any(|node| node == "Sort" || node == "Incremental Sort");
    let leaf = chain.last().map(String::as_str).unwrap_or_default();
    let ordered_scan = leaf == "Index Scan" || leaf == "Index Only Scan";
    Some(SortPlan {
        index_scan: !sorts && ordered_scan,
        chain,
        sort_method,
    })
}

/// The org whose page 1 costs the most, and its ticket count from `org_usage`.
///
/// `-1` means the org has no usage row yet (a fresh install, before the first
/// refresh) — unknown, not zero, so the caller still measures. `orgs` is the
/// source of the candidate set so an install with no usage rows is sampled at
/// all; `org_usage` only ranks it.
async fn largest_org(pool: &PgPool) -> DbResult<Option<(Uuid, i64)>> {
    let row: Option<(Uuid, i64)> = sqlx::query_as(
        r#"SELECT o.id,
                  COALESCE(u.tickets_open + u.tickets_in_progress
                           + u.tickets_waiting + u.tickets_closed, -1) AS tickets
             FROM orgs o
             LEFT JOIN org_usage u ON u.org_id = o.id
            ORDER BY tickets DESC, o.id
            LIMIT 1"#,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// One page-1 request through the endpoint's own path, returning its row count.
async fn page1_run(pool: &PgPool, org_id: Uuid) -> DbResult<usize> {
    let mut tx = pool.begin().await?;
    crate::set_org(&mut tx, org_id).await?;
    let rows = crate::tickets::list_tickets(&mut *tx, org_id, page1_filter()).await?;
    tx.commit().await?;
    Ok(rows.len())
}

/// p95 of the page-1 statement, over [`PAGE1_TIMED_RUNS`] runs, with the page
/// size the statement returned.
///
/// `None` when the page comes back empty: there is no page-1 latency to report
/// for an org with no tickets, and a number computed over an empty result would
/// be the cheapest series on the dashboard and the least meaningful. The check
/// rides the first warmup run, so it costs one query rather than the whole set.
pub async fn page1_p95(pool: &PgPool, org_id: Uuid) -> DbResult<Option<(f64, usize, usize)>> {
    let mut last_rows = 0;
    for _ in 0..PAGE1_WARMUP_RUNS {
        last_rows = page1_run(pool, org_id).await?;
    }
    if last_rows == 0 {
        return Ok(None);
    }
    let mut samples = Vec::with_capacity(PAGE1_TIMED_RUNS);
    let mut rows = last_rows;
    for _ in 0..PAGE1_TIMED_RUNS {
        let start = Instant::now();
        rows = page1_run(pool, org_id).await?;
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        if rows == 0 {
            return Ok(None);
        }
    }
    Ok(Some((percentile(&mut samples, 0.95), samples.len(), rows)))
}

/// The page-1 plan, classified; `None` when the plan cannot be read.
pub async fn page1_plan(pool: &PgPool, org_id: Uuid) -> DbResult<Option<SortPlan>> {
    let sql = page1_explain_sql(org_id);
    let plan: Option<serde_json::Value> = match sqlx::query_scalar("SELECT ops.explain_plan($1)")
        .bind(&sql)
        .fetch_optional(pool)
        .await
    {
        Ok(plan) => plan,
        Err(error) => {
            // "Not installed" and "broken" are different sentences. A database
            // that has not run migration 0145 has no `ops` schema and no
            // function, which is an absent signal and not a failed sample: the
            // latency half is still worth reporting, so this returns `None` and
            // says so quietly. Anything else — a dropped connection, a
            // permission change, SQL this code got wrong — is a warning,
            // because the next thing that happens is a silent loss of the plan
            // arm.
            let not_installed = error
                .as_database_error()
                .is_some_and(|db| matches!(db.code().as_deref(), Some("42883") | Some("3F000")));
            if not_installed {
                tracing::debug!(%error, "page-1 plan not readable; plan signal absent");
            } else {
                tracing::warn!(%error, "page-1 plan read failed; plan signal absent");
            }
            return Ok(None);
        }
    };
    Ok(plan.as_ref().and_then(classify_plan))
}

/// Both signals from one statement: the install's largest org, the timed p95,
/// and the plan.
///
/// `None` when the install has nothing to measure — no orgs, or an org with no
/// tickets. The caller (the worker's `capacity_watch`) turns `Some` into two
/// metric samples and `None` into no series at all.
pub async fn sample_page1(pool: &PgPool) -> DbResult<Option<Page1Sample>> {
    let Some((org_id, tickets)) = largest_org(pool).await? else {
        return Ok(None);
    };
    if tickets == 0 {
        tracing::debug!(%org_id, "page-1 sample skipped: org has no tickets");
        return Ok(None);
    }
    let Some((p95_ms, runs, rows)) = page1_p95(pool, org_id).await? else {
        tracing::debug!(%org_id, "page-1 sample skipped: the page came back empty");
        return Ok(None);
    };
    let plan = page1_plan(pool, org_id).await?;
    Ok(Some(Page1Sample {
        org_id,
        p95_ms,
        runs,
        rows,
        plan,
    }))
}

/// Nearest-rank percentile over an unsorted sample, in `[0, 1]`.
///
/// Same rule as the perf harness (`perf-harness::measure::percentile`) so a
/// sampled p95 and a measured p95 mean the same thing.
fn percentile(samples: &mut [f64], p: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN latencies"));
    let idx = (p * (samples.len() as f64 - 1.0)).round() as usize;
    samples[idx.min(samples.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The shape F4 recorded at 50k: a bounded top-N heapsort over the org's
    /// rows. This is what `index_scan: false` means, and it is the baseline the
    /// opt-in plan rule is written against.
    fn f4_baseline_plan() -> serde_json::Value {
        json!({
            "Plan": {
                "Node Type": "Limit",
                "Plan Rows": 50,
                "Plans": [{
                    "Node Type": "Sort",
                    "Sort Key": ["CASE ...", "due_at", "created_at", "id"],
                    "Sort Method": "top-N heapsort",
                    "Sort Space Used": 39,
                    "Plans": [{
                        "Node Type": "Seq Scan",
                        "Relation Name": "tickets",
                        "Filter": "(org_id = '...'::uuid)"
                    }]
                }]
            }
        })
    }

    #[test]
    fn the_f4_baseline_plan_is_a_materialized_sort() {
        let plan = classify_plan(&f4_baseline_plan()).expect("a plan");
        assert!(!plan.index_scan, "a top-N heapsort is not an index scan");
        assert_eq!(plan.sort_method.as_deref(), Some("top-N heapsort"));
        assert_eq!(plan.chain, vec!["Limit", "Sort", "Seq Scan"]);
    }

    /// The plan F4's candidate index produces: the same ORDER BY, reached
    /// through the index, with no Sort node. `index_scan: true` is the state the
    /// plan rule expects once that index ships.
    #[test]
    fn an_ordered_index_scan_is_the_index_arm() {
        let plan = classify_plan(&json!({
            "Plan": {
                "Node Type": "Limit",
                "Plans": [{
                    "Node Type": "Index Scan",
                    "Index Name": "idx_tickets_page1_order",
                    "Sort Key": null
                }]
            }
        }))
        .expect("a plan");
        assert!(plan.index_scan);
        assert_eq!(plan.sort_method, None);
        assert_eq!(plan.chain, vec!["Limit", "Index Scan"]);
    }

    /// An incremental sort rides an index and still materializes the rest of the
    /// tuple, so it is not the shape the trigger is asking about.
    #[test]
    fn an_incremental_sort_is_not_an_index_scan() {
        let plan = classify_plan(&json!({
            "Plan": {
                "Node Type": "Incremental Sort",
                "Sort Method": "quicksort",
                "Plans": [{ "Node Type": "Index Scan" }]
            }
        }))
        .expect("a plan");
        assert!(!plan.index_scan);
        assert_eq!(
            plan.chain,
            vec!["Incremental Sort", "Index Scan"],
            "the chain keeps what the plan did, for the log line"
        );
    }

    #[test]
    fn a_disk_sort_is_visible_in_the_method() {
        let plan = classify_plan(&json!({
            "Plan": {
                "Node Type": "Sort",
                "Sort Method": "external merge  Disk: 2048kB",
                "Plans": [{ "Node Type": "Seq Scan" }]
            }
        }))
        .expect("a plan");
        assert!(!plan.index_scan);
        assert!(plan
            .sort_method
            .as_deref()
            .is_some_and(|m| m.contains("Disk")));
    }

    #[test]
    fn a_value_that_is_not_a_plan_is_absent_not_zero() {
        for value in [json!({}), json!({"Plan": {}}), json!(null), json!([])] {
            assert_eq!(classify_plan(&value), None, "{value} must not classify");
        }
    }

    #[test]
    fn the_explain_statement_carries_no_binds() {
        let org = Uuid::nil();
        let sql = page1_explain_sql(org);
        assert!(
            !sql.contains('$'),
            "every bind must be filled in or EXPLAIN cannot run:\n{sql}"
        );
        assert!(sql.contains(&format!("'{org}'::uuid")));
        assert!(sql.contains("LIMIT 50 OFFSET 0"));
        assert!(sql.contains("ORDER BY"));
        // The product's own sort tuple, not a copy of it: the tiebreaker is the
        // part a hand-written EXPLAIN tends to lose.
        assert!(sql.contains("due_at NULLS LAST, created_at, id"));
    }

    /// The threshold is F4's "about two thirds of budget", and it is the number
    /// the alert rule was written against. If either constant moves, the rule
    /// comment is stale and this fails first.
    #[test]
    fn the_trigger_is_two_thirds_of_the_budget() {
        assert_eq!(PAGE1_BUDGET_MS, 150.0);
        assert_eq!(PAGE1_TRIGGER_MS, 100.0);
        assert!((PAGE1_BUDGET_MS * PAGE1_TRIGGER_RATIO - PAGE1_TRIGGER_MS).abs() < 0.001);
        const { assert!(PAGE1_TRIGGER_MS < PAGE1_BUDGET_MS) };
    }

    #[test]
    fn percentile_is_nearest_rank_over_the_sorted_sample() {
        let mut samples: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&mut samples, 0.95), 95.0);
        let mut single = vec![7.5];
        assert_eq!(percentile(&mut single, 0.95), 7.5);
    }
}
