//! Dev-tools surface gating (the private build tracker DX2).
//!
//! Purpose: prove the hard rule — `/api/dev/*` routes DO NOT EXIST unless
//! the process runs with MARULA_DEV_TOOLS=1. "Not exist" is the stronger
//! existence-sentinel shape from api_versioning: axum's unmatched 404 has
//! an EMPTY body and no content-type, while every real rejection returns a
//! JSON envelope. When the gate is on, the same probes must land on the
//! real handlers (JSON bodies, no auth required — the env IS the gate).
//!
//! Database contract follows settings_flow/api_versioning:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use crate::common::state_with;
use axum::Router;
use core_api::state::AppState;
use tower::ServiceExt;

async fn test_state(dev_tools: bool) -> AppState {
    state_with(move |config| config.dev_tools = dev_tools).await
}

async fn get(app: &Router, uri: &str) -> axum::response::Response {
    let request = axum::http::Request::builder()
        .method(axum::http::Method::GET)
        .uri(uri)
        .body(axum::body::Body::empty())
        .unwrap();
    app.clone().oneshot(request).await.unwrap()
}

async fn post_json(app: &Router, uri: &str, body: &str) -> axum::response::Response {
    let request = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();
    app.clone().oneshot(request).await.unwrap()
}

#[tokio::test]
async fn dev_routes_do_not_exist_without_the_gate() {
    let state = test_state(false).await;
    let app = core_api::routes::build_router(state);
    for uri in [
        "/api/dev/status",
        "/api/dev/seed",
        "/api/dev/reset-org",
        "/api/dev/time-travel",
        "/api/dev/events",
    ] {
        let response = get(&app, uri).await;
        assert_eq!(
            response.status().as_u16(),
            404,
            "{uri} must be an unmatched route when the gate is off"
        );
        // Existence sentinel: unmatched = EMPTY body, no content-type. A
        // rejection envelope here would mean the route was mounted anyway.
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            bytes.is_empty(),
            "{uri} returned a body ({bytes:?}) — the route exists without the gate"
        );
    }
}

#[tokio::test]
async fn dev_routes_exist_and_answer_with_the_gate_on() {
    let state = test_state(true).await;
    let app = core_api::routes::build_router(state);

    // Status: JSON, no auth, reports build info + demo state.
    let response = get(&app, "/api/dev/status").await;
    assert_eq!(response.status().as_u16(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["enabled"], serde_json::Value::Bool(true));
    assert!(
        body["migration_head"].is_i64(),
        "status must carry the migration head"
    );

    // Events: JSON array (empty on a fresh schema — still a real handler).
    let response = get(&app, "/api/dev/events?limit=5").await;
    assert_eq!(response.status().as_u16(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(body.is_array());

    // Time-travel validation: a bad hours value is a typed 422 from the
    // handler (proves the route is mounted and guarded, without writing).
    let response = post_json(&app, "/api/dev/time-travel", r#"{"hours": 0}"#).await;
    assert_eq!(response.status().as_u16(), 422);
}
