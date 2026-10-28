//! Shared harness for the `platform` test binary.
//!
//! Seven targets each carried a copy of this stack. The bodies here are those
//! copies, made `pub`, with the differences they actually carried kept as
//! parameters:
//!
//! - `state_with(tune)` builds the `AppState` four targets wanted,
//!   `boot_with(tune)` adds the composed router, and `boot_with_parts(tune,
//!   scanner)` is the store-returning form the secrets and quarantine suites
//!   use: they assert on what the object store did (or did not) receive, so it
//!   hands the same `MemoryStore` back and swaps in a scripted scan provider.
//! - `tune` carries the deterministic `secret_key` six targets set; the
//!   secrets and quarantine suites set none, as they did before.
//! - `send` is the bearer shape two targets used; `send_headers` is the
//!   header-list shape the other five used.
//! - `register` takes the payload, because the org name is what each flow
//!   registered under; `register_org`/`register_org_token` are the
//!   `owner@<slug>.test` shorthand.
//!
//! Behaviour drift the merge surfaces, deliberate and uniform: every target
//! built the reporting pool from `Config::database_url` BEFORE overriding it.
//! With `DATABASE_URL` exported that is the same URL; with only
//! `MARULA_TEST_DATABASE_URL` set it is the dev fallback, a database that need
//! not exist on the host. None of them reads a report and the pool is lazy, so
//! the misdirection stayed invisible. The merged harness points the reporting
//! pool at the resolved admin URL. The subscriber is initialised once per
//! process instead of once per target; no test asserts on log output.
//! `quarantine_admin` did not build a router before its tests; its boot now
//! builds one and drops it, which only stamps the same composed router every
//! other test in this binary builds anyway.
//!
//! Database contract is unchanged: `MARULA_TEST_DATABASE_URL` or `DATABASE_URL`,
//! migrations run on every boot, no schema is reset. `reports_flow` stays a
//! top-level target because its boot drops the schema once per process.
//!
//! The vault write budget (`routes::secrets::VAULT_BUDGETS`) is process-wide
//! and keyed per org; each test here registers its own org, so no case can
//! spend another's window.

#![allow(dead_code)]

use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;

/// Boot one `AppState` against the test database, without a router.
pub async fn state_with<F>(tune: F) -> AppState
where
    F: FnOnce(&mut Config),
{
    boot_with_parts(tune, None).await.0
}

/// The router-shaped boot five targets used.
pub async fn boot_with<F>(tune: F) -> (AppState, axum::Router)
where
    F: FnOnce(&mut Config),
{
    let (state, app, _store) = boot_with_parts(tune, None).await;
    (state, app)
}

/// The store-returning boot: the same state, with an optional scripted scan
/// provider and the object store handed back for assertions.
pub async fn boot_with_parts<F>(
    tune: F,
    scanner: Option<Arc<dyn core_mail::ScanProvider>>,
) -> (AppState, axum::Router, Arc<core_store::memory::MemoryStore>)
where
    F: FnOnce(&mut Config),
{
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("platform e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    tune(&mut config);
    let store = Arc::new(core_store::memory::MemoryStore::new());
    let mail = match scanner {
        Some(scanner) => Arc::new(core_api::email::MailStack {
            transport: Arc::new(core_mail::NoopTransport),
            scanner,
            from_address: config.email_from_address.clone(),
            from_name: config.email_from_name.clone(),
        }),
        None => Arc::new(core_api::email::MailStack::from_config(&config)),
    };
    let state = AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail,
        store: Arc::clone(&store) as Arc<dyn core_store::ObjectStore>,
        routing: Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&admin_url)
            .expect("lazy reporting pool"),
    };
    let app = core_api::routes::build_router(state.clone());
    (state, app, store)
}

/// The admin pool all seven targets carried under `connect`/`connect_admin`.
pub async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
}

/// The bearer-token shape two targets used.
pub async fn send(
    app: &axum::Router,
    method: axum::http::Method,
    uri: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    dispatch(app, builder, body).await
}

/// The header-list shape the other five used.
pub async fn send_headers(
    app: &axum::Router,
    method: axum::http::Method,
    uri: &str,
    headers: &[(&str, String)],
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, value.clone());
    }
    dispatch(app, builder, body).await
}

async fn dispatch(
    app: &axum::Router,
    builder: axum::http::request::Builder,
    body: Option<Value>,
) -> axum::response::Response {
    let request = match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(axum::body::Body::from(v.to_string()))
            .unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    };
    app.clone().oneshot(request).await.unwrap()
}

/// Parse a JSON body, naming the status and the raw bytes when it is not JSON.
pub async fn body_json(response: axum::response::Response) -> Value {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => panic!(
            "body_json parse failed ({status}): {} :: {e}",
            String::from_utf8_lossy(&bytes)
        ),
    }
}

/// `("authorization", "Bearer …")`, for the header-list `send_headers`.
pub fn auth(token: &str) -> (&'static str, String) {
    ("authorization", format!("Bearer {token}"))
}

/// POST a register payload and return the parsed body.
pub async fn register(app: &axum::Router, payload: Value) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(payload),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    body_json(response).await
}

/// The payload five targets shared: `owner@<slug>.test` and `Owner`.
pub async fn register_org(app: &axum::Router, org_name: &str, slug: &str) -> Value {
    register(
        app,
        json!({
            "org_name": org_name,
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        }),
    )
    .await
}

/// The bearer-only form of the same call.
pub async fn register_org_token(app: &axum::Router, org_name: &str, slug: &str) -> String {
    register_org(app, org_name, slug).await["access_token"]
        .as_str()
        .expect("access token")
        .to_string()
}
