//! Wave 12 lane 6 (`p12-ticket-email-gaps`) end-to-end coverage, driven
//! through the REAL core-api router against a real Postgres.
//!
//! Purpose: pin the whole lane bundle over HTTP —
//! * the per-ticket EMAIL read endpoint (in/out rows, reply-chain parent
//!   resolution, outbound delivery projection) with RLS org scoping;
//! * checklist N-A server state (shared truth over the toggle endpoint,
//!   checked semantics untouched, close-gate `blocking` respects N-A);
//! * the email greeting/sign-off settings-registry declarations
//!   (user-scope round-trip through the existing settings surface);
//! * the org-scoped pod-visibility store (defaults-first GET, shape
//!   validation, Admin:edit gate, cross-org isolation);
//! * the ticket READ view carrying type_id/category_id (PATCH accepts
//!   classification, GET returns it, clearing works);
//! * the scheduling template auto-email riding the `email_outbound`
//!   exactly-once ledger inside the booking transaction;
//! * the lifted booking horizon (window 45 → preview slots past day 31,
//!   booking beyond day 31 succeeds).
//!
//! Non-purpose: worker transport retries (db_suite owns the claim/report
//! primitives) and the UI surfaces riding close-out lane 11.

use axum::Router;
use chrono::{SubsecRound, Timelike};
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> (AppState, Router) {
    test_state_with_mail(None).await
}

/// `transport_override` swaps the mail stack (the scheduling auto-email
/// only enqueues when the transport is not none-mode).
async fn test_state_with_mail(
    transport_override: Option<Arc<dyn core_mail::MailTransport>>,
) -> (AppState, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("lane-6 e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage holds with lane-6 tables");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let mut mail = core_api::email::MailStack::from_config(&config);
    if let Some(transport) = transport_override {
        mail.transport = transport;
    }
    let state = AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(mail),
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
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("body_json parse failed ({e})"))
}

struct Org {
    token: String,
    org_id: Uuid,
    user_id: Uuid,
}

async fn register_org(app: &Router, slug: String) -> Org {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Lane Six Co",
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
        user_id: me["user_id"].as_str().unwrap().parse().unwrap(),
        org_id: me["org_id"].as_str().unwrap().parse().unwrap(),
        token,
    }
}

async fn default_board(app: &Router, org: &Org) -> Uuid {
    let body = body_json(
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
    body.as_array()
        .and_then(|boards| boards.first())
        .and_then(|b| b["id"].as_str())
        .expect("default board")
        .parse()
        .unwrap()
}

async fn create_ticket(app: &Router, org: &Org, board: Uuid, title: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(&org.token),
        Some(json!({ "board_id": board, "title": title, "priority": "p3" })),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket create must succeed");
    body_json(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

/// Inserts a same-org user with NO role (hence no permissions) directly
/// through the pool and logs them in for a bearer token.
async fn add_roleless_user(app: &Router, state: &AppState, org: &Org, label: &str) -> String {
    let email = format!("{label}@lane-six.test");
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
            Some(json!({ "email": email, "password": "correct-horse-battery" })),
        )
        .await,
    )
    .await;
    login["access_token"].as_str().unwrap().to_string()
}

async fn org_outbound_count(state: &AppState, org_id: Uuid) -> i64 {
    let mut tx = state.pool.begin().await.unwrap();
    core_db::set_org(&mut tx, org_id).await.unwrap();
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM email_outbound WHERE org_id = $1")
        .bind(org_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    count
}

// ===========================================================================
// 1. Per-ticket email read — rows, reply chains, delivery, scoping
// ===========================================================================

#[tokio::test]
async fn ticket_emails_list_inbound_outbound_reply_chains_and_scope() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("emails-{}", Uuid::now_v7())).await;
    let other = register_org(&app, format!("emails-other-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket_id = create_ticket(&app, &org, board, "Email thread target").await;
    let roleless = add_roleless_user(&app, &state, &org, "noroles").await;

    // Seed the 2.4 ledger directly (the pipeline is covered by its own
    // suites): one inbound row routed to the ticket, one outbound reply
    // threaded to it via In-Reply-To, and one outbound unrelated row.
    let mut tx = state.pool.begin().await.unwrap();
    core_db::set_org(&mut tx, org.org_id).await.unwrap();
    let inbound_id = Uuid::now_v7();
    core_db::emails::insert_message(
        &mut tx,
        &core_db::emails::NewMessage {
            id: inbound_id,
            org_id: org.org_id,
            direction: core_domain::entities::EmailDirection::Inbound,
            rfc_message_id: "inbound-1@client.test".into(),
            in_reply_to: None,
            email_refs: vec![],
            subject: Some("Printer on fire".into()),
            from_address: "client@example.test".into(),
            from_name: Some("Client".into()),
            to_addresses: vec!["help@meridian.test".into()],
            body_text: Some("It is very much on fire.".into()),
            ticket_id: Some(ticket_id),
            comment_id: None,
            scan_status: core_domain::entities::ScanStatus::Clean,
            transport: "webhook/scripted".into(),
        },
    )
    .await
    .unwrap()
    .expect("inbound insert");
    core_db::emails::enqueue_outbound(
        &mut tx,
        core_db::emails::NewOutbound {
            org_id: org.org_id,
            rfc_message_id: core_db::emails::new_outbound_message_id(),
            in_reply_to: Some("INBOUND-1@client.test".into()),
            references: vec!["inbound-1@client.test".into()],
            subject: "Re: [TICKET] Printer on fire".into(),
            from_address: "help@meridian.test".into(),
            from_name: None,
            to_addresses: vec!["client@example.test".into()],
            body_text: "On it.".into(),
            ticket_id: Some(ticket_id),
            transport: "recording".into(),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // The thread view: both rows, newest first, with the reply parent
    // resolved case-insensitively onto the inbound ledger row.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/tickets/{ticket_id}/emails"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let view = body_json(response).await;
    let messages = view["messages"].as_array().expect("messages array");
    assert_eq!(messages.len(), 2, "inbound + outbound both render");
    let outbound_row = messages
        .iter()
        .find(|m| m["direction"] == "outbound")
        .expect("outbound row renders");
    let inbound_row = messages
        .iter()
        .find(|m| m["direction"] == "inbound")
        .expect("inbound row renders");
    assert_eq!(
        inbound_row["delivery"],
        Value::Null,
        "inbound has no delivery"
    );
    let inbound_id_str = inbound_row["id"].as_str().unwrap();
    assert_eq!(
        outbound_row["reply_to_message_id"].as_str(),
        Some(inbound_id_str),
        "reply chain resolves to the in-ticket parent"
    );
    assert_eq!(
        outbound_row["delivery"]["status"], "queued",
        "outbound delivery projects the ledger state"
    );
    assert_eq!(outbound_row["delivery"]["attempts"], 0);
    assert_eq!(
        outbound_row["ticket_id"].as_str(),
        Some(ticket_id.to_string().as_str()),
        "flatten keeps the ledger's ticket linkage on the row"
    );

    // RLS scoping: another org's token sees NOTHING (its own ticket space
    // has no such id) — a clean 404, never a cross-org leak.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/tickets/{ticket_id}/emails"),
        Some(&other.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 404, "cross-org read is rls-sparse");

    // A permissionless member of the SAME org is denied by the matrix.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/tickets/{ticket_id}/emails"),
        Some(&roleless),
        None,
    )
    .await;
    assert_eq!(response.status(), 403, "roleless member cannot read");
}

// ===========================================================================
// 2. Checklist N-A server state
// ===========================================================================

#[tokio::test]
async fn checklist_na_is_shared_server_truth_with_checked_compat() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("na-{}", Uuid::now_v7())).await;
    let board: Uuid = default_board(&app, &org).await;
    let ticket_id = create_ticket(&app, &org, board, "N-A target").await;

    // A board template WITH enforcement: the required item is a real
    // close blocker until checked or N-A.
    let response = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/boards/{board}/checklist-template"),
        Some(&org.token),
        Some(json!({
            "name": "Close gate",
            "items": ["Root cause documented"],
            "require_checklist_close": true,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "template upsert");

    // Reading the checklist instantiates the template item.
    let view = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket_id}/checklist"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(view["enforcement_active"], true);
    assert_eq!(view["progress"]["required_open"], 1);
    assert_eq!(
        view["blocking"],
        json!(["Root cause documented"]),
        "the unchecked required item blocks"
    );
    let item_id: Uuid = view["items"][0]["id"].as_str().unwrap().parse().unwrap();

    // Toggle to N-A (legacy shape: checked=false + na=true — the checked
    // field stays REQUIRED on the wire so old callers are untouched).
    let toggled = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/tickets/{ticket_id}/checklist/{item_id}"),
            Some(&org.token),
            Some(json!({
                "checked": false,
                "na": true,
                "note_reason": "N/A'd pending platform upgrade (Wave 17 close gate)",
            })),
        )
        .await,
    )
    .await;
    assert_eq!(
        toggled["mutation"]["after"]["na"], true,
        "N-A rides the envelope"
    );
    // Wave 17 lane 11 (§3.4): the N-A flip composes its ledger line in the
    // SAME transaction — the note comes back on the response with
    // row-level machine provenance (never a body-text sentinel).
    assert_eq!(toggled["appended_note"]["workflow_meta"]["kind"], "task_na",);
    assert_eq!(
        toggled["appended_note"]["body"],
        "- **N/A:** N/A'd pending platform upgrade (Wave 17 close gate)"
    );
    assert_eq!(toggled["mutation"]["after"]["checked"], false);
    assert_eq!(
        toggled["items"][0]["na"], true,
        "the view carries shared N-A"
    );
    assert_eq!(
        toggled["progress"]["required_open"], 0,
        "N-A frees the gate"
    );
    assert_eq!(toggled["blocking"], json!([]), "nothing blocks anymore");
    assert_eq!(toggled["mutation"]["before"]["na"], false, "undo substrate");

    // Round-trip back: N-A off → the blocker returns (checked semantics
    // unchanged throughout).
    let untoggled = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/tickets/{ticket_id}/checklist/{item_id}"),
            Some(&org.token),
            Some(serde_json::json!({
                "checked": false,
                "na": false,
                "note_reason": "N/A overturned — scope landed again",
            })),
        )
        .await,
    )
    .await;
    assert_eq!(untoggled["items"][0]["na"], false);
    assert_eq!(
        untoggled["appended_note"]["workflow_meta"]["kind"],
        "task_reopen"
    );
    assert_eq!(untoggled["progress"]["required_open"], 1);

    // Asking for BOTH states is a client bug: typed 422.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/tickets/{ticket_id}/checklist/{item_id}"),
        Some(&org.token),
        Some(json!({ "checked": true, "na": true })),
    )
    .await;
    assert_eq!(response.status(), 422, "checked + na is rejected");

    // Checking works exactly as before (na omitted ⇒ false, legacy wire).
    let checked = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/tickets/{ticket_id}/checklist/{item_id}"),
            Some(&org.token),
            Some(json!({ "checked": true })),
        )
        .await,
    )
    .await;
    assert_eq!(
        checked["items"][0]["na"], false,
        "legacy toggle never touches N-A"
    );
    assert_eq!(checked["items"][0]["checked"], true);
    assert!(checked["items"][0]["checked_at"].is_string());
    assert_eq!(checked["progress"]["checked"], 1);
}

// ===========================================================================
// 3. Email greeting/sign-off settings declarations
// ===========================================================================

#[tokio::test]
async fn greeting_and_signoff_persist_as_user_settings() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("sig-{}", Uuid::now_v7())).await;

    // The registry carries both declarations at user scope.
    let registry = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/settings/registry",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    for key in ["email.greeting", "email.signoff"] {
        let decl = registry["settings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["key"] == key)
            .unwrap_or_else(|| panic!("{key} declared"));
        assert_eq!(decl["scope"], "user");
        assert_eq!(decl["section"], "email");
        assert_eq!(decl["visibility"], "ui");
    }

    // Untouched: effective values are the (empty) defaults.
    let values = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/settings/user",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    let greeting = values
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["key"] == "email.greeting")
        .expect("greeting in effective values");
    assert_eq!(greeting["value"], "");

    // PUT round-trips a real override.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/user",
        Some(&org.token),
        Some(json!({ "key": "email.greeting", "value": "Hi {name}," })),
    )
    .await;
    assert_eq!(response.status(), 200, "greeting override accepted");
    let values = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/settings/user",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    let greeting = values
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["key"] == "email.greeting")
        .unwrap();
    assert_eq!(greeting["value"], "Hi {name},");

    // Validation holds: wrong type and over-long text are 422.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/user",
        Some(&org.token),
        Some(json!({ "key": "email.signoff", "value": 3 })),
    )
    .await;
    assert_eq!(response.status(), 422);
    let long = "x".repeat(501);
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/user",
        Some(&org.token),
        Some(json!({ "key": "email.signoff", "value": long })),
    )
    .await;
    assert_eq!(response.status(), 422, "over the 500-char cap");
}

// ===========================================================================
// 4. Org pod-visibility store
// ===========================================================================

#[tokio::test]
async fn pod_visibility_store_roundtrip_validates_and_scopes() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("pods-{}", Uuid::now_v7())).await;
    let other = register_org(&app, format!("pods-other-{}", Uuid::now_v7())).await;
    let roleless = add_roleless_user(&app, &state, &org, "viewer").await;

    // Defaults-first: an untouched org reads the empty matrix.
    let view = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/org/pod-visibility",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(view["config"], json!({}));

    // Shape validation: unknown RBAC levels and non-boolean leaves 422.
    for bad in [
        json!({ "tasks": { "wizard": false } }),
        json!({ "tasks": { "read_only": "no" } }),
        json!({ "Bad Pod": {} }),
    ] {
        let response = send(
            &app,
            axum::http::Method::PUT,
            "/api/org/pod-visibility",
            Some(&org.token),
            Some(json!({ "config": bad })),
        )
        .await;
        assert_eq!(response.status(), 422, "invalid matrix rejected: {bad}");
    }

    // Admin write round-trips.
    let matrix = json!({ "tasks": { "read_only": false }, "notes": { "technician": true } });
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/org/pod-visibility",
        Some(&org.token),
        Some(json!({ "config": matrix })),
    )
    .await;
    assert_eq!(response.status(), 200, "admin write accepted");
    let written = body_json(response).await;
    assert_eq!(written["config"], matrix);
    assert_eq!(
        written["updated_by"].as_str(),
        Some(org.user_id.to_string().as_str())
    );

    // Every member may READ the org matrix (config, not secrets).
    let view = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/org/pod-visibility",
            Some(&roleless),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(view["config"], matrix);

    // …but writing needs the Admin grant.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/org/pod-visibility",
        Some(&roleless),
        Some(json!({ "config": {} })),
    )
    .await;
    assert_eq!(response.status(), 403, "roleless member cannot write");

    // Cross-org isolation: the OTHER org still reads its own empty matrix.
    let view = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/org/pod-visibility",
            Some(&other.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(view["config"], json!({}), "RLS keeps org matrices apart");
}

// ===========================================================================
// 5. Ticket READ view carries classification
// ===========================================================================

#[tokio::test]
async fn ticket_read_view_carries_type_and_category() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("cls-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket_id = create_ticket(&app, &org, board, "Classification target").await;

    // The read view starts unclassified (both fields PRESENT, additive).
    let view = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket_id}"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(view["type_id"], Value::Null);
    assert_eq!(view["category_id"], Value::Null);

    // Create taxonomy nodes and classify.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/taxonomy/types",
        Some(&org.token),
        Some(json!({ "name": format!("Incident-{}", Uuid::now_v7().simple()) })),
    )
    .await;
    assert_eq!(response.status(), 200, "taxonomy type create");
    let node_type = body_json(response).await;
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/taxonomy/categories",
        Some(&org.token),
        Some(json!({ "name": format!("Network-{}", Uuid::now_v7().simple()) })),
    )
    .await;
    assert_eq!(response.status(), 200, "taxonomy category create");
    let category = body_json(response).await;
    let type_id = node_type["id"].as_str().unwrap();
    let category_id = category["id"].as_str().unwrap();

    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/tickets/{ticket_id}"),
        Some(&org.token),
        Some(json!({
            "type_id": type_id,
            "category_id": category_id,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "classification patch");
    let view = body_json(response).await;
    assert_eq!(view["type_id"].as_str(), Some(type_id));
    assert_eq!(view["category_id"].as_str(), Some(category_id));

    // The READ view (not just the mutation echo) carries them.
    let view = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket_id}"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(view["type_id"].as_str(), Some(type_id));
    assert_eq!(view["category_id"].as_str(), Some(category_id));

    // Clearing works and shows on the read view.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/tickets/{ticket_id}"),
        Some(&org.token),
        Some(json!({ "type_id": null })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let view = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket_id}"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(view["type_id"], Value::Null, "clear is visible on read");
    assert_eq!(
        view["category_id"].as_str(),
        Some(category_id),
        "untouched member stays"
    );
}

// ===========================================================================
// 6. Scheduling auto-email rides the exactly-once ledger
// ===========================================================================

async fn create_meeting_type(app: &Router, org: &Org, window_days: i32, auto_email: bool) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/meeting-types",
        Some(&org.token),
        Some(json!({
            "name": format!("Service Desk 30 — {}", Uuid::now_v7().simple()),
            "duration_minutes": 30,
            "booking_window_days": window_days,
            "assignee_scope": "user",
            "assignee_user_id": org.user_id,
            "auto_email_enabled": auto_email,
            "auto_email_subject": "Confirmed: your service desk visit",
            "auto_email_body": "Hi {name}, your booking on {date} is confirmed.",
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "meeting type create");
    // IF2 envelope: { meeting_type, mutation }.
    body_json(response).await["meeting_type"].clone()
}

#[tokio::test]
async fn auto_email_rides_the_outbound_ledger_exactly_once() {
    let recording = Arc::new(core_mail::RecordingTransport::new());
    let transport: Arc<dyn core_mail::MailTransport> = recording.clone();
    let (state, app) = test_state_with_mail(Some(transport)).await;
    let org = register_org(&app, format!("ledger-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket_id = create_ticket(&app, &org, board, "Scheduling source ticket").await;
    let meeting_type = create_meeting_type(&app, &org, 14, true).await;

    // Mint a link FROM the ticket (the booking lands back on it).
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/scheduling-links",
        Some(&org.token),
        Some(json!({
            "meeting_type_id": meeting_type["id"],
            "ticket_id": ticket_id,
            "label": "pickup a slot",
        })),
    )
    .await;
    assert!(response.status().is_success(), "link minted");
    let link = body_json(response).await;
    let token = link["token"].as_str().expect("raw token returned once");
    let starts_at = (chrono::Utc::now() + chrono::Duration::days(5))
        .trunc_subsecs(0)
        .with_nanosecond(0)
        .unwrap();

    // Availability for the owner: a fresh org user has no windows, so the
    // member_free gate would reject; seed an all-week window directly.
    {
        let mut tx = state.pool.begin().await.unwrap();
        core_db::set_org(&mut tx, org.org_id).await.unwrap();
        for weekday in 0..7 {
            core_db::scheduling::create_availability_window(
                &mut *tx,
                &core_domain::entities::AvailabilityWindow {
                    id: Uuid::now_v7(),
                    org_id: org.org_id,
                    user_id: org.user_id,
                    weekday,
                    start_local: chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
                    end_local: chrono::NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
                    timezone: "UTC".into(),
                    effective_from: None,
                    effective_until: None,
                    created_at: chrono::Utc::now(),
                },
            )
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();
    }

    // Wave 19 lane 5: registration ships the email-verification leg, so
    // the baseline is ONE queued (unsent) outbound; the BOOKING must add
    // the next single row on top of it (that exact increment is asserted
    // below — the exactly-once ledger is preserved).
    assert_eq!(
        org_outbound_count(&state, org.org_id).await,
        1,
        "exactly the register-time verification mail enqueued"
    );
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/public/scheduling-links/{token}/book"),
        None,
        Some(json!({
            "starts_at": starts_at.to_rfc3339(),
            "booker_name": "Dana",
            "booker_email": "dana@client.example",
            "idempotency_key": "book-exactly-once",
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "booking succeeds");
    let booked = body_json(response).await;
    assert!(
        booked["mutation"].is_object(),
        "booking keeps the IF2 envelope"
    );

    // Exactly-once: ONE ledger row + ONE message row, threaded to the
    // originating ticket, body rendered with the placeholders expanded.
    assert_eq!(
        org_outbound_count(&state, org.org_id).await,
        2,
        "exactly ONE more ledger row per booking (verification baseline spent)"
    );
    let mut tx = state.pool.begin().await.unwrap();
    core_db::set_org(&mut tx, org.org_id).await.unwrap();
    let (count, ticket_set): (i64, bool) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(BOOL_OR(m.ticket_id IS NOT NULL), false) FROM email_messages m \
         JOIN email_outbound q ON q.email_message_id = m.id \
         WHERE m.org_id = $1 AND m.direction = 'outbound'",
    )
    .bind(org.org_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        count, 2,
        "one message row behind the ledger row (+register verification mail)"
    );
    assert!(ticket_set, "confirmation lands on the ticket timeline");

    // A second booking attempt through the same (now used) token is a
    // typed 409 — the link's single-winner gate plus the in-tx idempotency
    // claim are the exactly-once story: no second enqueue is reachable.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/public/scheduling-links/{token}/book"),
        None,
        Some(json!({
            "starts_at": starts_at.to_rfc3339(),
            "booker_name": "Dana",
            "booker_email": "dana@client.example",
            "idempotency_key": "book-exactly-once",
        })),
    )
    .await;
    assert_eq!(response.status(), 409, "a used link cannot book again");
    assert_eq!(
        org_outbound_count(&state, org.org_id).await,
        2,
        "the retry path never double-enqueues (still register baseline + 1 booking)"
    );

    // The transport is NOT touched on the write path: the WORKER claims
    // and sends later (side effects off the write path).
    assert_eq!(
        recording.count().await,
        0,
        "no fire-and-forget send anymore"
    );
}

// ===========================================================================
// 7. Lifted booking horizon (>31 days)
// ===========================================================================

#[tokio::test]
async fn booking_horizon_follows_the_window_past_day_31() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("horizon-{}", Uuid::now_v7())).await;
    let meeting_type = create_meeting_type(&app, &org, 45, false).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/scheduling-links",
        Some(&org.token),
        Some(json!({ "meeting_type_id": meeting_type["id"] })),
    )
    .await;
    assert!(response.status().is_success(), "link minted");
    let token = body_json(response).await["token"]
        .as_str()
        .unwrap()
        .to_string();

    // Availability for the owner across the whole horizon.

    {
        let mut tx = state.pool.begin().await.unwrap();
        core_db::set_org(&mut tx, org.org_id).await.unwrap();
        for weekday in 0..7 {
            core_db::scheduling::create_availability_window(
                &mut *tx,
                &core_domain::entities::AvailabilityWindow {
                    id: Uuid::now_v7(),
                    org_id: org.org_id,
                    user_id: org.user_id,
                    weekday,
                    start_local: chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
                    end_local: chrono::NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
                    timezone: "UTC".into(),
                    effective_from: None,
                    effective_until: None,
                    created_at: chrono::Utc::now(),
                },
            )
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();
    }

    // The preview now reaches past the old 31-day calendar cap.
    let view = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/public/scheduling-links/{token}"),
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(view["booking_window_days"], 45);
    let slots = view["slots"].as_array().expect("slots array");
    let day31 = chrono::Utc::now() + chrono::Duration::days(31);
    assert!(
        slots.iter().any(|s| {
            s["starts_at"]
                .as_str()
                .map(|t| chrono::DateTime::parse_from_rfc3339(t).unwrap() > day31)
                .unwrap_or(false)
        }),
        "slot preview covers beyond day 31 (chunked horizon)"
    );

    // …and a booking at day 35 is ACCEPTED (the old cap previewed nothing
    // past day 31, making such bookings unreachable through the UI flow).
    let day35 = (chrono::Utc::now() + chrono::Duration::days(35))
        .trunc_subsecs(0)
        .with_nanosecond(0)
        .unwrap();
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/public/scheduling-links/{token}/book"),
        None,
        Some(json!({
            "starts_at": day35.to_rfc3339(),
            "booker_name": "Late planner",
            "booker_email": "late@client.example",
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        201,
        "day-35 booking inside a 45-day window"
    );
}
