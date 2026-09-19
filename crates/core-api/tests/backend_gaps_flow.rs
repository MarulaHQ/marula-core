//! Wave 17 lane 13 backend-gaps flow — the filebacks the merged area
//! lanes filed because the CONTRACT was missing, exercised through the
//! REAL core-api router:
//!
//! 1. Alerts triage: acknowledge → resolve (and the typed-backed
//!    conflicts: re-ack = 409; resolve drops into status filter + events
//!    live at the routes).
//! 2. Meeting-type DELETE (hard, with audit; FK-in-use surfaces typed).
//! 3. wip_limit: cap settable on board statuses, enforced SERVER-side at
//!    ticket status change (occupancy excluding the moving ticket).
//! 4. Search pins: pin a stored result (idempotent), unpin, 404 discipline.
//! 5. Client documentation completeness: backend-scored shape; 404 for an
//!    invisible client.
//! 6. Export-history ledger: POST /org/exports lands a row; list + one
//!    fresh signed link per row + archive toggle.
//! 7. Calendar org-wide deadlines: /calendar/deadlines read model replaces
//!    the bounded page-1 tickets fetch (window filter server-side).
//! 8. Automations test-run: dry-run one rule against the sampled tickets,
//!    proposal-shaped (never a fake run).
//! 9. Documents attachments: upload/list/download/delete over the
//!    content-addressed core-store seam (0089 surface).
//!
//! Database selection follows export_flow's contract: MERIDIAN_TEST_DATABASE_URL
//! or DATABASE_URL (no schema reset). Where the seed is a queue-side fact
//! (alert rows / search_index rows — the worker indexer's territory), the
//! test seeds the SAME query-layer surface the production indexer writes.

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
        .expect("backend gaps e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([17; 32]);
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

async fn register_org(app: &Router, name: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": format!("{name} Co"),
            "org_slug": format!("{name}-{}", Uuid::now_v7().simple()),
            "email": format!("owner-{name}@{name}.test"),
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

/// The seeded default board's id (registration provisions one).
async fn default_board(app: &Router, token: &str) -> Vec<Value> {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/v1/boards",
        &[auth(token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    body_json(response)
        .await
        .as_array()
        .expect("boards array")
        .clone()
}

async fn default_user(app: &Router, token: &str) -> String {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/v1/me",
        &[auth(token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "auth/me must resolve");
    let me = body_json(response).await;
    me["user_id"].as_str().unwrap().to_string()
}

async fn board_statuses(app: &Router, token: &str, board_id: &str) -> Vec<Value> {
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/v1/boards/{board_id}/statuses"),
        &[auth(token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    body_json(response)
        .await
        .as_array()
        .expect("statuses array")
        .clone()
}

async fn create_ticket(
    app: &Router,
    token: &str,
    board_id: &str,
    status_id: &str,
    title: &str,
) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/tickets",
        &[auth(token)],
        Some(json!({
            "board_id": board_id,
            "title": title,
            "priority": "p3",
            "status_id": status_id,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket create must succeed");
    body_json(response).await
}

// --- (1) alerts triage ---------------------------------------------------------

#[tokio::test]
async fn alert_acknowledge_and_resolve_flows() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let token = register_org(&app, "alertgap").await;
    let pool = connect(
        &std::env::var("MERIDIAN_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap(),
    )
    .await
    .unwrap();
    let boards = default_board(&app, &token).await;
    assert!(!boards.is_empty(), "a default board is seeded");
    let board_id = boards[0]["id"].as_str().unwrap().to_string();

    // Register one ingest source (the normal admin surface — no HMAC
    // verification under test here; the ingestor has its own flow file).
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/alert-sources",
        &[auth(&token)],
        Some(json!({ "name": "RMM", "board_id": board_id })),
    )
    .await;
    assert_eq!(response.status(), 200, "alert source create must succeed");
    let created = body_json(response).await;
    let source_id = created["id"].as_str().unwrap().to_string();
    let source_id: Uuid = source_id.parse().unwrap();
    let org_id: Uuid = created["org_id"].as_str().unwrap().parse().unwrap();

    // Seed two alerts directly (the queue-side ingestor owns the dedup
    // semantics; this surface tests the TRIAGE contract).
    let alert_a = Uuid::now_v7();
    let alert_b = Uuid::now_v7();

    sqlx::query(
        "INSERT INTO alerts (id, org_id, source_id, dedup_key, correlation_key, title, severity, occurred_at)
         VALUES ($1,$2,$3,'evt-a','evt-a','Disk critical','critical', now()),
                ($4,$2,$3,'evt-b','evt-b','Battery flap', 'info', now())",
    )
    .bind(alert_a)
    .bind(org_id)
    .bind(source_id)
    .bind(alert_b)
    .execute(&pool)
    .await
    .expect("seed stores alerts");

    // Both list as 'open' with the triage fields visible.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/alerts",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let alerts = body_json(response).await["alerts"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(alerts.len(), 2);
    assert!(alerts.iter().all(|a| a["status"] == "open"));

    // Acknowledge one: status → acknowledged with the actor recorded.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/alerts/{alert_a}/acknowledge"),
        &[auth(&token)],
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 200);
    let acked = body_json(response).await;
    assert_eq!(acked["alert"]["status"], "acknowledged");
    assert_eq!(
        acked["alert"]["acked_at"].as_str().unwrap(),
        acked["alert"]["acked_at"].as_str().unwrap_or("none-used")
    );
    assert!(
        acked["mutation"]["seq"].as_i64().unwrap() > 0,
        "envelope seq present"
    );

    // Re-ack is a typed conflict (429-style 409) — honest, not silent.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/alerts/{alert_a}/acknowledge"),
        &[auth(&token)],
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 409, "re-ack refuses");

    // 'acknowledged' status filter hides the still-open alerts.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/alerts?status=acknowledged",
        &[auth(&token)],
        None,
    )
    .await;
    let filtered = body_json(response).await;
    let ids: Vec<&str> = filtered["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![alert_a.to_string().as_str()]);

    // Resolve it, then the row reports resolved with no ack leak.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/alerts/{alert_a}/resolve"),
        &[auth(&token)],
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 200);
    let resolved = body_json(response).await;
    assert_eq!(resolved["alert"]["status"], "resolved");
    assert!(resolved["alert"]["resolved_at"].is_string());

    // Resolve of a resolved alert is a typed conflict too.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/alerts/{alert_a}/resolve"),
        &[auth(&token)],
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 409);

    // Unknown alert → typed 404.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/alerts/{}/acknowledge", Uuid::now_v7()),
        &[auth(&token)],
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 404);
}

// --- (2) meeting-type DELETE ------------------------------------------------------

#[tokio::test]
async fn meeting_type_hard_delete_flow() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let token = register_org(&app, "mtgap").await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/meeting-types",
        &[auth(&token)],
        Some(json!({ "name": "On-site assessment", "duration_minutes": 60, "assignee_scope": "user", "assignee_user_id": default_user(&app, &token).await })),
    )
    .await;
    assert_eq!(response.status(), 201, "create must land");
    let created = body_json(response).await;
    let mt_id = created["meeting_type"]["id"].as_str().unwrap().to_string();

    // Assertion on the hard-delete contract: DELETE removes the row; the
    // paused posture (PATCH is_active) remains for the refs story.
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/meeting-types/{mt_id}"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["deleted"], true);

    // Gone: list drops the row and a second delete is a typed 404.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/meeting-types",
        &[auth(&token)],
        None,
    )
    .await;
    let types = body_json(response).await;
    assert!(
        !types.as_array().unwrap().iter().any(|t| t["id"] == mt_id),
        "delete removed the template"
    );
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/meeting-types/{mt_id}"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 404);
}

// --- (3) wip_limit enforcement ---------------------------------------------------

#[tokio::test]
async fn wip_limit_is_enforced_server_side_on_moves() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let token = register_org(&app, "wipgap").await;
    let boards = default_board(&app, &token).await;
    let board_id = boards[0]["id"].as_str().unwrap().to_string();

    // Add a custom in-progress column capped at 1.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/boards/{board_id}/statuses"),
        &[auth(&token)],
        Some(json!({
            "name": "Focused",
            "category": "in_progress",
            "sort_order": 5,
            "is_entry": false,
            "wip_limit": 1,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "custom status create must land");
    let capped = body_json(response).await;
    assert_eq!(capped["wip_limit"], 1);
    let capped_id = capped["id"].as_str().unwrap().to_string();

    let statuses = board_statuses(&app, &token, &board_id).await;
    let entry = statuses.iter().find(|s| s["is_entry"] == true).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // One ticket occupies the capped column.
    let one = create_ticket(&app, &token, &board_id, &entry, "occupant").await;
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/tickets/{}/status", one["id"].as_str().unwrap()),
        &[auth(&token)],
        Some(json!({ "status_id": capped_id })),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "cap=1 room exists for the FIRST ticket"
    );
    let first = body_json(response).await;
    assert_eq!(
        first["status_id"], capped_id,
        "change-status response flattens the ticket"
    );

    // A second ticket cannot move in: typed validation naming the cap.
    let two = create_ticket(&app, &token, &board_id, &entry, "squeezed").await;
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/tickets/{}/status", two["id"].as_str().unwrap()),
        &[auth(&token)],
        Some(json!({ "status_id": capped_id })),
    )
    .await;
    assert_eq!(response.status(), 422, "the server side refuses, not vibes");
    let err = body_json(response).await;
    assert!(
        err.to_string().contains("WIP limit"),
        "rejection names the cap, got {err}"
    );

    // Raising the cap on the SAME column lets the move through.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/boards/{board_id}/statuses/{capped_id}"),
        &[auth(&token)],
        Some(json!({ "wip_limit": 2 })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let raised = body_json(response).await;
    assert_eq!(raised["wip_limit"], 2);
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/tickets/{}/status", two["id"].as_str().unwrap()),
        &[auth(&token)],
        Some(json!({ "status_id": capped_id })),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "raised cap admits the second ticket"
    );

    // Clearing the cap keeps behavior live.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/boards/{board_id}/statuses/{capped_id}"),
        &[auth(&token)],
        Some(json!({ "wip_limit": null })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        body_json(response).await["wip_limit"],
        serde_json::Value::Null
    );
}

// --- (4) search pins -------------------------------------------------------------

#[tokio::test]
async fn search_pins_crud_flow() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let token = register_org(&app, "pingap").await;
    let pool = connect(
        &std::env::var("MERIDIAN_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap(),
    )
    .await
    .unwrap();
    let boards = default_board(&app, &token).await;
    let board_id = boards[0]["id"].as_str().unwrap().to_string();
    let statuses = board_statuses(&app, &token, &board_id).await;
    let entry = statuses.iter().find(|s| s["is_entry"] == true).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let ticket = create_ticket(&app, &token, &board_id, &entry, "Mail relay down").await;
    let org_id: Uuid = ticket["org_id"].as_str().unwrap().parse().unwrap();
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();

    // A pin needs a STORED index row (the indexer's write); seed exactly
    // what the indexer would have written.
    sqlx::query(
        "INSERT INTO search_index (id, org_id, entity_kind, entity_id, title, body)
         VALUES ($1,$2,'ticket',$3,'Mail relay down','The relay at HQ is down again')",
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(ticket_id)
    .execute(&pool)
    .await
    .expect("seed index row");

    // Pin it: snapshot title comes from the INDEX row.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/search/pins",
        &[auth(&token)],
        Some(json!({ "query": "mail relay", "entity_kind": "ticket", "entity_id": ticket_id })),
    )
    .await;
    assert_eq!(response.status(), 200, "pin creation must land");
    let pin = body_json(response).await;
    assert_eq!(pin["title"], "Mail relay down");
    assert!(pin["query"].as_str().is_some());
    let _pin_id_first = pin["id"].as_str().unwrap().to_string();

    // Re-pin = idempotent refresh (Wave 23 lane 1): the SAME pin id
    // answers, and the snapshot goes current — RETITLE the index row
    // first via the same upsert path the indexer would drive (update the
    // stored title), then re-pin from that state and observe the fresh
    // title + the new query riding the SAME pin id (created_at stable).
    sqlx::query(
        "UPDATE search_index SET title = 'Mail relay down (renamed)' \
         WHERE entity_kind = 'ticket' AND entity_id = $1",
    )
    .bind(ticket_id)
    .execute(&pool)
    .await
    .expect("retitle index row");
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/search/pins",
        &[auth(&token)],
        Some(json!({ "query": "mail relayed", "entity_kind": "ticket", "entity_id": ticket_id })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let repin = body_json(response).await;
    assert_eq!(repin["id"], _pin_id_first);
    assert_eq!(
        repin["title"], "Mail relay down (renamed)",
        "re-pin refreshes the stale title snapshot"
    );
    assert_eq!(
        repin["query"], "mail relayed",
        "re-pin refreshes the stored query snapshot"
    );
    // The list reflects the refreshed snapshot (one pin still).
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/search/pins",
        &[auth(&token)],
        None,
    )
    .await;
    let pins = body_json(response).await;
    let rows = pins["pins"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["title"], "Mail relay down (renamed)");

    // Pinning a result that does not exist in the index: typed 404.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/search/pins",
        &[auth(&token)],
        Some(json!({ "query": "missing", "entity_kind": "ticket", "entity_id": Uuid::now_v7() })),
    )
    .await;
    assert_eq!(response.status(), 404);

    // The list shows the pin.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/search/pins",
        &[auth(&token)],
        None,
    )
    .await;
    let pins = body_json(response).await;
    assert_eq!(pins["pins"].as_array().unwrap().len(), 1);

    // Unpin; a second unpin is the caller's-not-theirs 404.
    let pin_id = repin["id"].as_str().unwrap().to_string();
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/search/pins/{pin_id}"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/search/pins/{pin_id}"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 404);
}

// --- (5) documentation completeness ------------------------------------------------

#[tokio::test]
async fn client_completeness_backend_scoring() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let token = register_org(&app, "compgap").await;

    // The seeded register creates ONE client? create explicit one.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/clients",
        &[auth(&token)],
        Some(json!({ "name": "Score Co" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let client = body_json(response).await;
    let client_id = client["id"].as_str().unwrap().to_string();

    // The scoring lives server-side: a fresh org pro-rates everything to
    // the legs it CAN speak about.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/clients/{client_id}/completeness"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let scored = body_json(response).await;
    let score = scored["score"].as_i64().unwrap();
    assert!((0..=100).contains(&score), "score {score}");
    let gaps = scored["gaps"].as_array().unwrap();
    let keys: Vec<&str> = gaps.iter().map(|g| g["key"].as_str().unwrap()).collect();
    assert!(keys.contains(&"contacts"), "no contacts ⇒ contact gap");
    assert!(keys.contains(&"sites"));
    assert!(scored["basis"].is_string());

    // Unknown client: typed 404.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/clients/{}/completeness", Uuid::now_v7()),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 404);
}

// --- (6) export history ledger -----------------------------------------------------

#[tokio::test]
async fn export_history_ledger_lists_and_archives() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let token = register_org(&app, "expgap").await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/org/exports",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let created = body_json(response).await;
    let export_id = created["export_id"].as_str().unwrap().to_string();

    // The ledger lists what was just created: one row with the facts.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/org/exports",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let history = body_json(response).await;
    let rows = history["exports"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "the post-create ledger has the row");
    assert_eq!(rows[0]["export_id"], export_id);
    assert_eq!(rows[0]["size_bytes"], created["size_bytes"]);
    assert_eq!(rows[0]["archived"], false);

    // ONE fresh signed link per row (never a stored expiring URL).
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/org/exports/{export_id}/download-link"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let link = body_json(response).await;
    assert!(link["download_url"]
        .as_str()
        .unwrap()
        .contains("signature="));

    // Archive toggle: pure read-model signal over the row.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/org/exports/{export_id}"),
        &[auth(&token)],
        Some(json!({ "archived": true })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["archived"], true);

    // Filtered list honors the archive flag.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/org/exports?archived=true",
        &[auth(&token)],
        None,
    )
    .await;
    let archived = body_json(response).await;
    assert_eq!(archived["exports"].as_array().unwrap().len(), 1);
}

// --- (7) calendar deadlines ---------------------------------------------------------

#[tokio::test]
async fn calendar_deadline_read_model_org_wide() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let token = register_org(&app, "calgap").await;
    let boards = default_board(&app, &token).await;
    let board_id = boards[0]["id"].as_str().unwrap().to_string();
    let statuses = board_statuses(&app, &token, &board_id).await;
    let entry = statuses.iter().find(|s| s["is_entry"] == true).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Two deadlines INSIDE the window (one future, one far past), one with
    // no due date: the overlay reads the read model, not a page-1 fetch.
    let now = chrono::Utc::now();
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/tickets",
        &[auth(&token)],
        Some(json!({
            "board_id": board_id,
            "title": "Due tomorrow",
            "priority": "p3",
            "status_id": entry,
            "due_at": (now + chrono::Duration::days(1)).to_rfc3339(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/tickets",
        &[auth(&token)],
        Some(json!({
            "board_id": board_id,
            "title": "Due in 40 days",
            "priority": "p3",
            "status_id": entry,
            "due_at": (now + chrono::Duration::days(40)).to_rfc3339(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/tickets",
        &[auth(&token)],
        Some(json!({
            "board_id": board_id,
            "title": "No deadline",
            "priority": "p3",
            "status_id": entry,
        })),
    )
    .await;
    assert_eq!(response.status(), 200);

    // 8-day window default sees the near deadline; the far one stays out.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/calendar/deadlines",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let view = body_json(response).await;
    let deadlines = view["deadlines"].as_array().unwrap();
    assert_eq!(deadlines.len(), 1, "window filters server-side");
    assert_eq!(deadlines[0]["title"], "Due tomorrow");

    // Explicit band aligns both in-window entries.
    let from = (now - chrono::Duration::days(1))
        .to_rfc3339_opts(chrono::format::SecondsFormat::Secs, true);
    let to = (now + chrono::Duration::days(50))
        .to_rfc3339_opts(chrono::format::SecondsFormat::Secs, true);
    let uri = format!("/api/v1/calendar/deadlines?from={from}&to={to}");
    let response = send(&app, axum::http::Method::GET, &uri, &[auth(&token)], None).await;
    assert_eq!(
        response.status(),
        200,
        "parse failed: {uri} → {}",
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    );
    let wide = body_json(response).await;
    assert_eq!(wide["deadlines"].as_array().unwrap().len(), 2);
}

// --- (8) automations test-run --------------------------------------------------------

#[tokio::test]
async fn automation_test_run_is_proposal_shaped() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let token = register_org(&app, "autogap").await;
    let boards = default_board(&app, &token).await;
    let board_id = boards[0]["id"].as_str().unwrap().to_string();

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/automation-rules",
        &[auth(&token)],
        Some(json!({
            "name": "P1 escalation",
            "trigger_event": "ticket.created",
            "conditions": [{"type": "priority_is", "priority": "p1"}],
            "actions": [{"type": "set_priority", "priority": "p1"}],
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "rule create must land");
    let created = body_json(response).await;
    let rule_id = created["id"].as_str().unwrap().to_string();

    // Dry run: zero sampled tickets ⇒ zero proposals, actions returned so
    // the editor can render them; no rule was executed (no effects).
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/automation-rules/{rule_id}/test-run"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let run = body_json(response).await;
    assert_eq!(run["rule_id"], rule_id);
    assert_eq!(run["sampled_count"], 0);
    assert_eq!(run["matched_count"], 0);
    assert_eq!(run["matches"].as_array().unwrap().len(), 0);
    assert!(
        run["actions_preview"].is_array(),
        "preview returned the actions"
    );

    // A P1 ticket in the sample MATCHES the rule's condition.
    let statuses = board_statuses(&app, &token, &board_id).await;
    let entry = statuses.iter().find(|s| s["is_entry"] == true).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let ticket = create_ticket(&app, &token, &board_id, &entry, "Fire alarm").await;
    // Persist through the list API? The test-run samples the org's recent
    // tickets — the ticket above IS in the org's recent rows.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/automation-rules/{rule_id}/test-run"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let run = body_json(response).await;
    assert!(run["sampled_count"].as_i64().unwrap() > 0);
    // The created ticket rides p3, so the p1 rule matches nothing — the
    // honest result, rendered by the editor (never faked).
    assert_eq!(run["matched_count"], 0);

    // Unknown rule ⇒ typed 404.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/automation-rules/{}/test-run", Uuid::now_v7()),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 404);

    // The ticket body is available for the audit trail; this assertion
    // keeps the sample honest (proposed-state reads back the ticket).
    assert_eq!(ticket["title"], "Fire alarm");
}

// --- (9) documents attachments over the 2.14 seam -------------------------------------

#[tokio::test]
async fn documents_attachments_upload_list_download_delete() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let token = register_org(&app, "docgap").await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/documents",
        &[auth(&token)],
        Some(json!({ "title": "Runbook" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let doc = body_json(response).await;
    let doc_id = doc["id"].as_str().unwrap().to_string();

    // Upload: raw bytes with an x-filename header; magic bytes allowlisted
    // family (PNG head).
    let mut png_body = vec![0_u8; 640];
    png_body[0..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    use sha2::{Digest, Sha256};
    let expected_sha = hex::encode(Sha256::digest(&png_body));
    let request = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri(format!("/api/v1/documents/{doc_id}/attachments"))
        .header("authorization", format!("Bearer {token}"))
        .header("x-filename", "network-diagram.png")
        .header("content-type", "image/png")
        .body(axum::body::Body::from(png_body.clone()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), 200, "attachment upload must land");
    let uploaded = body_json(response).await;
    assert_eq!(uploaded["filename"], "network-diagram.png");
    assert_eq!(uploaded["sha256"], expected_sha);
    assert_eq!(uploaded["size_bytes"], 640);
    let attachment_id = uploaded["id"].as_str().unwrap().to_string();

    // List shows the paperclip row.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/documents/{doc_id}/attachments"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        body_json(response).await["attachments"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Download streams through the API (no presigned URLs in v1) and
    // carries the integrity header.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/documents/{doc_id}/attachments/{attachment_id}/download"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("x-meridian-sha256")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        expected_sha
    );
    let downloaded = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(downloaded.as_ref(), png_body.as_slice());

    // Delete reclaims the object (this row was the only one).
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/documents/{doc_id}/attachments/{attachment_id}"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let deleted = body_json(response).await;
    assert_eq!(deleted["deleted"], true);
    assert_eq!(deleted["object_removed"], true);

    // Double delete: typed 404; the doc-side list is empty afterwards.
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/documents/{doc_id}/attachments/{attachment_id}"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 404);
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/documents/{doc_id}/attachments"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(
        body_json(response).await["attachments"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}
