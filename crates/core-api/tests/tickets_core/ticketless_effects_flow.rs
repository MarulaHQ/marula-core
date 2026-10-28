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
//! Since Wave 58 lane `p58-effects-seam` the seam also runs as the rule's
//! author, so every case below creates a rule whose author holds the grant.
//! A batch that names no rule, or a rule with no author, is refused; those
//! cases live in `internal_effects_principal_flow.rs`.
//!
//! Database selection follows the documented contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use crate::common::{body_json, register_org_token, send, test_state_with};
use axum::Router;
use core_api::state::AppState;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

const INTERNAL_TOKEN: &str = "ticketless-effects-internal-token";

async fn test_state() -> (AppState, PgPool) {
    let (state, _app) = test_state_with(|config| {
        config.internal_token = Some(INTERNAL_TOKEN.to_string());
        config.secret_key = core_api::config::SecretKey([41; 32]);
    })
    .await;
    let pool = state.pool.clone();
    (state, pool)
}

async fn register_org(app: &Router, slug: &str) -> String {
    register_org_token(app, "Ticketless Effects Co", slug).await
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

/// A rule row authored by `author`, which is the principal the effects seam
/// runs as, carrying the actions the batch will send. The seam refuses a rule
/// without an author, and it refuses a batch whose actions are not the rule's
/// own, so both are part of the fixture now.
async fn authored_rule(pool: &PgPool, org_id: Uuid, author: Uuid, actions: Value) -> Uuid {
    let rule_id = Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO automation_rules
               (id, org_id, board_id, name, trigger_event, conditions, actions, enabled, created_by)
           VALUES ($1, $2, NULL, $3, 'ticket.updated', '[]'::jsonb, $4::jsonb, true, $5)"#,
    )
    .bind(rule_id)
    .bind(org_id)
    .bind(format!("ticketless rule {rule_id}"))
    .bind(actions.to_string())
    .bind(author)
    .execute(pool)
    .await
    .expect("rule row");
    rule_id
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
    let rule_id = authored_rule(
        &pool,
        org_id,
        recipient,
        json!([{ "type": "notify_users", "user_ids": [recipient],
                  "kind": "ticket_update", "title": "Board alert" }]),
    )
    .await;

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

    let owner: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE org_id = $1 LIMIT 1")
        .bind(org_id)
        .fetch_one(&pool)
        .await
        .expect("owner user row");
    // The batch claims exactly the actions the rule carries: the seam refuses a
    // batch that asks for something else.
    let assignee = Uuid::now_v7();
    let rule_id = authored_rule(
        &pool,
        org_id,
        owner,
        json!([
            { "type": "add_internal_comment", "body": "needs a ticket" },
            { "type": "assign_user", "user_id": assignee },
            { "type": "set_priority", "priority": "p1" }
        ]),
    )
    .await;

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
            "actions": [
                { "type": "add_internal_comment", "body": "needs a ticket" },
                { "type": "assign_user", "user_id": assignee },
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
    let (state, pool) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("tl-scope-{}", Uuid::now_v7().simple());
    // Registration creates the org the rule lives in; the token is not needed
    // because every write below crosses the machine seam.
    let _token = register_org(&app, &slug).await;
    let org_id: Uuid = sqlx::query_scalar("SELECT id FROM orgs WHERE slug = $1")
        .bind(&slug)
        .fetch_one(&pool)
        .await
        .expect("org row");
    let owner: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE org_id = $1 LIMIT 1")
        .bind(org_id)
        .fetch_one(&pool)
        .await
        .expect("owner user row");
    let rule_id = authored_rule(
        &pool,
        org_id,
        owner,
        json!([{ "type": "notify_users", "user_ids": [owner],
                  "kind": "ticket_update", "title": "not yours" }]),
    )
    .await;

    // The board belongs to a different org, so it cannot be a scope here.
    let other_slug = format!("tl-scope-other-{}", Uuid::now_v7().simple());
    let other_token = register_org(&app, &other_slug).await;
    let board = create_board(&app, &other_token, "Scope Board").await;

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
            "actions": [{ "type": "notify_users", "user_ids": [owner],
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

    let rule_id = authored_rule(
        &pool,
        org_id,
        user_id,
        json!([{ "type": "notify_users", "user_ids": [user_id],
                  "kind": "ticket_update", "title": "Ticket alert" }]),
    )
    .await;

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
            "rule_id": rule_id,
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
