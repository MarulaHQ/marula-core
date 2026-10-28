//! Directory-group grant ceiling through the REAL core-api router (Wave 55
//! lane `p55-directory-grant-ceiling`, security decision 10).
//!
//! Purpose: `directory_groups:write` is the machine grant a SCIM connector or a
//! directory-admin plugin holds on a key. It may mirror a directory. It may not
//! confer authority it does not itself hold, because a mapped role reaches
//! members through the same union as a direct assignment
//! (`core_db::users::effective_roles_for_user`) — so an unceilinged mapping
//! write is account takeover by proxy. Four cases pin the rule:
//!
//! 1. a key holding the write scope cannot map the seeded Admin preset onto a
//!    group: typed 403 naming the role, and no mapping row;
//! 2. the same key cannot add a person to a group that already carries a role
//!    above it: typed 403 naming the role, and no membership row;
//! 3. the same key maps a role inside its own set and adds a person to that
//!    group — the ceiling narrows the surface, it does not close it;
//! 4. the owner still maps Admin, and so does an admin, because the caller's
//!    own resolved set is what the comparison allows.
//!
//! Non-purpose: the storage union and the RLS boundary
//! (`crates/core-db/tests/directory_groups.rs` owns both), the comparator on
//! its own (unit tests in `routes/directory_groups.rs`), and SCIM, which is not
//! core.
//!
//! DB-backed and written not run in the lane: it needs a migrated database
//! (`MARULA_TEST_DATABASE_URL` or `DATABASE_URL`), the same requirement every
//! suite in this directory carries.

use crate::common::{body_json, register, send_with_api_key, test_state as boot_state};
use axum::Router;
use core_api::state::AppState;
use serde_json::{json, Value};
use uuid::Uuid;

async fn test_state() -> AppState {
    boot_state().await
}

enum Auth<'a> {
    /// No credential at all (registration, invite redemption).
    None,
    Bearer(&'a str),
    ApiKey(&'a str),
}

async fn send(
    app: &Router,
    method: axum::http::Method,
    uri: &str,
    auth: Auth<'_>,
    body: Option<Value>,
) -> axum::response::Response {
    match auth {
        Auth::None => crate::common::send(app, method, uri, None, body).await,
        Auth::Bearer(token) => crate::common::send(app, method, uri, Some(token), body).await,
        Auth::ApiKey(key) => send_with_api_key(app, method, uri, key, body).await,
    }
}

fn slug(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::now_v7().simple())
}

/// Registers a fresh org; returns the owner's access token.
async fn register_org(app: &Router, slug: &str) -> String {
    let body = register(
        app,
        json!({
            "org_name": "Directory Ceiling Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        }),
    )
    .await;
    body["access_token"].as_str().expect("access token").into()
}

struct RoleRef {
    id: Uuid,
    name: String,
}

/// One org role by its preset tag, from the roster the roles surface renders.
async fn role_by_builtin(app: &Router, bearer: &str, builtin: &str) -> RoleRef {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/roles",
        Auth::Bearer(bearer),
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "roles list must succeed");
    let body = body_json(response).await;
    let entry = body["roles"]
        .as_array()
        .expect("roles array")
        .iter()
        .find(|r| r["builtin"] == json!(builtin))
        .unwrap_or_else(|| panic!("preset '{builtin}' missing from the roster"));
    RoleRef {
        id: entry["id"]
            .as_str()
            .expect("role id")
            .parse()
            .expect("uuid"),
        name: entry["name"].as_str().expect("role name").to_string(),
    }
}

/// Creates a group as the given caller; returns its id.
async fn create_group(app: &Router, bearer: &str, name: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/directory-groups",
        Auth::Bearer(bearer),
        Some(json!({ "name": name })),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "group create must succeed for a writer"
    );
    let body = body_json(response).await;
    body["id"]
        .as_str()
        .expect("group id")
        .parse()
        .expect("uuid")
}

/// Invites a technician and returns their user id. The invite is the
/// org-users surface's own path, and it is how a real org gets a second
/// principal to move around.
async fn invite_technician(app: &Router, owner: &str, slug: &str) -> Uuid {
    let email = format!("tech@{slug}.test");
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/org/users",
        Auth::Bearer(owner),
        Some(json!({
            "email": email,
            "display_name": "Tech",
            "role": "technician",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "invite must succeed");
    let body = body_json(response).await;
    body["id"].as_str().expect("user id").parse().expect("uuid")
}

/// Mints a directory key. `directory_groups:write` is the scope the identity
/// pack's connector holds; the read scope travels with it because the connector
/// reads back what it wrote, exactly as the pack's documentation says.
async fn directory_key(app: &Router, owner: &str, name: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/api-keys",
        Auth::Bearer(owner),
        Some(json!({
            "name": name,
            "scopes": ["directory_groups:read", "directory_groups:write"],
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        201,
        "the directory scopes must be mintable"
    );
    let body = body_json(response).await;
    body["raw_key"].as_str().expect("raw key").to_string()
}

/// Forks the read-only preset and rewrites it to hold the directory write scope
/// alone. This is the "role inside the caller's own set" the ceiling must
/// admit: a directory mirror's mirror-role.
async fn directory_mirror_role(app: &Router, owner: &str) -> Uuid {
    let read_only = role_by_builtin(app, owner, "read_only").await;
    let forked = send(
        app,
        axum::http::Method::POST,
        &format!("/api/roles/{}/fork", read_only.id),
        Auth::Bearer(owner),
        Some(json!({ "name": "Directory Mirror" })),
    )
    .await;
    assert_eq!(forked.status(), 200, "fork must succeed");
    let forked_body = body_json(forked).await;
    let id: Uuid = forked_body["id"]
        .as_str()
        .expect("forked role id")
        .parse()
        .expect("uuid");
    let edited = send(
        app,
        axum::http::Method::PUT,
        &format!("/api/roles/{id}"),
        Auth::Bearer(owner),
        Some(json!({ "permissions": ["directory_groups:write"] })),
    )
    .await;
    assert_eq!(
        edited.status(),
        200,
        "a custom role may hold the directory write scope"
    );
    id
}

/// The group's detail as its reader sees it: the members and the mappings the
/// refused/written request would have changed.
async fn detail(app: &Router, reader: &str, group: Uuid) -> Value {
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/directory-groups/{group}"),
        Auth::Bearer(reader),
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "detail read must succeed");
    body_json(response).await
}

fn names(value: &Value, key: &str) -> Vec<String> {
    value[key]
        .as_array()
        .unwrap_or_else(|| panic!("'{key}' must be an array in {value}"))
        .iter()
        .map(|row| row["name"].as_str().unwrap_or_default().to_string())
        .collect()
}

// --- 1. the mapping write ----------------------------------------------------

#[tokio::test]
async fn a_directory_writer_cannot_map_the_admin_preset_onto_a_group() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = slug("dir-ceiling-map");
    let owner = register_org(&app, &slug).await;
    let group = create_group(&app, &owner, "Tier 2 Support").await;
    let admin = role_by_builtin(&app, &owner, "admin").await;
    let key = directory_key(&app, &owner, "scim-sync").await;

    let refused = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/directory-groups/{group}/roles/{}", admin.id),
        Auth::ApiKey(&key),
        None,
    )
    .await;
    assert_eq!(
        refused.status(),
        403,
        "a directory key must not map the admin preset"
    );
    let body = body_json(refused).await;
    assert_eq!(
        body["error"]["reason"], "forbidden",
        "the refusal is the typed forbidden, not a validation accident: {body}"
    );
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains(&admin.name),
        "the refusal names the role it refused: {message}"
    );
    assert!(
        message.contains("does not hold"),
        "the refusal states the direction of the comparison: {message}"
    );

    let read_back = detail(&app, &owner, group).await;
    assert!(
        names(&read_back, "roles").is_empty(),
        "a refused mapping writes no row: {read_back}"
    );
}

// --- 2. the member add -------------------------------------------------------

#[tokio::test]
async fn a_directory_writer_cannot_add_a_member_to_a_group_above_its_authority() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = slug("dir-ceiling-member");
    let owner = register_org(&app, &slug).await;
    let group = create_group(&app, &owner, "Admins").await;
    let admin = role_by_builtin(&app, &owner, "admin").await;
    let tech = invite_technician(&app, &owner, &slug).await;

    // The owner holds the admin set, so this mapping is allowed for the owner
    // and refused to everyone below it — the same group, two callers.
    let mapped = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/directory-groups/{group}/roles/{}", admin.id),
        Auth::Bearer(&owner),
        None,
    )
    .await;
    assert_eq!(mapped.status(), 200, "the owner may map Admin");

    let key = directory_key(&app, &owner, "scim-members").await;
    let refused = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/directory-groups/{group}/members/{tech}"),
        Auth::ApiKey(&key),
        None,
    )
    .await;
    assert_eq!(
        refused.status(),
        403,
        "a directory key must not hand out a membership that confers Admin"
    );
    let body = body_json(refused).await;
    assert_eq!(body["error"]["reason"], "forbidden");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains(&admin.name),
        "the refusal names the mapped role that blocks it: {body}"
    );

    let read_back = detail(&app, &owner, group).await;
    assert!(
        read_back["members"]
            .as_array()
            .expect("members array")
            .is_empty(),
        "a refused membership writes no row: {read_back}"
    );
    // The mapping the owner made is untouched by the refused add.
    assert_eq!(names(&read_back, "roles"), vec![admin.name.clone()]);
}

// --- 3. the ceiling narrows, it does not close -------------------------------

#[tokio::test]
async fn a_directory_writer_maps_and_adds_inside_its_own_set() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = slug("dir-ceiling-inside");
    let owner = register_org(&app, &slug).await;
    let group = create_group(&app, &owner, "Directory Mirror").await;
    let mirror = directory_mirror_role(&app, &owner).await;
    let tech = invite_technician(&app, &owner, &slug).await;
    let key = directory_key(&app, &owner, "scim-inside").await;

    let mapped = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/directory-groups/{group}/roles/{mirror}"),
        Auth::ApiKey(&key),
        None,
    )
    .await;
    assert_eq!(
        mapped.status(),
        200,
        "a role inside the caller's own set is mappable"
    );

    let added = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/directory-groups/{group}/members/{tech}"),
        Auth::ApiKey(&key),
        None,
    )
    .await;
    assert_eq!(
        added.status(),
        200,
        "a membership whose group carries only roles inside the caller's set is addable"
    );
    let body = body_json(added).await;
    assert_eq!(names(&body, "roles"), vec!["Directory Mirror".to_string()]);
    assert_eq!(
        body["members"].as_array().expect("members array").len(),
        1,
        "the membership landed: {body}"
    );

    // Idempotency survives the ceiling: the same two writes again are 200s that
    // change nothing, which is what a re-running sync depends on.
    for uri in [
        format!("/api/directory-groups/{group}/roles/{mirror}"),
        format!("/api/directory-groups/{group}/members/{tech}"),
    ] {
        let repeat = send(
            &app,
            axum::http::Method::PUT,
            &uri,
            Auth::ApiKey(&key),
            None,
        )
        .await;
        assert_eq!(
            repeat.status(),
            200,
            "a repeated write is a no-op 200: {uri}"
        );
    }
}

// --- 4. the ceiling's own authority still works ------------------------------

#[tokio::test]
async fn an_owner_and_an_admin_still_map_the_admin_preset() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = slug("dir-ceiling-owner");
    let owner = register_org(&app, &slug).await;
    let admin = role_by_builtin(&app, &owner, "admin").await;

    let owner_group = create_group(&app, &owner, "Owner Exec").await;
    let mapped = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/directory-groups/{owner_group}/roles/{}", admin.id),
        Auth::Bearer(&owner),
        None,
    )
    .await;
    assert_eq!(mapped.status(), 200, "the owner holds what Admin carries");
    let body = body_json(mapped).await;
    assert_eq!(names(&body, "roles"), vec![admin.name.clone()]);

    // An admin's resolved set is the owner's, so the same mapping is inside
    // their authority too. The user is invited as admin, then signs in.
    let email = format!("admin@{slug}.test");
    let invite = send(
        &app,
        axum::http::Method::POST,
        "/api/org/users",
        Auth::Bearer(&owner),
        Some(json!({
            "email": email,
            "display_name": "Admin",
            "role": "admin",
        })),
    )
    .await;
    assert_eq!(invite.status(), 200, "admin invite must succeed");
    let setup_token = body_json(invite).await["invite"]["setup_token"]
        .as_str()
        .expect("setup token")
        .to_string();
    let accepted = send(
        &app,
        axum::http::Method::POST,
        "/api/org/users/accept-invite",
        Auth::None,
        Some(json!({
            "token": setup_token,
            "password": "another-correct-horse",
        })),
    )
    .await;
    assert_eq!(accepted.status(), 200, "invite redemption must succeed");
    let admin_token = body_json(accepted).await["access_token"]
        .as_str()
        .expect("access token")
        .to_string();

    let admin_group = create_group(&app, &admin_token, "Admin Exec").await;
    let mapped = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/directory-groups/{admin_group}/roles/{}", admin.id),
        Auth::Bearer(&admin_token),
        None,
    )
    .await;
    assert_eq!(
        mapped.status(),
        200,
        "an admin holds the full admin set and may map it"
    );
}
