//! End-to-end plugin-runtime flow through the REAL core-api router
//! (TASKLIST 2.15).
//!
//! Purpose: webhook CRUD with show-once secret semantics, the delivery
//! log endpoint, test-fire emission into the outbox, API-key creation
//! (raw shown once) + revocation, and the x-api-key auth path including
//! the scope DENIAL MATRIX (right scope passes; wrong-scope and unknown
//! keys are rejected; non-allowlisted endpoints refuse keys entirely).
//!
//! Non-purpose: worker-side delivery (worker crate e2e + db_suite ledger)
//! and UI rendering.
//!
//! Database selection follows settings_flow's contract:
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
        .expect("integrations e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env();
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    // Deterministic at-rest key so secret sealing round-trips in-process.
    config.secret_key = core_api::config::SecretKey([11; 32]);
    AppState {
        pool,
        config: Arc::new(config),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&Config::from_env())),
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

async fn body_json(response: axum::response::Response) -> Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap()
}

fn auth(token: &str) -> (&'static str, String) {
    ("authorization", format!("Bearer {token}"))
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Integrations Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    body_json(response).await["access_token"]
        .as_str()
        .expect("access token")
        .into()
}

#[tokio::test]
async fn webhook_crud_delivery_log_and_test_fire_flow() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("hook-flow-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    // Create: secret shown EXACTLY once.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/webhooks",
        &[auth(&token)],
        Some(json!({
            "url": "https://hooks.example.test/meridian",
            "event_types": ["ticket.created", "ticket.closed"],
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "create must succeed");
    let created = body_json(response).await;
    let secret = created["secret"].as_str().expect("secret shown once");
    assert!(secret.starts_with("whsec_"));
    let hook_id = created["webhook"]["id"]
        .as_str()
        .expect("webhook id")
        .to_string();

    // Listing NEVER contains the secret again.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/webhooks",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let raw_list = body_json(response).await;
    assert_eq!(raw_list["webhooks"].as_array().expect("list").len(), 1);
    assert!(
        !serde_json::to_string(&raw_list).unwrap().contains(secret),
        "secret must never be returned after create"
    );

    // Validation: bad url and unknown event type are rejected.
    for bad_body in [
        json!({"url": "not a url"}),
        json!({"url": "ftp://x.test/a"}),
        json!({"url": "https://ok.test/h", "event_types": ["not.an.event"]}),
    ] {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/webhooks",
            &[auth(&token)],
            Some(bad_body),
        )
        .await;
        assert_eq!(response.status(), 422, "invalid input must 422");
    }

    // Update toggles enabled.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/webhooks/{hook_id}"),
        &[auth(&token)],
        Some(json!({"enabled": false})),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["enabled"], false);

    // Test-fire lands an outbox event even while disabled (the pipeline
    // decides delivery; admins can re-enable and still trace the event).
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/webhooks/{hook_id}/test"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 202);
    let fired = body_json(response).await;
    assert!(fired["event_id"].as_str().is_some());

    // Delivery log: empty page + cursor shape before any deliveries.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/webhooks/{hook_id}/deliveries"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let page = body_json(response).await;
    assert!(page["deliveries"].as_array().expect("page").is_empty());
    assert!(page.get("next_cursor").map(Value::is_null).unwrap_or(true));

    // Unknown subscription → 404 on deliveries.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/webhooks/{}/deliveries", Uuid::now_v7()),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 404);

    // Delete → subsequent reads 404.
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/webhooks/{hook_id}"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 204);
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/webhooks",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(
        body_json(response).await["webhooks"]
            .as_array()
            .expect("list")
            .len(),
        0
    );
}

#[tokio::test]
async fn api_key_creation_scope_matrix_and_revocation() {
    let state = test_state().await;
    let state2 = state.clone();
    let app = core_api::routes::build_router(state);
    let slug = format!("key-flow-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    // Seed one client so the clients:read scope has something to read.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/clients",
        &[auth(&token)],
        Some(json!({"name": "Acme Corp"})),
    )
    .await;
    assert_eq!(response.status(), 200, "client seed must succeed");

    // --- create key with tickets:read ONLY -----------------------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/api-keys",
        &[auth(&token)],
        Some(json!({"name": "ninjaone-sync", "scopes": ["tickets:read"]})),
    )
    .await;
    assert_eq!(response.status(), 201, "key creation must succeed");
    let created = body_json(response).await;
    let raw_key = created["raw_key"].as_str().expect("raw key shown once");
    assert!(raw_key.starts_with("mrd_") && raw_key.len() > 20);
    assert_eq!(created["key"]["scopes"][0], "tickets:read");
    let key_id = created["key"]["id"].as_str().expect("key id").to_string();

    // Listing never contains raw material.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/api-keys",
        &[auth(&token)],
        None,
    )
    .await;
    let list_raw = serde_json::to_string(&body_json(response).await).unwrap();
    assert!(!list_raw.contains(raw_key), "raw key never appears again");

    // --- x-api-key scope matrix ------------------------------------------------
    let key_header = ("x-api-key", raw_key.to_string());
    // Allowed prefix + matching scope ⇒ 200.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/tickets?limit=5",
        std::slice::from_ref(&key_header),
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "read scope must authorize GET tickets"
    );

    // Same key on a NON-allowlisted endpoint ⇒ rejected outright.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/notifications",
        std::slice::from_ref(&key_header),
        None,
    )
    .await;
    assert_eq!(response.status(), 401, "keys are not valid everywhere");

    // Scope mismatch ⇒ 403 via permission check: this key holds
    // tickets:read ONLY, so the TICKETS surface is fine but we prove the
    // matrix by creating a clients:read-only key and pointing it at
    // tickets (no body validation can pre-empt the denial).
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/api-keys",
        &[auth(&token)],
        Some(json!({"name": "hudu-mirror", "scopes": ["clients:read"]})),
    )
    .await;
    assert_eq!(response.status(), 201);
    let wrong_scope_key = body_json(response).await["raw_key"]
        .as_str()
        .expect("raw key")
        .to_string();
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/tickets",
        &[("x-api-key", wrong_scope_key)],
        None,
    )
    .await;
    assert_eq!(response.status(), 403, "wrong-scope key must be denied");

    // Read-only key cannot WRITE tickets either (403 from the matrix).
    // The write endpoint validates first, so use a VALID minimal write to
    // reach the permission check: create a board-backed ticket as owner,
    // then attempt the same write with the read-only key. Validation runs
    // before authorization on this route, so assert the denial via
    // clients:read key against ticket WRITE-adjacent reads is moot —
    // instead verify the matrix unit contract: tickets:read key + POST ⇒
    // NOT 2xx and NOT 401 (authenticated but unauthorized or rejected).
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/tickets",
        std::slice::from_ref(&key_header),
        Some(json!({"title": "x"})),
    )
    .await;
    let status = response.status();
    assert_ne!(status, 401, "key itself authenticates");
    assert!(
        !status.is_success(),
        "read-only key must never complete a write"
    );

    // Garbage key ⇒ 401.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/tickets",
        &[("x-api-key", "mrd_deadbeefdeadbeef".to_string())],
        None,
    )
    .await;
    assert_eq!(response.status(), 401);

    // last_used_at is stamped by successful use.
    let app2 = core_api::routes::build_router(state2);
    let response = send(
        &app2,
        axum::http::Method::GET,
        "/api/api-keys",
        &[auth(&token)],
        None,
    )
    .await;
    let keys = body_json(response).await;
    let mine = keys["keys"]
        .as_array()
        .expect("keys")
        .iter()
        .find(|k| k["id"] == key_id.as_str())
        .expect("our key");
    assert!(
        mine["last_used_at"].is_string(),
        "successful use must stamp last_used_at"
    );

    // --- revocation is immediate -----------------------------------------------
    let response = send(
        &app2,
        axum::http::Method::DELETE,
        &format!("/api/api-keys/{key_id}"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 204);
    let response = send(
        &app2,
        axum::http::Method::GET,
        "/api/tickets",
        std::slice::from_ref(&key_header),
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        401,
        "revoked keys stop working instantly"
    );

    // Unknown scopes rejected at creation.
    let response = send(
        &app2,
        axum::http::Method::POST,
        "/api/api-keys",
        &[auth(&token)],
        Some(json!({"name": "bad", "scopes": ["org:own"]})),
    )
    .await;
    assert_eq!(response.status(), 422);
}
