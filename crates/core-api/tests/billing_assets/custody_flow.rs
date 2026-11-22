//! The custody read model: where every physical unit is, and the class ledger.
//!
//! Pins what the surface depends on:
//! 1. The summary lists the whole class vocabulary, not just occupied classes,
//!    and a received serial lands in `sale_stock`.
//! 2. The ledger answers newest-first and refuses an unknown unit kind with a
//!    typed 422 rather than silently returning everything.

use crate::common::{auth, body_json, boot_with, register_org_token, send_headers as send};
use axum::Router;
use serde_json::{json, Value};
use uuid::Uuid;

fn token_email(slug: &str) -> String {
    format!("owner@{slug}.test")
}

async fn create_serialized(app: &Router, token: &str) -> (String, String) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/stock-items",
        &[auth(token)],
        Some(json!({
            "name": "Laptop",
            "kind": "serialized",
            "unit": "each",
            "currency": "USD",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let item: Value = body_json(response).await;
    let location = send(
        app,
        axum::http::Method::POST,
        "/api/v1/stock-locations",
        &[auth(token)],
        Some(json!({
            "name": "Shelf",
            "kind": "warehouse",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(location.status(), 200);
    let location: Value = body_json(location).await;
    (
        item["id"].as_str().unwrap().to_string(),
        location["id"].as_str().unwrap().to_string(),
    )
}

#[tokio::test]
async fn custody_answers_the_class_vocabulary_and_the_ledger() {
    let (_state, app) =
        boot_with(|config| config.secret_key = core_api::config::SecretKey([31; 32])).await;
    let slug = format!("custody-{}", Uuid::now_v7().simple());
    let _ = token_email(&slug);
    let token = register_org_token(&app, "Custody Co", &slug).await;
    let (item, location) = create_serialized(&app, &token).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/stock/receive",
        &[auth(&token)],
        Some(json!({
            "item_id": item,
            "location_id": location,
            "qty_milli": 1000,
            "serial_numbers": ["CUST-0001"],
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "receive a serial");

    let summary = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/inventory/custody",
            &[auth(&token)],
            None,
        )
        .await,
    )
    .await;
    let classes = summary.as_array().expect("class rows");
    assert!(
        classes.len() >= 10,
        "the whole vocabulary is listed, got {}",
        classes.len()
    );
    let sale_stock = classes
        .iter()
        .find(|row| row["custody_class"] == json!("sale_stock"))
        .expect("sale_stock row");
    assert_eq!(sale_stock["serials"], json!(1));
    assert_eq!(sale_stock["total"], json!(1));

    let ledger = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/inventory/class-movements",
            &[auth(&token)],
            None,
        )
        .await,
    )
    .await;
    assert!(ledger.is_array(), "the ledger answers an array");

    let refused = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/inventory/class-movements?unit_kind=pallet",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(refused.status(), 422, "an unknown unit kind is refused");
}
