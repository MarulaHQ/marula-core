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
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

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
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("portal e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
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

/// `send` twin for the cookie-session flow: carries request headers (the
/// browser's Cookie jar plus the echoed x-marula-csrf) and returns the
/// response with its Set-Cookie heads intact.
async fn send_with_headers(
    app: &Router,
    method: axum::http::Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
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

/// Every Set-Cookie value on a response, in order.
fn set_cookie_values(response: &axum::response::Response) -> Vec<String> {
    response
        .headers()
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(String::from))
        .collect()
}

/// One cookie's bare value out of a Set-Cookie head list.
fn cookie_value(set_cookies: &[String], name: &str) -> String {
    set_cookies
        .iter()
        .find_map(|c| c.strip_prefix(&format!("{name}=")))
        .map(|c| c.split(';').next().unwrap_or("").to_string())
        .unwrap_or_else(|| panic!("no {name} Set-Cookie among {set_cookies:?}"))
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
    let body = body_json(response).await;
    // This route answers with the portal user ITSELF — `body = PortalUser`
    // in its OpenAPI annotation — so the row carries its own `id`. The
    // `{"portal_user": {...}, "access_token": ...}` envelope is the LOGIN
    // shape (`portal_login` below); reading one as the other is a bug that
    // only shows up as an unwrap deep inside a test, so the helper pins the
    // contract instead.
    assert!(
        body.get("id").and_then(Value::as_str).is_some(),
        "the staff invite answers with the portal user itself, so it must carry its own id"
    );
    body
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

    // Portal email is unique PER ORG (uq_portal_users_org_email), and
    // `portal_resolve_login` resolves across orgs with an arbitrary
    // LIMIT 1. Running this test twice against the same database would
    // therefore log in as ANOTHER org's contact and assert against an
    // identity this run never made. The run tag keeps every address
    // provisioned here unique to this run.
    let contact_a_email = format!("contact-{tag}@a.test");
    let it_email = format!("it-{tag}@a.test");
    let contact_b_email = format!("contact-b-{tag}@b.test");

    // --- staff-side provisioning ------------------------------------------
    let contact_a =
        invite_portal_user(&app, &org.token, &org.client_a, &contact_a_email, "contact").await;
    assert_eq!(contact_a["status"], "invited");
    invite_portal_user(&app, &org.token, &org.client_a, &it_email, "it_manager").await;
    invite_portal_user(&app, &org.token, &org.client_b, &contact_b_email, "contact").await;

    // Duplicate email under the same org is rejected cleanly.
    let dup = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/clients/{}/portal-users", org.client_a),
        Some(&org.token),
        Some(json!({
            "email": contact_a_email.to_uppercase(),
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
    let contact = portal_login(&app, &contact_a_email).await;
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
        .find(|u| u["email"] == contact_a_email.as_str())
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

    let it = portal_login(&app, &it_email).await;
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
    let b = portal_login(&app, &contact_b_email).await;
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
    // Run-unique so a rerun against the same database cannot resolve to
    // another org's contact (portal email is unique per org, and the
    // resolver picks across orgs).
    let email = format!("rotate-{tag}@a.test");

    invite_portal_user(&app, &org.token, &org.client_a, &email, "contact").await;
    let login = portal_login(&app, &email).await;
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
        Some(json!({"email": email, "password": "definitely-wrong-password"})),
    )
    .await;
    assert_eq!(bad_password.status(), 401);
}

// ---------------------------------------------------------------------------
// Wave 47 lane A1 — the cookie-session CSRF contract (BUILD-PROMPT
// follow-up 11b; decision of record docs/adr/0022-portal-cookie-double-submit.md)
//
// The documented design (crates/core-api/src/auth/cookies.rs, the module
// comment, and docs/reference/auth-threat-model.md §A2) is double-submit:
// the refresh secret rides an HttpOnly cookie, and a JS-READABLE
// `marula_csrf` value is echoed back on `x-marula-csrf`. The browser can
// read its own cookie and a foreign origin cannot, so only same-origin
// JavaScript can produce the header. This test pins all three halves:
//
//   1. the shipped cookie flags (secret HttpOnly, csrf value NOT);
//   2. the browser's full-page-load path — cookie-only rotation with the
//      echoed header PASSES, so a reload re-auths;
//   3. the check is ENFORCED, not deleted — a missing or wrong header is
//      403 while an ambient session cookie is present.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn portal_cookie_session_double_submit_round_trip() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let tag = Uuid::now_v7().simple().to_string()[..8].to_string();
    let org = seed_org(&app, &tag).await;
    let email = format!("cookie-{tag}@a.test");

    invite_portal_user(&app, &org.token, &org.client_a, &email, "contact").await;

    // --- login, and read the cookie pair the browser would store ----------
    let login = send_with_headers(
        &app,
        axum::http::Method::POST,
        "/api/portal/login",
        &[],
        Some(json!({"email": email, "password": "portal-passphrase-1"})),
    )
    .await;
    assert_eq!(login.status(), 200, "portal login for {email}");
    let login_cookies = set_cookie_values(&login);

    let csrf_head = login_cookies
        .iter()
        .find(|c| c.starts_with("marula_csrf="))
        .unwrap_or_else(|| panic!("no marula_csrf Set-Cookie: {login_cookies:?}"));
    assert!(
        !csrf_head.contains("HttpOnly"),
        "the double-submit value MUST be readable by the browser's own JavaScript \
         — HttpOnly here is the follow-up 11b defect: {csrf_head}"
    );
    // Path scoping is the other half of readability: the app's pages live at
    // /portal and friends, so a /api-scoped cookie never shows up in
    // document.cookie and no client can echo the header (found live during
    // the lane: HttpOnly alone was not enough on the split web/api ports).
    assert!(
        csrf_head.contains("Path=/") && !csrf_head.contains("Path=/api"),
        "the csrf value must be host-wide so every page can read it: {csrf_head}"
    );
    assert!(csrf_head.contains("SameSite=Lax"), "{csrf_head}");
    let portal_head = login_cookies
        .iter()
        .find(|c| c.starts_with("marula_portal="))
        .unwrap_or_else(|| panic!("no marula_portal Set-Cookie: {login_cookies:?}"));
    assert!(
        portal_head.contains("HttpOnly"),
        "the refresh secret stays unreadable to JavaScript: {portal_head}"
    );
    assert!(
        portal_head.contains("Path=/api"),
        "the session jar stays /api-scoped — it only ever needs to be SENT: {portal_head}"
    );
    // The readable value IS the double-submit secret: 64 hex chars, the
    // same entropy class as the refresh tokens.
    let csrf_value = cookie_value(&login_cookies, "marula_csrf");
    assert_eq!(csrf_value.len(), 64, "csrf value is 32 random bytes");
    assert!(csrf_value.chars().all(|c| c.is_ascii_hexdigit()));

    // --- the browser's full-page-load path: jar only, no bearer ------------
    // A reload has no in-memory tokens: the httpOnly jar carries the
    // session, the readable csrf value rides the rotation header.
    let jar = format!(
        "marula_portal={}; marula_csrf={csrf_value}",
        cookie_value(&login_cookies, "marula_portal")
    );
    let rotated = send_with_headers(
        &app,
        axum::http::Method::POST,
        "/api/portal/refresh",
        &[
            ("cookie", jar.as_str()),
            ("x-marula-csrf", csrf_value.as_str()),
        ],
        Some(json!({})),
    )
    .await;
    assert_eq!(
        rotated.status(),
        200,
        "cookie-driven refresh with the echoed header must pass double_submit_ok"
    );
    let rotated_cookies = set_cookie_values(&rotated);
    let csrf_2 = cookie_value(&rotated_cookies, "marula_csrf");
    let jar_2 = format!(
        "marula_portal={}; marula_csrf={csrf_2}",
        cookie_value(&rotated_cookies, "marula_portal")
    );

    // --- the check is enforced, never deleted -------------------------------
    let no_header = send_with_headers(
        &app,
        axum::http::Method::POST,
        "/api/portal/refresh",
        &[("cookie", jar_2.as_str())],
        Some(json!({})),
    )
    .await;
    assert_eq!(
        no_header.status(),
        403,
        "an ambient session cookie without the echoed header is a refused CSRF attempt"
    );
    let wrong_header = send_with_headers(
        &app,
        axum::http::Method::POST,
        "/api/portal/refresh",
        &[
            ("cookie", jar_2.as_str()),
            ("x-marula-csrf", "not-the-cookie-value"),
        ],
        Some(json!({})),
    )
    .await;
    assert_eq!(
        wrong_header.status(),
        403,
        "a header that does not match the cookie is refused"
    );
    // The live pair still rotates once the correct header rides along.
    let rotated_again = send_with_headers(
        &app,
        axum::http::Method::POST,
        "/api/portal/refresh",
        &[
            ("cookie", jar_2.as_str()),
            ("x-marula-csrf", csrf_2.as_str()),
        ],
        Some(json!({})),
    )
    .await;
    assert_eq!(rotated_again.status(), 200, "the good path still works");

    // --- the bearer path is untouched (no ambient cookie ⇒ no check) ---------
    let re_login = send_with_headers(
        &app,
        axum::http::Method::POST,
        "/api/portal/login",
        &[],
        Some(json!({"email": email, "password": "portal-passphrase-1"})),
    )
    .await;
    assert_eq!(re_login.status(), 200);
    let re_login_cookies = set_cookie_values(&re_login);
    // Body credentials with NO Cookie header: cookie_authed is false, so the
    // double-submit check passes by construction (bearer callers carry no
    // ambient credential a forged request could ride).
    let pair = body_json(re_login).await;
    let bearer_rotated = send_with_headers(
        &app,
        axum::http::Method::POST,
        "/api/portal/refresh",
        &[],
        Some(json!({
            "refresh_token": pair["refresh_token"].as_str().unwrap(),
            "session_id": pair["session_id"].as_str().unwrap(),
        })),
    )
    .await;
    assert_eq!(
        bearer_rotated.status(),
        200,
        "the bearer refresh path carries no ambient credential and never checks the header"
    );
    // Sanity: the same body, but with an ambient portal cookie and no header,
    // is now refused — the gate keys on the jar, not on the body.
    let mixed = send_with_headers(
        &app,
        axum::http::Method::POST,
        "/api/portal/refresh",
        &[(
            "cookie",
            &format!(
                "marula_portal={}",
                cookie_value(&re_login_cookies, "marula_portal")
            ),
        )],
        Some(json!({
            "refresh_token": pair["refresh_token"].as_str().unwrap(),
            "session_id": pair["session_id"].as_str().unwrap(),
        })),
    )
    .await;
    assert_eq!(
        mixed.status(),
        403,
        "body credentials do not buy an exemption once the jar is ambient"
    );
}

#[tokio::test]
async fn portal_admin_can_disable_and_delete_portal_users() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let tag = Uuid::now_v7().simple().to_string()[..8].to_string();
    let org = seed_org(&app, &tag).await;
    let email = format!("gate-{tag}@a.test");

    let user = invite_portal_user(&app, &org.token, &org.client_a, &email, "approver").await;
    // A first login so the delete step can prove outstanding tokens die.
    let login = portal_login(&app, &email).await;
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
        Some(json!({"email": email, "password": "portal-passphrase-1"})),
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
        Some(json!({"email": email, "password": "brand-new-passphrase"})),
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

// --- portal magic-link sign-in (Wave 41 lane 13) ------------------------------
//
// The link is the client's primary way in, so its guarantees are pinned here
// rather than left to inspection:
//
//   1. the REQUEST half never reveals whether an address is a client contact
//      (one answer for a real contact, an invited contact, an unknown address
//      and a disabled contact alike);
//   2. redemption is SINGLE-USE — a replay is the same 401 as a forgery;
//   3. a link minted before a contact was DISABLED does not work, because the
//      claim itself carries the standing check;
//   4. an EXPIRED link does not work;
//   5. a redeemed link issues PORTAL tokens, and they are the only kind: the
//      bearer must pass `require_portal_auth` and must fail the staff guard.

/// Seeds a live portal link with a known raw token, so redemption can be driven
/// without reading the mail ledger. The server stores only sha256(token).
async fn seed_portal_link(
    pool: &sqlx::PgPool,
    org_id: Uuid,
    portal_user_id: Uuid,
    kind: &str,
    expires: &str,
) -> String {
    let raw: String = format!(
        "{:064x}",
        (Uuid::now_v7().as_u128() << 64) | Uuid::now_v7().as_u128()
    );
    use sha2::{Digest, Sha256};
    let hash = hex::encode(Sha256::digest(raw.as_bytes()));
    // `expires` is a SQL EXPRESSION this file's own call sites write
    // (`clock_timestamp() + interval '10 minutes'`), not a value:
    // `expires_at` is timestamptz, so binding the text fails with a
    // type error. Interpolating it is the only way the helper can mint a
    // link that is already past, and it never touches anything
    // user-supplied.
    sqlx::query(&format!(
        "INSERT INTO portal_auth_one_use_tokens \
           (id, org_id, portal_user_id, kind, token_hash, expires_at) \
         VALUES ($1, $2, $3, $4, $5, {expires})",
    ))
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(portal_user_id)
    .bind(kind)
    .bind(&hash)
    .execute(pool)
    .await
    .expect("portal link row");
    raw
}

#[tokio::test]
async fn portal_magic_link_is_single_use_portal_scoped_and_status_aware() {
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&admin_url)
        .await
        .expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");

    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let tag = Uuid::now_v7().simple().to_string()[..8].to_string();
    let org = seed_org(&app, &tag).await;
    let link_email = format!("link-{tag}@a.test");
    let contact = invite_portal_user(&app, &org.token, &org.client_a, &link_email, "contact").await;
    // The staff invite answers with the portal user itself (`body =
    // PortalUser` in the route's OpenAPI), not the login-shaped envelope
    // that carries `access_token` beside `portal_user`. The delete test
    // below reads the same response the same way.
    let portal_user_id: Uuid = Uuid::parse_str(contact["id"].as_str().unwrap()).expect("uuid");
    // The org comes off the portal user row rather than a shared helper, so
    // this test needs no change to `seed_org`.
    let org_id: Uuid = sqlx::query_scalar("SELECT org_id FROM portal_users WHERE id = $1")
        .bind(portal_user_id)
        .fetch_one(&pool)
        .await
        .expect("org for the portal user");

    // (1) ENUMERATION. Every address gets the same answer, whether or not it
    //     belongs to a client of this service desk.
    for address in [link_email.as_str(), "nobody@nowhere.test", "malformed"] {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/portal/magic-link/request",
            None,
            Some(json!({ "email": address })),
        )
        .await;
        assert_eq!(
            response.status(),
            200,
            "request must answer 200 for {address} — the answer cannot depend on the address"
        );
        assert_eq!(body_json(response).await["ok"], json!(true));
    }
    // A DISABLED contact is not a candidate for a link at all, and says so the
    // same way.
    sqlx::query("UPDATE portal_users SET status = 'disabled' WHERE id = $1")
        .bind(portal_user_id)
        .execute(&pool)
        .await
        .expect("disable");
    // The enumeration pass above already minted this identity's one link
    // while it was still `invited`, so the invariant is a DELTA: asking
    // again must not add a link for an account that is now disabled. A
    // mint also spends whatever is outstanding, so the earlier link staying
    // unused is the second half of the proof.
    let links_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM portal_auth_one_use_tokens \
         WHERE portal_user_id = $1 AND kind = 'portal_magic_link'",
    )
    .bind(portal_user_id)
    .fetch_one(&pool)
    .await
    .expect("count before disable");
    let outstanding_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM portal_auth_one_use_tokens \
         WHERE portal_user_id = $1 AND kind = 'portal_magic_link' AND used_at IS NULL",
    )
    .bind(portal_user_id)
    .fetch_one(&pool)
    .await
    .expect("outstanding before disable");
    assert_eq!(
        outstanding_before, 1,
        "the enumeration pass minted this identity one live link"
    );
    let disabled = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/magic-link/request",
        None,
        Some(json!({ "email": link_email })),
    )
    .await;
    assert_eq!(disabled.status(), 200);
    let links_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM portal_auth_one_use_tokens \
         WHERE portal_user_id = $1 AND kind = 'portal_magic_link'",
    )
    .bind(portal_user_id)
    .fetch_one(&pool)
    .await
    .expect("count");
    let outstanding_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM portal_auth_one_use_tokens \
         WHERE portal_user_id = $1 AND kind = 'portal_magic_link' AND used_at IS NULL",
    )
    .bind(portal_user_id)
    .fetch_one(&pool)
    .await
    .expect("outstanding after disable");
    assert_eq!(
        links_after, links_before,
        "a disabled contact must never be sent a sign-in link"
    );
    assert_eq!(
        outstanding_after, outstanding_before,
        "the request must not even spend the link the contact already had"
    );
    sqlx::query("UPDATE portal_users SET status = 'invited' WHERE id = $1")
        .bind(portal_user_id)
        .execute(&pool)
        .await
        .expect("re-invite");

    // A link minted BEFORE the disable must not work afterwards. This is the
    // claim's own status check, not a pre-read, so there is no window.
    let pre_disable = seed_portal_link(
        &pool,
        org_id,
        portal_user_id,
        "portal_magic_link",
        "clock_timestamp() + interval '10 minutes'",
    )
    .await;
    sqlx::query("UPDATE portal_users SET status = 'disabled' WHERE id = $1")
        .bind(portal_user_id)
        .execute(&pool)
        .await
        .expect("disable again");
    let refused = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/magic-link/confirm",
        None,
        Some(json!({ "token": pre_disable })),
    )
    .await;
    assert_eq!(
        refused.status(),
        axum::http::StatusCode::UNAUTHORIZED,
        "a link minted before the account was disabled must not sign in"
    );
    sqlx::query("UPDATE portal_users SET status = 'invited' WHERE id = $1")
        .bind(portal_user_id)
        .execute(&pool)
        .await
        .expect("re-invite again");

    // (4) An EXPIRED link is refused.
    let expired = seed_portal_link(
        &pool,
        org_id,
        portal_user_id,
        "portal_magic_link",
        "clock_timestamp() - interval '1 minute'",
    )
    .await;
    let stale = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/magic-link/confirm",
        None,
        Some(json!({ "token": expired })),
    )
    .await;
    assert_eq!(stale.status(), axum::http::StatusCode::UNAUTHORIZED);

    // (2) + (5) Redemption works once and issues PORTAL tokens.
    let live = seed_portal_link(
        &pool,
        org_id,
        portal_user_id,
        "portal_magic_link",
        "clock_timestamp() + interval '10 minutes'",
    )
    .await;
    let first = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/magic-link/confirm",
        None,
        Some(json!({ "token": live })),
    )
    .await;
    assert_eq!(first.status(), axum::http::StatusCode::OK);
    let body = body_json(first).await;
    let access = body["access_token"]
        .as_str()
        .expect("access token")
        .to_string();
    assert_eq!(
        body["portal_user"]["email"],
        json!(link_email),
        "the link resolves to the identity it was minted for"
    );
    // First sign-in through a link activates an invited contact, exactly as
    // the password path does.
    assert_eq!(body["portal_user"]["id"], json!(portal_user_id.to_string()));

    // The minted token is PORTAL family: it passes the portal guard…
    let me = send(
        &app,
        axum::http::Method::GET,
        "/api/portal/me",
        Some(&access),
        None,
    )
    .await;
    assert_eq!(me.status(), 200, "a portal link must mint a portal session");
    // …and fails the staff guard, so the two families stay crossed out.
    let staff = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/tickets",
        Some(&access),
        None,
    )
    .await;
    assert_eq!(
        staff.status(),
        401,
        "a portal token must never pass the staff guard"
    );

    // REPLAY: the same link again is the same 401 as a forgery.
    let replay = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/magic-link/confirm",
        None,
        Some(json!({ "token": live })),
    )
    .await;
    assert_eq!(replay.status(), axum::http::StatusCode::UNAUTHORIZED);

    // A FORGED token (never stored) is indistinguishable from a used one.
    let forged = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/magic-link/confirm",
        None,
        Some(json!({ "token": format!("{:064x}", Uuid::now_v7().as_u128()) })),
    )
    .await;
    assert_eq!(forged.status(), axum::http::StatusCode::UNAUTHORIZED);
}
