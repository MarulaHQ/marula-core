//! Event-type registry validation flow (TASKLIST Wave 43 lane 2).
//!
//! Verifies that webhook subscription create/update reject unknown or
//! ungranted event types, accept platform types without a grant, and
//! accept custom types when a grant row exists.

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
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
        .expect("registry e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&admin_url)
        .await
        .expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([11; 32]);
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
            "org_name": "Registry Co",
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

async fn org_id_for(state: &AppState, slug: &str) -> uuid::Uuid {
    let email = format!("owner@{slug}.test");
    let row: (uuid::Uuid,) = sqlx::query_as("SELECT org_id FROM users WHERE email = $1")
        .bind(&email)
        .fetch_one(&state.pool)
        .await
        .expect("org lookup for test");
    row.0
}

async fn grant_event_type(state: &AppState, org_id: uuid::Uuid, event_type: &str) {
    let mut tx = state.pool.begin().await.expect("tx begin");
    sqlx::query("SELECT set_config('app.org_id', $1, true)")
        .bind(org_id.to_string())
        .execute(&mut *tx)
        .await
        .expect("set local org id");
    sqlx::query(
        "INSERT INTO event_type_grants (org_id, event_type, granted_by) VALUES ($1, $2, NULL) \
         ON CONFLICT (org_id, event_type) DO NOTHING",
    )
    .bind(org_id)
    .bind(event_type)
    .execute(&mut *tx)
    .await
    .expect("grant insert");
    tx.commit().await.expect("tx commit");
}

#[tokio::test]
async fn platform_types_accepted_without_grant() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("reg-platform-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/webhooks",
        &[auth(&token)],
        Some(json!({
            "url": "https://hooks.example.test/marula",
            "event_types": ["ticket.created", "ticket.closed", "webhook.test"],
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "known platform types must succeed");
    let created = body_json(response).await;
    assert!(created["webhook"]["id"].as_str().is_some());
}

#[tokio::test]
async fn lane1_types_accepted_without_grant() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("reg-lane1-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/webhooks",
        &[auth(&token)],
        Some(json!({
            "url": "https://hooks.example.test/marula",
            "event_types": ["alert.ingested", "webhook.received"],
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "lane-1 platform types must succeed");
}

#[tokio::test]
async fn unknown_type_rejected_without_grant() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("reg-unknown-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/webhooks",
        &[auth(&token)],
        Some(json!({
            "url": "https://hooks.example.test/marula",
            "event_types": ["my.custom.event"],
        })),
    )
    .await;
    assert_eq!(response.status(), 422, "unregistered type must be rejected");
    let body = body_json(response).await;
    let msg = body["error"]["message"].as_str().expect("message");
    assert!(
        msg.contains("my.custom.event"),
        "error must name the offending type: {msg}"
    );
}

#[tokio::test]
async fn granted_custom_type_accepted() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("reg-granted-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let org_id = org_id_for(&state, &slug).await;

    grant_event_type(&state, org_id, "my.custom.event").await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/webhooks",
        &[auth(&token)],
        Some(json!({
            "url": "https://hooks.example.test/marula",
            "event_types": ["my.custom.event"],
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "granted type must succeed");
}

#[tokio::test]
async fn update_rejects_ungranted_type() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("reg-update-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/webhooks",
        &[auth(&token)],
        Some(json!({
            "url": "https://hooks.example.test/marula",
            "event_types": ["ticket.created"],
        })),
    )
    .await;
    assert_eq!(response.status(), 201);
    let hook_id = body_json(response).await["webhook"]["id"]
        .as_str()
        .expect("hook id")
        .to_string();

    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/webhooks/{hook_id}"),
        &[auth(&token)],
        Some(json!({ "event_types": ["not.registered"] })),
    )
    .await;
    assert_eq!(response.status(), 422, "update must reject unknown type");
}

#[tokio::test]
async fn mixed_platform_and_custom_types_partial_failure() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("reg-mixed-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let org_id = org_id_for(&state, &slug).await;

    grant_event_type(&state, org_id, "my.custom.event").await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/webhooks",
        &[auth(&token)],
        Some(json!({
            "url": "https://hooks.example.test/marula",
            "event_types": ["ticket.created", "my.custom.event"],
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "mixed valid types must succeed");
}
