//! Shared bootstrap for the email query tests — same database-selection
//! contract as `crates/core-db/tests/common/mod.rs`:
//!
//!   1. `MERIDIAN_TEST_DATABASE_URL` — explicit override, tried first.
//!   2. `DATABASE_URL`                — CI service container.
//!   3. Neither set                   — reusable local testcontainer
//!      (`pgvector/pgvector:pg18`, prod parity).
//!
//! The schema is reset + migrated once per test-binary process, making the
//! fixed deterministic UUIDs safe across runs. Pools are created per call
//! so they never outlive a test's runtime.

#![allow(dead_code)]

use sqlx::PgPool;
use std::time::Duration;
use testcontainers::core::{ContainerPort, ImageExt, ReuseDirective, WaitFor};
use testcontainers::{runners::AsyncRunner, ContainerRequest, GenericImage};
use uuid::Uuid;

#[must_use]
pub fn fixed_uuid(n: u64) -> Uuid {
    Uuid::from_u128(0x6D65_7269_6469_616E_u128 << 64 | u128::from(n))
}

struct Bootstrap {
    admin_url: String,
    _container: Option<testcontainers::ContainerAsync<GenericImage>>,
}

static BOOTSTRAP: tokio::sync::OnceCell<Bootstrap> = tokio::sync::OnceCell::const_new();

async fn bootstrap() -> &'static Bootstrap {
    BOOTSTRAP
        .get_or_init(|| async {
            if let Ok(url) = std::env::var("MERIDIAN_TEST_DATABASE_URL")
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
            let request = ContainerRequest::from(
                GenericImage::new("pgvector/pgvector", "pg18")
                    .with_wait_for(WaitFor::message_on_stderr(
                        "database system is ready to accept connections",
                    ))
                    .with_exposed_port(ContainerPort::Tcp(5432)),
            )
            .with_container_name("meridian-email-db-tests-pg18")
            .with_reuse(ReuseDirective::Always)
            .with_env_var("POSTGRES_USER", "meridian")
            .with_env_var("POSTGRES_PASSWORD", "meridian")
            .with_env_var("POSTGRES_DB", "meridian_email_test");
            let container = request
                .start()
                .await
                .expect("starting postgres testcontainer requires Docker");
            let host = container.get_host().await.expect("host");
            let port = container.get_host_port_ipv4(5432_u16).await.expect("port");
            let admin_url =
                format!("postgres://meridian:meridian@{host}:{port}/meridian_email_test");
            let admin = connect_with_retry(&admin_url).await;
            reset_schema(&admin).await;
            core_db::run_migrations(&admin).await.expect("migrations");
            Bootstrap {
                admin_url,
                _container: Some(container),
            }
        })
        .await
}

/// Admin (owner-role) pool against a freshly migrated schema. Email
/// pipeline tests mostly exercise owner-role paths; RLS-scoped assertions
/// open app-role transactions explicitly.
pub struct TestDb {
    pub admin: PgPool,
    pub app: PgPool,
}

pub async fn test_db() -> TestDb {
    let b = bootstrap().await;
    TestDb {
        admin: connect_with_retry(&b.admin_url).await,
        // App role pool for RLS checks; meridian_app is created by 0001.
        app: connect_with_retry(&app_url(&b.admin_url)).await,
    }
}

impl TestDb {
    /// A second app pool whose sessions never ran `set_config` — needed
    /// for true unscoped reads because Postgres keeps custom GUCs DEFINED
    /// after a transaction-local set ends, making reused sessions error
    /// (22P02) instead of default-deny. Mirrors core-db's common harness.
    pub async fn fresh_app_pool(&self) -> PgPool {
        let url = {
            let b = bootstrap().await;
            app_url(&b.admin_url)
        };
        connect_with_retry(&url).await
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
    loop {
        if let Ok(pool) = core_db::connect(url).await {
            if sqlx::query("SELECT 1").execute(&pool).await.is_ok() {
                return pool;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "postgres never became reachable at {url}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn app_url(admin_url: &str) -> String {
    let (scheme_rest, rest) = admin_url
        .split_once("://")
        .unwrap_or(("postgres", admin_url));
    match rest.split_once('@') {
        Some((_creds, host_part)) => {
            format!("{scheme_rest}://meridian_app:meridian_app@{host_part}")
        }
        None => format!("{scheme_rest}://meridian_app:meridian_app@{rest}"),
    }
}

/// Begins an org-bound transaction on the app pool (RLS scope).
pub async fn org_tx(app: &PgPool, org_id: Uuid) -> sqlx::Transaction<'_, sqlx::Postgres> {
    let mut tx = app.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    tx
}
