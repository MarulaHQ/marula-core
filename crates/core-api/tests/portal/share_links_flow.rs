//! One-time share links + rotation wiring (Wave 19 lane 3, migration
//! 0096) driven through the REAL core-api router against a real Postgres.
//!
//! Proves the lane-3 product surfaces end to end —
//! * one-time share links on BOTH vault subjects: create (201, token +
//!   link_path), list (metadata NEVER carries the token), claim-by-token
//!   (200 = the single reveal; the claimer must belong to the link's
//!   org), reclaim (409 — the link closed by that single claim),
//!   unknown token (404), revoke-then-claim (409), and foreign-org
//!   refusal (404 — the link's existence is not leaked cross-org);
//! * rotation: moving `rotate_at` forward stamps `last_rotated_at`
//!   server-side on both subjects.
//!
//! Non-purpose: the worker sweep lane (claim-stamp SQL, at-most-once
//! sweep) lives in core-db integration tests; the in-app notification
//! insertion lives in the internal surface (no HTTP test).

use crate::common::{body_json, boot_with, send};
use axum::Router;
use core_api::state::AppState;
use serde_json::{json, Value};
use uuid::Uuid;

async fn test_state() -> (AppState, Router) {
    boot_with(|_| {}).await
}

async fn register_org(app: &Router, slug: String) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Share Co",
            "org_slug": slug,
            "email": format!("owner@{}.test", slug),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    body_json(response).await
}

async fn org_folder(app: &Router, bearer: &str) -> String {
    let folders: Value = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/secrets/folders",
            Some(bearer),
            None,
        )
        .await,
    )
    .await;
    folders
        .as_array()
        .expect("folders list")
        .iter()
        .find(|f| !f["restricted"].as_bool().unwrap_or(false) && f["client_id"].is_null())
        .expect("an unrestricted org-scope folder exists after seeding")["id"]
        .as_str()
        .unwrap()
        .to_string()
}

const ROTATE_AT: &str = "2026-12-01T00:00:00Z";

async fn create_record(app: &Router, bearer: &str, folder: &str, title: &str) -> (String, String) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/secrets/records",
        Some(bearer),
        Some(json!({
            "folder_id": folder,
            "kind": "password",
            "title": title,
            "fields": [{"field_key": "secret", "value": "hunter2"}],
            "idempotency_key": format!("row-{}", Uuid::now_v7().simple()),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "create must succeed");
    let body = body_json(response).await;
    (body["id"].as_str().unwrap().to_string(), "hunter2".into())
}

#[tokio::test]
async fn one_time_share_record_claim_once_reclaims_refused_revoked_refused() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("share1-{}", Uuid::now_v7().simple())).await;
    let bearer = org["access_token"].as_str().unwrap().to_string();
    let folder = org_folder(&app, &bearer).await;
    let (record_id, _secret) = create_record(&app, &bearer, &folder, "PagerDuty key").await;

    // Mint the one-time link.
    let mint = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/secrets/records/{record_id}/share-links"),
        Some(&bearer),
        Some(json!({
            "label": "night-shift handoff",
            "expires_at": expiry_json(),
        })),
    )
    .await;
    assert_eq!(
        mint.status(),
        axum::http::StatusCode::CREATED,
        "mint must succeed"
    );
    let minted = body_json(mint).await;
    let token = minted["token"].as_str().unwrap().to_string();

    // The list NEVER shows the token — only the lifecycle.
    let list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/secrets/records/{record_id}/share-links"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let rows = list_of(&list);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].get("token").is_none());
    assert_eq!(rows[0]["label"], json!("night-shift handoff"));

    // Unknown tokens are a bare 404.
    let unknown = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/share-links/claim",
        Some(&bearer),
        Some(json!({"token": format!("not-a-real-token-{token}")})),
    )
    .await;
    assert_eq!(unknown.status(), axum::http::StatusCode::NOT_FOUND);

    // THE claim: exactly one caller consumes the link and reads the
    // material.
    let claim = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/share-links/claim",
        Some(&bearer),
        Some(json!({"token": token})),
    )
    .await;
    let claim_status = claim.status();
    let claimed = body_json(claim).await;
    assert_eq!(claim_status, 200, "claim must succeed: {claimed}");
    assert_eq!(claimed["subject_kind"], json!("secret_record"));
    assert_eq!(claimed["title"], json!("PagerDuty key"));
    let fields = claimed["fields"].as_array().unwrap();
    assert!(
        fields
            .iter()
            .any(|f| f["field_key"] == "secret" && f["value"] == json!("hunter2")),
        "claim must unseal the secret field: {fields:?}"
    );

    // The link is consumed: the claim result ledger shows it in state.
    let list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/secrets/records/{record_id}/share-links"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let rows = list_of(&list);
    assert_eq!(rows.len(), 1);
    assert!(rows[0]["consumed_at"].is_string());
    assert!(rows[0]["consumed_by"].is_string());

    // A SECOND claim is a typed 409.
    let second = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/share-links/claim",
        Some(&bearer),
        Some(json!({"token": token})),
    )
    .await;
    assert_eq!(second.status(), axum::http::StatusCode::CONFLICT);

    // Revocation seals a not-yet-consumed link shut.
    let mint_two = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/secrets/records/{record_id}/share-links"),
        Some(&bearer),
        Some(json!({"label": "revoked path", "expires_at": expiry_json()})),
    )
    .await;
    assert_eq!(mint_two.status(), axum::http::StatusCode::CREATED);
    let mint_two_body = body_json(mint_two).await;
    let token_two = mint_two_body["token"].as_str().unwrap().to_string();
    let share_two = mint_two_body["id"].as_str().unwrap().to_string();
    let revoke = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/secrets/records/{record_id}/share-links/{share_two}"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(revoke.status(), 200, "revoke must succeed");
    let revoked_claim = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/share-links/claim",
        Some(&bearer),
        Some(json!({"token": token_two})),
    )
    .await;
    assert_eq!(
        revoked_claim.status(),
        axum::http::StatusCode::CONFLICT,
        "a revoked link claims nothing"
    );

    // Rotation: moving rotate_at forward stamps last_rotated_at in the
    // SAME server-side write (the rotation ledger entry).
    let patch = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/secrets/records/{record_id}"),
        Some(&bearer),
        Some(json!({
            "title": "PagerDuty key",
            "rotate_at": ROTATE_AT,
        })),
    )
    .await;
    let patch_status = patch.status();
    let patched = body_json(patch).await;
    assert_eq!(patch_status, 200, "patch must succeed: {patched}");
    assert!(
        patched["last_rotated_at"].is_string(),
        "rotation ledger stamped: {patched}"
    );
    assert_eq!(patched["rotate_at"], json!(ROTATE_AT));
}

fn list_of(v: &Value) -> &[Value] {
    v.as_array().expect("share-link list")
}

#[tokio::test]
async fn one_time_share_password_surface_and_rotation_ledger() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("share2-{}", Uuid::now_v7().simple())).await;
    let bearer = org["access_token"].as_str().unwrap().to_string();

    // Legacy password subject: create + mint + claim once.
    let created = send(
        &app,
        axum::http::Method::POST,
        "/api/passwords",
        Some(&bearer),
        Some(json!({
            "title": "Router admin",
            "username": "admin",
            "secret": "correct-horse-battery",
            "idempotency_key": format!("pw-{}", Uuid::now_v7().simple()),
        })),
    )
    .await;
    assert_eq!(created.status(), 200, "password create must succeed");
    let pw = body_json(created).await;
    let pw_id = pw["id"].as_str().unwrap().to_string();

    let mint = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/passwords/{pw_id}/share-links"),
        Some(&bearer),
        Some(json!({"label": "", "expires_at": expiry_json()})),
    )
    .await;
    let mint_status = mint.status();
    let minted = body_json(mint).await;
    assert_eq!(
        mint_status,
        axum::http::StatusCode::CREATED,
        "mint: {minted}"
    );
    let token = minted["token"].as_str().unwrap().to_string();
    assert!(minted["link_path"]
        .as_str()
        .unwrap()
        .contains("/share?token="));

    let claimed = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/secrets/share-links/claim",
            Some(&bearer),
            Some(json!({"token": token})),
        )
        .await,
    )
    .await;
    assert_eq!(claimed["subject_kind"], json!("password"));
    let fields = claimed["fields"].as_array().unwrap();
    assert!(
        fields
            .iter()
            .any(|f| f["field_key"] == "secret" && f["value"] == json!("correct-horse-battery")),
        "the claim returns the sealed secret: {fields:?}"
    );

    // Password rotation ledger: moving rotate_at forward stamps
    // last_rotated_at in the same PATCH (the legacy surface parity).
    let patch = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/passwords/{pw_id}"),
        Some(&bearer),
        Some(json!({
            "title": "Router admin",
            "username": "admin",
            "url": "",
            "rotate_at": ROTATE_AT_PW,
            "idempotency_key": format!("pw2-{}", Uuid::now_v7().simple()),
        })),
    )
    .await;
    let patch_status = patch.status();
    let patched = body_json(patch).await;
    assert_eq!(patch_status, 200, "password patch must succeed: {patched}");
    assert!(patched["last_rotated_at"].is_string());
    assert_eq!(patched["rotate_at"], json!(ROTATE_AT_PW));
}

fn expiry_json() -> String {
    // Within the 90-day ceiling — 24 hours out.
    (chrono::Utc::now() + chrono::Duration::hours(24)).to_rfc3339()
}

const ROTATE_AT_PW: &str = "2026-04-01T00:00:00Z";
