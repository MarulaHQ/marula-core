//! Automation EFFECT application over a ticket-less subject (Wave 43 lane 3).
//!
//! The lane's promise is that a ticket-less envelope drives a board-scoped
//! rule END TO END: `match_rules` produces a match whose `ticket_id` is
//! null, and the `automation.effects` job that follows applies to a BOARD
//! rather than a ticket. This suite proves the second half through the
//! real internal route:
//!
//! 1. A ticket-less effects request (ticket_id null + board_id) applies a
//!    subject-agnostic effect (notify_users), writes the notification with
//!    no ticket link, and still emits `automation.triggered` with the loop
//!    guard `via_automation: true`.
//! 2. Ticket-writing effects are SKIPPED, not failed: a board-subject rule
//!    carrying `add_internal_comment` applies nothing and still returns
//!    200, so one bad rule can never wedge the queue.
//! 3. A board that does not resolve in the org is refused (404) — a board
//!    that is not ours is not a scope.
//! 4. The ticket path is unregressed: a normal effects request still
//!    applies to the ticket it names.
//!
//! Database selection follows the documented contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const INTERNAL_TOKEN: &str = "ticketless-effects-internal-token";

async fn test_state() -> (AppState, PgPool) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("ticketless effects flow needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    config.internal_token = Some(INTERNAL_TOKEN.to_string());
    config.secret_key = core_api::config::SecretKey([41; 32]);
    let reporting_url = admin_url.clone();
    let state = AppState {
        plugin_pool: Default::default(),
        pool: pool.clone(),
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    };
    (state, pool)
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

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Ticketless Effects Co",
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

async fn create_board(app: &Router, token: &str, name: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/boards",
        Some(token),
        Some(json!({ "name": name, "is_default": false })),
    )
    .await;
    assert_eq!(response.status(), 200, "board create must succeed");
    body_json(response).await["id"]
        .as_str()
        .expect("board id")
        .parse()
        .expect("uuid")
}

// --- tests ----------------------------------------------------------------------

/// The ticket-less subject applies a subject-agnostic effect and keeps the
/// loop guard on the emitted event.
#[tokio::test]
async fn ticket_less_effects_apply_without_a_ticket() {
    let (state, pool) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("tl-effects-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let board = create_board(&app, &token, "Effects Board").await;

    let org_id: Uuid = sqlx::query_scalar("SELECT id FROM orgs WHERE slug = $1")
        .bind(&slug)
        .fetch_one(&pool)
        .await
        .expect("org row");
    let recipient: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE org_id = $1 LIMIT 1")
        .bind(org_id)
        .fetch_one(&pool)
        .await
        .expect("owner user row");
    let rule_id = Uuid::now_v7();

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/internal/effects",
        Some(INTERNAL_TOKEN),
        Some(json!({
            "org_id": org_id,
            "rule_id": rule_id,
            "ticket_id": null,
            "board_id": board,
            "actions": [{ "type": "notify_users", "user_ids": [recipient],
                          "kind": "ticket_update", "title": "Board alert" }],
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket-less effects must apply");
    assert_eq!(body_json(response).await["applied"], json!(1));

    let (notification_ticket,): (Option<Uuid>,) =
        sqlx::query_as("SELECT ticket_id FROM notifications WHERE org_id = $1 AND title = $2")
            .bind(org_id)
            .bind("Board alert")
            .fetch_one(&pool)
            .await
            .expect("notification row");
    assert_eq!(
        notification_ticket, None,
        "a board-subject notification carries no ticket link"
    );

    // The loop guard rides the emitted payload exactly as on the ticket path.
    let payload: Value = sqlx::query_scalar(
        "SELECT payload FROM outbox_events WHERE org_id = $1 AND event_type = 'automation.triggered' ORDER BY occurred_at DESC LIMIT 1",
    )
    .bind(org_id)
    .fetch_one(&pool)
    .await
    .expect("automation.rule_triggered outbox row");
    assert_eq!(payload["via_automation"], json!(true));
    assert!(payload["ticket_id"].is_null());
}

/// Ticket-writing effects skip rather than fail on a ticket-less subject.
#[tokio::test]
async fn ticket_writing_effects_skip_on_a_board_subject() {
    let (state, pool) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("tl-skip-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let board = create_board(&app, &token, "Skip Board").await;

    let org_id: Uuid = sqlx::query_scalar("SELECT id FROM orgs WHERE slug = $1")
        .bind(&slug)
        .fetch_one(&pool)
        .await
        .expect("org row");

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/internal/effects",
        Some(INTERNAL_TOKEN),
        Some(json!({
            "org_id": org_id,
            "rule_id": Uuid::now_v7(),
            "ticket_id": null,
            "board_id": board,
            "actions": [
                { "type": "add_internal_comment", "body": "needs a ticket" },
                { "type": "assign_user", "user_id": Uuid::now_v7() },
                { "type": "set_priority", "priority": "p1" }
            ],
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "a board subject skips ticket effects, never fails the batch"
    );

    let comments: (i64,) = sqlx::query_as("SELECT count(*) FROM ticket_comments WHERE org_id = $1")
        .bind(org_id)
        .fetch_one(&pool)
        .await
        .expect("comment count");
    assert_eq!(
        comments.0, 0,
        "no ticket comment can exist without a ticket"
    );
}

/// A board outside the org is refused, not widened to org scope.
#[tokio::test]
async fn a_board_outside_the_org_is_refused() {
    let (state, _pool) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("tl-scope-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let board = create_board(&app, &token, "Scope Board").await;

    let other_org = Uuid::now_v7();
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/internal/effects",
        Some(INTERNAL_TOKEN),
        Some(json!({
            "org_id": other_org,
            "rule_id": Uuid::now_v7(),
            "ticket_id": null,
            "board_id": board,
            "actions": [{ "type": "notify_users", "user_ids": [Uuid::now_v7()],
                          "kind": "ticket_update", "title": "not yours" }],
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        404,
        "a board that does not resolve in the org is not a scope"
    );
}

/// The ticket path is unregressed: effects still apply to the ticket named.
#[tokio::test]
async fn ticket_effects_still_apply() {
    let (state, pool) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("tl-ticket-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let board = create_board(&app, &token, "Ticket Board").await;

    let org_id: Uuid = sqlx::query_scalar("SELECT id FROM orgs WHERE slug = $1")
        .bind(&slug)
        .fetch_one(&pool)
        .await
        .expect("org row");
    let user_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE org_id = $1 LIMIT 1")
        .bind(org_id)
        .fetch_one(&pool)
        .await
        .expect("owner user row");

    let ticket_id = Uuid::now_v7();
    let mut tx = pool.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    let status_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM board_statuses WHERE board_id = $1 AND is_entry LIMIT 1",
    )
    .bind(board)
    .fetch_one(&mut *tx)
    .await
    .expect("entry status");
    core_db::tickets::insert_ticket(
        &mut tx,
        core_db::tickets::NewTicket {
            id: ticket_id,
            org_id,
            number: "TL-1",
            board_id: board,
            status_id,
            title: "Board subject regression",
            body: Some("still applies"),
            priority: core_domain::status::Priority::P3,
            assignee_id: None,
            client_id: None,
            site_id: None,
            contact_id: None,
            requester_id: None,
            due_at: None,
        },
    )
    .await
    .expect("ticket");
    tx.commit().await.expect("commit");

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/internal/effects",
        Some(INTERNAL_TOKEN),
        Some(json!({
            "org_id": org_id,
            "rule_id": Uuid::now_v7(),
            "ticket_id": ticket_id,
            "actions": [{ "type": "notify_users", "user_ids": [user_id],
                          "kind": "ticket_update", "title": "Ticket alert" }],
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket effects must still apply");

    let (linked,): (Option<Uuid>,) =
        sqlx::query_as("SELECT ticket_id FROM notifications WHERE org_id = $1 AND title = $2")
            .bind(org_id)
            .bind("Ticket alert")
            .fetch_one(&pool)
            .await
            .expect("notification row");
    assert_eq!(
        linked,
        Some(ticket_id),
        "a ticket-subject notification links to its ticket"
    );
}

/// No token, no surface — the internal route stays machine-only.
#[tokio::test]
async fn effects_route_requires_the_internal_token() {
    let (state, _pool) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/internal/effects",
        None,
        Some(json!({
            "org_id": Uuid::now_v7(),
            "ticket_id": null,
            "board_id": Uuid::now_v7(),
            "actions": [],
        })),
    )
    .await;
    assert_eq!(response.status(), 401, "missing bearer token");
}
