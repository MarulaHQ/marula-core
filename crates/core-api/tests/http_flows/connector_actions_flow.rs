//! The connector read surfaces' HTTP contract: a record the caller may not
//! read answers 404, and an unknown record type is a typed 422.
//!
//! Purpose: the resolver is proven at the query layer in
//! `core-db/tests/connector_actions_resolution.rs`; what this target adds is
//! the refusal shape, which is the part a client branches on and the part a
//! careless route gets wrong:
//!
//!   * an unknown `record_type` names the accepted set and answers 422 —
//!     never a 500 and never a silent fallback;
//!   * a record the caller may not read is 404, not 403. The technician here
//!     holds `clients:view`, so the module gate passes and the answer comes
//!     from the restricted-client rule — the exact case where a 403 would
//!     confirm that the record exists;
//!   * a readable record with no references is `200 { capabilities: [] }`,
//!     which is the empty state the chip row renders, not an error.
//!
//! Non-purpose: manifest resolution (the core-db target) and the roster.

use crate::common::{body_json, register_org, send, test_state};
use axum::Router;
use core_api::state::AppState;
use serde_json::json;
use uuid::Uuid;

/// Seeds a technician who can read clients, and returns a bearer for them.
///
/// A technician is deliberate: the owner/admin bypass would skip the
/// restricted-client rule this test exists to exercise.
async fn technician_token(state: &AppState, app: &Router, org_id: Uuid, slug: &str) -> String {
    let user_id = Uuid::now_v7();
    let email = format!("tech@{slug}.test");
    let password = "correct-horse-battery";
    let hash = core_api::auth::password::hash(password).expect("hash");
    core_db::users::create_user(&state.pool, user_id, org_id, &email, "Tech", &hash, "UTC")
        .await
        .expect("technician");
    let roles = core_db::roles::list_roles(&state.pool, org_id)
        .await
        .expect("roles");
    let technician = roles
        .iter()
        .find(|role| role.builtin.as_deref() == Some("technician"))
        .expect("the technician preset is seeded at registration");
    core_db::users::assign_role(&state.pool, user_id, technician.id)
        .await
        .expect("assign role");

    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/login",
        None,
        Some(json!({ "email": email, "password": password })),
    )
    .await;
    assert_eq!(response.status(), 200, "technician login");
    let body = body_json(response).await;
    body["access_token"]
        .as_str()
        .expect("access token")
        .to_string()
}

/// One client plus one configuration, inserted exactly as the route layer
/// would find them. `restricted_to` decides who may read the client.
async fn seed_client_asset(
    state: &AppState,
    org_id: Uuid,
    restricted_to: Option<Uuid>,
    tag: &str,
) -> (Uuid, Uuid) {
    let client_id = Uuid::now_v7();
    let asset_id = Uuid::now_v7();
    let restricted = restricted_to
        .map(|user| json!([user]))
        .unwrap_or_else(|| json!([]));
    sqlx::query("INSERT INTO clients (id, org_id, name, restricted_to) VALUES ($1, $2, $3, $4)")
        .bind(client_id)
        .bind(org_id)
        .bind(format!("Client {tag}"))
        .bind(&restricted)
        .execute(&state.pool)
        .await
        .expect("client");
    core_db::assets::create_asset(
        &state.pool,
        org_id,
        asset_id,
        Some(client_id),
        "client",
        None,
        tag,
        "Dell Latitude",
        "laptop",
        "active",
        None,
        &json!({}),
    )
    .await
    .expect("asset");
    (client_id, asset_id)
}

fn reason(body: &serde_json::Value) -> &str {
    body["error"]["reason"].as_str().unwrap_or_default()
}

/// A restricted client's configuration is 404 for a technician, and the
/// endpoint never confirms that the record exists.
#[tokio::test]
async fn a_record_the_caller_may_not_read_is_404_not_403() {
    let (state, app) = test_state().await;
    let slug = format!("connector-404-{}", Uuid::now_v7().simple());
    let registered = register_org(&app, "Connector Co", &slug).await;
    let owner_token = registered["access_token"]
        .as_str()
        .expect("token")
        .to_string();
    let me = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/me",
            Some(&owner_token),
            None,
        )
        .await,
    )
    .await;
    let owner: Uuid = me["user_id"]
        .as_str()
        .expect("owner id")
        .parse()
        .expect("uuid");
    let org_id: Uuid = me["org_id"]
        .as_str()
        .expect("org id")
        .parse()
        .expect("uuid");
    let tech = technician_token(&state, &app, org_id, &slug).await;

    // Visible only to the OWNER, so the technician is refused by the same
    // rule the client's own detail page applies.
    let (restricted_client, restricted_asset) =
        seed_client_asset(&state, org_id, Some(owner), "LT-RESTRICTED").await;
    // A control: no restriction list, so the same caller reads it.
    let (_open_client, open_asset) = seed_client_asset(&state, org_id, None, "LT-OPEN").await;

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/records/configuration/{restricted_asset}/external-actions"),
        Some(&tech),
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        404,
        "a record outside the caller's access must not disclose that it exists"
    );

    // The client route answers the same way, which is what "same rule" means.
    let client_response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/clients/{restricted_client}"),
        Some(&tech),
        None,
    )
    .await;
    assert_eq!(client_response.status(), 404);

    let open_response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/records/configuration/{open_asset}/external-actions"),
        Some(&tech),
        None,
    )
    .await;
    assert_eq!(open_response.status(), 200);
    let body = body_json(open_response).await;
    assert_eq!(
        body["capabilities"],
        json!([]),
        "a readable record with no references is the empty chip row, not an error"
    );
}

/// Unknown and unsupported record types refuse with the accepted set, and a
/// missing record of a supported type is a plain 404.
#[tokio::test]
async fn unknown_record_types_are_a_typed_refusal_and_missing_records_are_404() {
    let (state, app) = test_state().await;
    let slug = format!("connector-types-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, "Connector Types", &slug).await["access_token"]
        .as_str()
        .expect("token")
        .to_string();

    let unknown = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/records/vehicles/{}/external-actions", Uuid::now_v7()),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(unknown.status(), 422, "an unknown record type is a refusal");
    let body = body_json(unknown).await;
    assert_eq!(reason(&body), "validation_failed");
    let message = body["error"]["message"].as_str().unwrap_or_default();
    for allowed in ["configuration", "ticket", "invoice", "contact", "client"] {
        assert!(
            message.contains(allowed),
            "the refusal names the accepted set (missing {allowed}): {message}"
        );
    }

    // `domain` is in the vocabulary but this route cannot authorise it yet, so
    // it refuses the same way rather than answering as if it had checked.
    let unsupported = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/records/domain/{}/external-actions", Uuid::now_v7()),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(unsupported.status(), 422);

    let missing = send(
        &app,
        axum::http::Method::GET,
        &format!(
            "/api/records/configuration/{}/external-actions",
            Uuid::now_v7()
        ),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(missing.status(), 404);
    assert_eq!(reason(&body_json(missing).await), "not_found");

    // The roster is a read, not a discovery channel: an unauthenticated call
    // never reaches it.
    let unauthenticated = send(
        &app,
        axum::http::Method::GET,
        "/api/connectors/roster",
        None,
        None,
    )
    .await;
    assert_eq!(unauthenticated.status(), 401);

    // The owner holds the Plugins view the roster asks for; the shape is the
    // contract the settings page renders.
    let roster = send(
        &app,
        axum::http::Method::GET,
        "/api/connectors/roster",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(roster.status(), 200);
    let roster = body_json(roster).await;
    assert_eq!(
        roster["connectors"],
        json!([]),
        "a fresh org has installed nothing, so the roster is empty"
    );

    let _ = state;
}
