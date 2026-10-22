//! End-to-end coaching scorecard flows (Wave 14 lane 3) driven through
//! the REAL core-api router against a real Postgres.
//!
//! Purpose: prove the R5 scorecard contract over HTTP — aggregation over
//! the read-model views counts APPROVED time entries only (drafts and
//! rejected entries are never scorecard truth), the privacy model
//! (self-read for every staff member; the team view and other members'
//! cards are owner/admin only), the opt-in gamification toggle (defaults
//! false, PATCH flips it and persists), and window validation.
//!
//! Non-purpose: SQL aggregation math (covered by the read-model views'
//! own semantics), event wire shapes (core-events golden conformance).

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
        .expect("scorecards e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
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
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!("body_json parse failed ({e}); last SEND lines above identify the endpoint")
    })
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Scorecards Co",
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

async fn me(app: &Router, bearer: &str) -> Value {
    body_json(send(app, axum::http::Method::GET, "/api/me", Some(bearer), None).await).await
}

async fn default_board(app: &Router, bearer: &str) -> String {
    let boards = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/boards",
            Some(bearer),
            None,
        )
        .await,
    )
    .await;
    boards
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["is_default"] == true)
        .expect("default board")["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn create_ticket(app: &Router, bearer: &str) -> String {
    let board_id = default_board(app, bearer).await;
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(bearer),
        Some(json!({
            "board_id": board_id,
            "title": "Scorecard fixture ticket",
            "priority": "p3",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket create must succeed");
    body_json(response).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn create_entry(app: &Router, bearer: &str, ticket_id: &str, hours_ago: i64) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/time-entries",
        Some(bearer),
        Some(json!({
            "ticket_id": ticket_id,
            "started_at": (chrono::Utc::now() - chrono::Duration::hours(hours_ago)),
            "ended_at": (chrono::Utc::now() - chrono::Duration::hours(hours_ago)
                + chrono::Duration::minutes(60)),
            "billable": true,
            "notes": "worked on the thing",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "entry create must succeed");
    body_json(response).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn approve_entry(app: &Router, bearer: &str, entry_id: &str) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/time-entries/approvals/decide",
        Some(bearer),
        Some(json!({ "entry_ids": [entry_id], "decision": "approve" })),
    )
    .await;
    assert_eq!(response.status(), 200, "owner approval must succeed");
}

async fn get_json(app: &Router, bearer: &str, uri: &str) -> (u16, Value) {
    let response = send(app, axum::http::Method::GET, uri, Some(bearer), None).await;
    let status = response.status().as_u16();
    (status, body_json(response).await)
}

#[tokio::test]
async fn scorecards_flow() {
    let (_state, app) = test_state().await;
    let bearer = register_org(&app, &format!("score-{}", Uuid::now_v7().simple())).await;
    let ticket_id = create_ticket(&app, &bearer).await;

    // APPROVED entry: submit → owner decision. This is the only workload
    // the scorecard may count.
    let approved = create_entry(&app, &bearer, &ticket_id, 4).await;
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/time-entries/{approved}/submit"),
        Some(&bearer),
        Some(json!({ "note": "for review" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    approve_entry(&app, &bearer, &approved).await;

    // DRAFT entry: logged but never approved — must stay invisible to the
    // scorecard's workload block.
    let _draft = create_entry(&app, &bearer, &ticket_id, 2).await;

    let (status, card) = get_json(&app, &bearer, "/api/scorecards/me").await;
    assert_eq!(status, 200, "{card}");
    assert_eq!(card["workload"]["approved_entries"], json!(1), "{card}");
    assert_eq!(card["workload"]["approved_minutes"], json!(60), "{card}");
    assert_eq!(
        card["workload"]["approved_billable_minutes"],
        json!(60),
        "{card}"
    );
    assert_eq!(card["workload"]["billable_ratio"], json!(1.0), "{card}");
    assert_eq!(card["csat"]["available"], json!(false), "{card}");
    assert_eq!(card["sla"]["response_total"], json!(0), "{card}");
    assert_eq!(card["sla"]["response_attainment"], json!(null), "{card}");

    // Settings: private-by-default (gamification off), PATCH flips and
    // persists.
    let (status, settings) = get_json(&app, &bearer, "/api/scorecards/me/settings").await;
    assert_eq!(status, 200, "{settings}");
    assert_eq!(settings["gamification_opt_in"], json!(false), "{settings}");
    let response = send(
        &app,
        axum::http::Method::PATCH,
        "/api/scorecards/me/settings",
        Some(&bearer),
        Some(json!({ "gamification_opt_in": true })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let (_, settings) = get_json(&app, &bearer, "/api/scorecards/me/settings").await;
    assert_eq!(settings["gamification_opt_in"], json!(true), "{settings}");

    // Window validation mirrors /api/reports: inverted window → 422.
    let (status, body) = get_json(
        &app,
        &bearer,
        "/api/scorecards/me?from=2026-01-02&to=2026-01-01",
    )
    .await;
    assert_eq!(status, 422, "{body}");

    // Team view: owner sees the org roster (one active member).
    let (status, team) = get_json(&app, &bearer, "/api/scorecards").await;
    assert_eq!(status, 200, "{team}");
    assert_eq!(team["scorecards"].as_array().unwrap().len(), 1, "{team}");

    // Privacy: a plain technician reads their own card + settings, but
    // the team view and another member's card are owner/admin only.
    let email = format!("tech-{}@test.local", Uuid::now_v7().simple());
    let invite = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/org/users",
            Some(&bearer),
            Some(json!({
                "email": email,
                "display_name": "Tech",
                "role": "technician",
            })),
        )
        .await,
    )
    .await;
    let setup_token = invite["invite"]["setup_token"]
        .as_str()
        .unwrap()
        .to_string();
    let accept = send(
        &app,
        axum::http::Method::POST,
        "/api/org/users/accept-invite",
        None,
        Some(json!({ "token": setup_token, "password": "another-correct-horse" })),
    )
    .await;
    assert_eq!(accept.status().as_u16(), 200, "redemption must succeed");
    let tech_bearer = body_json(accept).await["access_token"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, tech_card) = get_json(&app, &tech_bearer, "/api/scorecards/me").await;
    assert_eq!(status, 200, "{tech_card}");
    assert_eq!(
        tech_card["workload"]["approved_entries"],
        json!(0),
        "{tech_card}"
    );

    let (status, body) = get_json(&app, &tech_bearer, "/api/scorecards").await;
    assert_eq!(status, 403, "{body}");
    let owner_id = me(&app, &bearer).await["user_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, body) = get_json(&app, &tech_bearer, &format!("/api/scorecards/{owner_id}")).await;
    assert_eq!(status, 403, "{body}");
    // ...but a member's own card by id is fine.
    let tech_id = me(&app, &tech_bearer).await["user_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, body) = get_json(&app, &tech_bearer, &format!("/api/scorecards/{tech_id}")).await;
    assert_eq!(status, 200, "{body}");

    // Owner may read another member's card by id.
    let (status, body) = get_json(&app, &bearer, &format!("/api/scorecards/{tech_id}")).await;
    assert_eq!(status, 200, "{body}");
}
