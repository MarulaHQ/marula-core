//! One load run at the ADR's anchor.
//!
//! ADR 0023 Decision 2 measures the boundary at the anchor's queue depth
//! (`anchor_ladder.rs`). This file measures the other half the ADR's guardrail
//! arithmetic assumes: one plugin invocation on **every** request, at the
//! anchor's recorded rate — 695.3 requests/second, p50 25.9 ms, p95 93.2 ms,
//! p99 294.8 ms (`docs/perf.md:369`).
//!
//! Two facts make that a real load test rather than a loop:
//!
//! - the rate budget is per install (300/min sustained, burst 60,
//!   `abi::INVOCATIONS_PER_MINUTE_PER_INSTALL`), so a single install cannot be
//!   asked for 695/s. The run seeds enough orgs and installs that the buckets
//!   are not the thing being measured, and round-robins every call across them;
//! - the run is closed-loop and paced. Each caller waits for its own next slot
//!   instead of firing as fast as it can, so the achieved rate is the number
//!   under test and a slow invocation shows up as latency, not as a quieter
//!   queue.
//!
//! What it asserts: the target rate is achieved, no call is refused for any
//! reason the design does not intend to refuse under load, and the pool's added
//! p95 stays inside the ADR's 4,660 µs budget. Every number it measures is
//! printed, because the evidence document records them.
//!
//! ```text
//! cargo test -p plugin-host --test load_anchor -- --ignored --nocapture
//! ```

mod support;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use core_domain::entities::plugins::PluginCapability;
use core_store::memory::MemoryStore;
use plugin_host::abi::InvocationOutcome;
use plugin_host::exec::{invoke_surface, PoolSlot, SurfaceCall};
use plugin_host::surface::SurfaceKind;
use serde_json::json;
use sqlx::PgPool;
use support::{admin_pool, artifact, seed_install, Seed};
use uuid::Uuid;

/// The anchor: 695.3 requests/second.
const ANCHOR_RATE: f64 = 695.3;
/// The wave's guardrail: the pool may add at most 5% of the anchor's p95
/// (93.2 ms) on every request.
const BUDGET_P95_US: f64 = 4_660.0;
/// In-flight callers. The pool's connection budget, not the caller count, is
/// what bounds throughput here.
const IN_FLIGHT: usize = 32;

const NOOP: &str = r#"
    (module
      (memory (export "memory") 1)
      (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
      (func (export "marula_free") (param i32) (param i32))
      (func (export "marula_entry") (param i32) (param i32) (result i64)
        (i64.const 0)))
"#;

#[derive(Clone)]
struct Cell {
    org_id: Uuid,
    entry_key: String,
}

fn seconds() -> f64 {
    std::env::var("MARULA_LOAD_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .filter(|value: &f64| *value > 0.0)
        .unwrap_or(8.0)
}

fn rate() -> f64 {
    std::env::var("MARULA_LOAD_RATE")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .filter(|value: &f64| *value > 0.0)
        .unwrap_or(ANCHOR_RATE)
}

/// The number of installs the per-install rate budget needs, with headroom.
///
/// One install sustains `INVOCATIONS_PER_MINUTE_PER_INSTALL / 60` calls per
/// second, so the floor is `rate / 5`. The extra 40 cover the burst drain at the
/// start of the run and keep the round-robin from landing on one install twice
/// inside one bucket window.
fn installs_needed(rate: f64) -> usize {
    let per_install = plugin_host::abi::INVOCATIONS_PER_MINUTE_PER_INSTALL as f64 / 60.0;
    (rate / per_install).ceil() as usize + 40
}

fn percentile(sorted: &[u64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index] as f64
}

/// One paced run at `rate` for `seconds`, returning every admitted latency in
/// microseconds and the refusals by reason.
async fn run(
    pool: &PgPool,
    store: &Arc<MemoryStore>,
    slot: &PoolSlot,
    cells: &[Cell],
    rate: f64,
    seconds: f64,
) -> (Vec<u64>, BTreeMap<String, usize>, f64) {
    let total = (rate * seconds) as usize;
    let per_caller = total.div_ceil(IN_FLIGHT);
    let interval = Duration::from_secs_f64(1.0 / rate);
    let cursor = Arc::new(AtomicUsize::new(0));
    let started = Instant::now();

    let mut tasks = Vec::with_capacity(IN_FLIGHT);
    for caller in 0..IN_FLIGHT {
        let pool = pool.clone();
        let store = Arc::clone(store);
        let slot = slot.clone();
        let cells = cells.to_vec();
        let cursor = Arc::clone(&cursor);
        tasks.push(tokio::spawn(async move {
            let origin = Instant::now() + interval * caller as u32;
            let mut latencies = Vec::with_capacity(per_caller);
            let mut refused: BTreeMap<String, usize> = BTreeMap::new();
            for n in 0..per_caller {
                let due = origin + interval * (n * IN_FLIGHT) as u32;
                let now = Instant::now();
                if due > now {
                    tokio::time::sleep(due - now).await;
                }
                let index = cursor.fetch_add(1, Ordering::Relaxed);
                let cell = &cells[index % cells.len()];
                let started = Instant::now();
                let mut tx = match pool.begin().await {
                    Ok(tx) => tx,
                    Err(_) => {
                        *refused.entry("no_connection".to_string()).or_default() += 1;
                        continue;
                    }
                };
                if core_db::set_org(&mut tx, cell.org_id).await.is_err() {
                    *refused.entry("no_org_scope".to_string()).or_default() += 1;
                    continue;
                }
                let params = json!({});
                let call = SurfaceCall {
                    org_id: cell.org_id,
                    entry_key: &cell.entry_key,
                    surface: SurfaceKind::Widget,
                    params: &params,
                    ticket_id: None,
                };
                let outcome = invoke_surface(&mut tx, &pool, &*store, &slot, &call).await;
                let elapsed = started.elapsed();
                // A widget writes nothing; rolling back keeps the database the
                // same size after ten thousand invocations.
                let _ = tx.rollback().await;
                match outcome {
                    Ok(outcome) if outcome.outcome == InvocationOutcome::Ok => {
                        latencies.push(elapsed.as_micros() as u64)
                    }
                    Ok(outcome) => {
                        *refused
                            .entry(outcome.skip_reason().to_string())
                            .or_default() += 1;
                    }
                    Err(error) => {
                        *refused.entry(format!("db: {error}")).or_default() += 1;
                    }
                }
            }
            (latencies, refused)
        }));
    }

    let mut all = Vec::with_capacity(total);
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    for task in tasks {
        let (latencies, refusals) = task.await.expect("a caller task");
        all.extend(latencies);
        for (why, count) in refusals {
            *refused.entry(why).or_default() += count;
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    (all, refused, elapsed)
}

async fn cells_for(pool: &PgPool, store: &Arc<MemoryStore>, count: usize) -> Vec<Cell> {
    let module = artifact(NOOP);
    let mut cells = Vec::with_capacity(count);
    for _ in 0..count {
        let seed = Seed::widget(
            &format!("load-{}", Uuid::now_v7().simple()),
            &module,
            &[PluginCapability::UiWidgets],
        );
        let seeded = seed_install(pool, &**store, &seed).await;
        cells.push(Cell {
            org_id: seeded.org_id,
            entry_key: seeded.entry_key,
        });
    }
    cells
}

/// The contact time of one invocation with no queue: fifty calls, one caller.
///
/// Printed before the load run so the numbers can be read together. If contact
/// time is a few milliseconds and the paced run achieves a fraction of the
/// target, the ceiling is the queueing resource, not the executor — and a reader
/// of the evidence can tell a slow host from a slow path without re-running it.
async fn contact_time(
    pool: &PgPool,
    store: &Arc<MemoryStore>,
    slot: &PoolSlot,
    cell: &Cell,
) -> f64 {
    let mut samples = Vec::new();
    for _ in 0..50 {
        let mut tx = pool.begin().await.expect("transaction");
        core_db::set_org(&mut tx, cell.org_id)
            .await
            .expect("org scope");
        let params = json!({});
        let call = SurfaceCall {
            org_id: cell.org_id,
            entry_key: &cell.entry_key,
            surface: SurfaceKind::Widget,
            params: &params,
            ticket_id: None,
        };
        let started = Instant::now();
        let outcome = invoke_surface(&mut tx, &pool, &**store, slot, &call).await;
        samples.push(started.elapsed().as_micros() as u64);
        let _ = tx.rollback().await;
        outcome.expect("the probe invocation answers");
    }
    samples.sort_unstable();
    percentile(&samples, 0.50)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "needs Postgres via MARULA_TEST_DATABASE_URL or DATABASE_URL; run with --ignored --nocapture at the gate"]
async fn the_anchor_rate_is_served_inside_the_guardrail() {
    let seconds = seconds();
    let rate = rate();
    let pool = admin_pool(IN_FLIGHT as u32).await;
    let store = Arc::new(MemoryStore::new());
    let installs = installs_needed(rate);

    println!(
        "\nanchor load — one plugin invocation per request\n\
         target {rate:.1} req/s for {seconds:.0}s ({} calls), {installs} installs, {IN_FLIGHT} in flight\n",
        (rate * seconds) as usize
    );

    let cells = cells_for(&pool, &store, installs).await;
    let slot = support::pool_slot(&pool, 8).await;

    println!(
        "contact time (one caller, p50): {:.1} µs",
        contact_time(&pool, &store, &slot, &cells[0]).await
    );

    let mut results = Vec::new();
    for mode in ["in_process", "pool"] {
        // A fresh slot per mode: the pool's rate buckets and workers are its
        // own, and the comparison is between two equal starting points.
        let mode_slot = if mode == "pool" {
            slot.clone()
        } else {
            let in_process = PoolSlot::new();
            in_process
                .get_with(|| async {
                    plugin_host::runtime::PluginPool::start(plugin_host::runtime::PoolConfig {
                        mode: plugin_host::runtime::IsolationMode::InProcess,
                        workers: 8,
                        worker_bin: None,
                        health: None,
                        sandbox_limits: Default::default(),
                    })
                    .await
                })
                .await
                .expect("the in-process runtime starts");
            in_process
        };

        let (mut latencies, refused, elapsed) =
            run(&pool, &store, &mode_slot, &cells, rate, seconds).await;
        latencies.sort_unstable();
        let achieved = latencies.len() as f64 / elapsed;
        let p50 = percentile(&latencies, 0.50);
        let p95 = percentile(&latencies, 0.95);
        let p99 = percentile(&latencies, 0.99);
        let refused_total: usize = refused.values().sum();

        println!(
            "{mode:<10} ok {:>6}  refused {:>5}  achieved {:>8.1} req/s  p50 {:>8.1} µs  p95 {:>8.1} µs  p99 {:>8.1} µs",
            latencies.len(),
            refused_total,
            achieved,
            p50,
            p95,
            p99
        );
        for (why, count) in &refused {
            println!("           refused {count:>5}× {why}");
        }

        assert!(
            achieved >= rate * 0.90,
            "{mode}: achieved {achieved:.1} req/s against a {rate:.1} target — the run \
             did not reach the anchor, so its latency proves nothing about it"
        );
        assert!(
            refused_total == 0,
            "{mode}: {refused_total} calls were refused under load ({refused:?}); a load \
             run at the anchor is not a refusal test"
        );
        results.push((mode, p50, p95, p99));
    }

    let baseline = results[0].2;
    let pooled = results[1].2;
    let added = (pooled - baseline).max(0.0);
    println!(
        "\nguardrail: pool adds {added:.1} µs at p95 over in_process, budget {BUDGET_P95_US:.0} µs, \
         headroom {:.1}×",
        BUDGET_P95_US / added.max(1.0)
    );
    assert!(
        added <= BUDGET_P95_US,
        "the pool added {added:.1} µs at p95, over the ADR's {BUDGET_P95_US:.0} µs budget"
    );
}
