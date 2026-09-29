//! Perf-harness DB bootstrap: same env contract as the core-db suites
//! (`MARULA_TEST_DATABASE_URL` → `DATABASE_URL` → local testcontainer),
//! handing out an admin pool (migrations + seed) and an app pool that runs
//! under RLS as `marula_app` exactly like production traffic.
//!
//! Deliberately NOT the core-db `tests/common` module — that is
//! test-target-scoped and private to core-db; duplicating ~70 lines of
//! bootstrap beats coupling a test-infra crate to another crate's test
//! internals.

#![allow(dead_code)]

use sqlx::PgPool;
use std::time::Duration;
use testcontainers::{runners::AsyncRunner, ImageExt};

pub struct TestDb {
    /// Privileged pool: migrations + fixture seeding run as the tables'
    /// owner (RLS-exempt), exactly like the core-db harness.
    pub admin: PgPool,
    /// Connection URL (admin credentials) for deriving the RLS app pool.
    admin_url: String,
    _container: Option<testcontainers::ContainerAsync<testcontainers::GenericImage>>,
}

impl TestDb {
    /// Pool authenticating as `marula_app` (created by migration 0001),
    /// so measurement runs under RLS with the org GUC exactly like
    /// production traffic. Connects lazily AFTER migrations have run.
    pub async fn app(&self) -> PgPool {
        let app_url = perf_harness::app_pool_url(&self.admin_url);
        connect_with_retry(&app_url).await
    }
}

static BOOTSTRAP: tokio::sync::OnceCell<TestDb> = tokio::sync::OnceCell::const_new();

pub async fn test_db() -> &'static TestDb {
    BOOTSTRAP.get_or_init(bootstrap).await
}

async fn bootstrap() -> TestDb {
    if let Ok(url) =
        std::env::var("MARULA_TEST_DATABASE_URL").or_else(|_| std::env::var("DATABASE_URL"))
    {
        let admin = connect_with_retry(&url).await;
        // Env-attached DBs are shared; the perf org is wiped/re-seeded per
        // run by seed::seed, so no schema reset here (resetting would
        // destroy other suites' fixtures on a shared CI service DB).
        return TestDb {
            admin,
            admin_url: url,
            _container: None,
        };
    }
    container_db().await
}

async fn container_db() -> TestDb {
    let request = testcontainers::ContainerRequest::from(
        testcontainers::GenericImage::new("pgvector/pgvector", "pg18")
            .with_wait_for(testcontainers::core::WaitFor::message_on_stderr(
                "database system is ready to accept connections",
            ))
            .with_exposed_port(testcontainers::core::ContainerPort::Tcp(5432)),
    )
    .with_container_name("marula-perf-smoke-pg18")
    .with_reuse(testcontainers::core::ReuseDirective::Always)
    .with_env_var("POSTGRES_USER", "marula")
    .with_env_var("POSTGRES_PASSWORD", "marula")
    .with_env_var("POSTGRES_DB", "marula_perf");
    let container = request
        .start()
        .await
        .expect("docker required for perf smoke");
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432_u16).await.expect("port");
    let admin_url = format!("postgres://marula:marula@{host}:{port}/marula_perf");
    let admin = connect_with_retry(&admin_url).await;
    TestDb {
        admin,
        admin_url,
        _container: Some(container),
    }
}

async fn connect_with_retry(url: &str) -> PgPool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        if let Ok(pool) = core_db::connect(url).await {
            if sqlx::query("SELECT 1").execute(&pool).await.is_ok() {
                return pool;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "postgres unreachable at {url}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
