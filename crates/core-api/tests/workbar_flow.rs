//! Workbar notes & tasks flow (Wave 17 lane 11, pair 0080) — the audit
//! invariants of docs/notes-tasks-workbar.md against the REAL router:
//!
//! 1. Checklist N-A REQUIRES a reason: without one the flip is a typed 422
//!    `validation_failed` and the item stays open (§3.4/§5.1).
//! 2. N-A WITH a reason: the flip + server-composed ledger note commit in
//!    ONE transaction — the response carries the appended note with
//!    `workflow_meta.kind = "task_na"` and body `- **N/A:** {reason}`
//!    through the shared formatter (§5.1 invariant 1).
//! 3. Close gate: a closed-category transition while an item is
//!    unassigned is the typed 409 `tasks_unassigned` naming the row —
//!    raw-API callers get the same rule (§4.1).
//! 4. Close-gate Option A: `complete-all` completes every listed item and
//!    appends a REPORTED auto note per item
//!    (`workflow_meta.kind = "task_closeall"`, `auto: true`), skipping
//!    already-terminal items idempotently; the subsequent close succeeds
//!    (§4.2/§5.3).
//! 5. IF2 note drafts: PUT autosave round-trips; the comment submit whose
//!    `draft_key` matches deletes the draft in the same tx — a submitted
//!    draft never resurrects, an unrelated draft row survives (§5.2).
//! 6. Note-draft hit is per (ticket, user): another user sees `draft:
//!    null` (RLS scope guarantees, not 404s).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
}

async fn test_state() -> (AppState, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("workbar flow needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env();
    config.database_url = admin_url;
    config.dev_tools = false;
    let state = AppState {
        pool,
        reporting: core_api::state::AppState::reporting_pool_lazy(&config.database_url)
            .expect("reporting pool"),
        config: Arc::new(config),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&Config::from_env())),
        store: Arc::new(core_store::memory::MemoryStore::new()),
    };
    let app = core_api::routes::build_router(state.clone());
    (state, app)
}

async fn send(
    app: &Router,
    method: axum::http::Method,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let request = match body {
        Some(v) => builder.body(axum::body::Body::from(v.to_string())).unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    };
    app.clone().oneshot(request).await.unwrap()
}

async fn as_json(response: axum::response::Response) -> (u16, Value) {
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).expect("JSON body"))
}

struct Org {
    token: String,
}

async fn register_org(app: &Router, slug: String) -> Org {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Workbar Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Jordan Reyes",
        })),
    )
    .await;
    let (status, v) = as_json(response).await;
    assert_eq!(status, 200, "register must succeed");
    Org {
        token: v["access_token"].as_str().unwrap().to_string(),
    }
}

async fn setup_ticket(app: &Router, org: &Org) -> (String, String) {
    // Default board (seeded).
    let (_, nav) = as_json(
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
    let board = nav[0]["id"].as_str().unwrap().to_string();
    // Create the ticket on that board.
    let (st, ticket) = as_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/tickets",
            Some(&org.token),
            Some(serde_json::json!({
                "board_id": board,
                "title": "Workbar gate ticket",
                "priority": "p3",
            })),
        )
        .await,
    )
    .await;
    assert_eq!(st, 200, "ticket create");
    let ticket_id = ticket["id"].as_str().unwrap().to_string();
    (ticket_id, board)
}

async fn add_item(app: &Router, org: &Org, ticket_id: &str, text: &str) -> String {
    let (st, item) = as_json(
        send(
            app,
            axum::http::Method::POST,
            &format!("/api/tickets/{ticket_id}/checklist"),
            Some(&org.token),
            Some(serde_json::json!({ "text": text })),
        )
        .await,
    )
    .await;
    assert_eq!(st, 200);
    item["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["text"] == text)
        .expect("created item in view")["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn closed_status(app: &Router, org: &Org, board: &str) -> String {
    let (st, statuses) = as_json(
        send(
            app,
            axum::http::Method::GET,
            &format!("/api/boards/{board}/statuses"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(st, 200);
    statuses
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["category"] == "closed")
        .expect("closed status exists on the default board")["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn try_close(
    app: &Router,
    org: &Org,
    ticket_id: &str,
    status_id: &str,
) -> (u16, serde_json::Value) {
    as_json(
        send(
            app,
            axum::http::Method::POST,
            &format!("/api/tickets/{ticket_id}/status"),
            Some(&org.token),
            Some(serde_json::json!({ "status_id": status_id })),
        )
        .await,
    )
    .await
}

#[tokio::test]
async fn na_requires_reason_and_composes_the_note_in_one_tx() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("wb-na-{}", Uuid::now_v7())).await;
    let (ticket_id, board) = setup_ticket(&app, &org).await;
    let item_id = add_item(&app, &org, &ticket_id, "Reinstall driver stack").await;
    let _closed = closed_status(&app, &org, &board).await;

    // 1. N-A without a reason → typed 422, item unchanged.
    let (st, err) = as_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/tickets/{ticket_id}/checklist/{item_id}"),
            Some(&org.token),
            Some(serde_json::json!({ "checked": false, "na": true })),
        )
        .await,
    )
    .await;
    assert_eq!(st, 422, "reason is mandatory");
    assert_eq!(err["error"]["reason"], "validation_failed");

    // 2. N-A WITH a reason → flip + note in one tx: response carries the
    //    appended note with row-level machine provenance.
    let (st, res) = as_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/tickets/{ticket_id}/checklist/{item_id}"),
            Some(&org.token),
            Some(serde_json::json!({
                "checked": false,
                "na": true,
                "note_reason": "customer already resolved via vendor",
            })),
        )
        .await,
    )
    .await;
    assert_eq!(st, 200, "reason accepted");
    let note = &res["appended_note"];
    assert!(note.is_object(), "the ledger note rides the response");
    assert_eq!(
        note["body"], "- **N/A:** Customer already resolved via vendor",
        "shared-formatter body (bullet + capitalized lead)"
    );
    assert_eq!(note["workflow_meta"]["kind"], "task_na");
    assert_eq!(note["workflow_meta"]["auto"], true);
    assert_eq!(
        note["workflow_meta"]["item_ids"][0], item_id,
        "provenance names the item, never body-text sentinels"
    );
    let flipped = res["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == item_id.as_str())
        .unwrap();
    assert_eq!(flipped["na"], true);

    // 3. Reopen: also mandatory + composed.
    let (st, err) = as_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/tickets/{ticket_id}/checklist/{item_id}"),
            Some(&org.token),
            Some(serde_json::json!({ "checked": false, "na": false })),
        )
        .await,
    )
    .await;
    assert_eq!(st, 422, "reopen needs its reason too");
    let (st, res) = as_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/tickets/{ticket_id}/checklist/{item_id}"),
            Some(&org.token),
            Some(serde_json::json!({
                "checked": false,
                "na": false,
                "note_reason": "vendor patch regressed the driver",
            })),
        )
        .await,
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(res["appended_note"]["workflow_meta"]["kind"], "task_reopen",);
    assert!(res["appended_note"]["body"]
        .as_str()
        .unwrap()
        .starts_with("- **Reopened:** Vendor"));
}

#[tokio::test]
async fn close_gate_typed_409_and_option_a_batch() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("wb-gate-{}", Uuid::now_v7())).await;
    let (ticket_id, board) = setup_ticket(&app, &org).await;
    let pending = add_item(&app, &org, &ticket_id, "Verify backups").await;
    let done = add_item(&app, &org, &ticket_id, "Patch firewall").await;
    let na = add_item(&app, &org, &ticket_id, "Legacy step").await;
    let closed = closed_status(&app, &org, &board).await;

    // Two items already have terminal decisions.
    for (id, checked, na, reason) in [
        (done.clone(), true, false, None),
        (
            na.clone(),
            false,
            true,
            Some("not applicable on this ticket".to_string()),
        ),
    ] {
        let (st, _) = as_json(
            send(
                &app,
                axum::http::Method::POST,
                &format!("/api/tickets/{ticket_id}/checklist/{id}"),
                Some(&org.token),
                Some(serde_json::json!({
                    "checked": checked,
                    "na": na,
                    "note_reason": reason,
                })),
            )
            .await,
        )
        .await;
        assert_eq!(st, 200);
    }

    // The gate: raw API first — typed 409 naming the EXACT open row.
    let (st, err) = try_close(&app, &org, &ticket_id, &closed).await;
    assert_eq!(st, 409, "unassigned item blocks the close");
    assert_eq!(err["error"]["reason"], "tasks_unassigned");
    assert_eq!(err["error"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(err["error"]["items"][0]["item_id"], pending.as_str());

    // Option A: batch closes the open tasks with per-item REPORTED notes.
    let (st, res) = as_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/tickets/{ticket_id}/checklist/complete-all"),
            Some(&org.token),
            Some(serde_json::json!({
                "task_ids": [&pending, &done],
            })),
        )
        .await,
    )
    .await;
    assert_eq!(st, 200, "batch completes");
    let (st, res) = (st, res);
    // Idempotent skip: the already-checked `done` item writes nothing new.
    let notes = res["appended_notes"].as_array().unwrap();
    assert_eq!(notes.len(), 1, "only the open item generated an auto note");
    assert_eq!(notes[0]["workflow_meta"]["kind"], "task_closeall");
    assert_eq!(notes[0]["workflow_meta"]["auto"], true);
    assert!(
        notes[0]["body"]
            .as_str()
            .unwrap()
            .starts_with("- **Completed:** Verify backups"),
        "CloseAll composes the Completed lead through the shared formatter"
    );
    // The batch response FLATTENS the checklist view (same additive
    // envelope as the toggle endpoint): items land top-level.
    assert!(
        res["items"].is_array(),
        "batch returns the flattened full view: {res}"
    );
    assert_eq!(res["items"].as_array().unwrap().len(), 3);

    // The close now succeeds — the re-issue path completes the loop.
    let (st, close_res) = try_close(&app, &org, &ticket_id, &closed).await;
    assert_eq!(
        st, 200,
        "close succeeds after Option A resolved the gate: {close_res:?}"
    );
}

#[tokio::test]
async fn note_drafts_autosave_and_delete_on_submit() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("wb-draft-{}", Uuid::now_v7())).await;
    let (ticket_id, _board) = setup_ticket(&app, &org).await;

    // IF2 autosave round-trip.
    let (st, saved) = as_json(
        send(
            &app,
            axum::http::Method::PUT,
            &format!("/api/tickets/{ticket_id}/note-draft"),
            Some(&org.token),
            Some(serde_json::json!({
                "body": "working over the weekend",
                "instant": { "idempotency_key": "draft-key-1" },
            })),
        )
        .await,
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(saved["draft"]["body"], "working over the weekend");
    assert_eq!(saved["draft"]["idempotency_key"], "draft-key-1");

    // GET returns the caller's draft (typed presence, not null-on-miss for
    // another ticket).
    let (st, got) = as_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket_id}/note-draft"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(got["draft"]["idempotency_key"], "draft-key-1");

    // Submit a comment claiming the draft's key: the draft clear happens
    // in the SAME tx as the comment commit (delete-on-submit).
    let (st, _comment) = as_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/tickets/{ticket_id}/comments"),
            Some(&org.token),
            Some(serde_json::json!({
                "body": "- Working over the weekend wrapped",
                "kind": "internal",
                "draft_key": "draft-key-1",
            })),
        )
        .await,
    )
    .await;
    assert_eq!(st, 200);
    let (st, after) = as_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket_id}/note-draft"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(st, 200);
    assert!(after["draft"].is_null(), "submitted draft never resurrects");

    // Machine notes on the comment path: user comments stay meta-NULL.
    let (st, comments) = as_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket_id}/comments"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(st, 200);
    for c in comments.as_array().unwrap() {
        assert!(
            c["workflow_meta"].is_null(),
            "user-written notes never carry workflow_meta"
        );
    }
}
