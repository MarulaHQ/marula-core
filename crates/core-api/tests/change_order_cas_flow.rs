//! Change-order transitions decided by the database, not by a prior read
//! (Wave 54 lane S4a).
//!
//! The transition handler used to read the order's status, decide in Rust,
//! then UPDATE without naming a status. Under READ COMMITTED two operators
//! could both read `draft` — one approving, one rejecting — and the second
//! UPDATE silently overwrote the first decision, so the audit trail named an
//! approver whose approval no longer stood.
//!
//! Purpose: prove that the predecessor status is the UPDATE's predicate, that
//! exactly one of two simultaneous transitions wins, that the loser gets the
//! typed 409 rather than a 500 or a second success, and that a transition from
//! a status the action does not allow is refused.
//!
//! Non-purpose: the change-order CRUD shape, billing-run pricing (the run's own
//! `approved` → `billed` leg is a store-level concern), and the SLA-credit
//! lifecycle.

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> (AppState, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("change-order e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    let state = AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&admin_url)
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

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Change Order Co",
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
        .to_string()
}

/// Seeds one client + one active T&M contract; returns (org id, contract id).
async fn seed_contract(state: &AppState, app: &Router, bearer: &str) -> (Uuid, Uuid) {
    let me =
        body_json(send(app, axum::http::Method::GET, "/api/me", Some(bearer), None).await).await;
    let org_id: Uuid = me["org_id"].as_str().unwrap().parse().unwrap();
    let client_id = Uuid::now_v7();
    let contract_id = Uuid::now_v7();
    sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, 'Amendment Co')")
        .bind(client_id)
        .bind(org_id)
        .execute(&state.pool)
        .await
        .expect("client");
    sqlx::query(
        "INSERT INTO contracts (id, org_id, client_id, number, name, contract_type, status, \
                                currency, hourly_rate_minor, effective_from) \
         VALUES ($1, $2, $3, $4, 'Amendment T&M', 'time_and_materials', 'active', 'USD', 15000, CURRENT_DATE)",
    )
    .bind(contract_id)
    .bind(org_id)
    .bind(client_id)
    .bind(format!("C-{}", Uuid::now_v7().simple()))
    .execute(&state.pool)
    .await
    .expect("contract");
    (org_id, contract_id)
}

async fn create_draft_order(app: &Router, bearer: &str, contract: Uuid) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/contracts/{contract}/change-orders"),
        Some(bearer),
        Some(json!({
            "number": format!("CO-{}", Uuid::now_v7().simple()),
            "title": "Add two managed endpoints",
            "amount_minor": 45_000,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "change-order create must succeed");
    body_json(response).await["id"]
        .as_str()
        .expect("order id")
        .parse()
        .expect("order uuid")
}

async fn order_status(state: &AppState, order: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM contract_change_orders WHERE id = $1")
        .bind(order)
        .fetch_one(&state.pool)
        .await
        .expect("order status")
}

/// An approve and a reject fired at the same draft: one decision stands, the
/// other is the typed 409, and the stored status is the winner's.
#[tokio::test]
async fn concurrent_approve_and_reject_cannot_both_win() {
    let (state, app) = test_state().await;
    let slug = format!("co-race-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let (_org, contract) = seed_contract(&state, &app, &owner).await;
    let order = create_draft_order(&app, &owner, contract).await;
    assert_eq!(order_status(&state, order).await, "draft");

    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let mut calls = Vec::new();
    for action in ["approve", "reject"] {
        let router = app.clone();
        let protected = owner.clone();
        let barrier = Arc::clone(&barrier);
        calls.push(tokio::spawn(async move {
            barrier.wait().await;
            let response = send(
                &router,
                axum::http::Method::POST,
                &format!("/api/contracts/{contract}/change-orders/{order}/transition"),
                Some(&protected),
                Some(json!({ "action": action })),
            )
            .await;
            (action, response.status(), body_json(response).await)
        }));
    }
    let mut outcomes = Vec::new();
    for call in calls {
        outcomes.push(call.await.expect("joined the transition"));
    }
    outcomes.sort_by_key(|(_, status, _)| status.as_u16());
    assert_eq!(
        outcomes[0].1, 200,
        "exactly one transition wins: {outcomes:?}"
    );
    assert_eq!(
        outcomes[1].1, 409,
        "the loser is a typed refusal, not a 500 and not a silent overwrite: {outcomes:?}"
    );

    let winner = outcomes[0].0;
    let expected = if winner == "approve" {
        "approved"
    } else {
        "rejected"
    };
    assert_eq!(
        order_status(&state, order).await,
        expected,
        "the stored status is the winner's decision, not the loser's"
    );
}

/// A transition the current status does not allow is refused by name, and the
/// row keeps the status it had.
#[tokio::test]
async fn a_transition_from_the_wrong_status_is_refused() {
    let (state, app) = test_state().await;
    let slug = format!("co-wrong-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let (_org, contract) = seed_contract(&state, &app, &owner).await;
    let order = create_draft_order(&app, &owner, contract).await;

    let approved = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/contracts/{contract}/change-orders/{order}/transition"),
        Some(&owner),
        Some(json!({ "action": "approve" })),
    )
    .await;
    assert_eq!(approved.status(), 200);
    assert_eq!(order_status(&state, order).await, "approved");

    // Reject expects a draft; this order is approved.
    let rejected = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/contracts/{contract}/change-orders/{order}/transition"),
        Some(&owner),
        Some(json!({ "action": "reject" })),
    )
    .await;
    let status = rejected.status();
    let body = body_json(rejected).await;
    assert_eq!(
        status, 409,
        "a reject of an approved order is a typed conflict: {body}"
    );
    assert_eq!(
        order_status(&state, order).await,
        "approved",
        "the refused transition wrote nothing"
    );
}
