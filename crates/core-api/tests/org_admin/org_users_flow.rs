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

use crate::common::{body_json, register_org_token, send, test_state};
use axum::Router;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

async fn register_org(app: &Router, slug: &str) -> String {
    register_org_token(app, "Org Users Flow Co", slug).await
}

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

/// Two owners, two removals at the same moment, one survivor (Wave 54 lane
/// S4a).
///
/// The guard is a read (how many active owners?) followed by a write, so two
/// requests that both read "two owners" would both deactivate and leave the org
/// with no owner at all. The fix holds one lock across the read, the count and
/// the write. This test takes that lock first, so both requests are provably
/// queued before either can decide — a plain race would only be as reliable as
/// wall-clock luck, and on the unfixed tree both requests would sail straight
/// through the lock the probe holds.
#[tokio::test]
async fn concurrent_last_owner_removals_leave_exactly_one_owner() {
    let state = test_state().await;
    let app = core_api::router(state.clone());
    let slug = format!("owner-race-{}", Uuid::now_v7().simple());
    let first = register_org(&app, &slug).await;
    let me =
        body_json(send(&app, axum::http::Method::GET, "/api/me", Some(&first), None).await).await;
    let org_id = me["org_id"].as_str().expect("org id").to_string();
    let first_id = me["user_id"].as_str().expect("owner id").to_string();

    // A second OWNER: invited as one, redeemed, able to authenticate.
    let email = format!("second-{}@acme.test", Uuid::now_v7().simple());
    let (second_id, setup_token, _) =
        invite_user(&app, &first, &email, "Second Owner", "owner", None).await;
    let accepted = send(
        &app,
        axum::http::Method::POST,
        "/api/org/users/accept-invite",
        None,
        Some(json!({ "token": setup_token, "password": "another-correct-horse" })),
    )
    .await;
    assert_eq!(
        accepted.status(),
        200,
        "the second owner redeems the invite"
    );
    let second = body_json(accepted).await["access_token"]
        .as_str()
        .expect("second owner token")
        .to_string();

    // Hold the org's owner-guard key so both requests must queue on it.
    let mut probe = state.pool.begin().await.expect("probe transaction");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 42))")
        .bind(format!("org.owner_guard:{org_id}"))
        .execute(&mut *probe)
        .await
        .expect("hold the owner guard");

    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let mut calls = Vec::new();
    for (actor, target) in [
        (first.clone(), second_id.clone()),
        (second, first_id.clone()),
    ] {
        let router = app.clone();
        let barrier = Arc::clone(&barrier);
        calls.push(tokio::spawn(async move {
            barrier.wait().await;
            let response = send(
                &router,
                axum::http::Method::PATCH,
                &format!("/api/org/users/{target}/status"),
                Some(&actor),
                Some(json!({ "active": false })),
            )
            .await;
            (response.status(), body_json(response).await)
        }));
    }

    // Release once both are blocked on it. Bounded on purpose: a tree
    // without the lock never blocks, and the assertions below catch that
    // rather than the wait hanging.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND NOT granted",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);
        if waiting >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    probe.rollback().await.expect("release the owner guard");

    let mut outcomes = Vec::new();
    for call in calls {
        outcomes.push(call.await.expect("joined the removal"));
    }
    outcomes.sort_by_key(|(status, _)| status.as_u16());
    assert_eq!(outcomes[0].0, 200, "exactly one removal wins: {outcomes:?}");
    assert_eq!(
        outcomes[1].0, 409,
        "the other is a typed refusal, not a 500 and not a second success: {outcomes:?}"
    );

    let owners: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM users u \
           JOIN user_roles ur ON ur.user_id = u.id \
           JOIN roles r ON r.id = ur.role_id \
          WHERE u.org_id = $1 AND u.is_active AND r.builtin = 'owner'",
    )
    .bind(Uuid::parse_str(&org_id).expect("org uuid"))
    .fetch_one(&state.pool)
    .await
    .expect("owner count");
    assert_eq!(owners, 1, "the org keeps exactly one active owner");
}
