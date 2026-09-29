//! Shared test-support harness for core-db integration tests.
//!
//! Purpose: provision a real Postgres 17 (pgvector image, prod parity),
//! reset + migrate the schema once per test-binary process, and hand out
//! two pools — an admin pool for migrations/DDL probes and a
//! `marula_app` pool that runs under RLS exactly like production
//! traffic.
//!
//! Database selection mechanism (documented contract, see CI rust job):
//!   1. `MARULA_TEST_DATABASE_URL` — explicit test override, tried first.
//!   2. `DATABASE_URL`                — CI service-container Postgres; when
//!      set, tests attach directly and never touch Docker (no dind needed).
//!   3. Neither set                    — spin up a testcontainers
//!      `pgvector/pgvector:pg18` container locally.
//!
//! Notes:
//! - The env URL must point at a *privileged* role: the harness derives the
//!   app URL by switching credentials to `marula_app` (created by
//!   migration 0001), which only works if the login may create roles.
//! - The per-process schema reset makes fixed deterministic UUIDs safe
//!   across runs: every `cargo test` invocation starts from an empty,
//!   fully migrated schema.
//! - Pools are created PER CALL because each `#[tokio::test]` owns its own
//!   tokio runtime; a pool must not outlive the runtime that spawned its
//!   connection tasks. Only the container handle + bootstrapped URL are
//!   process-wide (dropping a `ContainerAsync` removes the container).
//!
//! Non-purpose: no query-layer logic, no fixtures beyond org seeding —
//! each test builds its own deterministic state via fixed UUIDs.

#![allow(dead_code)]

use sqlx::{PgPool, Postgres, Transaction};
use std::time::Duration;
use testcontainers::core::{ContainerPort, ImageExt, ReuseDirective, WaitFor};
use testcontainers::{runners::AsyncRunner, ContainerRequest, GenericImage};
use uuid::Uuid;

/// Fixed wall-clock anchors. Session expiry compares against real `now()`,
/// so "active" sessions expire far in the future and "expired" ones far in
/// the past — deterministic without clock injection.
pub const FUTURE: chrono::DateTime<chrono::Utc> =
    match chrono::DateTime::from_timestamp(4_102_444_800, 0) {
        Some(t) => t,
        None => unreachable!(),
    };
pub const PAST: chrono::DateTime<chrono::Utc> =
    match chrono::DateTime::from_timestamp(946_684_800, 0) {
        Some(t) => t,
        None => unreachable!(),
    };

/// Deterministic UUID from a small integer (stable across runs).
#[must_use]
pub fn fixed_uuid(n: u64) -> Uuid {
    Uuid::from_u128(0x6D65_7269_6469_616E_u128 << 64 | u128::from(n))
}

struct Bootstrap {
    admin_url: String,
    // Held so a self-started container outlives the whole test process;
    // dropping the handle stops and removes the container.
    _container: Option<testcontainers::ContainerAsync<GenericImage>>,
}

static BOOTSTRAP: tokio::sync::OnceCell<Bootstrap> = tokio::sync::OnceCell::const_new();

async fn bootstrap() -> &'static Bootstrap {
    BOOTSTRAP
        .get_or_init(|| async {
            if let Ok(url) = std::env::var("MARULA_TEST_DATABASE_URL")
                .or_else(|_| std::env::var("DATABASE_URL"))
            {
                let admin = connect_with_retry(&url).await;
                reset_schema(&admin).await;
                core_db::run_migrations(&admin)
                    .await
                    .expect("migrations must apply cleanly");
                return Bootstrap {
                    admin_url: url,
                    _container: None,
                };
            }
            // No env: spin up a throwaway pgvector/pgvector:pg18 container
            // (same image as prod). The container is NAMED and marked
            // reusable: local reruns attach to the same instance instead of
            // leaking an orphan per run (statics are never dropped at
            // test-process exit, so graceful auto-removal cannot run).
            // Override the name per worktree/agent lane via
            //   MARULA_TEST_PG_NAME=<unique>
            // so concurrent suites never fight over one fixed name; remove
            // a stale instance with:
            //   docker rm -f marula-core-db-tests-pg17
            let container_name = std::env::var("MARULA_TEST_PG_NAME")
                .unwrap_or_else(|_| "marula-core-db-tests-pg18".to_string());
            let request = ContainerRequest::from(
                GenericImage::new("pgvector/pgvector", "pg18")
                    .with_wait_for(WaitFor::message_on_stderr(
                        "database system is ready to accept connections",
                    ))
                    .with_exposed_port(ContainerPort::Tcp(5432)),
            )
            .with_container_name(container_name)
            .with_reuse(ReuseDirective::Always)
            .with_env_var("POSTGRES_USER", "marula")
            .with_env_var("POSTGRES_PASSWORD", "marula")
            .with_env_var("POSTGRES_DB", "marula_test");
            let container = request
                .start()
                .await
                .expect("starting postgres testcontainer requires Docker");
            let host = container.get_host().await.expect("container host");
            let port = container
                .get_host_port_ipv4(5432_u16)
                .await
                .expect("mapped port");
            let admin_url = format!("postgres://marula:marula@{host}:{port}/marula_test");
            let admin = connect_with_retry(&admin_url).await;
            reset_schema(&admin).await;
            core_db::run_migrations(&admin)
                .await
                .expect("migrations must apply cleanly");
            Bootstrap {
                admin_url,
                _container: Some(container),
            }
        })
        .await
}

/// Admin pool (migrations/probes) + app pool (`marula_app`, RLS applies),
/// freshly bound to the calling test's runtime.
pub struct TestDb {
    pub admin: PgPool,
    pub app: PgPool,
    admin_url: String,
}

impl TestDb {
    /// A second `marula_app` pool whose connections have never run
    /// `set_config('app.org_id', …)`. Needed because Postgres keeps custom
    /// GUCs DEFINED after a transaction-local set ends — on reused sessions
    /// `current_setting('app.org_id', true)` then yields `''` (not NULL),
    /// and policies raise `22P02` instead of default-deny. Unscoped-read
    /// assertions use this to exercise the true no-scope-never-set state
    /// (see the finding note in db_suite :: rls_blocks_cross_org…).
    pub async fn fresh_app_pool(&self) -> PgPool {
        connect_with_retry(&app_url(&self.admin_url)).await
    }
}

/// Returns database pools against a reset, fully migrated schema.
pub async fn test_db() -> TestDb {
    let b = bootstrap().await;
    TestDb {
        admin: connect_with_retry(&b.admin_url).await,
        app: connect_with_retry(&app_url(&b.admin_url)).await,
        admin_url: b.admin_url.clone(),
    }
}

async fn reset_schema(admin: &PgPool) {
    sqlx::query("DROP SCHEMA public CASCADE")
        .execute(admin)
        .await
        .expect("drop schema");
    sqlx::query("CREATE SCHEMA public")
        .execute(admin)
        .await
        .expect("create schema");
}

async fn connect_with_retry(url: &str) -> PgPool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    let mut last_err: Option<String>;
    loop {
        // Verify the connection is actually usable, not just pooled.
        match core_db::connect(url).await {
            Ok(pool) => match sqlx::query("SELECT 1").execute(&pool).await {
                Ok(_) => return pool,
                Err(e) => last_err = Some(e.to_string()),
            },
            Err(e) => last_err = Some(e.to_string()),
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "postgres never became reachable at {url}: last error {last_err:?}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Rewrites admin credentials to the RLS-constrained application role.
fn app_url(admin_url: &str) -> String {
    let (scheme_rest, rest) = admin_url
        .split_once("://")
        .unwrap_or(("postgres", admin_url));
    match rest.split_once('@') {
        Some((_creds, host_part)) => {
            format!("{scheme_rest}://marula_app:marula_app@{host_part}")
        }
        None => format!("{scheme_rest}://marula_app:marula_app@{rest}"),
    }
}

/// Begins a transaction bound to `org_id` (RLS scope) on the app pool.
pub async fn org_tx(app: &PgPool, org_id: Uuid) -> Transaction<'_, Postgres> {
    let mut tx = app.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    tx
}
