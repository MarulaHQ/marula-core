//! Shared harness for the `documents_kb` test binary.
//!
//! Six targets each carried a copy of this stack. The bodies here are those
//! copies, made `pub`, with the differences they actually carried kept as
//! parameters:
//!
//! - `state_with(tune)` builds the `AppState` all six wanted, and
//!   `boot_with(tune)` adds the composed router for the flow that drives one.
//!   `tune` carries the deterministic `secret_key` five of them set and the
//!   machine `internal_token` the KB and async-import suites need.
//! - `send_headers` is the header-list shape all six used; `send` is the
//!   bearer shorthand for the same thing.
//! - `register` takes the payload, because the org name is what each flow
//!   registered under; `register_org`/`register_org_token` are the
//!   `owner@<slug>.test` shorthand.
//!
//! Behaviour drift the merge surfaces, deliberate and uniform: four targets
//! built the reporting pool from `Config::database_url` BEFORE overriding it.
//! With `DATABASE_URL` exported that is the same URL; with only
//! `MARULA_TEST_DATABASE_URL` set it is the dev fallback, a database that need
//! not exist on the host. None of them reads a report and the pool is lazy, so
//! the misdirection stayed invisible. The merged harness points the reporting
//! pool at the resolved admin URL. The subscriber is initialised once per
//! process instead of once per target; no test asserts on log output.
//!
//! Database contract is unchanged: `MARULA_TEST_DATABASE_URL` or `DATABASE_URL`,
//! migrations run on every boot, no schema is reset. `imports_flow` stays a
//! top-level target because its boot drops the schema once per process.

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
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("documents/kb e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    tune(&mut config);
    AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&admin_url)
            .expect("lazy reporting pool"),
    }
}

/// `state_with`, plus the composed router.
pub async fn boot_with<F>(tune: F) -> (AppState, axum::Router)
where
    F: FnOnce(&mut Config),
{
    let state = state_with(tune).await;
    let app = core_api::routes::build_router(state.clone());
    (state, app)
}

/// The admin pool all six targets carried under `connect`.
pub async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
}

/// The header-list shape all six used.
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
    let request = match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(axum::body::Body::from(v.to_string()))
            .unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    };
    app.clone().oneshot(request).await.unwrap()
}

/// The bearer shorthand: one `authorization` header.
pub async fn send(
    app: &axum::Router,
    method: axum::http::Method,
    uri: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let headers: Vec<(&str, String)> = match bearer {
        Some(token) => vec![("authorization", format!("Bearer {token}"))],
        None => Vec::new(),
    };
    send_headers(app, method, uri, &headers, body).await
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
    let response = send_headers(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(payload),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    body_json(response).await
}

/// The payload all six shared: `owner@<slug>.test` and `Owner`.
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
