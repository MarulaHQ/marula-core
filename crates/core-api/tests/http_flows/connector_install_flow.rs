//! The connector install surface and the VoIP ingest.
//!
//! Both were registered in the *public* router block, which is merged before
//! `require_auth` and therefore never receives an `AuthedUser`. Every handler
//! here starts with `authed::<AuthedUser>(&extensions)?`, so every call
//! answered 401 — including the console's own install and canary calls.
//!
//! These tests exist so that can never happen quietly again: an authenticated
//! call must reach the handler, and an unauthenticated one must be refused by
//! the middleware rather than by a handler that assumes it ran.

use crate::common::{body_json, register_org, send, test_state};
use axum::Router;
use serde_json::json;
use uuid::Uuid;

struct Org {
    id: Uuid,
    owner: String,
}

async fn register(app: &Router, tag: &str) -> Org {
    let slug = format!("{tag}-{}", Uuid::now_v7().simple());
    let registered = register_org(app, "Install Co", &slug).await;
    let token = registered["access_token"]
        .as_str()
        .expect("token")
        .to_string();
    let me =
        body_json(send(app, axum::http::Method::GET, "/api/me", Some(&token), None).await).await;
    Org {
        id: me["org_id"]
            .as_str()
            .expect("org id")
            .parse()
            .expect("uuid"),
        owner: token,
    }
}

#[tokio::test]
async fn the_install_surface_refuses_an_anonymous_call_and_serves_a_session() {
    let (_state, app) = test_state().await;
    let org = register(&app, "install-surface").await;
    assert_ne!(org.id, Uuid::nil());

    let anonymous = send(&app, axum::http::Method::GET, "/api/connectors", None, None).await;
    assert_eq!(
        anonymous.status(),
        401,
        "the middleware must refuse this, not the handler"
    );

    let authenticated = send(
        &app,
        axum::http::Method::GET,
        "/api/connectors",
        Some(&org.owner),
        None,
    )
    .await;
    assert_eq!(
        authenticated.status(),
        200,
        "an owner reaches the install listing; a 401 here means the route is not behind require_auth"
    );
    let body = body_json(authenticated).await;
    assert!(
        body.is_object() || body.is_array(),
        "the listing answers a shape, not an error: {body}"
    );
}

/// The ingest is machine-called by a VoIP connector holding an API key. The
/// body is deliberately valid so that only authentication can decide the
/// outcome: a 401 would mean the middleware never ran, and 404 means this org
/// has no `voip` installation, which is the furthest the request can get
/// without one.
#[tokio::test]
async fn the_voip_ingest_authenticates_before_it_looks_for_an_install() {
    let (_state, app) = test_state().await;
    let org = register(&app, "voip-ingest").await;

    let body = json!({
        "call_id": "call-1",
        "direction": "in",
        "external_number": "+15550100",
        "duration_seconds": 42,
        "occurred_at": "2026-11-14T10:00:00Z"
    });

    let anonymous = send(
        &app,
        axum::http::Method::POST,
        "/api/connectors/voip/call-logs",
        None,
        Some(body.clone()),
    )
    .await;
    assert_eq!(anonymous.status(), 401);

    let authenticated = send(
        &app,
        axum::http::Method::POST,
        "/api/connectors/voip/call-logs",
        Some(&org.owner),
        Some(body),
    )
    .await;
    assert_ne!(
        authenticated.status(),
        401,
        "the request authenticated, so the answer is about the org's installations rather than the caller"
    );
}
