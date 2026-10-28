//! The workflow node seam applies the permission matrix (Wave 56 lane
//! `p56-workflow-seam`, audit record
//! `core-api/routes/internal.rs/workflow-node-seam-skips-permission-matrix`).
//!
//! Purpose: `POST /api/internal/workflow/node` used to run every node as the
//! request-supplied author under RLS alone — org scope, no matrix, no board
//! membership. A node the author could not run by hand would therefore run
//! through a workflow. These cases pin the fix from the outside:
//!
//! 1. an author without `tickets:edit` is refused, and the refusal is
//!    audited — with the ticket untouched;
//! 2. an author who holds it applies the node;
//! 3. a call that names no principal (absent, or the nil uuid) is refused and
//!    audited, because a node that acts for nobody has nothing to check;
//! 4. a board-scoped denial refuses the seam exactly as it refuses the ticket
//!    route for the same principal.
//!
//! Non-purpose: the mapping's own shape (core-api unit tests), the worker's
//! side of the call (worker unit tests), and plugin install grants (the
//! plugin-host suite).
//!
//! Database selection follows the documented contract in
//! `crates/core-db/tests/common/mod.rs` (MARULA_TEST_DATABASE_URL or
//! DATABASE_URL; no schema reset).

use crate::common::{body_json, send, state_with};
use axum::Router;
use core_api::state::AppState;
use serde_json::{json, Value};
use sqlx::PgPool;
use sqlx::Row as _;
use uuid::Uuid;

const INTERNAL_TOKEN: &str = "workflow-seam-internal-token";
const PASSWORD: &str = "correct-horse-battery";

async fn test_state() -> AppState {
    state_with(|config| {
        config.secret_key = core_api::config::SecretKey([56; 32]);
        config.internal_token = Some(INTERNAL_TOKEN.to_string());
    })
    .await
}

struct Org {
    token: String,
    org_id: Uuid,
}

async fn register_org(app: &Router, slug: String) -> Org {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Workflow Seam Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": PASSWORD,
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let tokens = body_json(response).await;
    let token = tokens["access_token"].as_str().unwrap().to_string();
    let me =
        body_json(send(app, axum::http::Method::GET, "/api/me", Some(&token), None).await).await;
    Org {
        token,
        org_id: me["org_id"].as_str().unwrap().parse().unwrap(),
    }
}

async fn default_board(app: &Router, org: &Org) -> Uuid {
    let nav = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/boards/nav",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    nav[0]["id"].as_str().unwrap().parse().expect("board id")
}

async fn create_ticket(app: &Router, org: &Org, board: Uuid, title: &str) -> Uuid {
    let created = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/tickets",
            Some(&org.token),
            Some(json!({ "board_id": board, "title": title, "priority": "p3" })),
        )
        .await,
    )
    .await;
    created["id"].as_str().unwrap().parse().expect("ticket id")
}

/// Inserts a same-org user with one builtin role and returns its id.
async fn add_user_with_role(state: &AppState, org_id: Uuid, builtin: &str, label: &str) -> Uuid {
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    let role: Uuid = sqlx::query_scalar("SELECT id FROM roles WHERE org_id = $1 AND builtin = $2")
        .bind(org_id)
        .bind(builtin)
        .fetch_one(&mut *tx)
        .await
        .expect("builtin role exists from registration");
    let user_id = Uuid::now_v7();
    core_db::users::create_user(
        &mut *tx,
        user_id,
        org_id,
        &format!("{label}-{user_id}@seam.test"),
        label,
        &core_api::auth::password::hash(PASSWORD).expect("hash"),
        "UTC",
    )
    .await
    .expect("user insert");
    core_db::users::assign_role(&mut *tx, user_id, role)
        .await
        .expect("role assign");
    tx.commit().await.expect("commit");
    user_id
}

/// Plays the worker: one node, on behalf of `author` (None = nobody named).
async fn seam_node(
    app: &Router,
    org_id: Uuid,
    author: Option<Uuid>,
    node_id: &str,
    params: Value,
    ticket_id: Uuid,
) -> axum::response::Response {
    let mut body = json!({
        "org_id": org_id,
        "node_id": node_id,
        "params": params,
        "context": {
            "event": { "ticket_id": ticket_id },
            "event_type": "ticket.created",
            "run_id": Uuid::now_v7(),
        },
    });
    if let Some(author) = author {
        body["actor_user_id"] = json!(author);
    }
    send(
        app,
        axum::http::Method::POST,
        "/api/internal/workflow/node",
        Some(INTERNAL_TOKEN),
        Some(body),
    )
    .await
}

/// The refusals this lane writes, newest first.
async fn refusals(pool: &PgPool, org_id: Uuid) -> Vec<(String, Option<String>, Value)> {
    let rows = sqlx::query(
        "SELECT actor_kind, actor_id, detail FROM audit_events \
         WHERE org_id = $1 AND action = 'workflow.node_denied' \
         ORDER BY occurred_at DESC",
    )
    .bind(org_id)
    .fetch_all(pool)
    .await
    .expect("audit read");
    rows.into_iter()
        .map(|row| {
            let kind: String = row.get("actor_kind");
            let id: Option<String> = row.get("actor_id");
            let detail: Value = row.get("detail");
            (kind, id, detail)
        })
        .collect()
}

async fn ticket_title(app: &Router, org: &Org, ticket_id: Uuid) -> String {
    let ticket = body_json(
        send(
            app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket_id}"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    ticket["title"].as_str().unwrap_or_default().to_string()
}

// ===========================================================================
// 1. An author without the grant is refused, audited, and writes nothing
// ===========================================================================

#[tokio::test]
async fn an_author_without_the_ticket_grant_is_refused_and_audited() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = register_org(&app, format!("seam-denied-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "Denied update").await;
    let viewer = add_user_with_role(&state, org.org_id, "read_only", "viewer").await;

    let response = seam_node(
        &app,
        org.org_id,
        Some(viewer),
        "ticket.update",
        json!({ "fields": { "title": "Changed by a viewer" } }),
        ticket,
    )
    .await;
    assert_eq!(
        response.status(),
        403,
        "a run authored by a read-only user cannot edit a ticket"
    );
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "forbidden");

    assert_eq!(
        ticket_title(&app, &org, ticket).await,
        "Denied update",
        "the refused node wrote nothing"
    );

    let rows = refusals(&state.pool, org.org_id).await;
    assert_eq!(rows.len(), 1, "the refusal is on the audit trail");
    let (kind, actor_id, detail) = &rows[0];
    assert_eq!(kind, "human");
    assert_eq!(
        actor_id.as_deref(),
        Some(viewer.to_string().as_str()),
        "the row names the principal the node acted for"
    );
    assert_eq!(detail["node_id"], json!("ticket.update"));
    assert_eq!(
        detail["required"],
        json!("tickets:edit"),
        "the row names the grant that was missing"
    );
}

// ===========================================================================
// 2. An author who holds the grant applies the node
// ===========================================================================

#[tokio::test]
async fn an_author_with_the_ticket_grant_applies_the_node() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = register_org(&app, format!("seam-allowed-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "Original title").await;
    let owner = add_user_with_role(&state, org.org_id, "owner", "second-owner").await;

    let response = seam_node(
        &app,
        org.org_id,
        Some(owner),
        "ticket.update",
        json!({ "fields": { "title": "Updated by the workflow" } }),
        ticket,
    )
    .await;
    assert_eq!(response.status(), 200, "the node applies");
    assert_eq!(
        body_json(response).await["output"]["ticketId"],
        json!(ticket.to_string())
    );
    assert_eq!(
        ticket_title(&app, &org, ticket).await,
        "Updated by the workflow"
    );
    assert!(
        refusals(&state.pool, org.org_id).await.is_empty(),
        "an applied node records no denial"
    );
}

// ===========================================================================
// 3. A call that names no principal is refused and audited
// ===========================================================================

#[tokio::test]
async fn a_call_that_names_no_principal_is_refused_and_audited() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = register_org(&app, format!("seam-nobody-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "Untouched").await;

    // Absent, and the nil uuid: both mean "nobody".
    for (label, author) in [("absent", None), ("nil", Some(Uuid::nil()))] {
        let response = seam_node(
            &app,
            org.org_id,
            author,
            "ticket.update",
            json!({ "fields": { "title": format!("Changed by {label}") } }),
            ticket,
        )
        .await;
        assert_eq!(response.status(), 403, "a call naming {label} is refused");
        let body = body_json(response).await;
        assert_eq!(
            body["error"]["reason"], "forbidden",
            "the refusal is typed, not a 500"
        );
    }
    assert_eq!(
        ticket_title(&app, &org, ticket).await,
        "Untouched",
        "neither refusal wrote anything"
    );

    let rows = refusals(&state.pool, org.org_id).await;
    assert_eq!(rows.len(), 2, "both refusals are audited");
    for (kind, actor_id, detail) in rows {
        assert_eq!(kind, "system", "nobody to attribute it to");
        assert!(actor_id.is_none());
        assert_eq!(detail["node_id"], json!("ticket.update"));
    }
}

// ===========================================================================
// 4. A board-scoped denial refuses the seam as the ticket route refuses it
// ===========================================================================

#[tokio::test]
async fn a_board_denial_refuses_the_seam_the_same_way_it_refuses_the_route() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = register_org(&app, format!("seam-board-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "Board-scoped").await;
    // A technician holds tickets:edit org-wide, but a viewer membership on
    // this board narrows it: the board denial wins over the org grant.
    let technician = add_user_with_role(&state, org.org_id, "technician", "board-viewer").await;
    let mut conn = state.pool.acquire().await.expect("conn");
    core_db::boards::upsert_member(
        &mut conn,
        org.org_id,
        board,
        technician,
        core_domain::policy::BoardRole::Viewer,
    )
    .await
    .expect("viewer membership");
    drop(conn);

    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(technician)
        .fetch_one(&state.pool)
        .await
        .expect("seeded email");
    let login = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/auth/login",
            None,
            Some(json!({ "email": email, "password": PASSWORD })),
        )
        .await,
    )
    .await;
    let token = login["access_token"].as_str().expect("login token");

    // The manual route refuses this principal on this board...
    let direct = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/tickets/{ticket}"),
        Some(token),
        Some(json!({ "title": "Patched by a board viewer" })),
    )
    .await;
    assert_eq!(direct.status(), 403, "the ticket route refuses the denial");

    // ...and the seam refuses the same node for the same principal.
    let response = seam_node(
        &app,
        org.org_id,
        Some(technician),
        "ticket.update",
        json!({ "fields": { "title": "Workflow on a denied board" } }),
        ticket,
    )
    .await;
    assert_eq!(
        response.status(),
        403,
        "a workflow cannot work a board its author is only a viewer on"
    );
    assert_eq!(
        ticket_title(&app, &org, ticket).await,
        "Board-scoped",
        "neither the route nor the seam wrote"
    );

    let rows = refusals(&state.pool, org.org_id).await;
    assert_eq!(rows.len(), 1, "the seam's refusal is audited");
    let (kind, actor_id, detail) = &rows[0];
    assert_eq!(kind, "human");
    assert_eq!(actor_id.as_deref(), Some(technician.to_string().as_str()));
    assert_eq!(detail["required"], json!("tickets:edit"));
    assert_eq!(
        detail["board_scoped"],
        json!(true),
        "the node is board-scoped; the board gate is what refused"
    );
    assert!(
        detail["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("board role"),
        "the reason names the board denial: {detail}"
    );
}
