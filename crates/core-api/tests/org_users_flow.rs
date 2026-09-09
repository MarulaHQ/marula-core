//! End-to-end org user management flow through the REAL core-api router
//! (Wave 11 lane 2).
//!
//! Purpose: manager+ directory gating, the full invite → accept-invite →
//! login lifecycle (user starts inactive, activation via one-time setup
//! token, token is single-use and unknown/used/expired are 404),
//! idempotency replay on invite, activate/deactivate with login blocking,
//! org-role assignment with the last-active-owner guard, and instant-first
//! envelope metadata on every mutation.
//!
//! Non-purpose: SQL-layer behavior in isolation, the web UI.

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
        .expect("org-users e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env();
    config.database_url = admin_url.clone();
    let reporting_url = admin_url;
    AppState {
        pool,
        config: Arc::new(config),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&Config::from_env())),
        store: Arc::new(core_store::memory::MemoryStore::new()),
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
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap()
}

/// Registers a fresh org; returns the owner's access token.
async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Org Users Flow Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let body = body_json(response).await;
    body["access_token"].as_str().expect("access token").into()
}

/// Invites one user; returns (user_id, setup_token, mutation envelope).
async fn invite_user(
    app: &Router,
    owner_token: &str,
    email: &str,
    display_name: &str,
    role: &str,
    idem_key: Option<String>,
) -> (String, String, Value) {
    let mut payload = json!({
        "email": email,
        "display_name": display_name,
        "role": role,
    });
    if let Some(key) = idem_key {
        payload["idempotency_key"] = json!(key);
    }
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/org/users",
        Some(owner_token),
        Some(payload),
    )
    .await;
    assert_eq!(response.status(), 200, "invite must succeed");
    let body = body_json(response).await;
    let user_id = body["id"].as_str().expect("invited user id").into();
    let setup_token = body["invite"]["setup_token"]
        .as_str()
        .expect("setup token")
        .into();
    assert!(
        body["mutation"]["version"].is_i64(),
        "invite must carry mutation metadata: {body}"
    );
    (user_id, setup_token, body)
}

#[tokio::test]
async fn org_users_full_lifecycle_flow() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("org-users-flow-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;

    // --- directory lists the owner, manager+ gate on reads -------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/org/users",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let directory = body_json(response).await;
    assert_eq!(directory["total"], 1);
    assert_eq!(directory["users"][0]["org_role"], "owner");

    // --- invite: idempotent create with a one-time setup token ---------------
    let email = format!("dana-{}@acme.test", Uuid::now_v7().simple());
    let key = format!("invite-{}", Uuid::now_v7().simple());
    let (dana_id, setup_token, first) = invite_user(
        &app,
        &owner,
        &email,
        "dana",
        "technician",
        Some(key.clone()),
    )
    .await;
    assert_eq!(first["is_active"], false, "invited user starts inactive");

    // Same key + same payload ⇒ byte-identical replay (same token).
    let replay = send(
        &app,
        axum::http::Method::POST,
        "/api/org/users",
        Some(&owner),
        Some(json!({
            "email": email,
            "display_name": "dana",
            "role": "technician",
            "idempotency_key": key,
        })),
    )
    .await;
    assert_eq!(replay.status(), 200);
    let replay_body = body_json(replay).await;
    assert_eq!(
        replay_body["invite"]["setup_token"], setup_token,
        "replay must return the SAME stored response"
    );

    // Duplicate email (any state) ⇒ typed 409.
    let duplicate = send(
        &app,
        axum::http::Method::POST,
        "/api/org/users",
        Some(&owner),
        Some(json!({ "email": email, "display_name": "dupe", "role": "technician" })),
    )
    .await;
    assert_eq!(duplicate.status(), 409);

    // Invalid role ⇒ 422.
    let bad_role = send(
        &app,
        axum::http::Method::POST,
        "/api/org/users",
        Some(&owner),
        Some(json!({ "email": format!("x-{}@acme.test", Uuid::now_v7().simple()), "display_name": "x", "role": "superadmin" })),
    )
    .await;
    assert_eq!(bad_role.status(), 422);

    // --- redemption: wrong token 404s; the real one activates + logs in ------
    let bad = send(
        &app,
        axum::http::Method::POST,
        "/api/org/users/accept-invite",
        None,
        Some(json!({ "token": "not-a-real-token", "password": "another-correct-horse" })),
    )
    .await;
    assert_eq!(bad.status(), 404, "unknown token must be indistinguishable");

    let accept = send(
        &app,
        axum::http::Method::POST,
        "/api/org/users/accept-invite",
        None,
        Some(json!({ "token": setup_token, "password": "another-correct-horse" })),
    )
    .await;
    assert_eq!(accept.status(), 200, "redemption must succeed");
    let tokens = body_json(accept).await;
    let dana = tokens["access_token"].as_str().expect("dana access token");

    // The token is single-use: second redemption 404s.
    let reuse = send(
        &app,
        axum::http::Method::POST,
        "/api/org/users/accept-invite",
        None,
        Some(json!({ "token": setup_token, "password": "another-correct-horse" })),
    )
    .await;
    assert_eq!(reuse.status(), 404, "used token must be indistinguishable");

    // Dana can now log in with her password.
    let login = send(
        &app,
        axum::http::Method::POST,
        "/api/auth/login",
        None,
        Some(json!({ "email": email, "password": "another-correct-horse" })),
    )
    .await;
    assert_eq!(login.status(), 200, "activated user must be able to log in");

    // --- directory gate: technician is NOT manager+, gets 403 ----------------
    let forbidden = send(
        &app,
        axum::http::Method::GET,
        "/api/org/users",
        Some(dana),
        None,
    )
    .await;
    assert_eq!(forbidden.status(), 403);

    // Mutations are strictly owner/admin: technician gets 403.
    let forbidden_write = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/org/users/{dana_id}/role"),
        Some(dana),
        Some(json!({ "role": "admin" })),
    )
    .await;
    assert_eq!(forbidden_write.status(), 403);

    // --- org-role assignment --------------------------------------------------
    let role_change = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/org/users/{dana_id}/role"),
        Some(&owner),
        Some(json!({ "role": "dispatcher" })),
    )
    .await;
    assert_eq!(role_change.status(), 200);
    let changed = body_json(role_change).await;
    assert_eq!(changed["org_role"], "dispatcher");
    assert!(
        changed["mutation"]["version"].is_i64(),
        "role change must carry mutation metadata"
    );

    // --- deactivate: sessions revoked, login blocked, never deleted ----------
    let deactivate = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/org/users/{dana_id}/status"),
        Some(&owner),
        Some(json!({ "active": false })),
    )
    .await;
    assert_eq!(deactivate.status(), 200);
    let deactivated = body_json(deactivate).await;
    assert_eq!(deactivated["is_active"], false);

    // Dana's existing access token still decodes (JWT), but login is blocked.
    let blocked_login = send(
        &app,
        axum::http::Method::POST,
        "/api/auth/login",
        None,
        Some(json!({ "email": email, "password": "another-correct-horse" })),
    )
    .await;
    assert_eq!(
        blocked_login.status(),
        401,
        "deactivated user cannot log in"
    );

    // Re-activate.
    let reactivate = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/org/users/{dana_id}/status"),
        Some(&owner),
        Some(json!({ "active": true })),
    )
    .await;
    assert_eq!(reactivate.status(), 200);

    // --- guards: cannot deactivate self; cannot demote the last owner --------
    let self_deactivate = send(
        &app,
        axum::http::Method::PATCH,
        &format!(
            "/api/org/users/{}/status",
            directory["users"][0]["id"].as_str().unwrap()
        ),
        Some(&owner),
        Some(json!({ "active": false })),
    )
    .await;
    assert_eq!(
        self_deactivate.status(),
        422,
        "self-deactivation is refused"
    );

    let last_owner = send(
        &app,
        axum::http::Method::PATCH,
        &format!(
            "/api/org/users/{}/role",
            directory["users"][0]["id"].as_str().unwrap()
        ),
        Some(&owner),
        Some(json!({ "role": "dispatcher" })),
    )
    .await;
    assert_eq!(
        last_owner.status(),
        409,
        "demoting the last active owner is a typed 409"
    );

    // --- directory reflects the full lifecycle --------------------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/org/users",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let directory = body_json(response).await;
    assert_eq!(directory["total"], 2);
    let dana_row = directory["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == json!(dana_id))
        .expect("dana in directory");
    assert_eq!(dana_row["org_role"], "dispatcher");
    assert_eq!(dana_row["is_active"], true);
}
