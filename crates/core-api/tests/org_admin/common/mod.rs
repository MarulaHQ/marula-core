//! Shared harness for the `org_admin` test binary.
//!
//! Eight targets each carried a copy of this stack. The bodies here are those
//! copies, made `pub`, with the differences they actually carried kept as
//! parameters:
//!
//! - `test_state_with(tune)` carries the one `Config` field five targets
//!   changed before building the state — a deterministic `secret_key` for the
//!   org-settings encryption paths and for the BYOK/DNS suites.
//! - `send` comes in the two shapes the targets used: a bearer token
//!   (`settings`, `byok`, `org_users`) and a header list (`org_custom_domain`,
//!   `org_suspension`, `org_usage`, `settings_gaps`, `onboarding`).
//! - `register()` takes the payload; org name, slug shape and mailbox are what
//!   each flow registered under. `register_org_with_state` is the
//!   register-then-read-the-org-id shape three of them used.
//!
//! Behaviour drift the merge surfaces, deliberate and uniform: all eight built
//! the reporting pool from `Config::database_url` BEFORE overriding it. With
//! `DATABASE_URL` exported that is the same URL; with only
//! `MARULA_TEST_DATABASE_URL` set it is the dev fallback, a database that need
//! not exist on the host. None of them reads a report and the pool is lazy, so
//! the misdirection stayed invisible. The merged harness points the reporting
//! pool at the resolved admin URL. The subscriber is initialised once per
//! process instead of once per target; no test asserts on log output.
//!
//! Database contract is unchanged: `MARULA_TEST_DATABASE_URL` or `DATABASE_URL`,
//! migrations run on every boot, no schema is reset.

#![allow(dead_code)]

use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::json;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

/// Boot against the test database.
pub async fn test_state() -> AppState {
    test_state_with(|_| {}).await
}

/// The same, with a hook that changes `Config` first.
pub async fn test_state_with<F>(tune: F) -> AppState
where
    F: FnOnce(&mut Config),
{
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("org_admin e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
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
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&admin_url)
            .expect("lazy reporting pool"),
    }
}

/// The admin pool all eight targets carried under the name `connect`.
pub async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
}

/// The bearer-token shape three targets used.
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
            "body_json parse failed ({status}): {} :: {e}; last SEND lines above identify the endpoint",
            String::from_utf8_lossy(&bytes)
        ),
    }
}

/// `("authorization", "Bearer …")`, for the header-list `send`.
pub fn auth(token: &str) -> (&'static str, String) {
    ("authorization", format!("Bearer {token}"))
}

/// A slug that is unique per run so the suite is re-runnable against a
/// persistent database.
pub fn unique_slug(prefix: &str) -> String {
    format!("{prefix}-{}", &Uuid::now_v7().simple().to_string()[..12])
}

/// POST a register payload and return the parsed body. The payload stays with
/// the flow: org name, slug shape and mailbox are what make each registration
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

/// Registers an org and reads its id back through the admin pool, the shape
/// `org_custom_domain`, `org_suspension` and `org_usage` used.
pub async fn register_org_with_state(
    app: &axum::Router,
    state: &AppState,
    org_name: &str,
    slug: &str,
) -> (String, Uuid) {
    let token = register_org_token(app, org_name, slug).await;
    let org_id: Uuid = sqlx::query_scalar("SELECT id FROM orgs WHERE slug = $1")
        .bind(slug)
        .fetch_one(&state.pool)
        .await
        .expect("org id");
    (token, org_id)
}
