//! Client-portal v1 end-to-end flow through the REAL core-api router.
//!
//! Purpose: pin the whole TASKLIST 2.13 contract over HTTP — portal login,
//! per-board exposure gating, the CROSS-CLIENT isolation matrix (contact A
//! cannot read client B even in the same org), per-tier note-visibility
//! rules (it_manager sees in-house updates, NOBODY sees internal), reply
//! provenance (actor_kind=portal_* in the audit log + normal
//! ticket.comment_added events), and strict JWT family separation both ways.
//!
//! Non-purpose: SQL-layer behavior (core-db portal suite in db_suite),
//! domain-rule unit tests (core-domain), UI rendering.
//!
//! Database selection follows settings_flow's contract:
//! MERIDIAN_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

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
        .expect("portal e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
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
    bearer: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
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
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => panic!(
            "non-JSON body ({status}): {} :: {e}",
            String::from_utf8_lossy(&bytes)
        ),
    }
}

struct Org {
    token: String,
    board_id: String,
    client_a: String,
    client_b: String,
    ticket_a: String,
    ticket_b: String,
    /// Client-A ticket on the UNEXPOSED second board.
    ticket_a_hidden_board: String,
}

/// Registers one org with two clients, two boards, and one ticket per
/// interesting combination. Staff notes of ALL THREE kinds land on
/// ticket_a.
async fn seed_org(app: &Router, tag: &str) -> Org {
    let slug = format!("portal-flow-{tag}-{}", Uuid::now_v7().simple());
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Portal Flow Co",
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

    // Clients A + B.
    let client_a = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/clients",
            Some(&token),
            Some(json!({"name": "Client A"})),
        )
        .await,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let client_b = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/clients",
            Some(&token),
            Some(json!({"name": "Client B"})),
        )
        .await,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Second board (registration seeds the default board).
    let boards = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/boards",
            Some(&token),
            None,
        )
        .await,
    )
    .await;
    let board_id = boards[0]["id"].as_str().unwrap().to_string();
    let board_b_id = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/boards",
            Some(&token),
            Some(
                json!({"name": format!("Hidden-{tag}"), "description": null, "is_default": false}),
            ),
        )
        .await,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();

    async fn make_ticket(
        app: &Router,
        token: &str,
        board: &str,
        title: &str,
        client: Option<&str>,
    ) -> String {
        let response = send(
            app,
            axum::http::Method::POST,
            "/api/tickets",
            Some(token),
            Some(json!({
                "board_id": board,
                "title": title,
                "priority": "p2",
                "client_id": client,
            })),
        )
        .await;
        assert_eq!(response.status(), 200, "ticket create must succeed");
        body_json(response).await["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    let ticket_a = make_ticket(app, &token, &board_id, "Client A issue", Some(&client_a)).await;
    let ticket_b = make_ticket(app, &token, &board_id, "Client B secret", Some(&client_b)).await;
    let ticket_a_hidden_board =
        make_ticket(app, &token, &board_b_id, "On hidden board", Some(&client_a)).await;

    // All three note kinds on ticket_a.
    for kind in ["internal", "client_visible", "in_house_update"] {
        let response = send(
            app,
            axum::http::Method::POST,
            &format!("/api/tickets/{ticket_a}/comments"),
            Some(&token),
            Some(json!({"body": format!("{kind} body"), "kind": kind})),
        )
        .await;
        assert_eq!(response.status(), 200, "staff comment {kind}");
    }

    Org {
        token,
        board_id,
        client_a,
        client_b,
        ticket_a,
        ticket_b,
        ticket_a_hidden_board,
    }
}

async fn invite_portal_user(
    app: &Router,
    staff_token: &str,
    client_id: &str,
    email: &str,
    role: &str,
) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/clients/{client_id}/portal-users"),
        Some(staff_token),
        Some(json!({
            "email": email,
            "display_name": "Portal Person",
            "password": "portal-passphrase-1",
            "role": role,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "invite must succeed");
    body_json(response).await
}

async fn portal_login(app: &Router, email: &str) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/portal/login",
        None,
        Some(json!({"email": email, "password": "portal-passphrase-1"})),
    )
    .await;
    assert_eq!(response.status(), 200, "portal login for {email}");
    body_json(response).await
}

#[tokio::test]
async fn portal_flow_isolation_visibility_provenance_and_jwt_separation() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let tag = Uuid::now_v7().simple().to_string()[..8].to_string();
    let org = seed_org(&app, &tag).await;

    // --- staff-side provisioning ------------------------------------------
    let contact_a =
        invite_portal_user(&app, &org.token, &org.client_a, "contact@a.test", "contact").await;
    assert_eq!(contact_a["status"], "invited");
    invite_portal_user(&app, &org.token, &org.client_a, "it@a.test", "it_manager").await;
    invite_portal_user(&app, &org.token, &org.client_b, "contact@b.test", "contact").await;

    // Duplicate email under the same org is rejected cleanly.
    let dup = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/clients/{}/portal-users", org.client_a),
        Some(&org.token),
        Some(json!({
            "email": "CONTACT@a.test",
            "display_name": "Dup",
            "password": "portal-passphrase-1",
            "role": "approver",
        })),
    )
    .await;
    assert_eq!(dup.status(), 409, "unique-per-org email enforced");

    // Exposure: default board → client A only. Nothing for client B, and
    // the second board stays unexposed even for A.
    let exposure = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/clients/{}/board-exposures", org.client_a),
        Some(&org.token),
        Some(json!({
            "board_id": org.board_id,
            "exposed": true,
            "visible_status_ids": [],
        })),
    )
    .await;
    assert_eq!(exposure.status(), 200);
    // Status narrowing referencing a foreign board's status is rejected.
    let bad_status = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/clients/{}/board-exposures", org.client_a),
        Some(&org.token),
        Some(json!({
            "board_id": org.board_id,
            "exposed": true,
            "visible_status_ids": [Uuid::now_v7()],
        })),
    )
    .await;
    assert_eq!(bad_status.status(), 422, "unknown status ids rejected");

    // --- login + profile ----------------------------------------------------
    let contact = portal_login(&app, "contact@a.test").await;
    assert_eq!(contact["portal_user"]["role"], "contact");
    assert_eq!(
        contact["portal_user"]["client_id"].as_str().unwrap(),
        org.client_a,
        "identity bound to its client"
    );
    let contact_token = contact["access_token"].as_str().unwrap();

    // Invited flips to active on first login.
    let directory = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/clients/{}/portal-users", org.client_a),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    let invited_now_active = directory
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["email"] == "contact@a.test")
        .unwrap();
    assert_eq!(invited_now_active["status"], "active");

    // --- JWT FAMILY SEPARATION, BOTH DIRECTIONS ------------------------------
    let staff_on_portal = send(
        &app,
        axum::http::Method::GET,
        "/api/portal/me",
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(
        staff_on_portal.status(),
        401,
        "staff JWT cannot open portal endpoints"
    );
    let portal_on_staff = send(
        &app,
        axum::http::Method::GET,
        "/api/me",
        Some(contact_token),
        None,
    )
    .await;
    assert_eq!(
        portal_on_staff.status(),
        401,
        "portal JWT cannot open staff endpoints"
    );

    // --- LIST: exposure gating + cross-client isolation ----------------------
    let list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/portal/tickets",
            Some(contact_token),
            None,
        )
        .await,
    )
    .await;
    let listed_ids: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        listed_ids.len(),
        1,
        "exposed board yields exactly own ticket"
    );
    assert_eq!(listed_ids[0], org.ticket_a);

    // Ticket on an UNEXPOSED board (same client!) is a 404.
    let hidden = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/portal/tickets/{}", org.ticket_a_hidden_board),
        Some(contact_token),
        None,
    )
    .await;
    assert_eq!(
        hidden.status(),
        404,
        "unexposed board hides the ticket entirely"
    );

    // --- DETAIL: per-tier visibility ----------------------------------------
    let detail = |bearer: String| {
        let app = app.clone();
        let id = org.ticket_a.clone();
        async move {
            body_json(
                send(
                    &app,
                    axum::http::Method::GET,
                    &format!("/api/portal/tickets/{id}"),
                    Some(&bearer),
                    None,
                )
                .await,
            )
            .await
        }
    };
    let contact_view = detail(contact_token.to_string()).await;
    let kinds: Vec<&str> = contact_view["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        vec!["client_visible"],
        "contact sees exactly the client-visible note"
    );
    assert_eq!(contact_view["can_view_in_house_updates"], false);

    // Contact A CANNOT read client B's ticket even though it sits on the
    // same EXPOSED board in the same org — the core isolation row.
    let cross_client = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/portal/tickets/{}", org.ticket_b),
        Some(contact_token),
        None,
    )
    .await;
    assert_eq!(
        cross_client.status(),
        404,
        "cross-client reads are NotFound"
    );

    let it = portal_login(&app, "it@a.test").await;
    assert_eq!(it["portal_user"]["role"], "it_manager");
    let it_token = it["access_token"].as_str().unwrap().to_string();
    let it_view = detail(it_token.clone()).await;
    let it_kinds: Vec<&str> = it_view["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["kind"].as_str().unwrap())
        .collect();
    assert_eq!(it_kinds.len(), 2, "it_manager widens by exactly one kind");
    assert!(it_kinds.contains(&"in_house_update"));
    assert!(
        !it_kinds.contains(&"internal"),
        "internal is unreachable from ANY portal tier — the binding v1 rule"
    );

    // Client B has NO exposures: empty list, and ticket A is invisible.
    let b = portal_login(&app, "contact@b.test").await;
    let b_token = b["access_token"].as_str().unwrap();
    let b_list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/portal/tickets",
            Some(b_token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(b_list.as_array().unwrap().len(), 0, "no exposure ⇒ nothing");
    let b_cross = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/portal/tickets/{}", org.ticket_a),
        Some(b_token),
        None,
    )
    .await;
    assert_eq!(b_cross.status(), 404);

    // --- REPLY: kind, provenance, events -------------------------------------
    let reply = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/portal/tickets/{}/reply", org.ticket_a),
        Some(contact_token),
        Some(json!({"body": "Thanks, the workaround works."})),
    )
    .await;
    assert_eq!(reply.status(), 200);
    let replied = body_json(reply).await;
    assert_eq!(
        replied["kind"], "client_visible",
        "portal replies are ALWAYS client-visible"
    );
    assert!(
        replied["author_id"].is_null(),
        "portal identities are not staff rows — author_id stays NULL"
    );
    let comment_id = replied["id"].as_str().unwrap().to_string();

    // Replying into another client's ticket is refused before any write.
    let cross_reply = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/portal/tickets/{}/reply", org.ticket_b),
        Some(contact_token),
        Some(json!({"body": "should not land"})),
    )
    .await;
    assert_eq!(cross_reply.status(), 404, "cross-client reply refused");

    // Audit provenance: distinct actor_kind per tier, stored not derived.
    let audit_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT actor_kind, action FROM audit_events WHERE entity_id = $1 ORDER BY occurred_at",
    )
    .bind(&comment_id)
    .fetch_all(&state.pool)
    .await
    .expect("audit rows");
    assert_eq!(
        audit_rows,
        vec![("portal_contact".into(), "portal.ticket_reply".into())],
        "reply audited under portal_contact provenance"
    );

    // The IT-manager's view of the SAME thread also carries provenance if
    // they reply; verify the tier distinction end-to-end.
    let it_reply = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/portal/tickets/{}/reply", org.ticket_a),
        Some(&it_token),
        Some(json!({"body": "Escalating internally."})),
    )
    .await;
    assert_eq!(it_reply.status(), 200);
    let it_comment_id = body_json(it_reply).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let it_audit: Vec<(String,)> =
        sqlx::query_as("SELECT actor_kind FROM audit_events WHERE entity_id = $1")
            .bind(&it_comment_id)
            .fetch_all(&state.pool)
            .await
            .expect("audit rows");
    assert_eq!(
        it_audit.first().map(|r| r.0.clone()).as_deref(),
        Some("portal_it_manager")
    );

    // Normal comment events reach the outbox so staff notification paths
    // work unchanged.
    let events: Vec<(String,)> =
        sqlx::query_as("SELECT event_type FROM outbox_events WHERE entity = 'ticket_comment'")
            .fetch_all(&state.pool)
            .await
            .expect("outbox rows");
    let count = events
        .iter()
        .filter(|(t,)| t == "ticket.comment_added")
        .count();
    assert!(
        count >= 2,
        "both portal replies emitted ticket.comment_added"
    );

    // Empty replies are rejected.
    let empty_reply = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/portal/tickets/{}/reply", org.ticket_a),
        Some(contact_token),
        Some(json!({"body": "   "})),
    )
    .await;
    assert_eq!(empty_reply.status(), 422);
}

#[tokio::test]
async fn portal_refresh_rotates_and_replay_fails() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let tag = Uuid::now_v7().simple().to_string()[..8].to_string();
    let org = seed_org(&app, &tag).await;

    invite_portal_user(&app, &org.token, &org.client_a, "rotate@a.test", "contact").await;
    let login = portal_login(&app, "rotate@a.test").await;
    let refresh_token = login["refresh_token"].as_str().unwrap().to_string();
    let session_id = login["session_id"].as_str().unwrap().to_string();

    // Rotation mints a NEW pair and retires the presented session.
    let rotated = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/refresh",
        None,
        Some(json!({"refresh_token": refresh_token, "session_id": session_id})),
    )
    .await;
    assert_eq!(rotated.status(), 200);
    let new_pair = body_json(rotated).await;
    assert_ne!(
        new_pair["refresh_token"].as_str().unwrap(),
        refresh_token,
        "rotation issues a fresh refresh token"
    );

    // Replaying the OLD refresh token fails closed.
    let replay = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/refresh",
        None,
        Some(json!({"refresh_token": refresh_token, "session_id": session_id})),
    )
    .await;
    assert_eq!(
        replay.status(),
        401,
        "replayed refresh tokens never resolve"
    );

    // Wrong password is a uniform 401 (never reveals which half failed).
    let bad_password = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/login",
        None,
        Some(json!({"email": "rotate@a.test", "password": "definitely-wrong-password"})),
    )
    .await;
    assert_eq!(bad_password.status(), 401);
}

#[tokio::test]
async fn portal_admin_can_disable_and_delete_portal_users() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let tag = Uuid::now_v7().simple().to_string()[..8].to_string();
    let org = seed_org(&app, &tag).await;

    let user = invite_portal_user(&app, &org.token, &org.client_a, "gate@a.test", "approver").await;
    // A first login so the delete step can prove outstanding tokens die.
    let login = portal_login(&app, "gate@a.test").await;
    let stale_access = login["access_token"].as_str().unwrap().to_string();
    let user_id = user["id"].as_str().unwrap().to_string();

    // Disable blocks login immediately (the resolver skips disabled rows).
    let patched = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/portal-users/{user_id}"),
        Some(&org.token),
        Some(json!({"status": "disabled"})),
    )
    .await;
    assert_eq!(patched.status(), 200);
    let blocked = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/login",
        None,
        Some(json!({"email": "gate@a.test", "password": "portal-passphrase-1"})),
    )
    .await;
    assert_eq!(blocked.status(), 401, "disabled identities cannot log in");

    // Re-enable with a password reset, then hard delete cascades sessions.
    let reenabled = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/portal-users/{user_id}"),
        Some(&org.token),
        Some(json!({"status": "active", "password": "brand-new-passphrase"})),
    )
    .await;
    assert_eq!(reenabled.status(), 200);
    let after_reset = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/login",
        None,
        Some(json!({"email": "gate@a.test", "password": "brand-new-passphrase"})),
    )
    .await;
    assert_eq!(after_reset.status(), 200, "reset credential logs in");

    let deleted = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/portal-users/{user_id}"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(deleted.status(), 200);
    let gone = send(
        &app,
        axum::http::Method::GET,
        "/api/portal/tickets",
        Some(stale_access.as_str()),
        None,
    )
    .await;
    assert_eq!(
        gone.status(),
        401,
        "deleted identity's outstanding token is dead"
    );
}
