//! The client-safe hold reason, end to end through the REAL core-api router.
//!
//! Purpose: a PM pausing a project can put the reason in the CLIENT's words.
//! That value is stored on hold, cleared when the project resumes, shown to the
//! portal, and absent (null) on a project that was never paused — the portal
//! must never invent a wording the PM did not type.
//!
//! Database selection follows the portal binary's contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL.

use crate::common::{body_json, register_org, send_caller as send, state_with};
use axum::http::Method;
use axum::Router;
use core_api::state::AppState;
use serde_json::{json, Value};
use uuid::Uuid;

async fn test_state() -> AppState {
    state_with(|_| {}).await
}

/// Creates a project on `client_id` and returns its id.
async fn create_project(app: &Router, token: &str, client_id: &str, name: &str) -> String {
    let created = body_json(
        send(
            app,
            Method::POST,
            "/api/v1/projects",
            Some(token),
            Some(json!({ "name": name, "client_id": client_id })),
        )
        .await,
    )
    .await;
    assert_eq!(
        created["client_id"],
        json!(client_id),
        "project must land on the client it was created for"
    );
    created["id"].as_str().expect("project id").to_string()
}

/// Invites a portal contact on the client and signs it in.
async fn portal_token(app: &Router, staff_token: &str, client_id: &str, email: &str) -> String {
    let invited = send(
        app,
        Method::POST,
        &format!("/api/clients/{client_id}/portal-users"),
        Some(staff_token),
        Some(json!({
            "email": email,
            "display_name": "Portal Person",
            "password": "portal-passphrase-1",
            "role": "contact",
        })),
    )
    .await;
    assert_eq!(invited.status(), 200, "invite must succeed");
    let login = body_json(
        send(
            app,
            Method::POST,
            "/api/portal/login",
            None,
            Some(json!({ "email": email, "password": "portal-passphrase-1" })),
        )
        .await,
    )
    .await;
    login["access_token"]
        .as_str()
        .expect("portal access token")
        .to_string()
}

#[tokio::test]
async fn hold_reason_is_stored_shown_to_the_client_and_cleared_on_resume() {
    let app = core_api::routes::build_router(test_state().await);
    let tag = Uuid::now_v7().simple().to_string();
    let token = register_org(&app, "Portal Hold Co", &format!("portal-hold-{tag}")).await
        ["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    let client = body_json(
        send(
            &app,
            Method::POST,
            "/api/clients",
            Some(&token),
            Some(json!({ "name": "Acme" })),
        )
        .await,
    )
    .await;
    let client_id = client["id"].as_str().expect("client id").to_string();
    let project_id = create_project(&app, &token, &client_id, "Network refresh").await;
    let portal = portal_token(&app, &token, &client_id, &format!("contact@{tag}.test")).await;

    // --- pausing with a reason stores it, for staff and for the client ------
    let reason = "Waiting on the new switch delivery.";
    let paused = send(
        &app,
        Method::PATCH,
        &format!("/api/v1/projects/{project_id}"),
        Some(&token),
        Some(json!({
            "status": "on_hold",
            "client_hold_reason": reason,
            "version": 1,
        })),
    )
    .await;
    assert_eq!(paused.status(), 200, "pause must succeed");
    let paused = body_json(paused).await;
    assert_eq!(paused["status"], "on_hold");
    assert_eq!(
        paused["client_hold_reason"],
        json!(reason),
        "the hold stores the reason the PM typed"
    );

    let staff_read = body_json(
        send(
            &app,
            Method::GET,
            &format!("/api/v1/projects/{project_id}"),
            Some(&token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(staff_read["client_hold_reason"], json!(reason));

    let portal_read = body_json(
        send(
            &app,
            Method::GET,
            &format!("/api/portal/projects/{project_id}"),
            Some(&portal),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(portal_read["status"], "on_hold");
    assert_eq!(portal_read["status_text"], "On hold");
    assert_eq!(
        portal_read["hold_reason"],
        json!(reason),
        "the portal quotes the PM's words, not a server-composed sentence"
    );

    // --- resuming clears it -------------------------------------------------
    let resumed = send(
        &app,
        Method::PATCH,
        &format!("/api/v1/projects/{project_id}"),
        Some(&token),
        Some(json!({ "status": "active", "version": 2 })),
    )
    .await;
    assert_eq!(resumed.status(), 200, "resume must succeed");
    let resumed = body_json(resumed).await;
    assert_eq!(resumed["status"], "active");
    assert_eq!(
        resumed["client_hold_reason"],
        Value::Null,
        "leaving hold clears the client-facing reason"
    );

    let portal_after = body_json(
        send(
            &app,
            Method::GET,
            &format!("/api/portal/projects/{project_id}"),
            Some(&portal),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        portal_after["hold_reason"],
        Value::Null,
        "a resumed project tells the client nothing about a past pause"
    );

    // --- a project that was never paused carries null, not a sentence ------
    let other_id = create_project(&app, &token, &client_id, "Phone rollout").await;
    let never_paused = body_json(
        send(
            &app,
            Method::GET,
            &format!("/api/portal/projects/{other_id}"),
            Some(&portal),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        never_paused["hold_reason"],
        Value::Null,
        "no pause means no reason; the portal must not invent one"
    );

    // --- a reason sent while the project stays active is refused ------------
    let refused = send(
        &app,
        Method::PATCH,
        &format!("/api/v1/projects/{other_id}"),
        Some(&token),
        Some(json!({ "client_hold_reason": "should not store", "version": 1 })),
    )
    .await;
    assert_eq!(
        refused.status(),
        422,
        "a client reason without a pause would be silently dropped, so it is refused"
    );
}
