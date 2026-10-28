//! Shared harness for the `auth_identity` test binary.
//!
//! Seven targets each carried a copy of this stack. The bodies here are those
//! copies, made `pub`, with the differences they actually carried kept as
//! parameters:
//!
//! - `test_state_on(url)` is the constructor three targets used: it takes the
//!   database URL, runs migrations, and builds the state with that URL in the
//!   config and in the reporting pool. `test_state()` is the same call against
//!   the URL from the environment, which is what the other four used.
//! - `test_state_with(tune)` carries the one `Config` field a target changed
//!   before building (`auth_timing` salts `jwt_secret` per instance so its
//!   durable per-IP fingerprints cannot collide with a parallel run's).
//! - `state_on(pool, url)` is the constructor over a caller-owned pool, which
//!   is how the two RLS/racer cases pin the pool shape; `rls_state` is its
//!   app-role sibling.
//! - `send` comes in the three shapes the targets used: bearer token, a header
//!   list, and bearer plus extra headers (the caller-IP header those suites
//!   stamp).
//! - `register()` takes the payload; password, org name and mailbox are what
//!   each flow registered with, and nothing else varied.
//!
//! `directory_groups` keeps its own `send`, because its fourth argument is an
//! `Auth` enum (none / bearer / API key) that no other module in this group
//! uses; it builds the headers and calls the same tower `oneshot` path.
//!
//! Behaviour drift the merge surfaces: `roles` and `directory_groups` already
//! pointed the reporting pool at the admin URL; `auth_timing`, `mfa_enroll_gate`
//! and `mfa_step_up` pointed it at `Config::database_url` before the override
//! (the dev fallback when only `MARULA_TEST_DATABASE_URL` is set). The merged
//! harness always uses the resolved admin URL. The subscriber is initialised
//! once per process instead of once per target; no test asserts on log output.
//!
//! Database contract is unchanged: `MARULA_TEST_DATABASE_URL` or `DATABASE_URL`,
//! migrations run on every boot, no schema is reset.

#![allow(dead_code)]

// The caller-identity fixture, declared once for the whole binary instead of
// once per target: its address counter is a process-global, so one copy per
// file would have been several counters handing out overlapping slots.
#[path = "../../support/caller.rs"]
pub mod caller;

use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::json;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

/// The test database URL, named once.
pub fn database_url() -> String {
    std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("auth_identity e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL")
}

/// A pool with the shape the flow suites used, before migrations.
pub async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .connect(url)
        .await
}

/// An owner pool, migrated. Migrations run as the owner because the app role
/// owns nothing and cannot run DDL.
pub async fn owner_pool(url: &str) -> PgPool {
    let pool = connect(url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    pool
}

/// Rewrites owner credentials to the RLS-constrained application role — the
/// role a deployed core-api connects as, and the only one where a policy can
/// swallow a write.
pub fn app_role_url(admin_url: &str) -> String {
    let (scheme, rest) = admin_url
        .split_once("://")
        .unwrap_or(("postgres", admin_url));
    match rest.split_once('@') {
        Some((_, host)) => format!("{scheme}://marula_app:marula_app@{host}"),
        None => format!("{scheme}://marula_app:marula_app@{rest}"),
    }
}

/// An `AppState` over a pool the caller owns, with `url` in the config and in
/// the reporting pool.
pub fn state_on(pool: PgPool, url: &str) -> AppState {
    state_on_with(pool, url, |_| {})
}

/// The same, with a hook that changes `Config` first.
pub fn state_on_with<F>(pool: PgPool, url: &str, tune: F) -> AppState
where
    F: FnOnce(&mut Config),
{
    let mut config = Config::from_env_allow_dev_fallback().expect("config");
    config.database_url = url.to_string();
    tune(&mut config);
    AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(url)
            .expect("lazy reporting pool"),
    }
}

/// Boot against the environment's test database.
pub async fn test_state() -> AppState {
    test_state_with(|_| {}).await
}

/// Boot against the environment's test database, with a `Config` tune-up.
pub async fn test_state_with<F>(tune: F) -> AppState
where
    F: FnOnce(&mut Config),
{
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let url = database_url();
    let pool = connect(&url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    state_on_with(pool, &url, tune)
}

/// Boot against a named database URL.
pub async fn test_state_on(admin_url: &str) -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let pool = connect(admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    state_on(pool, admin_url)
}

/// `test_state_on`, but the API connects as `marula_app` so every handler
/// statement runs under RLS exactly as it does in a deployed instance.
pub async fn rls_state(admin_url: &str) -> AppState {
    let _owner = owner_pool(admin_url).await;
    let pool = connect(&app_role_url(admin_url))
        .await
        .expect("app-role pool");
    state_on(pool, admin_url)
}

/// The bearer-token shape four of the seven targets used. No caller header:
/// these suites never stamped one, and adding it would change what they send.
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

/// The bearer shape of `session_revocation`, which stamped a fresh caller IP on
/// every request so a durable per-IP lock cannot refuse a sibling case.
pub async fn send_stamped(
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
    dispatch_stamped(app, builder, body).await
}

/// The header-list shape `auth_timing` used, where the caller-IP header is the
/// credential under test and every case names its own.
pub async fn send_headers(
    app: &axum::Router,
    method: axum::http::Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    dispatch(app, builder, body).await
}

/// The machine-client shape: an `x-api-key` instead of a bearer. Only
/// `directory_groups` uses it, through its own `Auth` enum.
pub async fn send_with_api_key(
    app: &axum::Router,
    method: axum::http::Method,
    uri: &str,
    api_key: &str,
    body: Option<Value>,
) -> axum::response::Response {
    let builder = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header("x-api-key", api_key);
    dispatch(app, builder, body).await
}

/// The shape `auth_hardening` used: a bearer plus whatever extra headers the
/// case needs (`x-forwarded-for`, a cookie).
pub async fn send_with_headers(
    app: &axum::Router,
    method: axum::http::Method,
    uri: &str,
    bearer: Option<&str>,
    extra_headers: &[(&'static str, &str)],
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    for (key, value) in extra_headers {
        builder = builder.header(*key, *value);
    }
    dispatch_stamped(app, builder, body).await
}

async fn dispatch(
    app: &axum::Router,
    builder: axum::http::request::Builder,
    body: Option<Value>,
) -> axum::response::Response {
    app.clone().oneshot(build(builder, body)).await.unwrap()
}

/// The same, with a caller address this request has to itself unless it names
/// one. The lockout rows are durable and per-IP, and a headerless request
/// resolves to the shared `unknown` bucket, which is how one case's lock used
/// to refuse another case's sign-in (Wave 54 follow-up F7).
async fn dispatch_stamped(
    app: &axum::Router,
    builder: axum::http::request::Builder,
    body: Option<Value>,
) -> axum::response::Response {
    let request = caller::fresh_caller_ip().stamp(build(builder, body));
    app.clone().oneshot(request).await.unwrap()
}

fn build(
    builder: axum::http::request::Builder,
    body: Option<Value>,
) -> axum::http::Request<axum::body::Body> {
    match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(axum::body::Body::from(v.to_string()))
            .unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    }
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

/// A slug that is unique per run so the suite is re-runnable against a
/// persistent database.
pub fn unique_slug(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        &uuid::Uuid::now_v7().simple().to_string()[..12]
    )
}

/// POST a register payload and return the parsed body. The payload stays with
/// the flow: password, org name and mailbox are what make each registration
/// distinct, and nothing else varied.
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

/// The payload three targets shared: `owner@<slug>.test` and `Owner`.
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
        .into()
}
