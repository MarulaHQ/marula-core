//! Restricted-category owner/admin grant seeding — HTTP e2e (Wave 16
//! lane 10, ADR 0006 directive plus migration 0083).
//!
//! Purpose: the new-org default at the ROUTE boundary — the owner/admin
//! pair is never locked out of their own restricted vault chains while
//! the deny-by-default posture holds for members:
//!   * owner creates a restricted category → WITHOUT touching the grants
//!     surface the org Admin can read (and edit) records in that chain,
//!     and the Technician is still denied (no grant, deny-by-default);
//!   * the runtime seeding rides the SAME transaction as the folder write
//!     (no self-lockout window mid-call);
//!   * role grants are still manageable: the seeded rows are the plain
//!     `secret_category_grants` data — deleting the admin grant re-locks
//!     the admin out until it is re-upserted (posture back, delibe-
//!     rately manageable).
//!
//! Non-purpose: store-level determinism of the backfill (the core-db
//! suite `restricted_grant_seeding` proves the migration idempotence and
//! the runtime/store twins).

use crate::common::{body_json, boot_with, register_org as common_register_org, send};
use axum::Router;
use core_api::state::AppState;
use serde_json::json;
use uuid::Uuid;

async fn test_state() -> (AppState, Router) {
    let (state, app) = boot_with(|_| {}).await;
    core_db::assert_rls_coverage(&state.pool)
        .await
        .expect("RLS coverage");
    (state, app)
}

async fn register_org(app: &Router, slug: String) -> String {
    common_register_org(app, "Grant Co", &slug).await["access_token"]
        .as_str()
        .expect("access token")
        .into()
}

async fn invite_and_activate(app: &Router, owner_token: &str, email: &str, role: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/org/users",
        Some(owner_token),
        Some(json!({
            "email": email,
            "display_name": role,
            "role": role,
            "idempotency_key": format!("invite-{}", Uuid::now_v7().simple()),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "invite must succeed");
    let body = body_json(response).await;
    let setup_token = body["invite"]["setup_token"].as_str().expect("setup token");
    let accept = send(
        app,
        axum::http::Method::POST,
        "/api/org/users/accept-invite",
        None,
        Some(json!({ "token": setup_token, "password": "another-correct-horse" })),
    )
    .await;
    assert_eq!(accept.status(), 200, "redemption must succeed");
    let tokens = body_json(accept).await;
    tokens["access_token"].as_str().expect("token").into()
}

#[tokio::test]
async fn new_org_restricted_chain_serves_admin_and_denies_technician() {
    let (_state, router) = test_state().await;
    let slug = format!("grant-flow-{}", Uuid::now_v7().simple());
    let owner = register_org(&router, slug.clone()).await;
    let admin = invite_and_activate(&router, &owner, &format!("admin@{slug}.test"), "admin").await;
    let technician =
        invite_and_activate(&router, &owner, &format!("tech@{slug}.test"), "technician").await;

    // Owner creates a RESTRICTED category in one call — the seeding rides
    // the same transaction (no moment where the folder exists ungranted).
    let created = send(
        &router,
        axum::http::Method::POST,
        "/api/secrets/folders",
        Some(&owner),
        Some(json!({
            "slug": "client-master-keys",
            "label": "Client master keys",
            "kind": "api_keys",
            "restricted": true,
        })),
    )
    .await;
    let status = created.status();
    let body = body_json(created).await;
    assert_eq!(status, 200, "folder create must succeed: {body}");
    let folder_id = body["id"].as_str().unwrap().to_string();

    // Put one record into the restricted chain.
    let record = send(
        &router,
        axum::http::Method::POST,
        "/api/secrets/records",
        Some(&owner),
        Some(json!({
            "folder_id": folder_id,
            "kind": "api_key",
            "title": "Vault master key",
            "fields": [{"field_key": "secret", "value": "s3cr3t"}]
        })),
    )
    .await;
    let record_body = body_json(record).await;
    assert!(
        record_body["id"].as_str().is_some(),
        "record create must succeed inside the fresh restricted chain: {record_body}"
    );
    let record_id = record_body["id"].as_str().expect("record id").to_string();

    // TECHNICIAN: deny-by-default — the record metadata is invisible.
    let tech_read = send(
        &router,
        axum::http::Method::GET,
        &format!("/api/secrets/records/{record_id}"),
        Some(&technician),
        None,
    )
    .await;
    assert_ne!(
        tech_read.status(),
        200,
        "technician must NOT read a restricted-chain record"
    );

    // ADMIN: the seeded owner/admin grants admit read AND edit.
    let admin_read = send(
        &router,
        axum::http::Method::GET,
        &format!("/api/secrets/records/{record_id}"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(
        admin_read.status(),
        200,
        "admin must read the restricted chain via the runtime-seeded grants"
    );

    // ADMIN edit lands on the restricted chain (can_edit seeded).
    let admin_edit = send(
        &router,
        axum::http::Method::PATCH,
        &format!("/api/secrets/records/{record_id}"),
        Some(&admin),
        Some(json!({ "title": "Vault master key (renamed)" })),
    )
    .await;
    let edit_status = admin_edit.status();
    assert_eq!(
        edit_status,
        axum::http::StatusCode::OK,
        "admin must edit the restricted chain via the runtime-seeded grants"
    );
}

/// The FOLDER-level category gate. A Technician holds the full Secrets matrix
/// (View/Create/Edit/Delete) but receives no restricted-category grant, because
/// the seeding hands those to owner/admin only. Before the fix, the folder
/// object itself was not gated, so the technician could PATCH the restricted
/// chain to `restricted: false` and then read the records the gate exists to
/// protect. The gate must deny the technician and still admit the granted
/// admin.
#[tokio::test]
async fn a_role_without_the_category_grant_cannot_unrestrict_a_restricted_chain() {
    let (_state, router) = test_state().await;
    let slug = format!("grant-unrestrict-{}", Uuid::now_v7().simple());
    let owner = register_org(&router, slug.clone()).await;
    let admin = invite_and_activate(&router, &owner, &format!("admin@{slug}.test"), "admin").await;
    let technician =
        invite_and_activate(&router, &owner, &format!("tech@{slug}.test"), "technician").await;

    let created = send(
        &router,
        axum::http::Method::POST,
        "/api/secrets/folders",
        Some(&owner),
        Some(json!({
            "slug": "restricted-chain",
            "label": "Restricted chain",
            "kind": "api_keys",
            "restricted": true,
        })),
    )
    .await;
    let body = body_json(created).await;
    let folder_id = body["id"]
        .as_str()
        .expect("restricted folder create returns an id")
        .to_string();

    let record = send(
        &router,
        axum::http::Method::POST,
        "/api/secrets/records",
        Some(&owner),
        Some(json!({
            "folder_id": folder_id,
            "kind": "api_key",
            "title": "Chain secret",
            "fields": [{"field_key": "secret", "value": "s3cr3t"}]
        })),
    )
    .await;
    let record_body = body_json(record).await;
    let record_id = record_body["id"]
        .as_str()
        .expect("record create returns an id")
        .to_string();

    // The technician clears the restricted flag: refused, because the folder
    // object carries the same category gate as the records inside it.
    let denied = send(
        &router,
        axum::http::Method::PATCH,
        &format!("/api/secrets/folders/{folder_id}"),
        Some(&technician),
        Some(json!({ "restricted": false })),
    )
    .await;
    assert_eq!(
        denied.status(),
        axum::http::StatusCode::FORBIDDEN,
        "a role without the category grant must not clear the restricted flag"
    );

    // The flag survived the attempt, so the record is still closed to it.
    let tech_read = send(
        &router,
        axum::http::Method::GET,
        &format!("/api/secrets/records/{record_id}"),
        Some(&technician),
        None,
    )
    .await;
    assert_ne!(
        tech_read.status(),
        200,
        "the chain must still deny the technician after the refused edit"
    );

    // The seeded admin grant still admits the same edit: the gate narrows to
    // the category, it does not close the folder to everyone.
    let allowed = send(
        &router,
        axum::http::Method::PATCH,
        &format!("/api/secrets/folders/{folder_id}"),
        Some(&admin),
        Some(json!({ "label": "Restricted chain (renamed)" })),
    )
    .await;
    assert_eq!(
        allowed.status(),
        axum::http::StatusCode::OK,
        "the runtime-seeded admin grant still allows a restricted-folder edit"
    );
}
