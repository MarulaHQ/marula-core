//! Undo as a core capability (the private build tracker IF7), driven through the REAL
//! core-api router against a real Postgres.
//!
//! Purpose: pin the `docs/instant-first.md` undo contract end to end:
//!
//! * The undo stack — the per-field / per-record applied-state history
//!   read model over the 0028 write trail, newest first, with the
//!   record's undo `mode`.
//! * Revert = a NORMAL authorized idempotent write through the IF1 path:
//!   idempotency-keyed, version-gated, read-your-write; it lands on the
//!   trail AGAIN (seq increments, op `revert`), emits the registered
//!   `ticket.reverted` event, and appends an audited audit_events row —
//!   the append-only trail is never rewritten.
//! * Modes: near-total while the record is open (whole-record steps and
//!   per-field reverts); actioned reverts-only after close (whole-record
//!   steps refused with the typed `undo_reverts_only` 409).
//! * Authorization = the same as the original write.

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
        .expect("undo e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let state = AppState {
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
    let mut builder = axum::http::Request::builder()
        .method(method.clone())
        .uri(uri);
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
        panic!("body_json parse failed ({e}); last SEND lines identify the endpoint")
    })
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
            "org_name": "Undo Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
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

/// Inserts a same-org user with NO role (hence no permissions) directly
/// through the admin pool and logs them in for a bearer token.
async fn add_roleless_user(app: &Router, state: &AppState, org: &Org, label: &str) -> String {
    let email = format!("{label}@undo.test");
    core_db::users::create_user(
        &state.pool,
        Uuid::now_v7(),
        org.org_id,
        &email,
        label,
        &core_api::auth::password::hash("correct-horse-battery").unwrap(),
        "UTC",
    )
    .await
    .expect("seed roleless user");
    let login = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/auth/login",
            None,
            Some(json!({"email": email, "password": "correct-horse-battery"})),
        )
        .await,
    )
    .await;
    login["access_token"].as_str().unwrap().to_string()
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
    nav[0]["id"]
        .as_str()
        .unwrap()
        .parse()
        .expect("default board id")
}

async fn create_ticket(app: &Router, org: &Org, board: Uuid, extra: Value) -> Value {
    let mut body = json!({
        "board_id": board,
        "title": "Undo target",
        "priority": "p3",
    });
    if let (Some(dst), Some(src)) = (body.as_object_mut(), extra.as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(&org.token),
        Some(body),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket create must succeed");
    body_json(response).await
}

async fn patch_ticket(app: &Router, org: &Org, id: Uuid, body: Value) -> Value {
    let response = send(
        app,
        axum::http::Method::PATCH,
        &format!("/api/tickets/{id}"),
        Some(&org.token),
        Some(body),
    )
    .await;
    assert_eq!(response.status(), 200, "patch must succeed");
    body_json(response).await
}

async fn undo_stack(app: &Router, org: &Org, id: Uuid) -> Value {
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/tickets/{id}/undo-stack"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "undo stack must read");
    body_json(response).await
}

async fn revert(app: &Router, org: &Org, id: Uuid, body: Value) -> axum::response::Response {
    send(
        app,
        axum::http::Method::POST,
        &format!("/api/tickets/{id}/revert"),
        Some(&org.token),
        Some(body),
    )
    .await
}

/// The board's closed-category status ("Resolved" on the default board).
async fn closed_status_id(app: &Router, org: &Org, board: Uuid) -> Uuid {
    let statuses = body_json(
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
    statuses
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["category"] == "closed")
        .map(|s| s["id"].as_str().unwrap().parse().unwrap())
        .expect("default board seeds a closed status")
}

async fn trail_rows(state: &AppState, org_id: Uuid, ticket_id: Uuid) -> Vec<(i64, String, Value)> {
    sqlx::query_as(
        "SELECT seq, op, after FROM entity_writes \
         WHERE org_id = $1 AND entity = 'ticket' AND entity_id = $2 ORDER BY seq",
    )
    .bind(org_id)
    .bind(ticket_id)
    .fetch_all(&state.pool)
    .await
    .expect("trail rows")
}

// ===========================================================================
// Undo stack — the per-field / per-record applied-state read model
// ===========================================================================

#[tokio::test]
async fn undo_stack_surfaces_record_and_field_history_over_the_trail() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("undo-stack-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(
        &app,
        &org,
        board,
        json!({ "title": "Autosaved title", "body": "first body" }),
    )
    .await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();

    let _ = patch_ticket(&app, &org, ticket_id, json!({ "title": "Second title" })).await;
    let _ = patch_ticket(
        &app,
        &org,
        ticket_id,
        json!({ "priority": "p2", "body": "second body" }),
    )
    .await;

    let stack = undo_stack(&app, &org, ticket_id).await;
    assert_eq!(stack["entity"], "ticket");
    assert_eq!(stack["mode"], "open", "a non-closed record undoes freely");
    assert_eq!(stack["current_version"], 3);

    // Steps: newest first, one per applied write, restorable flags honest.
    let steps = stack["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 3, "create + two updates");
    assert_eq!(steps[0]["seq"], 3);
    assert_eq!(steps[0]["op"], "update");
    assert_eq!(steps[0]["restorable"], true);
    assert_eq!(steps[0]["before"]["priority"], "p3");
    assert_eq!(steps[0]["after"]["priority"], "p2");
    assert_eq!(steps[2]["seq"], 1);
    assert_eq!(steps[2]["op"], "create");
    assert!(steps[2]["before"].is_null(), "create has no prior snapshot");
    assert_eq!(steps[2]["restorable"], false, "creates are not restorable");

    // Field rows: per-field granularity with prior vs new side by side.
    let fields = stack["fields"].as_array().unwrap();
    let title_row = fields
        .iter()
        .find(|f| f["field"] == "title" && f["seq"] == 2)
        .expect("title change row");
    assert_eq!(title_row["prior"], "Autosaved title");
    assert_eq!(title_row["new_value"], "Second title");
    assert_eq!(title_row["changed"], true);
    // The create row carries every field with a null prior.
    let create_priority = fields
        .iter()
        .find(|f| f["field"] == "priority" && f["seq"] == 1)
        .expect("create priority row");
    assert!(create_priority["prior"].is_null());
    assert_eq!(create_priority["new_value"], "p3");
    assert_eq!(create_priority["changed"], true);
    // Untouched between writes: changed is false, values identical.
    let body_row = fields
        .iter()
        .find(|f| f["field"] == "body" && f["seq"] == 2)
        .expect("body row at seq 2");
    assert_eq!(body_row["changed"], false, "seq 2 changed only the title");
    assert_eq!(body_row["prior"], "first body");
    assert_eq!(body_row["new_value"], "first body");
}

#[tokio::test]
async fn undo_stack_requires_the_record_to_be_visible() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("undo-scope-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, json!({})).await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/tickets/{ticket_id}/undo-stack"),
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), 401);

    // A foreign record is indistinguishable from a missing one.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/tickets/{}/undo-stack", Uuid::now_v7()),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 404);
}

// ===========================================================================
// Revert — a normal authorized idempotent write through the IF1 path
// ===========================================================================

#[tokio::test]
async fn field_revert_restores_prior_values_and_lands_on_the_trail_again() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("undo-revert-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, json!({ "title": "T1" })).await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();

    let _ = patch_ticket(&app, &org, ticket_id, json!({ "title": "T2" })).await;
    let _ = patch_ticket(&app, &org, ticket_id, json!({ "title": "T3" })).await;

    // Step back over seq 3 (T2 -> T3): restore title to the write's prior
    // value, exactly what Ctrl/Cmd+Z over the newest edit means.
    let response = revert(
        &app,
        &org,
        ticket_id,
        json!({ "seq": 3, "fields": ["title"], "version": 3 }),
    )
    .await;
    assert_eq!(response.status(), 200, "revert is a normal write");
    let reverted = body_json(response).await;
    assert_eq!(reverted["title"], "T2", "the prior value is restored");
    assert_eq!(
        reverted["mutation"]["seq"], 4,
        "the revert lands on the trail"
    );
    assert_eq!(reverted["mutation"]["version"], 4);
    assert_eq!(reverted["mutation"]["before"]["title"], "T3");
    assert_eq!(reverted["mutation"]["after"]["title"], "T2");
    assert_eq!(reverted["revert"]["reverted_seq"], 3);
    assert_eq!(reverted["revert"]["fields"], json!(["title"]));
    assert_eq!(reverted["revert"]["whole_record"], false);

    // The trail is append-only: the original rows survive untouched and
    // the revert appended op `revert`.
    let rows = trail_rows(&state, org.org_id, ticket_id).await;
    assert_eq!(rows.len(), 4, "create + two updates + the revert");
    assert_eq!(rows[0].0, 1);
    assert_eq!(rows[0].1, "create");
    assert_eq!(rows[1].2["title"], "T2", "seq 2 row is UNREWRITTEN");
    assert_eq!(rows[2].2["title"], "T3", "seq 3 row is UNREWRITTEN");
    assert_eq!(rows[3].1, "revert");
    assert_eq!(rows[3].2["title"], "T2");

    // Audited twice over: the contract-registered event on the bus AND the
    // audit-trail row.
    let outbox: Value = sqlx::query_scalar(
        "SELECT payload FROM outbox_events WHERE org_id = $1 AND event_type = 'ticket.reverted' \
         AND entity_id = $2 ORDER BY occurred_at DESC LIMIT 1",
    )
    .bind(org.org_id)
    .bind(ticket_id.to_string())
    .fetch_one(&state.pool)
    .await
    .expect("ticket.reverted outbox event");
    assert_eq!(outbox["reverted_seq"], 3);
    assert_eq!(outbox["fields"], json!(["title"]));
    let detail: Value = sqlx::query_scalar(
        "SELECT detail FROM audit_events WHERE org_id = $1 AND action = 'ticket.reverted' \
         AND entity_id = $2 ORDER BY occurred_at DESC LIMIT 1",
    )
    .bind(org.org_id)
    .bind(ticket_id.to_string())
    .fetch_one(&state.pool)
    .await
    .expect("ticket.reverted audit row");
    assert_eq!(detail["reverted_seq"], 3);
    assert_eq!(detail["target_op"], "update");

    // The undo stack now shows the revert as the newest step.
    let stack = undo_stack(&app, &org, ticket_id).await;
    assert_eq!(stack["steps"][0]["op"], "revert");
    assert_eq!(stack["steps"][0]["seq"], 4);
    assert_eq!(stack["current_version"], 4);
}

#[tokio::test]
async fn whole_record_revert_is_near_total_while_the_record_is_open() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("undo-whole-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(
        &app,
        &org,
        board,
        json!({ "title": "Whole T1", "priority": "p3", "body": "whole body" }),
    )
    .await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();

    let _ = patch_ticket(
        &app,
        &org,
        ticket_id,
        json!({ "title": "Whole T2", "priority": "p1", "body": "changed body" }),
    )
    .await;

    // No `fields` list = whole-record step: every editable field returns
    // to the prior snapshot of write seq 2 (the create state).
    let response = revert(&app, &org, ticket_id, json!({ "seq": 2, "version": 2 })).await;
    assert_eq!(response.status(), 200);
    let reverted = body_json(response).await;
    assert_eq!(reverted["title"], "Whole T1");
    assert_eq!(reverted["priority"], "p3");
    assert_eq!(reverted["body"], "whole body");
    assert_eq!(reverted["revert"]["whole_record"], true);
    let fields: Vec<String> = reverted["revert"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().to_string())
        .collect();
    assert_eq!(fields.len(), 8, "every editable field is named");

    // A no-op revert (record already matches) is refused, not appended.
    let response = revert(&app, &org, ticket_id, json!({ "seq": 2, "version": 3 })).await;
    assert_eq!(
        response.status(),
        422,
        "nothing to restore is a typed refusal"
    );
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "validation_failed");

    // Refused writes never touch the trail.
    let rows = trail_rows(&state, org.org_id, ticket_id).await;
    assert_eq!(rows.len(), 3, "create + update + the one real revert");
}

#[tokio::test]
async fn revert_restores_an_absent_body_by_clearing_it() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("undo-absent-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    // Created WITHOUT a body; the field is absent from the first snapshots.
    let ticket = create_ticket(&app, &org, board, json!({ "title": "No body yet" })).await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();

    let _ = patch_ticket(&app, &org, ticket_id, json!({ "title": "Renamed" })).await;
    let _ = patch_ticket(
        &app,
        &org,
        ticket_id,
        json!({ "body": "body appeared later" }),
    )
    .await;

    // Step back over seq 2 for body only: the prior snapshot has NO body,
    // so the revert clears it — the value-restore path handles absent keys.
    let response = revert(
        &app,
        &org,
        ticket_id,
        json!({ "seq": 2, "fields": ["body"], "version": 3 }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let reverted = body_json(response).await;
    assert!(
        reverted["body"].is_null(),
        "the absent prior value is restored"
    );
    assert_eq!(reverted["title"], "Renamed", "unselected fields untouched");
}

#[tokio::test]
async fn revert_is_idempotent_and_version_gated() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("undo-idem-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, json!({ "title": "I1" })).await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();
    let _ = patch_ticket(&app, &org, ticket_id, json!({ "title": "I2" })).await;
    let _ = patch_ticket(&app, &org, ticket_id, json!({ "title": "I3" })).await;

    let key = Uuid::now_v7().to_string();
    let body = json!({ "seq": 3, "fields": ["title"], "version": 3, "idempotency_key": key });
    let first = revert(&app, &org, ticket_id, body.clone()).await;
    assert_eq!(first.status(), 200);
    let first = body_json(first).await;
    assert_eq!(first["title"], "I2", "stepping back over seq 3 restores I2");

    // Debounce/retry replay: byte-identical, never double-applies.
    let replay = revert(&app, &org, ticket_id, body).await;
    assert_eq!(replay.status(), 200);
    let replay = body_json(replay).await;
    assert_eq!(first, replay, "replay returns the stored result verbatim");
    let rows = trail_rows(&state, org.org_id, ticket_id).await;
    assert_eq!(rows.len(), 4, "no second revert row was appended");

    // A concurrent write moves the entity forward; the stale revert is a
    // typed 409 naming expected/provided, BEFORE anything changes.
    let _ = patch_ticket(&app, &org, ticket_id, json!({ "title": "I4" })).await;
    let response = revert(
        &app,
        &org,
        ticket_id,
        json!({ "seq": 3, "fields": ["title"], "version": 4 }),
    )
    .await;
    assert_eq!(response.status(), 409);
    let conflict = body_json(response).await;
    assert_eq!(conflict["error"]["reason"], "version_conflict");
    assert_eq!(conflict["error"]["expected"], 5);
    assert_eq!(conflict["error"]["provided"], 4);

    // The same key with a DIFFERENT payload is the typed reuse rejection.
    let response = revert(
        &app,
        &org,
        ticket_id,
        json!({
            "seq": 3,
            "fields": ["title"],
            "version": 5,
            "idempotency_key": key,
        }),
    )
    .await;
    assert_eq!(response.status(), 409);
    let reused = body_json(response).await;
    assert_eq!(reused["error"]["reason"], "idempotency_key_reused");
}

#[tokio::test]
async fn closed_record_takes_actioned_field_reverts_only() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("undo-closed-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, json!({ "title": "C1" })).await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();
    let _ = patch_ticket(&app, &org, ticket_id, json!({ "title": "C2" })).await;

    // Close the record: the undo checkpoint.
    let resolved: Uuid = closed_status_id(&app, &org, board).await;
    let closed = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/tickets/{ticket_id}/status"),
        Some(&org.token),
        Some(json!({ "status_id": resolved })),
    )
    .await;
    assert_eq!(closed.status(), 200, "close must succeed");
    assert_eq!(body_json(closed).await["category"], "closed");

    // The stack reports the governed mode.
    let stack = undo_stack(&app, &org, ticket_id).await;
    assert_eq!(stack["mode"], "reverts_only");
    assert_eq!(stack["current_version"], 3, "create + update + close");

    // A silent whole-record step past the close is refused, typed.
    let response = revert(&app, &org, ticket_id, json!({ "seq": 2, "version": 3 })).await;
    assert_eq!(response.status(), 409);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "undo_reverts_only");

    // The actioned, explicit per-field revert still works after close.
    let response = revert(
        &app,
        &org,
        ticket_id,
        json!({ "seq": 2, "fields": ["title"], "version": 3 }),
    )
    .await;
    assert_eq!(response.status(), 200, "actioned reverts remain available");
    let reverted = body_json(response).await;
    assert_eq!(reverted["title"], "C1");
    assert_eq!(reverted["mutation"]["seq"], 4);
}

#[tokio::test]
async fn revert_authorization_and_target_validation_match_the_write_path() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("undo-authz-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, json!({ "title": "A1" })).await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();
    let _ = patch_ticket(&app, &org, ticket_id, json!({ "title": "A2" })).await;

    // A same-org user with no permissions is forbidden — same rule as the
    // autosave PATCH they would be undoing.
    let roleless = add_roleless_user(&app, &state, &org, "roleless").await;
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/tickets/{ticket_id}/revert"),
        Some(&roleless),
        Some(json!({ "seq": 2, "fields": ["title"], "version": 2 })),
    )
    .await;
    assert_eq!(response.status(), 403);

    // Unknown seq / foreign seq references are typed validation errors.
    let response = revert(
        &app,
        &org,
        ticket_id,
        json!({ "seq": 99, "fields": ["title"], "version": 2 }),
    )
    .await;
    assert_eq!(response.status(), 422);
    assert_eq!(
        body_json(response).await["error"]["reason"],
        "validation_failed"
    );

    // Creates are visible history, not restorable values.
    let response = revert(
        &app,
        &org,
        ticket_id,
        json!({ "seq": 1, "fields": ["title"], "version": 2 }),
    )
    .await;
    assert_eq!(response.status(), 422);

    // Non-restorable field names are rejected by name.
    let response = revert(
        &app,
        &org,
        ticket_id,
        json!({ "seq": 2, "fields": ["status_id"], "version": 2 }),
    )
    .await;
    assert_eq!(response.status(), 422);
    let body = body_json(response).await;
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("status_id"),
        "the rejection names the offending field"
    );

    // Empty field list is a bad request, not a whole-record revert.
    let response = revert(
        &app,
        &org,
        ticket_id,
        json!({ "seq": 2, "fields": [], "version": 2 }),
    )
    .await;
    assert_eq!(response.status(), 422);

    // A MUTATION whose target is invisible under the caller's scope stays
    // rls_sparse (a foreign ticket id is indistinguishable from a missing
    // one — and must never leak which).
    let response = revert(
        &app,
        &org,
        Uuid::now_v7(),
        json!({ "seq": 2, "fields": ["title"], "version": 2 }),
    )
    .await;
    assert_eq!(response.status(), 404);
    assert_eq!(body_json(response).await["error"]["reason"], "rls_sparse");
}
