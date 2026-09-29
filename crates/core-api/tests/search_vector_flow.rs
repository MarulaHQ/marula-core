//! End-to-end vector-search seam through the REAL core-api router.
//!
//! Purpose: none-mode degradation of every new surface — the search API's
//! response shape is byte-stable with the vector leg dark, the internal
//! upsert indexes rows without vectors, and the backfill reports a typed
//! reason and embeds nothing. Also: an org WITH a configured provider key
//! but no reachable endpoint degrades the same way (search never 5xxs
//! because AI is down), and the backfill sweep is idempotent (a completed
//! sweep embeds nothing; reason-only re-runs stay inert).
//!
//! Non-purpose: SQL-layer RLS/fusion behavior (core-db db_suite), live
//! provider HTTP (ai:: parse unit tests), the worker indexer loop.

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
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("vector e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    // Enable the internal machine surface for the upsert/backfill probes.
    config.internal_token = Some("test-internal-token".to_string());
    AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&config.database_url)
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
    send_with_headers(app, method, uri, bearer, body, &[]).await
}

/// Sends with extra headers — the internal effect surface authenticates via
/// `x-internal-token`, not the JWT Authorization header.
async fn send_with_headers(
    app: &Router,
    method: axum::http::Method,
    uri: &str,
    bearer: Option<&str>,
    body: Option<Value>,
    extra: &[(&str, &str)],
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    for (name, value) in extra {
        builder = builder.header(*name, *value);
    }
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

/// Registers a fresh org; returns (owner token, org_id).
async fn register_org(app: &Router, slug: &str) -> (String, Uuid) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Vector Flow Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let body = body_json(response).await;
    let token: String = body["access_token"].as_str().expect("access token").into();
    let org_id = claims_org(&claims_json(&token))
        .unwrap_or_else(|| panic!("org claim missing from access token"));
    (token, org_id)
}

fn claims_json(jwt: &str) -> Value {
    let payload = jwt.split('.').nth(1).expect("jwt payload segment");
    serde_json::from_slice(
        &data_encoding::BASE64URL_NOPAD
            .decode(payload.as_bytes())
            .expect("jwt payload base64url"),
    )
    .expect("jwt payload json")
}

fn claims_org(claims: &Value) -> Option<Uuid> {
    claims["org"].as_str().and_then(|value| value.parse().ok())
}

#[tokio::test]
async fn vector_leg_stays_dark_in_none_mode_and_backfill_is_inert() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("vector-flow-{}", Uuid::now_v7().simple());
    let (token, org_id) = register_org(&app, &slug).await;

    // The org master switch gates the vector leg too (default OFF turns
    // it dark for a different typed reason); this flow exercises none-mode
    // behavior specifically, so the switch is turned ON up front.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/org",
        Some(&token),
        Some(json!({ "key": "ai.features_enabled", "value": true })),
    )
    .await;
    assert_eq!(response.status(), 200, "master switch on");

    // --- internal upsert in none mode: rows land, vectors stay dark --------
    let entity = Uuid::now_v7();
    let response = send_with_headers(
        &app,
        axum::http::Method::POST,
        "/internal/search/upsert",
        None,
        Some(json!({
            "org_id": org_id,
            "entries": [{ "entity_kind": "ticket", "entity_id": entity,
                          "parent_id": null, "title": "Mail relay down",
                          "body": "outbound smtp failing" }]
        })),
        &[("x-internal-token", "test-internal-token")],
    )
    .await;
    assert_eq!(response.status(), 200, "none-mode upsert must succeed");
    assert_eq!(body_json(response).await["applied"], 1);

    let vectors: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM search_embeddings WHERE org_id = $1")
            .bind(org_id)
            .fetch_one(&state.pool)
            .await
            .expect("vector count");
    assert_eq!(vectors, 0, "none mode stores no vectors");

    // --- unified search: legacy shape, lexical results only ----------------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/search?q=relay",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    assert_eq!(body["total"], 1, "lexical leg matches the indexed ticket");
    assert_eq!(body["results"][0]["entity_kind"], "ticket");
    // The response contract is unchanged: same keys as before the vector leg.
    let keys: Vec<&str> = body["results"][0]
        .as_object()
        .expect("result object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec![
            "entity_id",
            "entity_kind",
            "parent_id",
            "score",
            "snippet",
            "title"
        ],
        "response contract unchanged: same six keys, any serialization order"
    );

    // --- backfill in none mode: typed reason, zero writes, re-run inert ----
    let response = send_with_headers(
        &app,
        axum::http::Method::POST,
        "/internal/search/backfill",
        None,
        Some(json!({ "org_id": org_id, "max_batches": 1 })),
        &[("x-internal-token", "test-internal-token")],
    )
    .await;
    assert_eq!(response.status(), 200);
    let report = body_json(response).await;
    assert_eq!(report["embedded"], 0);
    assert_eq!(report["complete"], true);
    assert!(
        report["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("none"),
        "none mode reports a typed reason: {report}"
    );

    // Re-running changes nothing (idempotent sweep).
    let response = send_with_headers(
        &app,
        axum::http::Method::POST,
        "/internal/search/backfill",
        None,
        Some(json!({ "org_id": org_id })),
        &[("x-internal-token", "test-internal-token")],
    )
    .await;
    assert_eq!(body_json(response).await["embedded"], 0);

    // --- internal token gate still applies to the new surface --------------
    let response = send_with_headers(
        &app,
        axum::http::Method::POST,
        "/internal/search/backfill",
        None,
        Some(json!({ "org_id": org_id })),
        &[("x-internal-token", "wrong-token")],
    )
    .await;
    assert_eq!(response.status(), 401, "invalid internal token rejected");
}

#[tokio::test]
async fn configured_but_unreachable_provider_degrades_search_never_5xxs() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("vector-degrade-{}", Uuid::now_v7().simple());
    let (token, org_id) = register_org(&app, &slug).await;

    // The org master switch gates the vector leg too; this flow exercises
    // provider-down degradation, so the switch is turned ON up front.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/org",
        Some(&token),
        Some(json!({ "key": "ai.features_enabled", "value": true })),
    )
    .await;
    assert_eq!(response.status(), 200, "master switch on");

    // Configure OpenAI BYOK against an unroutable gateway: provider present,
    // network dead. `OPENAI_BASE_URL` points at a closed port for this test.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/byok",
        Some(&token),
        Some(json!({ "provider": "openai", "api_key": "sk-test-degrade" })),
    )
    .await;
    assert_eq!(response.status(), 200, "byok seal must succeed");

    // Activate the provider: the BYOK key store and the settings-registry
    // active-provider setting are two declarations — one key, one switch.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/org",
        Some(&token),
        Some(json!({ "key": "ai.completion_provider", "value": "openai" })),
    )
    .await;
    assert_eq!(response.status(), 200, "activate openai must succeed");

    // Force the adapter at the local kernel loopback that closes every
    // connection immediately.
    std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9/v1");

    // The index row exists so the lexical leg has something to match.
    let response = send_with_headers(
        &app,
        axum::http::Method::POST,
        "/internal/search/upsert",
        None,
        Some(json!({
            "org_id": org_id,
            "entries": [{ "entity_kind": "ticket", "entity_id": Uuid::now_v7(),
                          "parent_id": null, "title": "Mail relay down", "body": "smtp" }]
        })),
        &[("x-internal-token", "test-internal-token")],
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "upsert succeeds even when AI is down"
    );

    // Search still answers 200 with lexical results — never a 5xx.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/search?q=relay",
        Some(&token),
        None,
    )
    .await;
    let status = response.status();
    assert!(
        status.is_success(),
        "search must degrade, not fail: {status}"
    );
    assert_eq!(body_json(response).await["total"], 1);

    // The backfill degrades identically (typed failure, partial sweep).
    let response = send_with_headers(
        &app,
        axum::http::Method::POST,
        "/internal/search/backfill",
        None,
        Some(json!({ "org_id": org_id })),
        &[("x-internal-token", "test-internal-token")],
    )
    .await;
    assert_eq!(response.status(), 200);
    let report = body_json(response).await;
    assert_eq!(report["embedded"], 0);
    assert_eq!(report["complete"], false);

    std::env::remove_var("OPENAI_BASE_URL");
}
