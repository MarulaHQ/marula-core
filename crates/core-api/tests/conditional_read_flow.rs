//! Conditional-read flow (Lane P3 `p52-principal-cache`, part C): the ticket
//! detail read's weak `ETag` and its `If-None-Match` short-circuit, driven
//! through the REAL core-api router against a real Postgres.
//!
//! Purpose: prove a caller holding revision N gets a bodyless `304 Not
//! Modified`, and that a later mutation moves the per-entity seq so the same
//! stale validator gets the full `200` representation again.
//!
//! Non-purpose: the validator grammar itself (the list, wildcard and
//! weak-vs-strong cases are unit tests inside `core_api::http_version`).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> (AppState, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("conditional-read e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    let reporting_url = admin_url;
    let state = AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    };
    let app = core_api::routes::build_router(state.clone());
    (state, app)
}

async fn connect_admin(url: &str) -> Result<PgPool, sqlx::Error> {
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

/// A ticket detail GET that optionally carries `If-None-Match` — the whole
/// point of the flow.
async fn get_ticket(
    app: &Router,
    ticket_id: &str,
    bearer: &str,
    if_none_match: Option<&str>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder()
        .method(axum::http::Method::GET)
        .uri(format!("/api/tickets/{ticket_id}"))
        .header("authorization", format!("Bearer {bearer}"));
    if let Some(tag) = if_none_match {
        builder = builder.header("if-none-match", tag);
    }
    let request = builder.body(axum::body::Body::empty()).unwrap();
    app.clone().oneshot(request).await.unwrap()
}

fn header(response: &axum::response::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!("body_json parse failed ({e}); last SEND lines above identify the endpoint")
    })
}

async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec()
}

async fn register_org(app: &Router, slug: &str) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Conditional Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    body_json(response).await
}

#[tokio::test]
async fn ticket_detail_revalidates_with_a_weak_etag() {
    let (_state, app) = test_state().await;
    let tokens = register_org(&app, &format!("cond-{}", Uuid::now_v7().simple())).await;
    let bearer = tokens["access_token"].as_str().unwrap().to_string();

    let boards = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/boards",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let board = boards[0]["id"].as_str().unwrap().to_string();

    let created = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/tickets",
            Some(&bearer),
            Some(json!({
                "board_id": board,
                "title": "conditional read probe",
                "priority": "p2",
            })),
        )
        .await,
    )
    .await;
    let ticket_id = created["id"].as_str().unwrap().to_string();

    // First read: a plain 200 that now also carries the weak validator.
    let first = get_ticket(&app, &ticket_id, &bearer, None).await;
    assert_eq!(first.status(), axum::http::StatusCode::OK);
    let etag = header(&first, "etag").expect("the 200 carries the entity's weak ETag");
    assert!(
        etag.starts_with("W/\""),
        "validator is weak, not strong: {etag}"
    );
    let expected_suffix = format!("ticket:{ticket_id}:1\"");
    assert!(
        etag.ends_with(expected_suffix.as_str()),
        "a freshly created entity sits at seq 1: {etag}"
    );
    let body = body_json(first).await;
    assert_eq!(
        body["id"].as_str().unwrap(),
        ticket_id.as_str(),
        "the 200 body is the ticket representation, unchanged in shape"
    );

    // Second read with the same validator: no body, just a 304.
    let second = get_ticket(&app, &ticket_id, &bearer, Some(&etag)).await;
    assert_eq!(second.status(), axum::http::StatusCode::NOT_MODIFIED);
    assert_eq!(
        header(&second, "etag").as_deref(),
        Some(etag.as_str()),
        "the 304 echoes the validator that matched"
    );
    assert!(
        body_bytes(second).await.is_empty(),
        "304 Not Modified carries no body"
    );

    // Mutate the ticket: the per-entity seq advances.
    let patched = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/tickets/{ticket_id}"),
        Some(&bearer),
        Some(json!({ "title": "conditional read probe (edited)" })),
    )
    .await;
    assert_eq!(
        patched.status(),
        axum::http::StatusCode::OK,
        "patch succeeds"
    );

    // The OLD validator is stale now, so the same header must yield a full
    // 200 with a NEW validator — never a false 304.
    let third = get_ticket(&app, &ticket_id, &bearer, Some(&etag)).await;
    assert_eq!(
        third.status(),
        axum::http::StatusCode::OK,
        "a stale validator must not revalidate"
    );
    let new_etag = header(&third, "etag").expect("the mutated read carries a validator");
    assert_ne!(new_etag, etag, "the seq moved, so the validator changed");
    let body = body_json(third).await;
    assert_eq!(
        body["title"].as_str().unwrap(),
        "conditional read probe (edited)",
        "the 200 carries the edited representation"
    );
}
