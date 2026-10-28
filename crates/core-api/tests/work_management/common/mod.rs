//! Shared harness for the `work_management` test binary.
//!
//! Eight targets each carried their own copy of a stack that was mostly the
//! same function under different names. The bodies here are those copies, made
//! `pub`, with the differences they actually carried kept as parameters:
//!
//! - `test_state` builds the same `AppState` + router everywhere, but four
//!   targets changed one `Config` field before building it (`secret_key` for
//!   the two HMAC suites, `dev_tools = false` for the workbar, and the internal
//!   mail token for time discipline). `test_state_with` takes that tune-up as a
//!   closure instead of forking the constructor.
//! - `send` comes in the three shapes the targets used: bearer token, an
//!   arbitrary header list, and bearer plus the internal mail token.
//! - `connect` and `connect_admin` were the same body under two names, so
//!   `connect` is the one name and the old one is an import alias.
//!
//! Behaviour drift the merge surfaces, each deliberate:
//!
//! - Four targets built the reporting pool from `Config::database_url` BEFORE
//!   overriding it. With `DATABASE_URL` exported that is the same URL the pool
//!   should use; with only `MARULA_TEST_DATABASE_URL` set it is the dev
//!   fallback, a database that need not exist on the host. They never read a
//!   report and the pool is lazy, so the misdirection stayed invisible. The
//!   merged harness points the reporting pool at the resolved admin URL, which
//!   is what `approvals`, `time_discipline` and `workbar` already did.
//! - One tracing subscriber is initialised per process instead of once per
//!   target, and the first caller wins. `time_discipline` asked for DEBUG and
//!   the rest for WARN, so verbosity now depends on scheduler order. No test
//!   asserts on log output.
//! - `workbar`'s `send` always set `content-type: application/json`, even on a
//!   bodyless request. The shared `send` sets it only when it sends a body,
//!   which is what the other seven did.
//!
//! Database contract is unchanged: `MARULA_TEST_DATABASE_URL` or `DATABASE_URL`,
//! migrations run on every `test_state()` call, and no schema is reset. The
//! `two_weeks` module keeps its own bootstrap in `tests/support/` (it resets the
//! schema once per process), so it does not use this `test_state`.

#![allow(dead_code)]

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;

/// Boot one `AppState` + router against the test database.
pub async fn test_state() -> (AppState, Router) {
    test_state_with(|_| {}).await
}

/// The same, with a hook to change `Config` first: the four targets that
/// differed only in one config field keep that field and nothing else.
pub async fn test_state_with<F>(tune: F) -> (AppState, Router)
where
    F: FnOnce(&mut Config),
{
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("work_management e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    tune(&mut config);
    let state = AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&admin_url)
            .expect("lazy reporting pool"),
    };
    let app = core_api::routes::build_router(state.clone());
    (state, app)
}

/// The admin pool four targets called `connect` and three called
/// `connect_admin`; one body, one name.
pub async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
}

/// The bearer-token shape five of the eight targets used.
pub async fn send(
    app: &Router,
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

/// The header-list shape `alerts` and `backend_gaps` used, for requests that
/// carry a signature header as well as (or instead of) a bearer.
pub async fn send_headers(
    app: &Router,
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

/// The shape `time_discipline` used: a staff bearer plus the internal mail
/// token, for the worker-facing endpoints.
pub async fn send_internal(
    app: &Router,
    method: axum::http::Method,
    uri: &str,
    bearer: Option<&str>,
    internal_token: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    if let Some(token) = internal_token {
        builder = builder.header("X-Internal-Token", token);
    }
    dispatch(app, builder, body).await
}

async fn dispatch(
    app: &Router,
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
            "body_json parse failed ({status}): {} :: {e}; last SEND lines above identify the endpoint",
            String::from_utf8_lossy(&bytes)
        ),
    }
}

/// Status plus parsed body, for flows that assert on both together.
pub async fn as_json(response: axum::response::Response) -> (u16, Value) {
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).expect("JSON body"))
}

/// `("authorization", "Bearer …")`, for the header-list `send`.
pub fn auth(token: &str) -> (&'static str, String) {
    ("authorization", format!("Bearer {token}"))
}

/// POST a register payload and return the parsed body. The payload stays with
/// the flow: slug, org name and display name are what make each registration
/// distinct, and nothing else varied.
pub async fn register(app: &Router, payload: Value) -> Value {
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

/// The payload six targets shared: `owner@<slug>.test` and `Owner`.
pub async fn register_org(app: &Router, org_name: &str, slug: &str) -> Value {
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

/// The bearer-only form four targets used.
pub async fn register_org_token(app: &Router, org_name: &str, slug: &str) -> String {
    register_org(app, org_name, slug).await["access_token"]
        .as_str()
        .expect("access token")
        .to_string()
}
