//! Event-type registry validation flow (the private build tracker Wave 43 lane 2).
//!
//! Verifies that webhook subscription create/update reject unknown or
//! ungranted event types, accept platform types without a grant, and
//! accept custom types when a grant row exists.

use crate::common::{auth, body_json, register_org_token, send_headers as send, state_with};
use axum::Router;
use core_api::state::AppState;
use serde_json::json;
use uuid::Uuid;

async fn test_state() -> AppState {
    state_with(|config| config.secret_key = core_api::config::SecretKey([11; 32])).await
}

async fn register_org(app: &Router, slug: &str) -> String {
    register_org_token(app, "Registry Co", slug).await
}

async fn org_id_for(state: &AppState, slug: &str) -> uuid::Uuid {
    let email = format!("owner@{slug}.test");
    let row: (uuid::Uuid,) = sqlx::query_as("SELECT org_id FROM users WHERE email = $1")
        .bind(&email)
        .fetch_one(&state.pool)
        .await
        .expect("org lookup for test");
    row.0
}

async fn grant_event_type(state: &AppState, org_id: uuid::Uuid, event_type: &str) {
    let mut tx = state.pool.begin().await.expect("tx begin");
    sqlx::query("SELECT set_config('app.org_id', $1, true)")
        .bind(org_id.to_string())
        .execute(&mut *tx)
        .await
        .expect("set local org id");
    sqlx::query(
        "INSERT INTO event_type_grants (org_id, event_type, granted_by) VALUES ($1, $2, NULL) \
         ON CONFLICT (org_id, event_type) DO NOTHING",
    )
    .bind(org_id)
    .bind(event_type)
    .execute(&mut *tx)
    .await
    .expect("grant insert");
    tx.commit().await.expect("tx commit");
}

#[tokio::test]
async fn platform_types_accepted_without_grant() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("reg-platform-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/webhooks",
        &[auth(&token)],
        Some(json!({
            "url": "https://hooks.example.test/marula",
            "event_types": ["ticket.created", "ticket.closed", "webhook.test"],
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "known platform types must succeed");
    let created = body_json(response).await;
    assert!(created["webhook"]["id"].as_str().is_some());
}

#[tokio::test]
async fn lane1_types_accepted_without_grant() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("reg-lane1-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/webhooks",
        &[auth(&token)],
        Some(json!({
            "url": "https://hooks.example.test/marula",
            "event_types": ["alert.ingested", "webhook.received"],
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "lane-1 platform types must succeed");
}

#[tokio::test]
async fn unknown_type_rejected_without_grant() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("reg-unknown-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/webhooks",
        &[auth(&token)],
        Some(json!({
            "url": "https://hooks.example.test/marula",
            "event_types": ["my.custom.event"],
        })),
    )
    .await;
    assert_eq!(response.status(), 422, "unregistered type must be rejected");
    let body = body_json(response).await;
    let msg = body["error"]["message"].as_str().expect("message");
    assert!(
        msg.contains("my.custom.event"),
        "error must name the offending type: {msg}"
    );
}

#[tokio::test]
async fn granted_custom_type_accepted() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("reg-granted-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let org_id = org_id_for(&state, &slug).await;

    grant_event_type(&state, org_id, "my.custom.event").await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/webhooks",
        &[auth(&token)],
        Some(json!({
            "url": "https://hooks.example.test/marula",
            "event_types": ["my.custom.event"],
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "granted type must succeed");
}

#[tokio::test]
async fn update_rejects_ungranted_type() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("reg-update-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/webhooks",
        &[auth(&token)],
        Some(json!({
            "url": "https://hooks.example.test/marula",
            "event_types": ["ticket.created"],
        })),
    )
    .await;
    assert_eq!(response.status(), 201);
    let hook_id = body_json(response).await["webhook"]["id"]
        .as_str()
        .expect("hook id")
        .to_string();

    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/webhooks/{hook_id}"),
        &[auth(&token)],
        Some(json!({ "event_types": ["not.registered"] })),
    )
    .await;
    assert_eq!(response.status(), 422, "update must reject unknown type");
}

#[tokio::test]
async fn mixed_platform_and_custom_types_partial_failure() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("reg-mixed-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let org_id = org_id_for(&state, &slug).await;

    grant_event_type(&state, org_id, "my.custom.event").await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/webhooks",
        &[auth(&token)],
        Some(json!({
            "url": "https://hooks.example.test/marula",
            "event_types": ["ticket.created", "my.custom.event"],
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "mixed valid types must succeed");
}
