//! Shared harness for the `portal` test binary.
//!
//! Four targets each carried a copy of this stack. The bodies here are those
//! copies, made `pub`, with the differences they actually carried kept as
//! parameters:
//!
//! - `boot_with(tune)` is the router-shaped boot all four used; `tune` is
//!   empty everywhere today, and stays a parameter so a flow that needs a
//!   `Config` field does not have to fork the boot again.
//! - `send` is the plain bearer shape the share-link and client-vault flows
//!   used. `send_caller` and `send_caller_headers` are the portal shapes: they
//!   mint one `CallerIp` per request and stamp it as `X-Forwarded-For`,
//!   because the portal sign-in gate is durable and keyed per address, and a
//!   headerless `oneshot` collapses every case into the shared `unknown`
//!   bucket (Wave 54 follow-up F7).
//! - `register_org` is the `owner@<slug>.test` shorthand; the flows that read
//!   the org back, or return an `Org`, keep their own wrapper.
//!
//! The caller fixture is one module for the whole binary (`mod caller` in
//! `main.rs`), not one copy per flow. Two copies would draw two independent
//! address bases, which is exactly the cross-binary collision the fixture
//! documents; one module makes every mint in this process a distinct slot.
//!
//! Behaviour drift the merge surfaces, deliberate and uniform: all four built
//! their pool from the resolved URL already, so nothing moved there. The
//! subscriber is initialised once per process instead of once per target; no
//! test asserts on log output.
//!
//! Database contract is unchanged: `MARULA_TEST_DATABASE_URL` or `DATABASE_URL`,
//! migrations run on every boot, no schema is reset. `portal_hash_bound` stays
//! a top-level target: its cases drive `hash_gate::global`, a process-wide
//! `OnceLock` whose first caller fixes the slot count, and every one of its
//! cases is `serial`.

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
        .expect("portal e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
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

/// `state_with`, plus the composed router all four targets used.
pub async fn boot_with<F>(tune: F) -> (AppState, axum::Router)
where
    F: FnOnce(&mut Config),
{
    let state = state_with(tune).await;
    let app = core_api::routes::build_router(state.clone());
    (state, app)
}

/// The admin pool all four targets carried under `connect`/`connect_admin`.
pub async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
}

/// The plain bearer shape the share-link and client-vault flows used.
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
    dispatch(app, builder, body, false).await
}

/// The portal shape: one minted caller per request.
pub async fn send_caller(
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
    dispatch(app, builder, body, true).await
}

/// The portal cookie-session twin: request headers plus a minted caller.
pub async fn send_caller_headers(
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
    dispatch(app, builder, body, true).await
}

async fn dispatch(
    app: &axum::Router,
    builder: axum::http::request::Builder,
    body: Option<Value>,
    stamp_caller: bool,
) -> axum::response::Response {
    let request = match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(axum::body::Body::from(v.to_string()))
            .unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    };
    let request = if stamp_caller {
        crate::caller::fresh_caller_ip().stamp(request)
    } else {
        request
    };
    app.clone().oneshot(request).await.unwrap()
}

/// Parse a JSON body, naming the status and the raw bytes when it is not JSON.
pub async fn body_json(response: axum::response::Response) -> Value {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
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
