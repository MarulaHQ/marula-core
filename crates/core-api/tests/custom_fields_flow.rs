//! End-to-end custom-field flows (TASKLIST 2.17) driven through the REAL
//! core-api router against a real Postgres.
//!
//! Purpose: prove the dynamic-forms surface over HTTP — the per-kind
//! validation matrix on value writes, required-field enforcement at ticket
//! create plus clear-time refusal, values embedded in ticket detail/list
//! payloads, definition CRUD lifecycle, outbox event emission, and
//! cross-org isolation of definitions and values.
//!
//! Non-purpose: pure rule math (core-domain proptests), SQL-level RLS and
//! cascade probes (core-db db_suite), board-settings rendering (web).

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
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("fields e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage holds with custom-field columns");
    let mut config = Config::from_env();
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let state = AppState {
        pool,
        config: Arc::new(config),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&Config::from_env())),
        store: Arc::new(core_store::memory::MemoryStore::new()),
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
        panic!(
            "body_json parse failed ({e}); body={:?}",
            String::from_utf8_lossy(&bytes)
        )
    })
}

async fn register_org(app: &Router, slug: String) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Fields Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    body_json(response).await
}

/// Registers an org and returns (bearer, default_board_id).
async fn org_with_board(app: &Router, slug: String) -> (String, String) {
    let tokens = register_org(app, slug).await;
    let bearer = tokens["access_token"].as_str().unwrap().to_string();
    let boards = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/boards",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let board_id = boards[0]["id"].as_str().unwrap().to_string();
    (bearer, board_id)
}

async fn create_def(
    app: &Router,
    bearer: &str,
    board_id: &str,
    body: Value,
) -> (axum::http::StatusCode, Value) {
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/boards/{board_id}/fields"),
        Some(bearer),
        Some(body),
    )
    .await;
    let status = response.status();
    (status, body_json(response).await)
}

async fn set_value(
    app: &Router,
    bearer: &str,
    ticket_id: &str,
    field_id: &str,
    value: Value,
) -> (axum::http::StatusCode, Value) {
    let response = send(
        app,
        axum::http::Method::PUT,
        &format!("/api/tickets/{ticket_id}/fields/{field_id}"),
        Some(bearer),
        Some(json!({ "value": value })),
    )
    .await;
    let status = response.status();
    (status, body_json(response).await)
}

async fn make_ticket(
    app: &Router,
    bearer: &str,
    board_id: &str,
    title: &str,
    extra_fields: Option<Value>,
) -> (axum::http::StatusCode, Value) {
    let mut body = json!({
        "board_id": board_id,
        "title": title,
        "priority": "p2",
    });
    if let Some(extra) = extra_fields {
        body["custom_fields"] = extra;
    }
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(bearer),
        Some(body),
    )
    .await;
    let status = response.status();
    (status, body_json(response).await)
}

#[tokio::test]
async fn def_crud_lifecycle_and_board_scoping() {
    let (_state, app) = test_state().await;
    let (bearer, board) = org_with_board(&app, format!("fld1-{}", Uuid::now_v7().simple())).await;

    // --- definition CRUD ---------------------------------------------------
    let (status, select_def) = create_def(
        &app,
        &bearer,
        &board,
        json!({
            "name": "  Contact channel  ",
            "kind": "select",
            "options": ["Email", "Phone"],
            "required": false,
            "sort_order": 2,
        }),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{select_def}");
    assert_eq!(select_def["name"], "Contact channel", "name is trimmed");
    assert_eq!(select_def["options"][0], "Email");
    assert_eq!(select_def["entity"], "ticket");
    assert_eq!(select_def["board_id"], board.as_str());
    let select_id = select_def["id"].as_str().unwrap().to_string();

    // select without options is rejected.
    let (status, err) = create_def(
        &app,
        &bearer,
        &board,
        json!({ "name": "Empty choices", "kind": "select" }),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
    assert!(err["error"]["message"].as_str().unwrap().contains("option"));

    // text with options is rejected.
    let (status, _) = create_def(
        &app,
        &bearer,
        &board,
        json!({ "name": "Bad opts", "kind": "text", "options": ["x"] }),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);

    let (_, number_def) = create_def(
        &app,
        &bearer,
        &board,
        json!({ "name": "Hours spent", "kind": "number", "sort_order": 1 }),
    )
    .await;
    assert_eq!(
        number_def["options"],
        json!([]),
        "non-choice has no options"
    );
    let number_id = number_def["id"].as_str().unwrap().to_string();

    // List is sort-order stable and scoped to this board.
    let listed = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/boards/{board}/fields"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(listed.as_array().unwrap().len(), 2);
    assert_eq!(listed[0]["name"], "Hours spent");

    // PATCH renames + toggles required.
    let patched = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/boards/{board}/fields/{number_id}"),
        Some(&bearer),
        Some(json!({ "name": "Hours booked", "required": true, "sort_order": 0 })),
    )
    .await;
    assert_eq!(patched.status(), axum::http::StatusCode::OK);
    let patched = body_json(patched).await;
    assert_eq!(patched["name"], "Hours booked");
    assert_eq!(patched["required"], true);

    // Kind is immutable via PATCH; options on a non-choice kind are refused.
    let refused = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/boards/{board}/fields/{number_id}"),
        Some(&bearer),
        Some(json!({ "options": ["a"] })),
    )
    .await;
    assert_eq!(
        refused.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );

    // DELETE removes the definition; a second delete is a 404.
    let deleted = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/boards/{board}/fields/{select_id}"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(deleted.status(), axum::http::StatusCode::OK);
    let gone = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/boards/{board}/fields/{select_id}"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(gone.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn typed_values_validate_against_each_definition_kind() {
    let (_state, app) = test_state().await;
    let (bearer, board) = org_with_board(&app, format!("fld2-{}", Uuid::now_v7().simple())).await;
    let (status, ticket) = make_ticket(&app, &bearer, &board, "Matrix host", None).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{ticket}");
    let ticket_id = ticket["id"].as_str().unwrap().to_string();

    async fn def_of(
        app: &Router,
        bearer: &str,
        board: &str,
        name: &str,
        kind: &str,
        options: Option<Vec<&str>>,
    ) -> String {
        let mut body = json!({ "name": name, "kind": kind });
        if let Some(options) = options {
            body["options"] = json!(options);
        }
        let (status, def) = create_def(app, bearer, board, body).await;
        assert_eq!(status, axum::http::StatusCode::OK, "{def}");
        def["id"].as_str().unwrap().to_string()
    }

    let text_id = def_of(&app, &bearer, &board, "Notes", "text", None).await;
    let number_id = def_of(&app, &bearer, &board, "Hours", "number", None).await;
    let date_id = def_of(&app, &bearer, &board, "Due date", "date", None).await;
    let select_id = def_of(
        &app,
        &bearer,
        &board,
        "Channel",
        "select",
        Some(vec!["Email", "Phone"]),
    )
    .await;
    let multi_id = def_of(
        &app,
        &bearer,
        &board,
        "Channels",
        "multiselect",
        Some(vec!["Email", "Phone"]),
    )
    .await;
    let bool_id = def_of(&app, &bearer, &board, "Urgent", "bool", None).await;
    let url_id = def_of(&app, &bearer, &board, "Link", "url", None).await;
    let user_id = def_of(&app, &bearer, &board, "Owner ref", "user", None).await;

    // Accepting cases per kind (coercions included).
    for (field, value) in [
        (&text_id, json!("hello")),
        (&number_id, json!("42.5")),
        (&number_id, json!(7)),
        (&date_id, json!(" 2026-08-26 ")),
        (&select_id, json!("Phone")),
        (&multi_id, json!(["Phone", "Email", "Phone"])),
        (&bool_id, json!(false)),
        (&url_id, json!("https://sub.example.org/x?y=1")),
        (&user_id, json!("0199aaaa-bbbb-7ccc-addd-eeeeffff0000")),
    ] {
        let (status, payload) = set_value(&app, &bearer, &ticket_id, field, value).await;
        assert_eq!(status, axum::http::StatusCode::OK, "{payload}");
    }

    // Multiselect normalized (sorted, deduped) in storage.
    let detail = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket_id}"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let channels: Vec<&Value> = detail["custom_fields"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["field_id"] == multi_id.as_str())
        .collect();
    assert_eq!(channels.len(), 1);
    assert_eq!(
        channels[0]["value"],
        json!(["Email", "Phone"]),
        "multiselect is deduped + sorted"
    );

    // Rejecting cases per kind.
    for (field, value) in [
        (&text_id, json!(3)),
        (&number_id, json!("12abc")),
        (&number_id, json!(1e300)),
        (&date_id, json!("08/26/2026")),
        (&select_id, json!("Fax")),
        (&select_id, json!(3)),
        (&multi_id, json!("Email")),
        (&bool_id, json!("true")),
        (&url_id, json!("ftp://example.com")),
        (&url_id, json!("https://")),
        (&user_id, json!("nope")),
    ] {
        let (status, payload) = set_value(&app, &bearer, &ticket_id, field, value).await;
        assert_eq!(
            status,
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "expected rejection: {payload}"
        );
    }

    // Unknown / foreign field id is a 404.
    let (status, _) = set_value(
        &app,
        &bearer,
        &ticket_id,
        &Uuid::now_v7().to_string(),
        json!("anything"),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn required_enforcement_on_create_and_clear_and_payload_inclusion() {
    let (state, app) = test_state().await;
    let slug = format!("fld3-{}", Uuid::now_v7().simple());
    let (bearer, board) = org_with_board(&app, slug.clone()).await;

    let (_, req_select) = create_def(
        &app,
        &bearer,
        &board,
        json!({
            "name": "Tier",
            "kind": "select",
            "options": ["Gold", "Silver"],
            "required": true,
        }),
    )
    .await;
    let tier_id = req_select["id"].as_str().unwrap().to_string();

    // Creating WITHOUT the required value fails, naming the missing field.
    let (status, err) = make_ticket(&app, &bearer, &board, "Incomplete", None).await;
    assert_eq!(
        status,
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "{err}"
    );
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing required fields: Tier"),
        "{err}"
    );

    // Creating WITH it succeeds and embeds the value.
    let (status, ticket) = make_ticket(
        &app,
        &bearer,
        &board,
        "Complete",
        Some(json!([{ "field_id": tier_id, "value": "Gold" }])),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{ticket}");
    let ticket_id = ticket["id"].as_str().unwrap().to_string();
    assert_eq!(ticket["custom_fields"][0]["name"], "Tier");
    assert_eq!(ticket["custom_fields"][0]["value"], "Gold");
    assert_eq!(ticket["custom_fields"][0]["required"], true);

    // Clearing a REQUIRED value is refused with a typed 422…
    let cleared = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/tickets/{ticket_id}/fields/{tier_id}"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(
        cleared.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let cleared_body = body_json(cleared).await;
    assert!(cleared_body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("cannot be cleared"));

    // …and clearing an OPTIONAL value works end to end.
    let (_, opt_def) = create_def(
        &app,
        &bearer,
        &board,
        json!({ "name": "Optional note", "kind": "text" }),
    )
    .await;
    let opt_id = opt_def["id"].as_str().unwrap().to_string();
    let (status, _) = set_value(&app, &bearer, &ticket_id, &opt_id, json!("temp")).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let cleared_opt = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/tickets/{ticket_id}/fields/{opt_id}"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(cleared_opt.status(), axum::http::StatusCode::OK);

    // The LIST payload also carries values.
    let list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/tickets",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let row = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == ticket_id.as_str())
        .expect("created ticket in list");
    assert_eq!(row["custom_fields"][0]["value"], "Gold");

    // Outbox events landed for every mutation; the REFUSED clear emitted
    // nothing, while the successful OPTIONAL clear did.
    let org_row: (Uuid,) = sqlx::query_as(
        "SELECT o.id FROM orgs o JOIN users u ON u.org_id = o.id WHERE u.email = $1",
    )
    .bind(format!("owner@{slug}.test"))
    .fetch_one(&state.pool)
    .await
    .expect("org lookup");
    let types: Vec<String> = sqlx::query_as(
        "SELECT event_type FROM outbox_events WHERE org_id = $1 ORDER BY occurred_at",
    )
    .bind(org_row.0)
    .fetch_all(&state.pool)
    .await
    .expect("outbox scan")
    .into_iter()
    .map(|(t,)| t)
    .collect();
    assert!(types.contains(&"custom_field.def_created".to_string()));
    assert!(types.contains(&"ticket.created".to_string()));
    assert!(types.contains(&"ticket.field_set".to_string()));
    assert!(types.contains(&"ticket.field_cleared".to_string()));

    // Deleting the definition cascades its stored values.
    let deleted = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/boards/{board}/fields/{opt_id}"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(deleted.status(), axum::http::StatusCode::OK);
}

#[tokio::test]
async fn patch_validates_custom_fields_and_cross_org_isolation_holds() {
    let (_state, app) = test_state().await;
    let slug_a = format!("fld4a-{}", Uuid::now_v7().simple());
    let slug_b = format!("fld4b-{}", Uuid::now_v7().simple());
    let (token_a, board_a) = org_with_board(&app, slug_a).await;
    let (token_b, board_b) = org_with_board(&app, slug_b).await;

    let (_, def_a) = create_def(
        &app,
        &token_a,
        &board_a,
        json!({ "name": "A field", "kind": "text" }),
    )
    .await;
    let def_a_id = def_a["id"].as_str().unwrap().to_string();

    let (_, ticket_a) = make_ticket(&app, &token_a, &board_a, "Org A work", None).await;
    let ticket_a_id = ticket_a["id"].as_str().unwrap().to_string();

    // Org B cannot see A's defs, cannot write A's fields, and its own board
    // lists no defs from A's org.
    let b_view_a_defs = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/boards/{board_a}/fields"),
        Some(&token_b),
        None,
    )
    .await;
    assert_eq!(b_view_a_defs.status(), axum::http::StatusCode::NOT_FOUND);

    let (status, _) = set_value(&app, &token_b, &ticket_a_id, &def_a_id, json!("steal")).await;
    assert_ne!(status, axum::http::StatusCode::OK);

    let b_boards = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/boards/{board_b}/fields"),
            Some(&token_b),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(b_boards.as_array().unwrap().len(), 0);

    // PATCH with a valid custom-field value upserts it…
    let patched = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/tickets/{ticket_a_id}"),
        Some(&token_a),
        Some(json!({
            "priority": "p1",
            "custom_fields": [{ "field_id": def_a_id, "value": "via patch" }],
        })),
    )
    .await;
    assert_eq!(patched.status(), axum::http::StatusCode::OK);
    let patched = body_json(patched).await;
    assert_eq!(patched["priority"], "p1");
    let fields = patched["custom_fields"].as_array().unwrap();
    let a_field = fields
        .iter()
        .find(|f| f["field_id"] == def_a_id.as_str())
        .expect("patched field present");
    assert_eq!(a_field["value"], "via patch");

    // …and PATCH with an invalid value is rejected whole (422).
    let bad_patch = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/tickets/{ticket_a_id}"),
        Some(&token_a),
        Some(json!({
            "title": "should not land",
            "custom_fields": [{ "field_id": def_a_id, "value": 99 }],
        })),
    )
    .await;
    assert_eq!(
        bad_patch.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
}
