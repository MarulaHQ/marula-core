//! Shared harness for core-projects integration tests (same contract as
//! the core-db suite): `MERIDIAN_TEST_DATABASE_URL` / `DATABASE_URL` else
//! a reusable pgvector container; admin + `meridian_app` pools; per-process
//! schema reset so fixed UUIDs are safe.

#![allow(dead_code)]

use sqlx::{PgPool, Postgres, Transaction};
use std::time::Duration;
use testcontainers::core::{ContainerPort, ImageExt, ReuseDirective, WaitFor};
use testcontainers::{runners::AsyncRunner, ContainerRequest, GenericImage};
use uuid::Uuid;

/// Deterministic UUID from a small integer. The projects lane owns slots
/// 300–399 so parallel suites sharing one database stay collision-free.
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
            let container_name = std::env::var("MERIDIAN_TEST_PG_NAME")
                .unwrap_or_else(|_| "meridian-projects-tests-pg17".to_string());
            let request = ContainerRequest::from(
                GenericImage::new("pgvector/pgvector", "pg17")
                    .with_wait_for(WaitFor::message_on_stderr(
                        "database system is ready to accept connections",
                    ))
                    .with_exposed_port(ContainerPort::Tcp(5432)),
            )
            .with_container_name(container_name)
            .with_reuse(ReuseDirective::Always)
            .with_env_var("POSTGRES_USER", "meridian")
            .with_env_var("POSTGRES_PASSWORD", "meridian")
            .with_env_var("POSTGRES_DB", "meridian_projects_test");
            let container = request
                .start()
                .await
                .expect("starting postgres testcontainer requires Docker");
            let host = container.get_host().await.expect("container host");
            let port = container.get_host_port_ipv4(5432_u16).await.expect("port");
            let admin_url =
                format!("postgres://meridian:meridian@{host}:{port}/meridian_projects_test");
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

pub struct TestDb {
    pub admin: PgPool,
    pub app: PgPool,
}

/// Admin pool (probes) + app pool (`meridian_app`, RLS applies), bound to
/// the calling test's runtime.
pub async fn test_db() -> TestDb {
    let b = bootstrap().await;
    TestDb {
        admin: connect_with_retry(&b.admin_url).await,
        app: connect_with_retry(&app_url(&b.admin_url)).await,
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
            "postgres unreachable at {url}"
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

/// Begins a transaction bound to `org_id` (RLS scope) on the app pool.
pub async fn org_tx(app: &PgPool, org_id: Uuid) -> Transaction<'_, Postgres> {
    let mut tx = app.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    tx
}

/// Seeds an org + default roles + a user (mirroring org-creation seeding).
/// Returns (org, user).
pub async fn seed_org(db: &TestDb, n: u64) -> (Uuid, Uuid) {
    let org = fixed_uuid(n);
    let mut tx = org_tx(&db.app, org).await;
    core_db::orgs::insert_org(
        &mut *tx,
        org,
        "Projects Co",
        &format!("projects-{n}"),
        "PRJ",
    )
    .await
    .expect("org");
    core_db::orgs::seed_default_roles(&mut tx, org)
        .await
        .expect("roles");
    let user = fixed_uuid(n + 1);
    core_db::users::create_user(
        &mut *tx,
        user,
        org,
        &format!("owner-{n}@projects.test"),
        "Projects Owner",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user");
    tx.commit().await.expect("commit");
    (org, user)
}
