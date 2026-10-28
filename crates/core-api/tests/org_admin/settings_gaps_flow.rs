//! Wave 41 lane 12 (p41-ui-settings) — the settings pack's backend gaps,
//! exercised through the REAL core-api router, because every one of them is a
//! promise about what the API returns rather than a unit of pure logic:
//!
//! 1. Per-status usage: the TICKETS column and the delete confirm's counts
//!    come from one read, and they agree with the tickets that actually exist.
//! 2. Client visibility: a hidden status shows the client the category word
//!    and never its own label, while the ticket still reports where it is.
//! 3. Delete with reassignment: the tickets move (with the denormalised
//!    category re-derived, which is what SLA and board math read), each one
//!    gets its own timeline event, and the route reports what it did.
//! 4. Automations naming a deleted status are PAUSED, never repointed.
//! 5. The shadow run log reads the real `decision_answers` store and reports
//!    above-threshold against the surface's own act threshold.
//!
//! Database selection follows backend_gaps_flow's contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL, migrations run in-process, no
//! schema reset (each test namespaces its own rows).

use crate::common::{auth, send_headers as send, test_state_with};
use axum::Router;
use core_api::state::AppState;
use serde_json::{json, Value};
use uuid::Uuid;

async fn test_state() -> AppState {
    test_state_with(|config| config.secret_key = core_api::config::SecretKey([23; 32])).await
}

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// Bootstraps an org (registration provisions a default board) and returns an
/// admin token plus the org id, which the portal and RLS-scoped inserts need.
async fn scaffold(app: &Router, label: &str) -> (String, String) {
    let slug = format!("{label}-{}", Uuid::now_v7().simple());
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": format!("Settings gaps {label}"),
            "org_slug": slug,
            "email": format!("owner-{slug}@settings-gaps.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let body = body_json(response).await;
    let token = body["access_token"]
        .as_str()
        .expect("access token")
        .to_string();
    // The register response carries no org id, and the portal/RLS-scoped
    // inserts below need one: the provisioned default board carries it.
    let boards = send(
        app,
        axum::http::Method::GET,
        "/api/v1/boards",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(boards.status(), 200, "the new org has its default board");
    let org_id = body_json(boards).await[0]["org_id"]
        .as_str()
        .expect("org id on the board")
        .to_string();
    (token, org_id)
}

/// Registration provisions one default board; the seeded statuses live on it,
/// which is exactly the board a real settings page opens.
async fn create_board(app: &Router, token: &str, name: &str) -> String {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/v1/boards",
        &[auth(token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "boards read");
    let boards = body_json(response).await;
    let board = boards
        .as_array()
        .expect("boards array")
        .iter()
        .find(|b| b["is_default"] == json!(true))
        .unwrap_or_else(|| panic!("registration provisions a default board for {name}"));
    board["id"].as_str().unwrap().to_string()
}

async fn statuses(app: &Router, token: &str, board_id: &str) -> Vec<Value> {
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

async fn usage(app: &Router, token: &str, board_id: &str) -> Vec<Value> {
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/v1/boards/{board_id}/statuses/usage"),
        &[auth(token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "usage read must resolve");
    body_json(response)
        .await
        .as_array()
        .expect("usage array")
        .clone()
}

async fn add_status(
    app: &Router,
    token: &str,
    board_id: &str,
    name: &str,
    category: &str,
    is_entry: bool,
) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/v1/boards/{board_id}/statuses"),
        &[auth(token)],
        Some(json!({
            "name": name,
            "category": category,
            "sort_order": 90,
            "is_entry": is_entry,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "status {name} must be created");
    body_json(response).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A ticket is created in the board's entry status — the API seats it, and
/// `status_id` is not a create field — then MOVED, which is the real flow the
/// counts and the reassign test are about.
async fn create_ticket(app: &Router, token: &str, board_id: &str, title: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/tickets",
        &[auth(token)],
        Some(json!({
            "board_id": board_id,
            "title": title,
            "body": "created by the settings gaps flow",
            "priority": "p3",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket {title}");
    body_json(response).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn move_ticket(app: &Router, token: &str, ticket_id: &str, status_id: &str) {
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/v1/tickets/{ticket_id}/status"),
        &[auth(token)],
        Some(json!({ "status_id": status_id })),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "ticket {ticket_id} must reach {status_id}"
    );
}

/// Opens one board to one client, with every status readable for that client
/// (an empty `visible_status_ids` means "all of them").
async fn expose_board_to_client(org_id: &str, board_id: &str, client_id: &str) {
    sqlx::query(
        r#"INSERT INTO board_portal_exposure (id, org_id, board_id, client_id, exposed, visible_status_ids)
           VALUES ($1, $2, $3, $4, true, '{}')"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::parse_str(org_id).expect("org uuid"))
    .bind(Uuid::parse_str(board_id).expect("board uuid"))
    .bind(Uuid::parse_str(client_id).expect("client uuid"))
    .execute(&test_state().await.pool)
    .await
    .expect("exposure insert");
}

async fn create_client(app: &Router, token: &str, name: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/clients",
        &[auth(token)],
        Some(json!({ "name": name })),
    )
    .await;
    assert_eq!(response.status(), 200, "client create");
    body_json(response).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn usage_for(rows: &[Value], status_id: &str) -> (i64, i64) {
    let row = rows
        .iter()
        .find(|row| row["status_id"] == status_id)
        .unwrap_or_else(|| panic!("usage must carry a row for {status_id}"));
    (
        row["ticket_count"].as_i64().expect("ticket_count"),
        row["automation_count"].as_i64().expect("automation_count"),
    )
}

#[tokio::test]
async fn usage_counts_match_the_tickets_that_actually_exist() {
    let app = core_api::routes::build_router(test_state().await);
    let (token, _org) = scaffold(&app, "usage").await;
    let board = create_board(&app, &token, "Usage board").await;
    let seeded = statuses(&app, &token, &board).await;
    let new_status = seeded
        .iter()
        .find(|s| s["category"] == "open")
        .expect("an open status")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let waiting = add_status(&app, &token, &board, "Waiting on client", "waiting", false).await;
    let empty = add_status(&app, &token, &board, "Awaiting parts", "waiting", false).await;

    for index in 0..3 {
        let ticket = create_ticket(&app, &token, &board, &format!("Waiting ticket {index}")).await;
        move_ticket(&app, &token, &ticket, &waiting).await;
    }

    let rows = usage(&app, &token, &board).await;
    // One row per status, in sort order, with the counts the table draws.
    assert_eq!(rows.len(), seeded.len() + 2, "one usage row per status");
    assert_eq!(usage_for(&rows, &waiting), (3, 0), "three tickets wait");
    assert_eq!(
        usage_for(&rows, &empty),
        (0, 0),
        "a status nobody sits in counts zero, not nothing"
    );
    assert_eq!(
        usage_for(&rows, &new_status),
        (0, 0),
        "the seed board has no tickets in its entry status"
    );
}

#[tokio::test]
async fn a_hidden_status_never_shows_its_label_to_a_client() {
    let app = core_api::routes::build_router(test_state().await);
    let (token, org_id) = scaffold(&app, "client-visible").await;
    let board = create_board(&app, &token, "Portal board").await;
    let internal = add_status(
        &app,
        &token,
        &board,
        "Escalated to L2",
        "in_progress",
        false,
    )
    .await;

    // Default is visible: the label is the client's to read. A ticket only
    // reaches a client through its client, so the flow creates one.
    let client_id = create_client(&app, &token, "Waiting client").await;
    let ticket = create_ticket(&app, &token, &board, "Portal ticket").await;
    let attached = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/tickets/{ticket}"),
        &[auth(&token)],
        Some(json!({ "client_id": client_id })),
    )
    .await;
    assert_eq!(attached.status(), 200, "the ticket must reach its client");
    // A board is invisible to a client until it is exposed, which is the
    // per-client half of the story; the per-status switch below is the org's
    // half. Both are real and they compose.
    expose_board_to_client(&org_id, &board, &client_id).await;
    move_ticket(&app, &token, &ticket, &internal).await;
    let client_token = portal_client(&app, &org_id, &client_id).await;

    let listed = portal_tickets(&app, &client_token).await;
    let row = listed
        .as_array()
        .expect("portal rows")
        .iter()
        .find(|t| t["id"] == ticket)
        .expect("the ticket is in the client's list");
    assert_eq!(
        row["status_label"],
        json!("Escalated to L2"),
        "a client-visible status shows its own words"
    );

    // Hide it, and the client gets the category word instead — never the label.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/boards/{board}/statuses/{internal}"),
        &[auth(&token)],
        Some(json!({ "client_visible": false })),
    )
    .await;
    assert_eq!(response.status(), 200, "the toggle must save");
    assert_eq!(body_json(response).await["client_visible"], json!(false));

    let listed = portal_tickets(&app, &client_token).await;
    let row = listed
        .as_array()
        .expect("portal rows")
        .iter()
        .find(|t| t["id"] == ticket)
        .expect("still listed");
    assert_eq!(
        row["status_label"],
        Value::Null,
        "a hidden status sends no label"
    );
    assert_eq!(
        row["category"],
        json!("in_progress"),
        "and the ticket still says where it stands"
    );

    // Turning it back on restores the label, so the switch is the whole
    // mechanism rather than a one-way door.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/boards/{board}/statuses/{internal}"),
        &[auth(&token)],
        Some(json!({ "client_visible": true })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let listed = portal_tickets(&app, &client_token).await;
    let row = listed
        .as_array()
        .expect("portal rows")
        .iter()
        .find(|t| t["id"] == ticket)
        .expect("still listed");
    assert_eq!(row["status_label"], json!("Escalated to L2"));
}

#[tokio::test]
async fn deleting_with_reassignment_moves_the_tickets_and_says_so() {
    let app = core_api::routes::build_router(test_state().await);
    let (token, _org) = scaffold(&app, "reassign").await;
    let board = create_board(&app, &token, "Reassign board").await;
    let waiting = add_status(&app, &token, &board, "Waiting on client", "waiting", false).await;
    let seeded = statuses(&app, &token, &board).await;
    let home = seeded
        .iter()
        .find(|s| s["category"] == "open")
        .expect("an open status")["id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut moved = Vec::new();
    for index in 0..2 {
        let ticket = create_ticket(&app, &token, &board, &format!("Stranded {index}")).await;
        move_ticket(&app, &token, &ticket, &waiting).await;
        moved.push(ticket);
    }

    // Without a replacement, a status holding tickets is refused and nothing
    // moves: the confirm is the only way to get here, and it always asks.
    let refused = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/boards/{board}/statuses/{waiting}"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(
        refused.status(),
        409,
        "a used status is not silently dropped"
    );
    assert_eq!(usage_for(&usage(&app, &token, &board).await, &waiting).0, 2);

    let deleted = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/boards/{board}/statuses/{waiting}?reassign_to={home}"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(deleted.status(), 200, "reassignment makes the delete legal");
    let result = body_json(deleted).await;
    assert_eq!(
        result["moved_tickets"],
        json!(2),
        "the route reports what it moved"
    );

    // The tickets are home, and their denormalised category re-derived: the
    // waiting category must not survive the move, or SLA math breaks.
    let usage = usage(&app, &token, &board).await;
    assert_eq!(usage_for(&usage, &home).0, 2, "both tickets are home");
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/tickets/{}", moved[0]),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let ticket = body_json(response).await;
    assert_eq!(ticket["status_id"], json!(home), "the ticket moved");
    assert_eq!(
        ticket["category"],
        json!("open"),
        "the denormalised category follows the status it now sits in"
    );

    // Every moved ticket got its own timeline event, so a bulk move audits
    // like a single one.
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox_events WHERE entity = 'ticket' AND entity_id = ANY($1)",
    )
    .bind(&moved)
    .fetch_one(&test_state().await.pool)
    .await
    .expect("outbox count");
    assert!(
        events >= 2,
        "one ticket.status_changed row per moved ticket, got {events}"
    );
}

#[tokio::test]
async fn automations_naming_a_deleted_status_are_paused_not_repointed() {
    let app = core_api::routes::build_router(test_state().await);
    let (token, org_id) = scaffold(&app, "automations").await;
    let board = create_board(&app, &token, "Automation board").await;
    let status_id = add_status(&app, &token, &board, "Awaiting parts", "waiting", false).await;
    let rule_id = Uuid::new_v4();

    sqlx::query(
        r#"INSERT INTO automation_rules (id, org_id, board_id, name, trigger_event, conditions, actions, enabled)
           VALUES ($1, $2, $3, 'Pause on waiting', 'ticket.status_changed', '[]'::jsonb, $4::jsonb, true)"#,
    )
    .bind(rule_id)
    .bind(Uuid::parse_str(&org_id).expect("org uuid"))
    .bind(Uuid::parse_str(&board).expect("board uuid"))
    .bind(json!({ "set_status_id": status_id }).to_string())
    .execute(&test_state().await.pool)
    .await
    .expect("rule insert");

    assert_eq!(
        usage_for(&usage(&app, &token, &board).await, &status_id).1,
        1,
        "the confirm can count the rules that mention this status"
    );

    let seeded = statuses(&app, &token, &board).await;
    let home = seeded[0]["id"].as_str().unwrap().to_string();
    let deleted = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/boards/{board}/statuses/{status_id}?reassign_to={home}"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(deleted.status(), 200);
    assert_eq!(
        body_json(deleted).await["paused_automations"],
        json!(1),
        "the confirm's number and the delete's number are the same one"
    );

    let enabled: bool = sqlx::query_scalar("SELECT enabled FROM automation_rules WHERE id = $1")
        .bind(rule_id)
        .fetch_one(&test_state().await.pool)
        .await
        .expect("rule read");
    assert!(!enabled, "the rule is paused, waiting for a person");
}

#[tokio::test]
async fn the_run_log_reads_the_real_store_and_gates_on_the_surfaces_own_threshold() {
    let app = core_api::routes::build_router(test_state().await);
    let (token, org_id) = scaffold(&app, "runs").await;
    let board = create_board(&app, &token, "Run log board").await;
    let ticket = create_ticket(&app, &token, &board, "Shadow subject").await;

    // Grant a surface and pin a model, the way the settings drawer does.
    let grant = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/settings/ai/decision/decision_dispatch/config",
        &[auth(&token)],
        Some(json!({
            "idempotency_key": "settings-gaps-grant",
            "enabled": true,
            "mode": "shadow",
            "model_provider": "anthropic",
            "model_id": "claude-sonnet-4-6",
            "config": {
                "state_template": {
                    "fields": { "title": { "from": { "path": "title" } } }
                },
                "questions": [{
                    "id": "urgency",
                    "kind": "choice",
                    "instructions": "How urgent is this?",
                    "options": { "source": "fixed", "options": ["Low", "High"] },
                    "thresholds": { "act": 0.9, "review": 0.5 }
                }]
            }
        })),
    )
    .await;
    assert_eq!(grant.status(), 200, "grant must land: {}", grant.status());

    assert_eq!(
        body_json(grant).await["surface"]["customized"],
        json!(true),
        "the grant forked the surface, so the log reads THIS org's thresholds"
    );

    // Two recorded runs, exactly as the edge writes them: a picked label with
    // its confidence. One sits below the act threshold, one above it, so the
    // summary proves the gate rather than the fixture.
    for (run, label, confidence) in [
        (Uuid::new_v4(), "Low", 0.42_f64),
        (Uuid::new_v4(), "High", 0.97_f64),
    ] {
        sqlx::query(
            r#"INSERT INTO decision_answers
                 (id, org_id, surface, entity_type, entity_id, mode, provider, model, answers, created_by)
               VALUES ($1, $2, 'decision_dispatch', 'ticket', $3, 'shadow', 'anthropic', 'claude-sonnet-4-6', $4, NULL)"#,
        )
        .bind(run)
        .bind(Uuid::parse_str(&org_id).expect("org uuid"))
        .bind(Uuid::parse_str(&ticket).expect("ticket uuid"))
        .bind(json!({
            "urgency": { "kind": "choice", "picked": 0, "picked_label": label, "confidence": confidence }
        }))
        .execute(&test_state().await.pool)
        .await
        .expect("run insert");
    }

    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/settings/ai/decision/decision_dispatch/runs?days=7",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "the run log must resolve");
    let log = body_json(response).await;

    assert_eq!(log["summary"]["runs"], json!(2), "two recorded runs");
    assert_eq!(
        log["summary"]["threshold"],
        json!(0.9),
        "the org's own act threshold"
    );
    assert_eq!(
        log["summary"]["above_threshold"],
        json!(1),
        "0.97 clears 0.9 and 0.42 does not, so exactly one is above"
    );
    assert!(log["summary"]["has_evidence"] == json!(true));

    // Newest first, and each run carries the word, never an index.
    let runs = log["runs"].as_array().expect("run rows");
    let above = runs
        .iter()
        .find(|r| r["above_threshold"] == json!(true))
        .expect("the confident run");
    assert_eq!(above["answers"][0]["label"], json!("High"));
    assert_eq!(above["mode"], json!("shadow"));
    assert!(above["ticket_number"].is_string(), "the ticket is named");
    let below = runs
        .iter()
        .find(|r| r["above_threshold"] == json!(false))
        .expect("the unsure run");
    assert_eq!(
        below["answers"][0]["label"],
        json!("Low"),
        "the log shows the word, never an index"
    );
    let run = below;
    // Agreement is measured against where the ticket actually sits: the run
    // said "Low" and the ticket is in a status called something else, so the
    // product says "did not agree" rather than guessing.
    assert!(
        run["current_status"].is_string(),
        "the ticket still exists, so it is comparable"
    );
    assert_eq!(log["summary"]["comparable"], json!(2));
    assert_eq!(run["agreed"], json!(false), "measured, not narrated");

    // Raising nothing: the log refuses to invent runs that do not exist.
    let empty = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/settings/ai/decision/decision_dispatch/runs?days=1",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(empty.status(), 200);
}

// --- portal helpers ---------------------------------------------------------

/// Creates a portal user for the client that owns `ticket` and logs it in.
async fn portal_client(app: &Router, org_id: &str, client_id: &str) -> String {
    let pool = &test_state().await.pool;
    let client_id = Uuid::parse_str(client_id).expect("client uuid");
    let email = format!("client+{}@portal-gaps.test", Uuid::new_v4().simple());
    let password = "correct horse battery staple";
    let user_id = Uuid::new_v4();
    let hash = core_api::auth::password::hash(password).expect("hash");
    sqlx::query(
        r#"INSERT INTO portal_users
             (id, org_id, client_id, email, display_name, password_hash, role, status)
           VALUES ($1, $2, $3, $4, 'Client contact', $5, 'it_manager', 'active')"#,
    )
    .bind(user_id)
    .bind(Uuid::parse_str(org_id).expect("org uuid"))
    .bind(client_id)
    .bind(&email)
    .bind(&hash)
    .execute(pool)
    .await
    .expect("portal user insert");
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/portal/login",
        &[],
        Some(json!({ "email": email, "password": password })),
    )
    .await;
    assert_eq!(response.status(), 200, "portal login must resolve");
    body_json(response).await["access_token"]
        .as_str()
        .expect("portal token")
        .to_string()
}

async fn portal_tickets(app: &Router, portal_token: &str) -> Value {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/v1/portal/tickets",
        &[auth(portal_token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "portal tickets must resolve");
    body_json(response).await
}
