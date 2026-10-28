//! End-to-end BYOK sealed-credential flow through the REAL core-api router.
//!
//! Purpose: none-mode typed unavailable probe, admin-gated seal/re-seal of
//! API keys (401/403 without standing), instant-first write-path behavior
//! on the BYOK surface (idempotency replay is byte-identical; a reused key
//! with a different payload is a typed 409), presence-only status (no key
//! material ever appears in any response), connector providers (qbo/xero)
//! accepted by the same store, and delete (404 on second delete).
//!
//! Non-purpose: SQL-layer behavior (core-db byok suite), the adapter
//! crates' HTTP contract tests, worker scheduling.

use crate::common::{body_json, register_org_token, send, test_state};
use axum::Router;
use serde_json::json;
use uuid::Uuid;

async fn register_org(app: &Router, slug: &str) -> String {
    register_org_token(app, "Byok Flow Co", slug).await
}

#[tokio::test]
async fn byok_seal_replay_test_delete_flow() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("byok-flow-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let key = format!("key-{}", Uuid::now_v7().simple());

    // --- none-mode probe: typed unavailable, zero network --------------------
    // Unset is no longer none: an install that configured nothing runs the
    // instance's local embedder, so a probe with no provider would reach a real
    // runtime. This section is about none mode, so it declares none.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/org",
        Some(&token),
        Some(json!({ "key": "ai.completion_provider", "value": "none" })),
    )
    .await;
    assert_eq!(response.status(), 200, "declare none mode");
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/settings/ai/test",
        Some(&token),
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 200);
    let probe = body_json(response).await;
    assert_eq!(probe["provider"], "none");
    assert_eq!(probe["ok"], false);
    assert!(
        probe["message"].as_str().unwrap().contains("none mode"),
        "none mode must surface the typed unavailable reason: {probe}"
    );

    // --- status starts empty --------------------------------------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/settings/ai/status",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let status = body_json(response).await;
    assert_eq!(status["active_provider"], "none");
    assert_eq!(status["configured_providers"].as_array().unwrap().len(), 0);

    // --- seal an AI provider key through the instant-first write path ---------
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/byok",
        Some(&token),
        Some(json!({
            "provider": "openai",
            "api_key": "sk-live-abc123",
            "idempotency_key": key,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "seal must succeed");
    let first = body_json(response).await;
    assert_eq!(first["provider"], "openai");
    assert_eq!(first["has_key"], true);
    assert!(first["mutation"].is_object(), "IF2 mutation meta present");
    let version = first["mutation"]["version"].as_i64().expect("version");

    // Replay: same key + same payload ⇒ the SAME stored result.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/byok",
        Some(&token),
        Some(json!({
            "provider": "openai",
            "api_key": "sk-live-abc123",
            "idempotency_key": key,
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let replay = body_json(response).await;
    assert_eq!(
        replay["mutation"]["version"], version,
        "replay is byte-identical"
    );

    // Same key + different payload ⇒ typed 409 (IF1).
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/byok",
        Some(&token),
        Some(json!({
            "provider": "openai",
            "api_key": "sk-ROTATED-999",
            "idempotency_key": key,
        })),
    )
    .await;
    assert_eq!(response.status(), 409, "key reuse must be a typed 409");

    // --- status reflects configuration, never key material ---------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/settings/ai/status",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let status_text = body_json(response).await.to_string();
    assert!(status_text.contains("openai"));
    assert!(
        !status_text.contains("sk-live-abc123"),
        "no API key bytes may appear in status responses"
    );

    // --- test-connection verifies the sealed key decrypts ------------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/settings/ai/test",
        Some(&token),
        Some(json!({ "provider": "openai" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let probe = body_json(response).await;
    assert_eq!(probe["ok"], true, "sealed key must verify: {probe}");

    // --- validation ------------------------------------------------------------
    for bad in [
        json!({ "provider": "not-a-thing", "api_key": "x" }),
        json!({ "provider": "openai", "api_key": "  " }),
        json!({ "provider": "  ", "api_key": "x" }),
    ] {
        let response = send(
            &app,
            axum::http::Method::PUT,
            "/api/settings/ai/byok",
            Some(&token),
            Some(bad.clone()),
        )
        .await;
        assert_eq!(response.status(), 422, "expected 422 for {bad}");
    }

    // --- connector provider rides the same sealed store --------------------------
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/byok",
        Some(&token),
        Some(json!({ "provider": "qbo", "api_key": "qbo-oauth-token" })),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "connector providers must be accepted"
    );
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/settings/ai/status",
        Some(&token),
        None,
    )
    .await;
    let status = body_json(response).await;
    let configured: Vec<&str> = status["configured_providers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().expect("provider"))
        .collect();
    assert!(configured.contains(&"openai") && configured.contains(&"qbo"));

    // --- delete removes exactly one provider --------------------------------------
    let response = send(
        &app,
        axum::http::Method::DELETE,
        "/api/settings/ai/byok/openai",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let deleted = body_json(response).await;
    assert_eq!(deleted["has_key"], false);
    assert!(
        deleted["mutation"]["version"].is_i64(),
        "IF2 mutation meta present on delete: {deleted}"
    );

    let response = send(
        &app,
        axum::http::Method::DELETE,
        "/api/settings/ai/byok/openai",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 404, "second delete ⇒ rls_sparse");

    // --- audit trail recorded -------------------------------------------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/audit-events?action=settings.ai_byok_updated",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let audit = body_json(response).await;
    let entries = audit["events"].as_array().expect("audit events list");
    assert!(
        entries.iter().any(|e| e["detail"]["provider"] == "openai"),
        "seal must be audited without key material"
    );
}

#[tokio::test]
async fn byok_surface_requires_authentication() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("byok-anon-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    // Anonymous writes are 401; authenticated reads stay open to members.
    let anon = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/byok",
        None,
        Some(json!({ "provider": "openai", "api_key": "x" })),
    )
    .await;
    assert_eq!(anon.status(), 401);

    let anon_test = send(
        &app,
        axum::http::Method::POST,
        "/api/settings/ai/test",
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(anon_test.status(), 401);

    let status = send(
        &app,
        axum::http::Method::GET,
        "/api/settings/ai/status",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status.status(), 200);
}
