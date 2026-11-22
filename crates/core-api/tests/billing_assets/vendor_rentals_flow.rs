//! Vendor rentals (the inbound half of the rental model) through the REAL
//! core-api router.
//!
//! Pins three things the surface depends on:
//! 1. Recording a rental creates the unit as a rental-in asset and the view
//!    names it, tag and all — a register row identifies the thing.
//! 2. The register answers by status with the same asset fields, so a page
//!    can show what is out without a second read.
//! 3. A return closes the row with its condition, and a lost unit records the
//!    loss rather than a condition.

use crate::common::{auth, body_json, boot_with, register_org_token, send_headers as send};
use axum::Router;
use serde_json::{json, Value};
use uuid::Uuid;

fn today() -> String {
    chrono::Utc::now().date_naive().to_string()
}

async fn create_rental(app: &Router, token: &str, tag: &str) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/vendor-rentals",
        &[auth(token)],
        Some(json!({
            "vendor_name": "Borrowed Gear Ltd",
            "asset": { "tag": tag, "name": "Loaned switch", "kind": "device" },
            "cycle": "monthly",
            "cost_minor": 25_000,
            "currency": "USD",
            "deposit_minor": 10_000,
            "deposit_currency": "USD",
            "start_on": today(),
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, 200, "record a vendor rental: {body}");
    body["rental"].clone()
}

#[tokio::test]
async fn a_vendor_rental_names_its_unit_and_returns_with_a_condition() {
    let (_state, app) =
        boot_with(|config| config.secret_key = core_api::config::SecretKey([29; 32])).await;
    let token = register_org_token(
        &app,
        "Borrowed Gear",
        &format!("vr-{}", Uuid::now_v7().simple()),
    )
    .await;

    let rental = create_rental(&app, &token, "VR-001").await;
    assert_eq!(rental["asset_tag"], json!("VR-001"));
    assert_eq!(rental["asset_name"], json!("Loaned switch"));
    assert_eq!(rental["status"], json!("open"));
    assert_eq!(rental["cost_minor"], json!(25_000));
    let rental_id = rental["id"].as_str().expect("rental id").to_string();

    let list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/vendor-rentals?status=open",
            &[auth(&token)],
            None,
        )
        .await,
    )
    .await;
    let rows = list.as_array().expect("a register array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["asset_tag"], json!("VR-001"));

    let returned = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/vendor-rentals/{rental_id}/return"),
        &[auth(&token)],
        Some(json!({
            "return_condition": "fair",
            "damage_minor": 1_500,
            "notes": "Scratched lid",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    let status = returned.status();
    let body = body_json(returned).await;
    assert_eq!(status, 200, "return: {body}");
    assert_eq!(body["rental"]["status"], json!("returned"));
    assert_eq!(body["rental"]["return_condition"], json!("fair"));
    assert_eq!(body["rental"]["damage_minor"], json!(1_500));
    assert_eq!(body["rental"]["asset_tag"], json!("VR-001"));

    // A second return is a typed refusal, and the first one's record stands.
    let again = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/vendor-rentals/{rental_id}/return"),
        &[auth(&token)],
        Some(json!({
            "return_condition": "damaged",
            "damage_minor": 9_000,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(
        again.status(),
        409,
        "a closed rental cannot be returned twice"
    );
    let closed = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/vendor-rentals?status=returned",
            &[auth(&token)],
            None,
        )
        .await,
    )
    .await;
    let closed_rows = closed.as_array().expect("a register array");
    assert_eq!(closed_rows.len(), 1);
    assert_eq!(closed_rows[0]["return_condition"], json!("fair"));
    assert_eq!(closed_rows[0]["damage_minor"], json!(1_500));

    let empty = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/vendor-rentals?status=open",
            &[auth(&token)],
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        empty.as_array().expect("array").len(),
        0,
        "nothing is still out"
    );
}

#[tokio::test]
async fn a_lost_unit_records_the_loss_not_a_condition() {
    let (_state, app) =
        boot_with(|config| config.secret_key = core_api::config::SecretKey([29; 32])).await;
    let token = register_org_token(
        &app,
        "Borrowed Gear",
        &format!("vr-lost-{}", Uuid::now_v7().simple()),
    )
    .await;
    let rental = create_rental(&app, &token, "VR-002").await;
    let rental_id = rental["id"].as_str().expect("rental id").to_string();

    let returned = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/vendor-rentals/{rental_id}/return"),
        &[auth(&token)],
        Some(json!({
            "lost": true,
            "loss_minor": 40_000,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    let status = returned.status();
    let body = body_json(returned).await;
    assert_eq!(status, 200, "return: {body}");
    assert_eq!(body["rental"]["lost"], json!(true));
    assert_eq!(body["rental"]["loss_minor"], json!(40_000));
}
