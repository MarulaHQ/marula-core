//! Ticket-list p95 measurement (deliverable 1 — measure first).
//!
//! Measures the EXACT server-side query path of `GET /api/tickets`
//! (crates/core-api/src/routes/tickets.rs `list`):
//!
//!   tx.begin → set_org_and_user → core_db::tickets::list_tickets
//!            → core_db::custom_fields::list_values_many → commit
//!
//! i.e. both SQL round-trips inside one RLS-scoped transaction, minus HTTP,
//! auth and JSON serialization. The HTTP end-to-end numbers come from the
//! load runner (`load::run`) against a booted core-api; the SQL path here
//! is what the CI perf smoke enforces the budget on (CI has no server to
//! boot and the SQL is the part the budget exists to protect).
//!
//! Latencies are collected per request mix (page-1 default, priority,
//! assignee, client, deep-page), warmup runs are discarded, and p50/p95/p99
//! are computed over the timed runs. `explain_page1` captures
//! `EXPLAIN (ANALYZE, BUFFERS)` for the page-1 mix so baseline reports
//! carry plan evidence, not just numbers.

use anyhow::Context;
use chrono::Utc;
use core_domain::status::Priority;
use sqlx::{PgPool, Postgres, Row, Transaction};
use std::time::Instant;
use uuid::Uuid;

use crate::seed::md5_uuid;

/// Latency summary in milliseconds for one request mix.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MixResult {
    pub name: String,
    pub runs: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    /// Mean over timed runs — included for trend tracking only; budgets
    /// are p95.
    pub mean_ms: f64,
}

/// Full measurement output for one seeded org.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MeasureReport {
    pub org_id: Uuid,
    pub ticket_count: i64,
    pub measured_at: chrono::DateTime<Utc>,
    pub warmup_runs: usize,
    pub timed_runs: usize,
    pub mixes: Vec<MixResult>,
    /// EXPLAIN (ANALYZE, BUFFERS) text for the page-1 default mix.
    pub explain_default: Option<String>,
}

/// The page shape every mix shares: the endpoint clamps limit to 1..=200
/// and the UI defaults to a 50-row page.
fn filter(name: &'static str, f: impl FnOnce(&mut core_db::tickets::TicketFilter)) -> Named {
    let mut filter = core_db::tickets::TicketFilter {
        board_id: None,
        assignee_id: None,
        client_id: None,
        category: None,
        priority: None,
        tag_ids: None,
        limit: 50,
        offset: 0,
        cursor: None,
    };
    f(&mut filter);
    Named { name, filter }
}

/// One named request mix (private type; `mixes` builds the standard set).
pub(crate) struct Named {
    pub name: &'static str,
    pub filter: core_db::tickets::TicketFilter,
}

/// The standard mix set. `assignee`/`client` target deterministic seeded
/// rows (harness user 1, client 1). The keyset deep-page mix is appended
/// by [`measure_ticket_list`] once it has resolved an anchor at depth
/// 40k — same depth the offset mix walks.
pub(crate) fn mixes(assignee: Uuid, client: Uuid) -> Vec<Named> {
    vec![
        filter("page1_default", |_| {}),
        filter("priority_p1", |f| f.priority = Some(Priority::P1)),
        filter("assignee_mine", |f| f.assignee_id = Some(assignee)),
        filter("client_scoped", |f| f.client_id = Some(client)),
        filter("deep_page_offset40k", |f| f.offset = 40_000),
    ]
}

/// Resolves the sort-tuple anchor of the row at `depth` — the keyset
/// cursor the deep-page mix resumes from. One untimed setup query (the
/// very O(offset) scan the cursor exists to avoid), fetched through the
/// plain offset path under the same RLS-scoped transaction the endpoint
/// uses.
async fn keyset_anchor(
    tx: &mut Transaction<'_, Postgres>,
    org_id: Uuid,
    user_id: Uuid,
    depth: i64,
) -> anyhow::Result<Option<core_db::tickets::TicketCursor>> {
    core_db::set_org_and_user(tx, org_id, user_id)
        .await
        .context("set org GUC (keyset anchor)")?;
    let mut page = core_db::tickets::list_tickets(
        &mut **tx,
        org_id,
        core_db::tickets::TicketFilter {
            limit: 1,
            offset: depth,
            cursor: None,
            ..empty_filter()
        },
    )
    .await?;
    if page.is_empty() {
        return Ok(None);
    }
    let t = page.remove(0);
    Ok(Some(core_db::tickets::TicketCursor {
        priority_rank: match t.priority {
            Priority::P1 => 0,
            Priority::P2 => 1,
            Priority::P3 => 2,
            Priority::P4 => 3,
        },
        due_at: t.due_at,
        created_at: t.created_at,
        id: t.id,
    }))
}

fn empty_filter() -> core_db::tickets::TicketFilter {
    core_db::tickets::TicketFilter {
        board_id: None,
        assignee_id: None,
        client_id: None,
        category: None,
        priority: None,
        tag_ids: None,
        limit: 0,
        offset: 0,
        cursor: None,
    }
}

/// The same two queries the endpoint issues, timed end-to-end within one
/// RLS-scoped transaction. Returns the page size actually returned.
async fn one_request(
    tx: &mut Transaction<'_, Postgres>,
    org_id: Uuid,
    user_id: Uuid,
    f: &core_db::tickets::TicketFilter,
) -> anyhow::Result<usize> {
    core_db::set_org_and_user(tx, org_id, user_id)
        .await
        .context("set org GUC")?;
    let rows = core_db::tickets::list_tickets(&mut **tx, org_id, clone_filter(f)).await?;
    let ids: Vec<Uuid> = rows.iter().map(|t| t.id).collect();
    let values = core_db::custom_fields::list_values_many(&mut **tx, org_id, &ids).await?;
    let _ = values;
    Ok(rows.len())
}

fn clone_filter(f: &core_db::tickets::TicketFilter) -> core_db::tickets::TicketFilter {
    core_db::tickets::TicketFilter {
        board_id: f.board_id,
        assignee_id: f.assignee_id,
        client_id: f.client_id,
        category: f.category,
        priority: f.priority,
        tag_ids: f.tag_ids.clone(),
        limit: f.limit,
        offset: f.offset,
        cursor: f.cursor,
    }
}

/// Measures all mixes against a seeded perf org.
pub async fn measure_ticket_list(
    app: &PgPool,
    org_id: Uuid,
    user_id: Uuid,
    warmup_runs: usize,
    timed_runs: usize,
) -> anyhow::Result<MeasureReport> {
    // Count INSIDE an org-GUC transaction: the app pool is default-deny
    // under RLS, so an unscoped count reads zero rows.
    let ticket_count = {
        let mut tx = app.begin().await?;
        core_db::set_org(&mut tx, org_id)
            .await
            .context("set org GUC for count")?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM tickets")
            .fetch_one(&mut *tx)
            .await?;
        count
    };

    let mut results = Vec::new();
    for mix in mixes(user_id, md5_uuid("perf-client-1")) {
        // Warmup: pool + planner caches, then discarded.
        for _ in 0..warmup_runs {
            let mut tx = app.begin().await?;
            one_request(&mut tx, org_id, user_id, &mix.filter).await?;
            tx.commit().await?;
        }
        let mut samples = Vec::with_capacity(timed_runs);
        for _ in 0..timed_runs {
            let start = Instant::now();
            let mut tx = app.begin().await?;
            let rows = one_request(&mut tx, org_id, user_id, &mix.filter).await?;
            tx.commit().await?;
            anyhow::ensure!(rows > 0, "mix '{}' returned an empty page", mix.name);
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        results.push(summarize(mix.name, &samples));
    }

    // Keyset deep-page mix: the Phase-4 product fix, measured against the
    // SAME depth the offset mix walks. The anchor is the sort tuple of the
    // row at 40k (one untimed setup query); the timed runs resume strictly
    // after it — the cost that MUST be flat because this is the walk
    // every deep-page client is expected to use going forward.
    let anchor = {
        let mut tx = app.begin().await?;
        keyset_anchor(&mut tx, org_id, user_id, 40_000).await?
    };
    if let Some(anchor) = anchor {
        let mix = filter("deep_page_keyset40k", |_| {});
        let mut keyset_filter = mix.filter;
        keyset_filter.cursor = Some(anchor);
        for _ in 0..warmup_runs {
            let mut tx = app.begin().await?;
            one_request(&mut tx, org_id, user_id, &keyset_filter).await?;
            tx.commit().await?;
        }
        let mut samples = Vec::with_capacity(timed_runs);
        for _ in 0..timed_runs {
            let start = Instant::now();
            let mut tx = app.begin().await?;
            let rows = one_request(&mut tx, org_id, user_id, &keyset_filter).await?;
            tx.commit().await?;
            anyhow::ensure!(rows > 0, "mix '{}' returned an empty page", mix.name);
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        results.push(summarize(mix.name, &samples));
    }

    let explain_default = explain_page1(app, org_id, user_id).await;

    // Search mixes (Wave 52 lane P5): the same three shapes the ticket list
    // measures, over the indexed corpus the seeder now writes. They share a
    // budget namespace with the ticket mixes because the claim this lane
    // makes — "keyset is flat at depth" — has to hold for search too.
    results.extend(measure_search(app, org_id, user_id, warmup_runs, timed_runs).await?);

    Ok(MeasureReport {
        org_id,
        ticket_count,
        measured_at: Utc::now(),
        warmup_runs,
        timed_runs,
        mixes: results,
        explain_default,
    })
}

/// The query the search mixes run. One common term ("network") is in every
/// seeded title, so a deep page is a deep page rather than an empty one.
const SEARCH_Q: &str = "network";
const SEARCH_DEPTH: i64 = 40_000;

/// One search request through the real query path: the endpoint's own
/// `search_page` / `search_page_after`, inside the same RLS-scoped
/// transaction the handler opens.
async fn one_search_request(
    tx: &mut Transaction<'_, Postgres>,
    org_id: Uuid,
    user_id: Uuid,
    limit: i64,
    offset: i64,
    cursor: Option<core_db::search::SearchCursor>,
) -> anyhow::Result<usize> {
    core_db::set_org_and_user(tx, org_id, user_id)
        .await
        .context("set org GUC")?;
    let kinds = ["ticket".to_string()];
    let hits = match cursor {
        Some(anchor) => {
            core_db::search::search_page_after(
                &mut **tx,
                org_id,
                &kinds,
                SEARCH_Q,
                false,
                None,
                limit,
                Some(anchor),
            )
            .await?
        }
        None => {
            core_db::search::search_page(
                &mut **tx, org_id, &kinds, SEARCH_Q, false, None, limit, offset,
            )
            .await?
        }
    };
    Ok(hits.len())
}

/// Search mixes: first page, the legacy deep offset, and the keyset walk at
/// the same depth. The anchor is resolved once, untimed, through the very
/// offset scan the cursor exists to avoid.
async fn measure_search(
    app: &PgPool,
    org_id: Uuid,
    user_id: Uuid,
    warmup_runs: usize,
    timed_runs: usize,
) -> anyhow::Result<Vec<MixResult>> {
    const LIMIT: i64 = 25;
    let mut out = Vec::new();

    for (name, offset) in [
        ("search_page1", 0_i64),
        ("search_deep_offset40k", SEARCH_DEPTH),
    ] {
        for _ in 0..warmup_runs {
            let mut tx = app.begin().await?;
            one_search_request(&mut tx, org_id, user_id, LIMIT, offset, None).await?;
            tx.commit().await?;
        }
        let mut samples = Vec::with_capacity(timed_runs);
        for _ in 0..timed_runs {
            let start = Instant::now();
            let mut tx = app.begin().await?;
            let rows = one_search_request(&mut tx, org_id, user_id, LIMIT, offset, None).await?;
            tx.commit().await?;
            anyhow::ensure!(rows > 0, "mix '{name}' returned an empty page");
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        out.push(summarize(name, &samples));
    }

    // Anchor at the same depth the offset mix walks.
    let anchor = {
        let mut tx = app.begin().await?;
        core_db::set_org_and_user(&mut tx, org_id, user_id)
            .await
            .context("set org GUC (search keyset anchor)")?;
        let hits = core_db::search::search_page(
            &mut *tx,
            org_id,
            &["ticket".to_string()],
            SEARCH_Q,
            false,
            None,
            1,
            SEARCH_DEPTH,
        )
        .await?;
        hits.first().map(|h| core_db::search::SearchCursor {
            score: h.score,
            indexed_at: h.indexed_at,
            entity_id: h.entity_id,
        })
    };
    if let Some(anchor) = anchor {
        for _ in 0..warmup_runs {
            let mut tx = app.begin().await?;
            one_search_request(&mut tx, org_id, user_id, LIMIT, 0, Some(anchor)).await?;
            tx.commit().await?;
        }
        let mut samples = Vec::with_capacity(timed_runs);
        for _ in 0..timed_runs {
            let start = Instant::now();
            let mut tx = app.begin().await?;
            let rows = one_search_request(&mut tx, org_id, user_id, LIMIT, 0, Some(anchor)).await?;
            tx.commit().await?;
            anyhow::ensure!(rows > 0, "mix 'search_keyset40k' returned an empty page");
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        out.push(summarize("search_keyset40k", &samples));
    }

    Ok(out)
}

/// Fails when a mix's p95 exceeds its budget — the gate the smoke test and
/// the CLI enforce.
///
/// Budgets (docs/perf.md): the PRIMARY ticket-list budget is 150ms p95 and
/// applies to every mix. Since keyset pagination shipped (Wave 15 lane 3,
/// `deep_page_keyset40k`), the deep-page walk IS budgeted like every other
/// shape — it is the coupon that made the offset tripwire enforceable
/// (F2). `deep_page_offset40k` stays on its own headroom budget: legacy
/// OFFSET clients keep regression coverage but the budget they hit is not
/// the product budget.
pub fn assert_budget(report: &MeasureReport, budget_ms: f64) -> anyhow::Result<()> {
    for mix in &report.mixes {
        // The legacy OFFSET shapes keep regression coverage on their own
        // headroom budget: clients on that path are mid-migration, and the
        // product budget belongs to the keyset walk (F2).
        let mix_budget = if matches!(
            mix.name.as_str(),
            "deep_page_offset40k" | "search_deep_offset40k"
        ) {
            DEEP_PAGE_BUDGET_MS
        } else {
            budget_ms
        };
        anyhow::ensure!(
            mix.p95_ms <= mix_budget,
            "p95 budget exceeded: mix '{}' p95 {:.1}ms > {:.1}ms budget \
             (@{} tickets/org)",
            mix.name,
            mix.p95_ms,
            mix_budget,
            report.ticket_count
        );
    }
    Ok(())
}

/// Deep-page headroom (regression tripwire, not a product budget): measured
/// baseline ~237ms p95 locally; the 400ms gate catches order-of-magnitude
/// regressions without flaking on OFFSET cost.
pub const DEEP_PAGE_BUDGET_MS: f64 = 400.0;

fn percentile(sorted: &[f64], p: f64) -> f64 {
    // Nearest-rank on the sorted sample; p in [0,1].
    let idx = (p * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn summarize(name: &str, samples: &[f64]) -> MixResult {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("no NaN latencies"));
    let mean = sorted.iter().sum::<f64>() / sorted.len() as f64;
    MixResult {
        name: name.to_string(),
        runs: sorted.len(),
        p50_ms: percentile(&sorted, 0.50),
        p95_ms: percentile(&sorted, 0.95),
        p99_ms: percentile(&sorted, 0.99),
        max_ms: sorted[sorted.len() - 1],
        mean_ms: mean,
    }
}

/// EXPLAIN (ANALYZE, BUFFERS) of the page-1 default query, replicating
/// `list_tickets`'s SQL (that function interpolates TICKET_COLS, so the
/// EXPLAIN uses the same predicate/sort text to keep the plan honest).
async fn explain_page1(app: &PgPool, org_id: Uuid, user_id: Uuid) -> Option<String> {
    let mut tx = app.begin().await.ok()?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .ok()?;
    let sql = "EXPLAIN (ANALYZE, BUFFERS) \
               SELECT t.* FROM tickets t \
               WHERE org_id = $1 \
                 AND ($2::uuid IS NULL OR board_id = $2) \
                 AND ($3::uuid IS NULL OR assignee_id = $3) \
                 AND ($4::uuid IS NULL OR client_id = $4) \
                 AND ($5::text IS NULL OR category = $5) \
                 AND ($6::text IS NULL OR priority = $6) \
               ORDER BY \
                 CASE priority WHEN 'p1' THEN 0 WHEN 'p2' THEN 1 WHEN 'p3' THEN 2 ELSE 3 END, \
                 due_at NULLS LAST, created_at \
               LIMIT $7 OFFSET $8";
    let rows = sqlx::query(sql)
        .bind(org_id)
        .bind(Option::<Uuid>::None)
        .bind(Option::<Uuid>::None)
        .bind(Option::<Uuid>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(50i64)
        .bind(0i64)
        .fetch_all(&mut *tx)
        .await
        .ok()?;
    tx.commit().await.ok()?;
    let plan = rows
        .iter()
        .map(|r| r.try_get::<String, _>(0).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    Some(plan)
}
