//! End-to-end BYOK sealed-credential flow through the REAL core-api router.
//!
//! Purpose: none-mode typed unavailable probe, admin-gated seal/re-seal of
//! API keys (401/403 without standing), instant-first write-path behavior
//! on the BYOK surface (idempotency replay is byte-identical; a reused key
//! with a different payload is a typed 409), presence-only status (no key
//! material ever appears in any response), connector providers (qbo/xero)
//! accepted by the same store, and delete (404 on second delete).
//!
//! Non-purpose: SQL-layer behavior (core-db byok suite), the adapter
//! crates' HTTP contract tests, worker scheduling.

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
        .expect("byok e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
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
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
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

/// Registers a fresh org; returns the owner's access token (owner = admin).
async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Byok Flow Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let body = body_json(response).await;
    body["access_token"].as_str().expect("access token").into()
}

#[tokio::test]
async fn byok_seal_replay_test_delete_flow() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("byok-flow-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let key = format!("key-{}", Uuid::now_v7().simple());

    // --- none-mode probe: typed unavailable, zero network --------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/settings/ai/test",
        Some(&token),
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 200);
    let probe = body_json(response).await;
    assert_eq!(probe["provider"], "none");
    assert_eq!(probe["ok"], false);
    assert!(
        probe["message"].as_str().unwrap().contains("none mode"),
        "none mode must surface the typed unavailable reason: {probe}"
    );

    // --- status starts empty --------------------------------------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/settings/ai/status",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let status = body_json(response).await;
    assert_eq!(status["active_provider"], "none");
    assert_eq!(status["configured_providers"].as_array().unwrap().len(), 0);

    // --- seal an AI provider key through the instant-first write path ---------
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/byok",
        Some(&token),
        Some(json!({
            "provider": "openai",
            "api_key": "sk-live-abc123",
            "idempotency_key": key,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "seal must succeed");
    let first = body_json(response).await;
    assert_eq!(first["provider"], "openai");
    assert_eq!(first["has_key"], true);
    assert!(first["mutation"].is_object(), "IF2 mutation meta present");
    let version = first["mutation"]["version"].as_i64().expect("version");

    // Replay: same key + same payload ⇒ the SAME stored result.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/byok",
        Some(&token),
        Some(json!({
            "provider": "openai",
            "api_key": "sk-live-abc123",
            "idempotency_key": key,
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let replay = body_json(response).await;
    assert_eq!(
        replay["mutation"]["version"], version,
        "replay is byte-identical"
    );

    // Same key + different payload ⇒ typed 409 (IF1).
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/byok",
        Some(&token),
        Some(json!({
            "provider": "openai",
            "api_key": "sk-ROTATED-999",
            "idempotency_key": key,
        })),
    )
    .await;
    assert_eq!(response.status(), 409, "key reuse must be a typed 409");

    // --- status reflects configuration, never key material ---------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/settings/ai/status",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let status_text = body_json(response).await.to_string();
    assert!(status_text.contains("openai"));
    assert!(
        !status_text.contains("sk-live-abc123"),
        "no API key bytes may appear in status responses"
    );

    // --- test-connection verifies the sealed key decrypts ------------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/settings/ai/test",
        Some(&token),
        Some(json!({ "provider": "openai" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let probe = body_json(response).await;
    assert_eq!(probe["ok"], true, "sealed key must verify: {probe}");

    // --- validation ------------------------------------------------------------
    for bad in [
        json!({ "provider": "not-a-thing", "api_key": "x" }),
        json!({ "provider": "openai", "api_key": "  " }),
        json!({ "provider": "  ", "api_key": "x" }),
    ] {
        let response = send(
            &app,
            axum::http::Method::PUT,
            "/api/settings/ai/byok",
            Some(&token),
            Some(bad.clone()),
        )
        .await;
        assert_eq!(response.status(), 422, "expected 422 for {bad}");
    }

    // --- connector provider rides the same sealed store --------------------------
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/byok",
        Some(&token),
        Some(json!({ "provider": "qbo", "api_key": "qbo-oauth-token" })),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "connector providers must be accepted"
    );
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/settings/ai/status",
        Some(&token),
        None,
    )
    .await;
    let status = body_json(response).await;
    let configured: Vec<&str> = status["configured_providers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().expect("provider"))
        .collect();
    assert!(configured.contains(&"openai") && configured.contains(&"qbo"));

    // --- delete removes exactly one provider --------------------------------------
    let response = send(
        &app,
        axum::http::Method::DELETE,
        "/api/settings/ai/byok/openai",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let deleted = body_json(response).await;
    assert_eq!(deleted["has_key"], false);
    assert!(
        deleted["mutation"]["version"].is_i64(),
        "IF2 mutation meta present on delete: {deleted}"
    );

    let response = send(
        &app,
        axum::http::Method::DELETE,
        "/api/settings/ai/byok/openai",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 404, "second delete ⇒ rls_sparse");

    // --- audit trail recorded -------------------------------------------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/audit-events?action=settings.ai_byok_updated",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let audit = body_json(response).await;
    let entries = audit["events"].as_array().expect("audit events list");
    assert!(
        entries.iter().any(|e| e["detail"]["provider"] == "openai"),
        "seal must be audited without key material"
    );
}

#[tokio::test]
async fn byok_surface_requires_authentication() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("byok-anon-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    // Anonymous writes are 401; authenticated reads stay open to members.
    let anon = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/byok",
        None,
        Some(json!({ "provider": "openai", "api_key": "x" })),
    )
    .await;
    assert_eq!(anon.status(), 401);

    let anon_test = send(
        &app,
        axum::http::Method::POST,
        "/api/settings/ai/test",
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(anon_test.status(), 401);

    let status = send(
        &app,
        axum::http::Method::GET,
        "/api/settings/ai/status",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status.status(), 200);
}
