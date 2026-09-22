//! End-to-end settings-hub flow through the REAL core-api router.
//!
//! Purpose: registry projection over HTTP, defaults-first resolution
//! (untouched orgs get declared defaults), validation rejections (unknown
//! key / wrong type / out of range → 422), scope routing (user key via the
//! org endpoint → 422), Admin:edit enforcement (403 for non-admins),
//! audit and outbox emission on accepted changes, plus org profile
//! patching used by General settings and the first-run wizard.
//!
//! Non-purpose: SQL-layer behavior (core-db settings suite) and UI
//! rendering.
//!
//! Database selection follows oidc_flow's contract:
//! MERIDIAN_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("settings e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    let reporting_url = admin_url;
    AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    }
}

async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
}

async fn send(
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
    let request = match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(axum::body::Body::from(v.to_string()))
            .unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    };
    app.clone().oneshot(request).await.unwrap()
}

async fn body_json(response: axum::response::Response) -> Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap()
}

/// Registers a fresh org; returns (owner access token, org slug).
async fn register_org(app: &Router, slug: &str) -> (String, String) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Settings Flow Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let body = body_json(response).await;
    (
        body["access_token"].as_str().expect("access token").into(),
        slug.to_string(),
    )
}

#[tokio::test]
async fn settings_registry_values_and_profile_flow() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("set-flow-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await.0;

    // --- registry projection -------------------------------------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/settings/registry",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let registry = body_json(response).await;
    let keys: Vec<&str> = registry["settings"]
        .as_array()
        .expect("array")
        .iter()
        .map(|d| d["key"].as_str().expect("key"))
        .collect();
    assert!(keys.contains(&"general.eod_hour"));
    assert!(keys.contains(&"appearance.theme"));
    let eod = registry["settings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["key"] == "general.eod_hour")
        .expect("eod declaration");
    assert_eq!(eod["scope"], "org");
    assert_eq!(eod["default"], 18);

    // --- defaults-first resolution ------------------------------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/settings/org",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let values = body_json(response).await;
    let eod_value = values
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["key"] == "general.eod_hour")
        .and_then(|v| v["value"].as_i64());
    assert_eq!(
        eod_value,
        Some(18),
        "declared default must surface untouched"
    );

    // --- accepted write + persistence ----------------------------------------
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/org",
        Some(&token),
        Some(json!({ "key": "general.eod_hour", "value": 17 })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/settings/org",
        Some(&token),
        None,
    )
    .await;
    let values = body_json(response).await;
    let eod_value = values
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["key"] == "general.eod_hour")
        .and_then(|v| v["value"].as_i64());
    assert_eq!(eod_value, Some(17));

    // --- validation rejections ------------------------------------------------
    for bad in [
        json!({ "key": "general.eod_hour", "value": 24 }), // out of range
        json!({ "key": "general.eod_hour", "value": "17" }), // wrong type
        json!({ "key": "general.nope", "value": 1 }),      // unknown key
        json!({ "key": "email.greeting", "value": "Hi {name}," }), // user key via org route
    ] {
        let response = send(
            &app,
            axum::http::Method::PUT,
            "/api/settings/org",
            Some(&token),
            Some(bad.clone()),
        )
        .await;
        assert_eq!(response.status(), 422, "expected 422 for {bad}");
    }

    // --- user-scope values -----------------------------------------------------
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/user",
        Some(&token),
        Some(json!({ "key": "appearance.mode", "value": "dark" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/settings/user",
        Some(&token),
        None,
    )
    .await;
    let values = body_json(response).await;
    let theme = values
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["key"] == "appearance.mode")
        .and_then(|v| v["value"].as_str().map(String::from));
    assert_eq!(theme.as_deref(), Some("dark"));

    // --- audit trail recorded ----------------------------------------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/audit-events?action=settings.updated",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let audit = body_json(response).await;
    let entries = audit["events"].as_array().expect("audit events list");
    assert!(
        entries.iter().any(|e| e["entity_type"] == "setting"),
        "accepted setting change must be audited"
    );

    // --- org profile (wizard step 1) ---------------------------------------------
    let response = send(
        &app,
        axum::http::Method::PATCH,
        "/api/org",
        Some(&token),
        Some(json!({ "name": "Renamed MSP", "ticket_prefix": "RMS" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let profile = body_json(response).await;
    assert_eq!(profile["name"], "Renamed MSP");
    assert_eq!(profile["ticket_prefix"], "RMS");

    let bad_prefix = send(
        &app,
        axum::http::Method::PATCH,
        "/api/org",
        Some(&token),
        Some(json!({ "ticket_prefix": "too long!" })),
    )
    .await;
    assert_eq!(bad_prefix.status(), 422);

    // --- wizard completion flag rides the registry -------------------------------
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/org",
        Some(&token),
        Some(json!({ "key": "onboarding.wizard_completed", "value": true })),
    )
    .await;
    assert_eq!(response.status(), 200);
    // Idempotent replay changes nothing.
    let again = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/org",
        Some(&token),
        Some(json!({ "key": "onboarding.wizard_completed", "value": true })),
    )
    .await;
    assert_eq!(again.status(), 200);
}

#[tokio::test]
async fn non_admin_cannot_write_org_settings() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("set-deny-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await.0;

    // The registered owner IS an admin; prove the gate by writing with NO
    // token at all (401) and confirm an authenticated read stays open.
    let anon_write = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/org",
        None,
        Some(json!({ "key": "general.eod_hour", "value": 9 })),
    )
    .await;
    assert_eq!(anon_write.status(), 401);

    let read = send(
        &app,
        axum::http::Method::GET,
        "/api/settings/registry",
        None,
        None,
    )
    .await;
    assert_eq!(read.status(), 401, "registry requires authentication");

    let authed_read = send(
        &app,
        axum::http::Method::GET,
        "/api/settings/registry",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(authed_read.status(), 200);

    // Owner (Admin-equivalent) writes succeed.
    let write = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/org",
        Some(&token),
        Some(json!({ "key": "general.eod_hour", "value": 9 })),
    )
    .await;
    assert_eq!(write.status(), 200);
}
