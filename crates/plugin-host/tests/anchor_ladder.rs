//! The request-path ladder ADR 0023 Decision 2 owes: `invoke_surface` at the
//! anchor's own queue depth, `disabled` against `in_process` against the
//! supervised pool, with the audit insert in the path.
//!
//! ## What this measures and what it does not
//!
//! The guardrail in the ADR is an *added* cost: how much latency the plugin
//! subsystem puts on a request that used to run a hardcoded effect. So the
//! comparison is between three isolation modes of the same call:
//!
//! - `disabled` — the executor's gates and the audit write, no sandbox. This is
//!   the "plugin off" baseline.
//! - `in_process` — wasmtime in a blocking task inside this process.
//! - `pool` — the supervised worker child, two members, real frames.
//!
//! Every cell runs the same guest (a module that returns immediately, no host
//! calls) through the same `exec::invoke_surface`, on its own transaction,
//! against a real database. The audit row is written by that same path, which is
//! the piece Decision 7 says the earlier wasmtime-only ladder did not cover.
//!
//! It does not measure the HTTP anchor's 695 req/s journey, and it does not
//! pretend to: it measures one invocation against a stated concurrency, which is
//! what the guardrail arithmetic is written in.
//!
//! ## Running it
//!
//! `#[ignore]`d because it needs Postgres, exactly like `kv_sql.rs`:
//!
//! ```text
//! scripts/e2e-stack.sh up
//! set -a; . e2e/.e2e-stack.env; set +a
//! cargo test -p plugin-host --test anchor_ladder -- --ignored --nocapture
//! scripts/e2e-stack.sh down $MARULA_RUN_ID
//! ```
//!
//! Nothing resets the schema: it makes its own org and installs, so it can share
//! a database with the rest of the run.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use core_domain::entities::plugins::{
    PluginCapability, PluginCatalogEntry, PluginInstall, PluginInstallStatus, PluginManifestV2,
    PluginSurfaceProfile, PluginVersion, SurfaceSlot, SurfaceWidget, SurfaceWidgetKind,
    PLUGIN_RUNTIME_INTERFACE_VERSION,
};
use core_store::memory::MemoryStore;
use core_store::{plugin_artifact_key, ObjectStore};
use plugin_host::abi::InvocationOutcome;
use plugin_host::exec::{invoke_surface, PoolSlot, SurfaceCall};
use plugin_host::isolation::ModuleArtifact;
use plugin_host::runtime::{IsolationMode, PluginPool, PoolConfig};
use plugin_host::surface::SurfaceKind;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// A guest that returns immediately. No host calls on purpose: this measures the
/// transport and the gates, not a plugin's own work.
const NOOP: &str = r#"
    (module
      (memory (export "memory") 1)
      (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
      (func (export "marula_free") (param i32) (param i32))
      (func (export "marula_entry") (param i32) (param i32) (result i64) (i64.const 0)))
"#;

/// The queues the ADR's ladder uses: the anchor's median, its p95 and its tail.
const CONCURRENCY: [usize; 3] = [18, 65, 205];
/// Calls per cell. Enough for a stable p99 without turning the gate into a soak.
const SAMPLES_PER_CELL: usize = 1_200;
/// The p95 budget this is measured against (ADR 0023 Decision 2).
const BUDGET_P95_US: f64 = 4_660.0;

fn artifact(source: &str) -> ModuleArtifact {
    let bytes = wat::parse_str(source).expect("valid WAT");
    ModuleArtifact {
        digest: ModuleArtifact::digest_of(&bytes),
        bytes,
    }
}

fn manifest() -> PluginManifestV2 {
    serde_json::from_value(json!({
        "name": "Anchor ladder probe",
        "version": "1.0.0",
        "interface_version": PLUGIN_RUNTIME_INTERFACE_VERSION,
        "description": "Returns immediately; measures the request path.",
        "capabilities": [PluginCapability::UiWidgets.as_str()],
        "surface_profile": PluginSurfaceProfile {
            widgets: vec![SurfaceWidget {
                id: "ladder_probe".into(),
                title: "Ladder probe".into(),
                slot: SurfaceSlot::TicketPane,
                kind: SurfaceWidgetKind::List,
                props_schema: None,
                default_props: None,
            }],
            ..PluginSurfaceProfile::default()
        },
    }))
    .expect("the manifest is the v2 shape")
}

/// A pool sized like a deployment's connection budget rather than like the
/// caller count: 205 callers queue on 24 connections in every mode, so the
/// queueing is equal on both sides of the comparison.
async fn bench_pool() -> PgPool {
    let url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("the ladder needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(24)
        .acquire_timeout(Duration::from_secs(30))
        .connect(&url)
        .await
        .expect("bench pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    pool
}

async fn seed_install(
    pool: &PgPool,
    store: &dyn ObjectStore,
    slug: &str,
    artifact: &ModuleArtifact,
) -> (Uuid, PluginInstall) {
    let org_id = Uuid::now_v7();
    let catalog_id = Uuid::now_v7();
    let version_id = Uuid::now_v7();
    let install_id = Uuid::now_v7();

    store
        .put(
            &plugin_artifact_key(org_id, &artifact.digest),
            "application/wasm",
            artifact.bytes.clone().into(),
        )
        .await
        .expect("the module is stored under its digest");

    let mut tx = pool.begin().await.expect("transaction");
    core_db::set_org(&mut tx, org_id).await.expect("org scope");
    sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
        .bind(org_id)
        .bind("Anchor ladder org")
        .bind(slug)
        .execute(&mut *tx)
        .await
        .expect("org row");

    let entry = PluginCatalogEntry {
        id: catalog_id,
        slug: format!("ladder-{slug}"),
        name: "Anchor ladder probe".into(),
        description: "Returns immediately.".into(),
        publisher: "marula".into(),
        origin_org_id: Some(org_id),
        latest_version: Some("1.0.0".into()),
        interface_version: PLUGIN_RUNTIME_INTERFACE_VERSION,
        revoked_at: None,
        revoked_reason: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let version = PluginVersion {
        id: version_id,
        catalog_id,
        version: "1.0.0".into(),
        manifest: manifest(),
        module_digest: artifact.digest.clone(),
        created_at: Utc::now(),
    };
    core_db::plugins::create_catalog_entry(&mut tx, &entry, &version)
        .await
        .expect("catalog entry");

    let install = PluginInstall {
        id: install_id,
        org_id,
        catalog_id,
        slug: entry.slug.clone(),
        version: "1.0.0".into(),
        module_digest: artifact.digest.clone(),
        status: PluginInstallStatus::Enabled,
        granted_capabilities: vec![PluginCapability::UiWidgets.as_str().to_string()],
        installed_by: None,
        installed_at: Utc::now(),
        updated_at: Utc::now(),
        revoked_at: None,
        revoked_by: None,
        revoked_reason: None,
        previous_version: None,
        previous_module_digest: None,
        previous_capabilities: Vec::new(),
    };
    core_db::plugins::create_install(&mut tx, &install)
        .await
        .expect("install row");

    core_db::plugin_surface::sync_surface_section(
        &mut tx,
        org_id,
        install_id,
        SurfaceKind::Widget.as_str(),
        &[(
            format!("plugin.{}.ladder_probe", entry.slug),
            "Ladder probe".to_string(),
            json!({ "slot": "ticket_pane", "kind": "list" }),
        )],
    )
    .await
    .expect("surface entry");

    tx.commit().await.expect("seed commits");
    (org_id, install)
}

async fn slot_for(mode: IsolationMode, pool: &PgPool) -> PoolSlot {
    let slot = PoolSlot::new();
    slot.get_with(|| async move {
        PluginPool::start(PoolConfig {
            mode,
            // Two members, the count Decision 2 uses for the `split` ladder: one
            // is the `all-in-one` case, measured separately by the pool's tests.
            workers: 2,
            worker_bin: Some(env!("CARGO_BIN_EXE_marula-plugin-worker").into()),
            health: Some(pool.clone()),
            sandbox_limits: Default::default(),
        })
        .await
    })
    .await
    .expect("the pool starts from this package's worker binary");
    slot
}

/// One cell: `callers` concurrent tasks, each making its share of the samples.
/// Returns every admitted latency in microseconds and the number refused.
async fn cell(
    pool: &PgPool,
    store: &Arc<MemoryStore>,
    slot: &PoolSlot,
    org_id: Uuid,
    entry_key: &str,
    callers: usize,
) -> (Vec<u64>, usize) {
    let per_caller = (SAMPLES_PER_CELL / callers).max(1);
    let mut tasks = Vec::with_capacity(callers);
    for _ in 0..callers {
        let pool = pool.clone();
        let store = Arc::clone(store);
        let slot = slot.clone();
        let entry_key = entry_key.to_string();
        tasks.push(tokio::spawn(async move {
            let mut latencies = Vec::with_capacity(per_caller);
            let mut refused = 0usize;
            for _ in 0..per_caller {
                let started = Instant::now();
                let mut tx = match pool.begin().await {
                    Ok(tx) => tx,
                    Err(_) => {
                        refused += 1;
                        continue;
                    }
                };
                if core_db::set_org(&mut tx, org_id).await.is_err() {
                    refused += 1;
                    continue;
                }
                let params = json!({});
                let call = SurfaceCall {
                    org_id,
                    entry_key: &entry_key,
                    surface: SurfaceKind::Widget,
                    params: &params,
                    ticket_id: None,
                };
                let outcome = invoke_surface(&mut tx, &pool, &*store, &slot, &call).await;
                let elapsed = started.elapsed();
                // Nothing is written by a no-op widget; rolling back keeps the
                // database the same size after 10,000 invocations.
                let _ = tx.rollback().await;
                match outcome {
                    Ok(outcome) if outcome.outcome == InvocationOutcome::Ok => {
                        latencies.push(elapsed.as_micros() as u64)
                    }
                    _ => refused += 1,
                }
            }
            (latencies, refused)
        }));
    }

    let mut all = Vec::with_capacity(SAMPLES_PER_CELL);
    let mut refused = 0usize;
    for task in tasks {
        let (latencies, refusals) = task.await.expect("a caller task");
        all.extend(latencies);
        refused += refusals;
    }
    (all, refused)
}

fn percentile(sorted: &[u64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index] as f64
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "needs Postgres via MARULA_TEST_DATABASE_URL or DATABASE_URL; run with --ignored at the gate"]
async fn the_request_path_at_anchor_queue_depth() {
    let pool = bench_pool().await;
    let store = Arc::new(MemoryStore::new());
    let slug = format!("ladder-{}", Uuid::now_v7().simple());
    let module = artifact(NOOP);
    let (org_id, install) = seed_install(&pool, &*store, &slug, &module).await;
    let entry_key = format!("plugin.{}.ladder_probe", install.slug);

    println!(
        "\nanchor ladder — invoke_surface end to end, audit insert included\n\
         budget: p95 {BUDGET_P95_US:.0} µs   pool: 2 workers   samples/cell: {SAMPLES_PER_CELL}\n"
    );
    println!(
        "{:<10} {:>4} {:>7} {:>7} {:>8} {:>8} {:>8} {:>10}",
        "mode", "C", "ok", "ref", "p50 µs", "p95 µs", "p99 µs", "Δp95 vs off"
    );

    let modes = [
        ("disabled", IsolationMode::Disabled),
        ("in_process", IsolationMode::InProcess),
        ("pool", IsolationMode::Pool),
    ];

    let mut baseline_p95 = std::collections::HashMap::new();
    for (label, mode) in modes {
        let slot = slot_for(mode, &pool).await;
        for callers in CONCURRENCY {
            let (mut latencies, refused) =
                cell(&pool, &store, &slot, org_id, &entry_key, callers).await;
            latencies.sort_unstable();
            let p50 = percentile(&latencies, 0.50);
            let p95 = percentile(&latencies, 0.95);
            let p99 = percentile(&latencies, 0.99);
            if mode == IsolationMode::Disabled {
                baseline_p95.insert(callers, p95);
            }
            let delta = match baseline_p95.get(&callers) {
                Some(base) if mode != IsolationMode::Disabled => {
                    format!("{:+.1} µs", p95 - base)
                }
                Some(_) => "—".to_string(),
                None => "n/a".to_string(),
            };
            println!(
                "{label:<10} {callers:>4} {:>7} {refused:>7} {p50:>8.1} {p95:>8.1} {p99:>8.1} {delta:>10}",
                latencies.len()
            );
        }
    }

    // The guardrail line the ADR asks for, computed rather than eyeballed.
    println!(
        "\nRead the Δp95 column against the budget: an added p95 below {BUDGET_P95_US:.0} µs \
         leaves a factor of {:.0} at the anchor.",
        BUDGET_P95_US / 100.0
    );
}
