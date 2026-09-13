//! Perf smoke (deliverable 3): fast, CI-runnable budget check.
//!
//! Contract: seeds the 50k-ticket budget dataset, then asserts the
//! ticket-list p95 budget (150ms default) across the standard request
//! mixes through the REAL RLS-scoped query path (list_tickets + the
//! batched custom-field enrichment inside one org-bound transaction).
//!
//! Gated behind `MERIDIAN_PERF_SMOKE=1` so the plain `cargo test
//! --workspace` sweep stays fast: the dedicated CI job (see
//! .github/workflows/ci.yml `perf-smoke`) and `scripts/perf-smoke.sh` set
//! the gate. Database selection mirrors the core-db suites:
//! `MERIDIAN_TEST_DATABASE_URL` → `DATABASE_URL` → a local testcontainer.
//!
//! Budget override: `PERF_SMOKE_BUDGET_MS` (default 150). CI runners are
//! noisy; the documented discipline is: a failure is investigated, not
//! shrugged off (see docs/perf.md) — overrides are for local triage.

#![allow(missing_docs)]

mod bootstrap;
use perf_harness::{measure, seed, SeedScale, PERF_ORG_ID};

fn budget_ms() -> f64 {
    std::env::var("PERF_SMOKE_BUDGET_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(150.0)
}

#[tokio::test]
async fn ticket_list_p95_within_budget_at_50k() {
    if std::env::var("MERIDIAN_PERF_SMOKE").as_deref() != Ok("1") {
        eprintln!(
            "skipped: perf smoke is env-gated (set MERIDIAN_PERF_SMOKE=1; \
             CI runs it as its own job)"
        );
        return;
    }
    let db = bootstrap::test_db().await;
    core_db::run_migrations(&db.admin)
        .await
        .expect("migrations");
    let scale = SeedScale::budget();
    let report = seed::seed(&db.admin, &scale).await.expect("seed");
    println!("seeded {} tickets in {}ms", report.tickets, report.seed_ms);

    let app = db.app().await;
    let measurement = measure::measure_ticket_list(&app, PERF_ORG_ID, seed::perf_user_id(), 5, 40)
        .await
        .expect("measure");
    for mix in &measurement.mixes {
        println!(
            "{:<24} p50 {:>7.1}ms  p95 {:>7.1}ms  p99 {:>7.1}ms  max {:>7.1}ms",
            mix.name, mix.p50_ms, mix.p95_ms, mix.p99_ms, mix.max_ms
        );
    }
    if let Some(plan) = &measurement.explain_default {
        println!("--- EXPLAIN (ANALYZE, BUFFERS) page1_default ---\n{plan}");
    }
    measure::assert_budget(&measurement, budget_ms())
        .unwrap_or_else(|e| panic!("PERF SMOKE FAIL (budget {}ms): {e}", budget_ms()));
}
