//! End-to-end ticket taxonomy flow through the REAL core-api router
//! (Wave 11 lane 3).
//!
//! Purpose: the shared standard seed on a freshly registered org, the
//! taxonomy CRUD surface (create / rename / reparent / soft retire) with
//! instant-first envelope metadata, ticket classification patching with
//! the retired-node guard, and the board move contract — status remapped
//! through the semantic categories, the typed
//! `board_move_status_conflict` 409 listing target statuses, the explicit
//! `status_id` retry path, SLA preserved by default, SLA re-initialized
//! only on `reset_sla: true`.
//!
//! Non-purpose: SQL-layer behavior in isolation (core-db
//! taxonomy_queries.rs), the manifest section (manifest_queries.rs), UI.

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
        .expect("taxonomy e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
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

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Taxonomy Flow Co",
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

/// The default board id from the nav listing.
async fn default_board_id(app: &Router, token: &str) -> String {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/boards/nav",
        Some(token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    let boards = body.as_array().expect("nav array");
    let board = boards
        .iter()
        .find(|b| b["is_default"] == true)
        .expect("default board");
    board["id"].as_str().expect("board id").into()
}

async fn create_ticket(app: &Router, token: &str, board_id: &str, title: &str) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(token),
        Some(json!({
            "board_id": board_id,
            "title": title,
            "priority": "p3",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket create must succeed");
    body_json(response).await
}

// --- tests -----------------------------------------------------------------------

#[tokio::test]
async fn fresh_org_gets_the_standard_taxonomy_seed() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let app = &app;
    let token = register_org(app, &format!("taxo-seed-{}", Uuid::now_v7().simple())).await;

    let response = send(
        app,
        axum::http::Method::GET,
        "/api/taxonomy/types",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let types = body_json(response).await;
    let names: Vec<&str> = types
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["Incident", "Service Request", "Problem", "Change"]
    );

    let response = send(
        app,
        axum::http::Method::GET,
        "/api/taxonomy/categories",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let cats = body_json(response).await;
    let arr = cats.as_array().unwrap();
    assert_eq!(arr.len(), 37, "8 roots + 29 subcategories, all active");
    let roots = arr.iter().filter(|c| c["parent_id"].is_null()).count();
    assert_eq!(roots, 8);
}

#[tokio::test]
async fn taxonomy_crud_full_lifecycle_with_envelope() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let app = &app;
    let token = register_org(app, &format!("taxo-crud-{}", Uuid::now_v7().simple())).await;

    // CREATE type with idempotency: the retry replays the same node.
    let key = Uuid::now_v7().to_string();
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/taxonomy/types",
        Some(&token),
        Some(json!({"name": "RMA", "sort_order": 90, "idempotency_key": key})),
    )
    .await;
    assert_eq!(response.status(), 200);
    let first = body_json(response).await;
    let type_id = first["id"].as_str().unwrap().to_string();
    assert!(
        first["mutation"]["seq"].is_i64(),
        "the applied mutation carries its monotonic seq"
    );

    let response = send(
        app,
        axum::http::Method::POST,
        "/api/taxonomy/types",
        Some(&token),
        Some(json!({"name": "RMA", "sort_order": 90, "idempotency_key": key})),
    )
    .await;
    assert_eq!(response.status(), 200);
    let replay = body_json(response).await;
    assert_eq!(
        replay["id"].as_str().unwrap(),
        type_id,
        "same key must replay the same node, never duplicate"
    );

    // CREATE category + subcategory, then REPARENT the sub.
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/taxonomy/categories",
        Some(&token),
        Some(json!({"name": "Field Services", "sort_order": 80})),
    )
    .await;
    assert_eq!(response.status(), 200);
    let root_id = body_json(response).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    let response = send(
        app,
        axum::http::Method::POST,
        "/api/taxonomy/categories",
        Some(&token),
        Some(json!({"name": "Site Surveys", "parent_id": root_id})),
    )
    .await;
    assert_eq!(response.status(), 200);
    let sub_id = body_json(response).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    // A sub under a sub is a third level: rejected.
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/taxonomy/categories",
        Some(&token),
        Some(json!({"name": "Depth Probe", "parent_id": sub_id})),
    )
    .await;
    assert_eq!(response.status(), 422, "the tree is exactly two levels");

    // Find the Network root and reparent under it.
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/taxonomy/categories",
        Some(&token),
        None,
    )
    .await;
    let cats = body_json(response).await;
    let network_id = cats
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Network")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/taxonomy/categories/{sub_id}/reparent"),
        Some(&token),
        Some(json!({"parent_id": network_id, "idempotency_key": Uuid::now_v7().to_string()})),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["parent_id"], json!(network_id));

    // RENAME.
    let response = send(
        app,
        axum::http::Method::PATCH,
        &format!("/api/taxonomy/types/{type_id}"),
        Some(&token),
        Some(json!({"name": "RMA / Return", "sort_order": 95, "idempotency_key": Uuid::now_v7().to_string()})),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["name"], "RMA / Return");

    // RETIRE: hidden from the default list, exported with include_retired,
    // and the second retire is an idempotent 200.
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/taxonomy/types/{type_id}/retire"),
        Some(&token),
        Some(json!({"idempotency_key": Uuid::now_v7().to_string()})),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["is_retired"], true);
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/taxonomy/types/{type_id}/retire"),
        Some(&token),
        Some(json!({"idempotency_key": Uuid::now_v7().to_string()})),
    )
    .await;
    assert_eq!(response.status(), 200, "retire is idempotent");

    let response = send(
        app,
        axum::http::Method::GET,
        "/api/taxonomy/types",
        Some(&token),
        None,
    )
    .await;
    let names: Vec<String> = body_json(response)
        .await
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert!(
        !names.iter().any(|n| n == "RMA / Return"),
        "retired hides from pickers"
    );

    let response = send(
        app,
        axum::http::Method::GET,
        "/api/taxonomy/types?include_retired=true",
        Some(&token),
        None,
    )
    .await;
    let all: Vec<Value> = body_json(response).await.as_array().unwrap().clone();
    assert!(
        all.iter()
            .any(|t| t["name"] == "RMA / Return" && t["is_retired"] == true),
        "the admin/manifest surface keeps retired rows"
    );
}

#[tokio::test]
async fn classification_patch_and_retired_guard() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let app = &app;
    let token = register_org(app, &format!("taxo-cls-{}", Uuid::now_v7().simple())).await;

    let response = send(
        app,
        axum::http::Method::GET,
        "/api/taxonomy/types",
        Some(&token),
        None,
    )
    .await;
    let incident_id = body_json(response).await.as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let board_id = default_board_id(app, &token).await;
    let ticket = create_ticket(app, &token, &board_id, "Laptop wont boot").await;
    let ticket_id = ticket["id"].as_str().unwrap().to_string();

    // SET classification.
    let response = send(
        app,
        axum::http::Method::PATCH,
        &format!("/api/tickets/{ticket_id}"),
        Some(&token),
        Some(json!({
            "type_id": incident_id,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200);

    // RETIRE the node, then re-assigning it is a typed 422; clearing is
    // still legal.
    send(
        app,
        axum::http::Method::POST,
        &format!("/api/taxonomy/types/{incident_id}/retire"),
        Some(&token),
        Some(json!({"idempotency_key": Uuid::now_v7().to_string()})),
    )
    .await;
    let response = send(
        app,
        axum::http::Method::PATCH,
        &format!("/api/tickets/{ticket_id}"),
        Some(&token),
        Some(json!({
            "type_id": incident_id,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        422,
        "retired nodes cannot be newly assigned"
    );
    let response = send(
        app,
        axum::http::Method::PATCH,
        &format!("/api/tickets/{ticket_id}"),
        Some(&token),
        Some(json!({
            "type_id": Value::Null,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "clearing is always legal");
}

#[tokio::test]
async fn board_move_remaps_through_semantic_categories_and_preserves_sla() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let app = &app;
    let token = register_org(app, &format!("taxo-move-{}", Uuid::now_v7().simple())).await;
    let board_id = default_board_id(app, &token).await;

    // Target board gets the same four semantic categories by default.
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/boards",
        Some(&token),
        Some(json!({"name": "Escalations", "is_default": false})),
    )
    .await;
    assert_eq!(response.status(), 200);
    let target_board = body_json(response).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    let ticket = create_ticket(app, &token, &board_id, "VPN flapping").await;
    let ticket_id = ticket["id"].as_str().unwrap().to_string();

    // SLA state exists from creation; record the deadline the move must
    // PRESERVE.
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/tickets/{ticket_id}/sla"),
        Some(&token),
        None,
    )
    .await;
    let sla_before = body_json(response).await;
    assert_eq!(sla_before["applied"], true);
    let due_before = sla_before["state"]["resolution_due"].clone();

    // MOVE with remap: the ticket is in an open-category status, so the
    // target board's open-category status wins.
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/tickets/{ticket_id}/move-board"),
        Some(&token),
        Some(json!({
            "target_board_id": target_board,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "body: {}", response_into(&response));
    let moved = body_json(response).await;
    assert_eq!(moved["board_id"], json!(target_board));
    assert_ne!(moved["board_id"], json!(board_id));
    assert_eq!(
        moved["category"], "open",
        "the semantic category rides along"
    );

    // SLA PRESERVED BY DEFAULT: identical deadline, not re-anchored.
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/tickets/{ticket_id}/sla"),
        Some(&token),
        None,
    )
    .await;
    let sla_after = body_json(response).await;
    assert_eq!(
        sla_after["state"]["resolution_due"], due_before,
        "the default move never touches the SLA clocks"
    );

    // Explicit reset_sla re-initializes: the audit row records the
    // operator decision (the calendar-rounds deadline itself may coincide).
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/tickets/{ticket_id}/move-board"),
        Some(&token),
        Some(json!({
            "target_board_id": board_id,
            "reset_sla": true,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/audit-events?action=ticket.board_move&limit=50",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let audit = body_json(response).await;
    let entries = audit["events"]
        .as_array()
        .or_else(|| audit.as_array())
        .expect("audit listing");
    let move_entry = entries
        .iter()
        .find(|e| e["entity_id"] == json!(ticket_id))
        .expect("the board move is audited");
    assert_eq!(
        move_entry["detail"]["reset_sla"], true,
        "reset_sla is on the record"
    );
}

#[tokio::test]
async fn board_move_without_counterpart_is_a_typed_conflict_listing_statuses() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let app = &app;
    let token = register_org(app, &format!("taxo-conflict-{}", Uuid::now_v7().simple())).await;
    let board_id = default_board_id(app, &token).await;

    let response = send(
        app,
        axum::http::Method::POST,
        "/api/boards",
        Some(&token),
        Some(json!({"name": "Intake Only", "is_default": false})),
    )
    .await;
    let target_board = body_json(response).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Strip the target board's waiting-category status so the current
    // category has no counterpart.
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/boards/{target_board}/statuses"),
        Some(&token),
        None,
    )
    .await;
    let statuses = body_json(response).await.as_array().unwrap().clone();
    let waiting = statuses
        .iter()
        .find(|s| s["category"] == "waiting")
        .expect("default waiting status")
        .clone();
    let response = send(
        app,
        axum::http::Method::DELETE,
        &format!(
            "/api/boards/{target_board}/statuses/{}",
            waiting["id"].as_str().unwrap()
        ),
        Some(&token),
        None,
    )
    .await;
    assert!(response.status().is_success(), "status removal succeeds");

    // Put the ticket into waiting on the source board, then attempt the
    // move: typed 409 naming every remaining target status.
    let ticket = create_ticket(app, &token, &board_id, "Waiting on client").await;
    let ticket_id = ticket["id"].as_str().unwrap().to_string();
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/boards/{board_id}/statuses"),
        Some(&token),
        None,
    )
    .await;
    let src_waiting = body_json(response)
        .await
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["category"] == "waiting")
        .expect("source waiting status")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/tickets/{ticket_id}/status"),
        Some(&token),
        Some(json!({
            "status_id": src_waiting,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "open → waiting follows the category floor"
    );

    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/tickets/{ticket_id}/move-board"),
        Some(&token),
        Some(json!({
            "target_board_id": target_board,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 409);
    let conflict = body_json(response).await;
    assert_eq!(conflict["error"]["reason"], "board_move_status_conflict");
    assert_eq!(conflict["error"]["from_category"], "waiting");
    let options = conflict["error"]["target_statuses"]
        .as_array()
        .expect("target status options");
    assert!(!options.is_empty());
    assert!(
        options.iter().all(|o| o["category"] != "waiting"),
        "the missing counterpart is exactly why the move was rejected"
    );

    // The retry path pins a target status explicitly.
    let pinned = options[0]["id"].as_str().unwrap().to_string();
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/tickets/{ticket_id}/move-board"),
        Some(&token),
        Some(json!({
            "target_board_id": target_board,
            "status_id": pinned,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "the pinned retry lands");
    let moved = body_json(response).await;
    assert_eq!(moved["board_id"], json!(target_board));
    assert_eq!(moved["status_id"], json!(pinned));
}

/// Debug helper for assertion messages (responses are consumed by
/// body_json, so this only renders the status).
fn response_into(response: &axum::response::Response) -> String {
    format!("status {}", response.status())
}
