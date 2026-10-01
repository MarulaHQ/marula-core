//! Wave 42 (`p42-email-signature-preferences`) router-level coverage,
//! driven through the REAL core-api router against a real Postgres.
//!
//! Purpose: pin the decisions this lane made, over HTTP, where a reader can
//! see them end to end —
//! * the org's unified greeting + signature editor and the policy flag
//!   (allowed | org_locked) round-tripping through the mail settings surface;
//! * the FULL resolution chain, in the order a member would ask about it:
//!   per-message override > the sender's own default > the board's signature
//!   > the org's, with the applied rung on the ledger row;
//! * the org_locked refusal, on BOTH the stored default and a per-message
//!   override — a lock that merely ignored a write would leave a preference
//!   waiting for the day it is lifted;
//! * send-time resolution: a signature saved with `{{user.name}}` picks up a
//!   display-name change made AFTER the save, because nothing is baked in
//!   when the member types it;
//! * the token write gate: a placeholder outside the signature vocabulary is
//!   refused on the field that holds it.
//!
//! Non-purpose: the branded wrapper's HTML/text rendering (core-mail's own
//! suite) and the delivery transport (db_suite owns the claim/report
//! primitives).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

/// The real router over a real Postgres, assembled the same way the
/// sibling email suites do it (so a harness change upstream shows up here
/// as a compile error rather than as a silently different AppState).
async fn test_state() -> (AppState, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("wave-42 needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage holds with the wave-42 settings + ledger columns");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let mail = core_api::email::MailStack::from_config(&config);
    let state = AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(mail),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    };
    let app = core_api::routes::build_router(state.clone());
    (state, app)
}

async fn connect_admin(url: &str) -> Result<PgPool, sqlx::Error> {
    PgPool::connect(url).await
}

async fn send(
    app: &Router,
    method: axum::http::Method,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(path);
    if let Some(t) = token {
        builder = builder.header("authorization", format!("Bearer {t}"));
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
            "org_name": "Marula Field Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner Person",
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

async fn first_board(app: &Router, org: &Org) -> Uuid {
    let body = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/v1/boards/nav",
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
        "/api/v1/tickets",
        Some(&org.token),
        Some(json!({ "board_id": board, "title": title, "priority": "p3" })),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket create");
    body_json(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

/// Sets one org-scope mail setting through the mail settings surface.
async fn put_outbound(app: &Router, org: &Org, body: Value) -> Value {
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/v1/email/outbound-settings",
        Some(&org.token),
        Some(body),
    )
    .await;
    assert_eq!(response.status(), 200, "outbound settings write");
    body_json(response).await
}

/// Sets one of the caller's own user-scope settings; returns the status so a
/// test can assert on a REFUSAL as easily as on an acceptance.
async fn put_user_setting(
    app: &Router,
    org: &Org,
    key: &str,
    value: &str,
) -> axum::http::StatusCode {
    send(
        app,
        axum::http::Method::PUT,
        "/api/v1/settings/user",
        Some(&org.token),
        Some(json!({ "key": key, "value": value })),
    )
    .await
    .status()
}

/// The caller's resolved envelope: the live preview, through the same chain
/// the send path walks.
async fn envelope(app: &Router, org: &Org) -> Value {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/v1/settings/user/email-envelope",
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "envelope read");
    body_json(response).await
}

/// A second, roleless user in the org, returned as (id, email). The
/// client-visible notification only goes to a ticket's REQUESTER and never
/// back to its author, so the chain tests need one to exist.
async fn add_requester(state: &AppState, org: &Org, label: &str) -> (Uuid, String) {
    let email = format!("{label}@requester.test");
    let id = Uuid::now_v7();
    core_db::users::create_user(
        &state.pool,
        id,
        org.org_id,
        &email,
        label,
        &core_api::auth::password::hash("correct-horse-battery").unwrap(),
        "UTC",
    )
    .await
    .expect("seed requester");
    (id, email)
}

/// Creates a ticket whose requester is a CLIENT account, so a client-visible
/// note has someone to notify — which is the only path that enqueues the
/// message the chain is supposed to resolve.
async fn create_ticket_for_client(
    state: &AppState,
    app: &Router,
    org: &Org,
    board: Uuid,
    title: &str,
) -> (Uuid, Uuid) {
    let ticket = create_ticket(app, org, board, title).await;
    let (requester_id, _) = add_requester(state, org, &format!("client-{}", &title[..4])).await;
    let mut tx = state.pool.begin().await.unwrap();
    sqlx::query("UPDATE tickets SET requester_id = $1 WHERE id = $2")
        .bind(requester_id)
        .bind(ticket)
        .execute(&mut *tx)
        .await
        .expect("attach requester");
    tx.commit().await.unwrap();
    (ticket, requester_id)
}

/// Posts a client-visible note, optionally carrying the per-message
/// envelope, and returns the response status.
async fn post_client_note(
    app: &Router,
    org: &Org,
    ticket: Uuid,
    body: &str,
    envelope_override: Option<Value>,
) -> Value {
    let mut payload = json!({ "body": body, "kind": "client_visible" });
    if let Some(envelope) = envelope_override {
        payload["email_envelope"] = envelope;
    }
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/v1/tickets/{ticket}/comments"),
        Some(&org.token),
        Some(payload),
    )
    .await;
    assert_eq!(response.status(), 200, "client-visible note");
    body_json(response).await
}

/// The most recent outbound row for a ticket, with the body and the two
/// provenance labels the ledger now carries.
async fn last_outbound(
    state: &AppState,
    org_id: Uuid,
    ticket: Uuid,
) -> (String, Option<String>, Option<String>) {
    let mut tx = state.pool.begin().await.unwrap();
    core_db::set_org(&mut tx, org_id).await.unwrap();
    let row: (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT body_text, greeting_source, signature_source
           FROM email_messages
          WHERE org_id = $1 AND direction = 'outbound' AND ticket_id = $2
          ORDER BY created_at DESC, id DESC
          LIMIT 1",
    )
    .bind(org_id)
    .bind(ticket)
    .fetch_one(&mut *tx)
    .await
    .expect("outbound row");
    tx.commit().await.unwrap();
    row
}

// ===========================================================================
// 1. The org editor and the policy
// ===========================================================================

#[tokio::test]
async fn org_greeting_signature_and_policy_round_trip_together() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("p42-org-{}", Uuid::now_v7())).await;

    // Defaults first: an org with nothing set reads as empty, not as zeros
    // dressed up, and the policy is permissive.
    let settings = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/email/settings",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(settings["outbound"]["org_greeting"], "");
    assert_eq!(settings["outbound"]["signature_policy"], "allowed");

    let settings = put_outbound(
        &app,
        &org,
        json!({
            "org_greeting": "Hello from Acme,",
            "signature": "{{org.name}} Support",
            "signature_policy": "org_locked",
        }),
    )
    .await;
    assert_eq!(settings["outbound"]["org_greeting"], "Hello from Acme,");
    assert_eq!(settings["outbound"]["signature"], "{{org.name}} Support");
    assert_eq!(settings["outbound"]["signature_policy"], "org_locked");

    // A policy outside the option set is refused, and the stored one is
    // left exactly as it was — a bad write must not fall back to the
    // permissive default behind the admin's back.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/email/outbound-settings",
        Some(&org.token),
        Some(json!({ "signature_policy": "everyone_wins" })),
    )
    .await;
    assert_eq!(response.status(), 422, "an unknown policy is refused");
    let settings = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/email/settings",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        settings["outbound"]["signature_policy"], "org_locked",
        "the refused write must not reset the policy"
    );
}

// ===========================================================================
// 2. The full chain, and the ledger's memory of it
// ===========================================================================

#[tokio::test]
async fn the_chain_runs_message_override_then_user_then_board_then_org() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("p42-chain-{}", Uuid::now_v7())).await;
    let board = first_board(&app, &org).await;
    let (ticket, _requester) =
        create_ticket_for_client(&state, &app, &org, board, "Signature chain").await;

    // The org rung, and the org greeting.
    put_outbound(
        &app,
        &org,
        json!({
            "org_greeting": "Hello from Acme,",
            "signature": "{{org.name}} Support",
        }),
    )
    .await;

    // Nothing personal yet: the org answers, and the ledger says so.
    post_client_note(&app, &org, ticket, "No personal block yet.", None).await;
    let (body, greeting_source, signature_source) = last_outbound(&state, org.org_id, ticket).await;
    assert!(
        body.contains("Marula Field Co Support"),
        "the org signature must be expanded, not printed raw: {body}"
    );
    assert!(body.contains("Hello from Acme,"), "org greeting: {body}");
    assert_eq!(signature_source.as_deref(), Some("org_default"));
    assert_eq!(greeting_source.as_deref(), Some("org_default"));

    // The board rung beats the org's, and the ledger names the board.
    let response = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/v1/boards/{board}/email-identity"),
        Some(&org.token),
        Some(json!({ "signature": "Acme Projects" })),
    )
    .await;
    assert_eq!(response.status(), 200, "board identity signature");
    post_client_note(&app, &org, ticket, "Board block now.", None).await;
    let (body, _, signature_source) = last_outbound(&state, org.org_id, ticket).await;
    assert!(body.contains("Acme Projects"), "board signature: {body}");
    assert_eq!(signature_source.as_deref(), Some("board_identity"));

    // The sender's own default beats the board's — the recorded decision:
    // a personal signature is what `allowed` buys, and a board signature is
    // the shared fallback one rung above the org.
    assert_eq!(
        put_user_setting(&app, &org, "email.signoff", "Owner Person\nowner@acme.test").await,
        200
    );
    post_client_note(&app, &org, ticket, "My own block now.", None).await;
    let (body, _, signature_source) = last_outbound(&state, org.org_id, ticket).await;
    assert!(
        body.contains("Owner Person"),
        "the sender's own block: {body}"
    );
    assert!(
        !body.contains("Acme Projects"),
        "a personal default outranks the board's: {body}"
    );
    assert_eq!(signature_source.as_deref(), Some("user_default"));

    // A per-message override beats everything the policy allows.
    post_client_note(
        &app,
        &org,
        ticket,
        "Just this once.",
        Some(json!({ "signature": "Kind regards,\nOwner" })),
    )
    .await;
    let (body, _, signature_source) = last_outbound(&state, org.org_id, ticket).await;
    assert!(body.contains("Kind regards,"), "the override: {body}");
    assert!(
        !body.contains("owner@acme.test"),
        "the override replaces the default rather than joining it: {body}"
    );
    assert_eq!(signature_source.as_deref(), Some("message_override"));

    // The override travels with the note, NOT as a stored preference: the
    // member's default is untouched by a one-off.
    let preview = envelope(&app, &org).await;
    assert_eq!(preview["user_signature"], "Owner Person\nowner@acme.test");
    assert_eq!(preview["signature_source"], "user_default");
}

// ===========================================================================
// 3. The lock
// ===========================================================================

#[tokio::test]
async fn a_locked_org_refuses_personal_signatures_instead_of_ignoring_them() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("p42-lock-{}", Uuid::now_v7())).await;
    let board = first_board(&app, &org).await;
    let (ticket, _requester) =
        create_ticket_for_client(&state, &app, &org, board, "Locked signatures").await;
    put_outbound(&app, &org, json!({ "signature": "{{org.name}} Support" })).await;

    // A personal default is accepted while the policy is permissive.
    assert_eq!(
        put_user_setting(&app, &org, "email.signoff", "Owner Person").await,
        200
    );
    // After the lock: the WRITE is refused, and it says which policy did it.
    put_outbound(&app, &org, json!({ "signature_policy": "org_locked" })).await;
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/settings/user",
        Some(&org.token),
        Some(json!({ "key": "email.signoff", "value": "Trying to be personal" })),
    )
    .await;
    assert_eq!(response.status(), 422, "a locked org refuses the write");
    let refusal = body_json(response).await;
    let message = refusal["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("locked to the organization"), "{message}");

    // The stored value is the one from BEFORE the lock — the refusal did
    // not half-apply and did not wipe a member's existing default either.
    let preview = envelope(&app, &org).await;
    assert_eq!(preview["policy"], "org_locked");
    assert_eq!(preview["user_signature"], "Owner Person");

    // A message still goes out — with the org's block, labelled as such.
    post_client_note(&app, &org, ticket, "Locked and sending.", None).await;
    let (body, _, signature_source) = last_outbound(&state, org.org_id, ticket).await;
    assert!(body.contains("Marula Field Co Support"), "{body}");
    assert!(
        !body.contains("Owner Person"),
        "a locked org sends no personal block: {body}"
    );
    assert_eq!(signature_source.as_deref(), Some("org_default"));

    // And a per-message override is refused rather than quietly dropped: a
    // workspace that locked signatures must not be routable around one.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/tickets/{ticket}/comments"),
        Some(&org.token),
        Some(json!({
            "body": "Trying to route around the lock.",
            "kind": "client_visible",
            "email_envelope": { "signature": "Kind regards,\nOwner" },
        })),
    )
    .await;
    assert_eq!(response.status(), 422, "the override is refused too");
    let refusal = body_json(response).await;
    let message = refusal["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("locks email signatures"), "{message}");

    // Unlocking restores the member's stored default: the lock closed the
    // rungs, it never deleted the preference behind them.
    put_outbound(&app, &org, json!({ "signature_policy": "allowed" })).await;
    let preview = envelope(&app, &org).await;
    assert_eq!(preview["signature_source"], "user_default");
    post_client_note(&app, &org, ticket, "Unlocked again.", None).await;
    let (body, _, signature_source) = last_outbound(&state, org.org_id, ticket).await;
    assert!(body.contains("Owner Person"), "{body}");
    assert_eq!(signature_source.as_deref(), Some("user_default"));
}

// ===========================================================================
// 4. Send-time resolution
// ===========================================================================

#[tokio::test]
async fn a_signature_expands_against_the_senders_row_at_send_time() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("p42-sendtime-{}", Uuid::now_v7())).await;
    let board = first_board(&app, &org).await;
    let (ticket, _requester) =
        create_ticket_for_client(&state, &app, &org, board, "Send time").await;

    // Saved with a placeholder, not with today's values.
    assert_eq!(
        put_user_setting(
            &app,
            &org,
            "email.signoff",
            "{{user.name}}\n{{user.email}} · {{org.name}}"
        )
        .await,
        200
    );
    post_client_note(&app, &org, ticket, "Before the rename.", None).await;
    let (body, _, _) = last_outbound(&state, org.org_id, ticket).await;
    assert!(body.contains("Owner Person"), "first send: {body}");

    // The member's row changes AFTER the save — a display name and an email
    // address. Nothing is re-saved, and nothing needs to be.
    let mut tx = state.pool.begin().await.unwrap();
    core_db::set_org_and_user(&mut tx, org.org_id, org.user_id)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE users SET display_name = 'Renamed Person', email = 'renamed@acme.test' WHERE id = $1",
    )
    .bind(org.user_id)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    post_client_note(&app, &org, ticket, "After the rename.", None).await;
    let (body, _, signature_source) = last_outbound(&state, org.org_id, ticket).await;
    assert!(
        body.contains("Renamed Person") && body.contains("renamed@acme.test"),
        "the next send must use the NEW row: {body}"
    );
    assert!(
        !body.contains("Owner Person"),
        "the old row must not be baked into a saved signature: {body}"
    );
    assert_eq!(signature_source.as_deref(), Some("user_default"));

    // The preview moves with the row too — it is the same resolution.
    let preview = envelope(&app, &org).await;
    assert!(preview["signature"]
        .as_str()
        .unwrap_or_default()
        .contains("Renamed Person"));
    let tokens = preview["tokens"].as_array().expect("token rows");
    let name = tokens
        .iter()
        .find(|t| t["key"] == "user.name")
        .expect("user.name token");
    assert_eq!(name["value"], "Renamed Person");
}

// ===========================================================================
// 5. The write gate and the honest preview
// ===========================================================================

#[tokio::test]
async fn a_signature_refuses_placeholders_the_send_path_cannot_expand() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("p42-tokens-{}", Uuid::now_v7())).await;

    // Out of vocabulary: a ticket placeholder is a real token in the email
    // dialect and a nonsense one in a signature.
    for bad in ["{{ticket.number}}", "{{user.title}}", "{{client.name}}"] {
        let response = send(
            &app,
            axum::http::Method::PUT,
            "/api/v1/settings/user",
            Some(&org.token),
            Some(json!({ "key": "email.signoff", "value": bad })),
        )
        .await;
        assert_eq!(response.status(), 422, "{bad} must be refused");
    }
    // The org pair takes the same gate.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/email/outbound-settings",
        Some(&org.token),
        Some(json!({ "signature": "{{contact.email}}" })),
    )
    .await;
    assert_eq!(response.status(), 422, "the org signature is gated too");

    // In vocabulary: accepted, and the preview shows the value it will
    // expand to rather than the placeholder.
    assert_eq!(
        put_user_setting(&app, &org, "email.signoff", "Kind regards,\n{{user.name}}").await,
        200
    );
    let preview = envelope(&app, &org).await;
    assert_eq!(preview["user_signature"], "Kind regards,\n{{user.name}}");
    assert!(preview["signature"]
        .as_str()
        .unwrap_or_default()
        .contains("Owner Person"));
    // A member with no org signature is told so, rather than shown a blank.
    let preview = envelope(&app, &org).await;
    assert_eq!(preview["org_signature"], Value::Null);
    assert_eq!(preview["signature_source"], "user_default");
}
