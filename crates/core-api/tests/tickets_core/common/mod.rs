//! Shared harness for the `tickets_core` test binary.
//!
//! Eight targets each carried a copy of this stack. The bodies here are those
//! copies, made `pub`, with the differences they actually carried kept as
//! parameters:
//!
//! - `boot` builds the `AppState` + router and also returns the typed memory
//!   store, because `attachments` asserts on it directly (`state.store` is the
//!   `dyn ObjectStore` trait object and cannot be downcast there).
//! - `boot` takes a scanner override for the same reason: two attachment tests
//!   inject a scripted scanner so a verdict can be forced, the rest use the
//!   config-built edge. `None` is that config-built edge.
//! - `test_state_with(tune)` carries the one `Config` field three targets
//!   changed (`secret_key` twice, the internal worker token once).
//! - `send` comes in the three shapes the targets used: bearer token, an
//!   arbitrary header list, and bearer plus `x-api-key` for the machine-client
//!   surface.
//! - `register()` takes the payload; the org name, slug shape and mailbox are
//!   what each flow registered under, and nothing else varied.
//!
//! Behaviour drift the merge surfaces, deliberate and uniform:
//!
//! - All eight built the reporting pool from `Config::database_url` BEFORE
//!   overriding it. With `DATABASE_URL` exported that is the same URL; with
//!   only `MARULA_TEST_DATABASE_URL` set it is the dev fallback, a database
//!   that need not exist on the host. None of them reads a report and the pool
//!   is lazy, so the misdirection stayed invisible. The merged harness points
//!   the reporting pool at the resolved admin URL.
//! - One tracing subscriber is initialised per process instead of once per
//!   target; the first caller wins. No test asserts on log output.
//!
//! Database contract is unchanged: `MARULA_TEST_DATABASE_URL` or `DATABASE_URL`,
//! migrations run on every boot, no schema is reset.

#![allow(dead_code)]

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use core_store::memory::MemoryStore;
use serde_json::json;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;

/// Everything one boot produces. `store` is the same object `state.store`
/// points at, typed, for the flows that assert on stored objects.
pub struct Boot {
    pub state: AppState,
    pub app: Router,
    pub store: Arc<MemoryStore>,
}

/// Boot against the test database with the config-built mail stack.
pub async fn test_state() -> (AppState, Router) {
    let boot = boot(|_| {}, None).await;
    (boot.state, boot.app)
}

/// Boot with a hook that changes `Config` before the state is built.
pub async fn test_state_with<F>(tune: F) -> (AppState, Router)
where
    F: FnOnce(&mut Config),
{
    let boot = boot(tune, None).await;
    (boot.state, boot.app)
}

/// Boot with a mail scanner override, returning the typed store. `None` keeps
/// the config-built edge (scanning off), which is what every pre-gate test
/// exercises; the gate tests inject a scripted scanner.
pub async fn test_state_scanned(
    scanner: Option<Arc<dyn core_mail::ScanProvider>>,
) -> (AppState, Arc<MemoryStore>) {
    let boot = boot(|_| {}, scanner).await;
    (boot.state, boot.store)
}

/// One `AppState` + router against the test database.
pub async fn boot<F>(tune: F, scanner: Option<Arc<dyn core_mail::ScanProvider>>) -> Boot
where
    F: FnOnce(&mut Config),
{
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("tickets_core e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    tune(&mut config);
    let store = Arc::new(MemoryStore::new());
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
        store: std::sync::Arc::clone(&store) as Arc<dyn core_store::ObjectStore>,
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&admin_url)
            .expect("lazy reporting pool"),
    };
    let app = core_api::routes::build_router(state.clone());
    Boot { state, app, store }
}

/// The admin pool three targets called `connect` and two called
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

/// The header-list shape `labels_scan` and `merge_workbench` used.
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

/// The shape `dispatch_v2` used: a staff bearer or a machine API key.
pub async fn send_api_key(
    app: &Router,
    method: axum::http::Method,
    uri: &str,
    bearer: Option<&str>,
    api_key: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    if let Some(key) = api_key {
        builder = builder.header("x-api-key", key);
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

/// Raw response bytes, for downloads the attachments flow asserts on.
pub async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec()
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

/// A slug that is unique per run so the suite is re-runnable against a
/// persistent database.
pub fn unique_slug(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        &uuid::Uuid::now_v7().simple().to_string()[..12]
    )
}

/// POST a register payload and return the parsed body. The payload stays with
/// the flow: org name, slug shape and mailbox are what make each registration
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

/// The payload four targets shared: `owner@<slug>.test` and `Owner`.
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

/// The bearer-only form of the same call.
pub async fn register_org_token(app: &Router, org_name: &str, slug: &str) -> String {
    register_org(app, org_name, slug).await["access_token"]
        .as_str()
        .expect("access token")
        .to_string()
}
