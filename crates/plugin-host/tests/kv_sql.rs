//! The COMPOSED path, against a real database: guest → host-call frame →
//! `exec::TransactionHostCalls` → `core_db::plugin_kv` → Postgres (W53-11).
//!
//! Why this file exists: every other KV test stops one layer short. The pool
//! tests prove the transport against a memory-backed service; `db_suite` proves
//! the SQL against a transaction; neither runs the two together, and the gap
//! between them is where a mismatch would live (a namespace the SQL constraint
//! rejects, an org scope the service forgot, a value that survives the frame but
//! not the column).
//!
//! It needs a database, so both tests are `#[ignore]`d and the group gate runs
//! them explicitly:
//!
//! ```text
//! cargo test -p plugin-host --test kv_sql -- --ignored
//! ```
//!
//! `cargo test -p plugin-host` stays green without a database, which is why this
//! package's default suite is hermetic and why the mark is a reason string and
//! not a silent skip: an ignored test is visible in the run, a test that returns
//! early is not. The URL comes from `MARULA_TEST_DATABASE_URL` or
//! `DATABASE_URL`, the same pair `core-api`'s flow tests use, and the migrations
//! run first. Nothing resets the schema: the test makes its own org, slug and
//! installs, so it can share a database with every other test in the run.
//!
//! The worker binary is the one `cargo` builds for this package, through
//! `CARGO_BIN_EXE_marula-plugin-worker` — so the frames are the real frames and
//! the process is a real child.

use std::sync::Arc;

use chrono::Utc;
use core_domain::entities::plugins::{
    PluginCapability, PluginCatalogEntry, PluginInstall, PluginInstallStatus, PluginManifestV2,
    PluginSurfaceProfile, PluginVersion, SurfaceSlot, SurfaceWidget, SurfaceWidgetKind,
    PLUGIN_RUNTIME_INTERFACE_VERSION,
};
use core_store::memory::MemoryStore;
use core_store::{plugin_artifact_key, ObjectStore};
use plugin_host::exec::{invoke_surface, PoolSlot, SurfaceCall};
use plugin_host::isolation::ModuleArtifact;
use plugin_host::runtime::{IsolationMode, PluginPool, PoolConfig};
use plugin_host::surface::SurfaceKind;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// The committed WAT a guest is built from: put a value, read it back, and
/// return the `kv_get` data frame. Memory: namespace "cache" at 0, key "k" at
/// 16, value "bridged" at 32; `marula_alloc` hands out space from 1024.
const PUT_AND_GET: &str = r#"
    (module
      (import "marula" "kv_put" (func $put (param i32 i32 i32 i32 i32 i32) (result i32)))
      (import "marula" "kv_get" (func $get (param i32 i32 i32 i32) (result i64)))
      (memory (export "memory") 1)
      (data (i32.const 0) "cache")
      (data (i32.const 16) "k")
      (data (i32.const 32) "bridged")
      (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
      (func (export "marula_free") (param i32) (param i32))
      (func (export "marula_entry") (param i32) (param i32) (result i64)
        (if (i32.ne
              (call $put (i32.const 0) (i32.const 5)
                         (i32.const 16) (i32.const 1)
                         (i32.const 32) (i32.const 7))
              (i32.const 0))
            (then (unreachable)))
        (call $get (i32.const 0) (i32.const 5) (i32.const 16) (i32.const 1))))
"#;

/// The same key, read only: the second install's guest.
const GET_ONLY: &str = r#"
    (module
      (import "marula" "kv_get" (func $get (param i32 i32 i32 i32) (result i64)))
      (memory (export "memory") 1)
      (data (i32.const 0) "cache")
      (data (i32.const 16) "k")
      (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
      (func (export "marula_free") (param i32) (param i32))
      (func (export "marula_entry") (param i32) (param i32) (result i64)
        (call $get (i32.const 0) (i32.const 5) (i32.const 16) (i32.const 1))))
"#;

fn artifact(source: &str) -> ModuleArtifact {
    let bytes = wat::parse_str(source).expect("valid WAT");
    ModuleArtifact {
        digest: ModuleArtifact::digest_of(&bytes),
        bytes,
    }
}

fn manifest() -> PluginManifestV2 {
    serde_json::from_value(json!({
        "name": "KV bridge probe",
        "version": "1.0.0",
        "interface_version": PLUGIN_RUNTIME_INTERFACE_VERSION,
        "description": "Stores and reads one key through the parent bridge.",
        "capabilities": [PluginCapability::UiWidgets.as_str()],
        "surface_profile": PluginSurfaceProfile {
            widgets: vec![SurfaceWidget {
                id: "kv_probe".into(),
                title: "KV probe".into(),
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

async fn admin_pool() -> PgPool {
    let url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("the composed KV test needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    pool
}

/// One org, one catalog entry, and one enabled install of `artifact` with the
/// `ui.widgets` grant. Returns `(org_id, catalog_id, install)`.
async fn seed_install(
    pool: &PgPool,
    store: &dyn ObjectStore,
    slug: &str,
    artifact: &ModuleArtifact,
) -> (Uuid, Uuid, PluginInstall) {
    let org_id = Uuid::now_v7();
    let catalog_id = Uuid::now_v7();
    let version_id = Uuid::now_v7();
    let install_id = Uuid::now_v7();

    // The object store addresses a module by org and digest, exactly as
    // `lifecycle::load_module` will look it up.
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
        .bind("KV bridge probe org")
        .bind(slug)
        .execute(&mut *tx)
        .await
        .expect("org row");

    let entry = PluginCatalogEntry {
        id: catalog_id,
        slug: format!("kv-probe-{slug}"),
        name: "KV bridge probe".into(),
        description: "Stores and reads one key through the parent bridge.".into(),
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

    // The registry row the invocation resolves first: `plugin.<slug>.<id>`.
    core_db::plugin_surface::sync_surface_section(
        &mut tx,
        org_id,
        install_id,
        SurfaceKind::Widget.as_str(),
        &[(
            format!("plugin.{}.kv_probe", entry.slug),
            "KV probe".to_string(),
            json!({ "slot": "ticket_pane", "kind": "list" }),
        )],
    )
    .await
    .expect("surface entry");

    tx.commit().await.expect("seed commits");
    (org_id, catalog_id, install)
}

/// The pool a caller would own, with the worker binary this package builds.
async fn pool_slot(pool: &PgPool) -> PoolSlot {
    let slot = PoolSlot::new();
    slot.get_with(|| async {
        PluginPool::start(PoolConfig {
            mode: IsolationMode::Pool,
            workers: 1,
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

/// One row of the store, read back the way an operator would look at it.
#[derive(sqlx::FromRow)]
struct StoredEntry {
    org_id: Uuid,
    install_id: Uuid,
    namespace: String,
    key: String,
    value: Vec<u8>,
    bytes: i64,
}

#[tokio::test]
#[ignore = "needs Postgres via MARULA_TEST_DATABASE_URL or DATABASE_URL; run with --ignored at the gate"]
async fn a_guest_writes_and_reads_one_key_through_the_real_frames_and_sql() {
    let pool = admin_pool().await;
    let store = Arc::new(MemoryStore::new());
    let slug = format!("kv-bridge-{}", Uuid::now_v7().simple());
    let writer = artifact(PUT_AND_GET);
    let reader = artifact(GET_ONLY);

    let (org_id, _catalog_id, install) = seed_install(&pool, &*store, &slug, &writer).await;

    // A second install in the SAME org. RLS cannot tell them apart, so it is the
    // store's own install_id predicate that has to keep them apart.
    let (_, _, other_install) = seed_install(&pool, &*store, &format!("{slug}-b"), &reader).await;

    let slot = pool_slot(&pool).await;
    let entry_key = format!("plugin.{}.kv_probe", install.slug);

    // --- install A: put then get, through the bridge -------------------------
    let mut tx = pool.begin().await.expect("transaction");
    core_db::set_org(&mut tx, org_id).await.expect("org scope");
    let outcome = invoke_surface(
        &mut tx,
        &pool,
        &*store,
        &slot,
        &SurfaceCall {
            org_id,
            entry_key: &entry_key,
            surface: SurfaceKind::Widget,
            params: &json!({}),
            ticket_id: None,
        },
    )
    .await
    .expect("the invocation runs");

    assert_eq!(
        outcome.outcome,
        plugin_host::abi::InvocationOutcome::Ok,
        "{:?}",
        outcome.reason
    );
    let output = outcome.output.clone().expect("a kv_get data frame");
    assert_eq!(
        plugin_host::abi::parse_frame(&output).expect("an ok frame"),
        b"bridged",
        "the guest read back what it wrote, through the frame and the SQL"
    );

    // The row landed, under the actor's org and the invoking install.
    let row: StoredEntry = sqlx::query_as(
        r#"SELECT org_id, install_id, namespace, key, value, octet_length(value) AS bytes
             FROM plugin_kv_entries
            WHERE install_id = $1 AND namespace = 'cache' AND key = 'k'"#,
    )
    .bind(install.id)
    .fetch_one(&mut *tx)
    .await
    .expect("the entry row");
    assert_eq!(row.org_id, org_id);
    assert_eq!(row.install_id, install.id);
    assert_eq!(row.namespace, "cache");
    assert_eq!(row.key, "k");
    assert_eq!(row.value, b"bridged");
    assert_eq!(row.bytes, 7);

    // The counter moved with it, in the same transaction.
    let usage = core_db::plugin_kv::reconcile(&mut tx, org_id, install.id)
        .await
        .expect("reconcile");
    assert!(
        usage.matched(),
        "the quota counter and the rows agree: {usage:?}"
    );

    // The invocation's own audit row is in the same transaction, and it says ok.
    let audited: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM audit_events
            WHERE org_id = $1 AND action = 'plugin.invoke'
              AND detail->>'outcome' = 'ok'"#,
    )
    .bind(org_id)
    .fetch_one(&mut *tx)
    .await
    .expect("audit rows");
    assert_eq!(audited, 1, "one row per invocation (ADR 0023 Decision 7)");

    // --- install B, same org, same namespace and key: nothing ---------------
    // Directly first: the store's predicate, not RLS, is what must hold.
    assert_eq!(
        core_db::plugin_kv::get(&mut tx, org_id, other_install.id, "cache", "k")
            .await
            .expect("the other install's read"),
        None
    );

    // And through the bridge, so the answer a guest sees is the empty value and
    // not an error: `kv_get` has one refusal code and it is for a namespace.
    let other_entry_key = format!("plugin.{}.kv_probe", other_install.slug);
    let other_org_id = other_install.org_id;
    let other = invoke_surface(
        &mut tx,
        &pool,
        &*store,
        &slot,
        &SurfaceCall {
            org_id: other_org_id,
            entry_key: &other_entry_key,
            surface: SurfaceKind::Widget,
            params: &json!({}),
            ticket_id: None,
        },
    )
    .await
    .expect("the second invocation runs");
    assert_eq!(
        other.outcome,
        plugin_host::abi::InvocationOutcome::Ok,
        "{other:?}"
    );
    let output = other.output.clone().expect("a data frame");
    assert!(
        plugin_host::abi::parse_frame(&output)
            .expect("an ok frame")
            .is_empty(),
        "the other install answered the empty value"
    );

    // Nothing was written under the other install: the bridge named the install
    // it was dispatched for, and the guest only ever named a label.
    let other_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM plugin_kv_entries WHERE install_id = $1")
            .bind(other_install.id)
            .fetch_one(&mut *tx)
            .await
            .expect("the other install's rows");
    assert_eq!(other_rows, 0);

    tx.commit().await.expect("the request commits");
}

#[tokio::test]
#[ignore = "needs Postgres via MARULA_TEST_DATABASE_URL or DATABASE_URL; run with --ignored at the gate"]
async fn a_dead_pool_is_a_fault_and_not_a_skip() {
    // The other half of the composed path: when the worker cannot start, the
    // invocation is a FAULT (the effect did not happen because something is
    // broken), which the callers turn into a 503 rather than a skip. Seeding is
    // the same shape as the test above, so the only difference is the pool.
    let pool = admin_pool().await;
    let store = Arc::new(MemoryStore::new());
    let slug = format!("kv-bridge-fault-{}", Uuid::now_v7().simple());
    let (org_id, _, install) = seed_install(&pool, &*store, &slug, &artifact(PUT_AND_GET)).await;

    let slot = PoolSlot::new();
    slot.get_with(|| async {
        PluginPool::start(PoolConfig {
            mode: IsolationMode::Pool,
            workers: 1,
            worker_bin: Some("/nonexistent/marula-plugin-worker".into()),
            health: Some(pool.clone()),
            sandbox_limits: Default::default(),
        })
        .await
    })
    .await
    .expect_err("a pool with no worker binary must not start");

    // The slot did not cache the failure, so the invocation's own `get` retries
    // and fails the same way — which is what makes this a fault, not a refusal.
    let mut tx = pool.begin().await.expect("transaction");
    core_db::set_org(&mut tx, org_id).await.expect("org scope");
    let outcome = invoke_surface(
        &mut tx,
        &pool,
        &*store,
        &slot,
        &SurfaceCall {
            org_id,
            entry_key: &format!("plugin.{}.kv_probe", install.slug),
            surface: SurfaceKind::Widget,
            params: &json!({}),
            ticket_id: None,
        },
    )
    .await
    .expect("a fault is an outcome, not an error");

    assert_eq!(
        outcome.outcome,
        plugin_host::abi::InvocationOutcome::WorkerFailed
    );
    assert!(outcome.is_fault(), "a dead pool is a fault");
    assert_eq!(outcome.skip_reason(), "plugin_worker_failed");
    tx.rollback().await.expect("nothing to commit");
}
