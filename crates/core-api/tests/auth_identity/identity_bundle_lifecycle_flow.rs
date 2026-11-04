//! Identity-bundle lifecycle through the REAL core-api router (Wave 62 lane
//! `p62-identity-lifecycle`).
//!
//! Purpose: `auth::identity_bundle_service` was written, audited and tested in
//! Wave 52 and no route called it, so an approved bundle could be installed and
//! never switched on except by SQL. This suite pins the surface that fixes that
//! — and the three decisions that travel with it:
//!
//! 1. **The lifecycle is reachable.** List, enable, disable and revoke answer on
//!    `/api/plugins/identity-bundles`, the transitions land on the row, and
//!    revoke is terminal (the second enable is a typed 409).
//! 2. **The credential is the install flow's last step.** Minting returns raw
//!    key material once, carrying `directory_groups:read` and
//!    `directory_groups:write`; the key then lists and writes groups through
//!    the same gates every other machine credential meets. A revoked bundle
//!    mints nothing.
//! 3. **A machine write reads as a machine write.** A group created with that
//!    key lands in the audit trail as `connector` with `api-key:<key-id>`, not
//!    as a human who happens to share an id with a credential.
//!
//! The listing cursor is pinned here too: two pages of three groups, in
//! `(name, id)` order, with a resume that neither repeats nor skips.
//!
//! Non-purpose: the staging and approval path (core-db
//! `plugin_distribution_queries.rs` owns the row shapes), the state machine
//! itself (unit tests in `core-domain`), and the grant ceiling
//! (`directory_groups_flow.rs` owns it).
//!
//! DB-backed: it needs a migrated database (`MARULA_TEST_DATABASE_URL` or
//! `DATABASE_URL`), the same requirement every suite in this directory carries.
//! The install row is seeded directly because staging an artifact needs bytes
//! from a source this suite would otherwise have to serve; approval is not what
//! this lane changed.

use crate::common::{
    body_json, register_org_token, send, send_with_api_key, test_state, unique_slug,
};
use axum::Router;
use core_api::state::AppState;
use serde_json::{json, Value};
use uuid::Uuid;

/// The digest a seed row carries. The column is CHECKed as sha256 hex, so a
/// short placeholder would refuse the insert.
fn digest() -> String {
    "a".repeat(64)
}

/// Seeds an APPROVED install row the way the approval path mints one: DISABLED,
/// JIT off, digest pinned, installed a few minutes ago so the database's
/// "enable is an event" check (`updated_at > installed_at`) can be satisfied by
/// the route's own clock.
async fn seed_install(state: &AppState, org_id: Uuid, slug: &str) -> Uuid {
    let id = Uuid::now_v7();
    let manifest = json!({
        "name": slug,
        "version": "1.0.0",
        "interface_version": 1,
        "provider_kinds": ["oidc"],
        "settings_schema": { "type": "object", "properties": {} },
        "scopes": ["openid", "email"]
    });
    let mut tx = state.pool.begin().await.expect("seed tx");
    sqlx::query("SELECT set_config('app.org_id', $1, true)")
        .bind(org_id.to_string())
        .execute(&mut *tx)
        .await
        .expect("org scope");
    sqlx::query(
        "INSERT INTO identity_bundle_installs (
             id, org_id, slug, name, version, interface_version, provider_kinds, scopes,
             manifest, provenance_source_kind, provenance_source_url, provenance_fetched_ref,
             provenance_digest, status, jit_enabled, revoked_reason, installed_by,
             installed_at, updated_at)
         VALUES (
             $1, $2, $3, $3, '1.0.0', 1, ARRAY['oidc'], ARRAY['openid', 'email'],
             $4, 'manual_upload'::text::plugin_source_kind, 'local://suite', NULL,
             $5, 'disabled'::text::plugin_install_status, false, NULL, $6,
             now() - interval '5 minutes', now() - interval '5 minutes')",
    )
    .bind(id)
    .bind(org_id)
    .bind(slug)
    .bind(&manifest)
    .bind(digest())
    .bind(Uuid::now_v7())
    .execute(&mut *tx)
    .await
    .expect("seed install");
    tx.commit().await.expect("seed commit");
    id
}

/// The org id behind an owner token, from the JWT's own answer (`/api/me`).
async fn org_id_of(app: &Router, token: &str) -> Uuid {
    let response = send(app, axum::http::Method::GET, "/api/me", Some(token), None).await;
    assert_eq!(response.status(), 200, "me must answer");
    body_json(response).await["org_id"]
        .as_str()
        .expect("org id")
        .parse()
        .expect("uuid")
}

async fn call(
    app: &Router,
    method: axum::http::Method,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (axum::http::StatusCode, Value) {
    let response = send(app, method, uri, Some(token), body).await;
    let status = response.status();
    (status, body_json(response).await)
}

#[tokio::test]
async fn a_bundle_is_listed_enabled_disabled_and_revoked_through_the_routes() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = unique_slug("bundle-lifecycle");
    let owner = register_org_token(&app, "Bundle Lifecycle Co", &slug).await;
    let org_id = org_id_of(&app, &owner).await;
    let bundle_slug = format!("corp-okta-{}", Uuid::now_v7().simple());
    let id = seed_install(&state, org_id, &bundle_slug).await;

    // The list is how an operator learns the id. It is addressed by id, so
    // without this read the lifecycle would still be SQL-only in practice.
    let (status, list) = call(
        &app,
        axum::http::Method::GET,
        "/api/plugins/identity-bundles",
        &owner,
        None,
    )
    .await;
    assert_eq!(status, 200, "the install list must answer: {list}");
    let listed = list
        .as_array()
        .expect("install list is an array")
        .iter()
        .find(|row| row["id"] == json!(id))
        .unwrap_or_else(|| panic!("the seeded install must be listed: {list}"));
    assert_eq!(listed["status"], "disabled", "approval adopts disabled");

    let (status, enabled) = call(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/identity-bundles/{id}/enable"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, 200, "enable must be reachable: {enabled}");
    assert_eq!(enabled["status"], "enabled", "{enabled}");
    assert_eq!(enabled["slug"], json!(bundle_slug));

    let (status, disabled) = call(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/identity-bundles/{id}/disable"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, 200, "disable must be reachable: {disabled}");
    assert_eq!(disabled["status"], "disabled", "{disabled}");

    let (status, refused) = call(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/identity-bundles/{id}/revoke"),
        &owner,
        Some(json!({ "reason": " " })),
    )
    .await;
    assert_eq!(
        status, 422,
        "a blank revocation reason is refused: {refused}"
    );

    let (status, revoked) = call(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/identity-bundles/{id}/revoke"),
        &owner,
        Some(json!({ "reason": "directory decommissioned" })),
    )
    .await;
    assert_eq!(status, 200, "revoke must be reachable: {revoked}");
    assert_eq!(revoked["status"], "revoked", "{revoked}");
    assert_eq!(revoked["revoked_reason"], "directory decommissioned");
    assert_eq!(
        revoked["provenance_digest"],
        json!(digest()),
        "the pin survives the lifecycle"
    );

    // Terminal, not a toggle: the domain refuses a revive and the route answers
    // the typed conflict rather than a silent 200.
    let (status, again) = call(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/identity-bundles/{id}/enable"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, 409, "a revoked bundle cannot be enabled: {again}");
    assert_eq!(again["error"]["reason"], "conflict", "{again}");
}

#[tokio::test]
async fn the_credential_minted_for_a_bundle_mirrors_the_directory() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = unique_slug("bundle-credential");
    let owner = register_org_token(&app, "Bundle Credential Co", &slug).await;
    let org_id = org_id_of(&app, &owner).await;
    let id = seed_install(
        &state,
        org_id,
        &format!("corp-scim-{}", Uuid::now_v7().simple()),
    )
    .await;

    let (status, created) = call(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/identity-bundles/{id}/credential"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, 201, "the mint must answer 201: {created}");
    let raw_key = created["raw_key"]
        .as_str()
        .expect("raw key material is returned once")
        .to_string();
    assert!(
        raw_key.starts_with("mrl_"),
        "the mint uses the key surface's own material: {raw_key}"
    );
    let key_id: Uuid = created["key"]["id"]
        .as_str()
        .expect("key id")
        .parse()
        .expect("uuid");
    let scopes: Vec<&str> = created["key"]["scopes"]
        .as_array()
        .expect("scopes")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        scopes,
        vec!["directory_groups:read", "directory_groups:write"],
        "both directory scopes, and nothing else"
    );

    // The read half: the connector lists the groups with its own key.
    let listed = send_with_api_key(
        &app,
        axum::http::Method::GET,
        "/api/directory-groups",
        &raw_key,
        None,
    )
    .await;
    assert_eq!(listed.status(), 200, "the minted key reads the surface");

    // The write half: a group create lands, through the real gates.
    let created_group = send_with_api_key(
        &app,
        axum::http::Method::POST,
        "/api/directory-groups",
        &raw_key,
        Some(json!({ "name": "Tier 2 Support" })),
    )
    .await;
    assert_eq!(
        created_group.status(),
        200,
        "the minted key writes the surface"
    );
    let group_id: Uuid = body_json(created_group).await["id"]
        .as_str()
        .expect("group id")
        .parse()
        .expect("uuid");

    // The attribution decision at the column a reader joins: a membership the
    // connector wrote records `added_by = null` (the column holds a staff user
    // id), while the audit row above names the credential.
    let invited = send(
        &app,
        axum::http::Method::POST,
        "/api/org/users",
        Some(&owner),
        Some(json!({
            "email": format!("tech@{slug}.test"),
            "display_name": "Tech",
            "role": "technician",
        })),
    )
    .await;
    assert_eq!(invited.status(), 200, "the invite must succeed");
    let tech_id: Uuid = body_json(invited).await["id"]
        .as_str()
        .expect("user id")
        .parse()
        .expect("uuid");
    let added = send_with_api_key(
        &app,
        axum::http::Method::PUT,
        &format!("/api/directory-groups/{group_id}/members/{tech_id}"),
        &raw_key,
        None,
    )
    .await;
    assert_eq!(
        added.status(),
        200,
        "the connector may add a member to a group it created"
    );
    let (status, detail) = call(
        &app,
        axum::http::Method::GET,
        &format!("/api/directory-groups/{group_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, 200, "the detail must answer the owner: {detail}");
    let member = &detail["members"][0];
    assert_eq!(member["user_id"], json!(tech_id), "{detail}");
    assert!(
        member["added_by"].is_null(),
        "a machine write leaves the staff-attribution column null: {detail}"
    );

    // The trail half of the same decision: a machine write is a connector in
    // the audit row, never a human whose id happens to be a key id.
    let (status, trail) = call(
        &app,
        axum::http::Method::GET,
        "/api/audit-events?action=directory_group.created&limit=50",
        &owner,
        None,
    )
    .await;
    assert_eq!(status, 200, "the trail must answer the owner: {trail}");
    let event = trail["events"]
        .as_array()
        .expect("events array")
        .iter()
        .find(|event| event["action"] == "directory_group.created")
        .unwrap_or_else(|| panic!("the machine write must be audited: {trail}"));
    assert_eq!(event["actor"]["kind"], "connector", "{event}");
    assert_eq!(
        event["actor"]["id"],
        json!(format!("api-key:{key_id}")),
        "the actor id names the credential, in the import surfaces' spelling: {event}"
    );

    // Revoked bundles mint nothing, so a kill switch is not cosmetic.
    let (status, _) = call(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/identity-bundles/{id}/revoke"),
        &owner,
        Some(json!({ "reason": "pack retired" })),
    )
    .await;
    assert_eq!(status, 200, "revoke must land");
    let (status, refused) = call(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/identity-bundles/{id}/credential"),
        &owner,
        None,
    )
    .await;
    assert_eq!(
        status, 409,
        "a revoked bundle mints no credential: {refused}"
    );
}

#[tokio::test]
async fn the_group_listing_pages_on_name_and_resumes_without_gaps() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = unique_slug("group-paging");
    let owner = register_org_token(&app, "Group Paging Co", &slug).await;

    for name in ["Alpha", "Bravo", "Charlie"] {
        let (status, body) = call(
            &app,
            axum::http::Method::POST,
            "/api/directory-groups",
            &owner,
            Some(json!({ "name": name })),
        )
        .await;
        assert_eq!(status, 200, "group create must succeed: {body}");
    }

    let (status, first) = call(
        &app,
        axum::http::Method::GET,
        "/api/directory-groups?limit=2",
        &owner,
        None,
    )
    .await;
    assert_eq!(status, 200, "the first page must answer: {first}");
    let names: Vec<&str> = first["groups"]
        .as_array()
        .expect("groups array")
        .iter()
        .filter_map(|row| row["name"].as_str())
        .collect();
    assert_eq!(names, vec!["Alpha", "Bravo"], "{first}");
    let cursor = first["next_cursor"]
        .as_str()
        .expect("a full page carries a cursor")
        .to_string();

    let (status, second) = call(
        &app,
        axum::http::Method::GET,
        &format!("/api/directory-groups?limit=2&cursor={cursor}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, 200, "the resume must answer: {second}");
    let names: Vec<&str> = second["groups"]
        .as_array()
        .expect("groups array")
        .iter()
        .filter_map(|row| row["name"].as_str())
        .collect();
    assert_eq!(
        names,
        vec!["Charlie"],
        "the resume neither repeats nor skips: {second}"
    );
    assert!(
        second.get("next_cursor").is_none(),
        "a short page ends the walk: {second}"
    );

    // A checkpoint this build did not issue is a typed 422, not a silent first
    // page: an identity sync would otherwise re-deliver every group.
    let (status, refused) = call(
        &app,
        axum::http::Method::GET,
        "/api/directory-groups?cursor=not-a-cursor",
        &owner,
        None,
    )
    .await;
    assert_eq!(status, 422, "a broken cursor is refused: {refused}");
}
