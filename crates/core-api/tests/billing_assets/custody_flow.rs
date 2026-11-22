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

    // An empty organisation still answers the whole vocabulary, with zeroes.
    let empty = body_json(
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
    let fixture_classes: Vec<String> = empty
        .as_array()
        .expect("class rows")
        .iter()
        .map(|row| row["custody_class"].as_str().unwrap().to_string())
        .collect();
    for class in [
        "sale_stock",
        "internal_fleet",
        "rental_in",
        "rental_out",
        "client_custody",
        "ewaste_custody",
        "tools",
        "sold",
        "recycled",
        "destroyed",
        "returned",
    ] {
        assert!(
            fixture_classes.iter().any(|c| c == class),
            "the vocabulary lists {class}"
        );
    }
    assert!(empty
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["total"] == json!(0)));

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

    // Two class movements: a loan takes the serial out of sale stock, and the
    // return puts it back.
    let client = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/clients",
        &[auth(&token)],
        Some(json!({
            "name": "Acme",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(client.status(), 200, "client");
    let client: Value = body_json(client).await;
    let serial_id = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/v1/stock-items/{item}/serials"),
            &[auth(&token)],
            None,
        )
        .await,
    )
    .await[0]["id"]
        .as_str()
        .expect("serial id")
        .to_string();
    let loan = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/client-loans",
        &[auth(&token)],
        Some(json!({
            "client_id": client["id"],
            "expected_return_at": (chrono::Utc::now() + chrono::Duration::days(7))
                .date_naive()
                .to_string(),
            "items": [{ "stock_item_id": item, "serial_id": serial_id }],
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    let status = loan.status();
    let loan: Value = body_json(loan).await;
    assert_eq!(status, 200, "issue a loan: {loan}");
    let line_id = loan["lines"][0]["id"]
        .as_str()
        .expect("loan line id")
        .to_string();
    let back = send(
        &app,
        axum::http::Method::POST,
        &format!(
            "/api/v1/client-loans/{}/return",
            loan["loan"]["id"].as_str().unwrap()
        ),
        &[auth(&token)],
        Some(json!({
            "to_location_id": location,
            "lines": [{ "line_id": line_id, "return_condition": "good" }],
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(back.status(), 200, "return the loan");

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
    let rows = ledger.as_array().expect("the ledger answers an array");
    assert!(
        rows.len() >= 2,
        "the loan wrote two movements, got {}",
        rows.len()
    );
    assert!(
        rows[0]["created_at"].as_str() >= rows[1]["created_at"].as_str(),
        "newest first"
    );
    assert_eq!(rows[0]["unit_kind"], json!("serial"));
    assert_eq!(rows[0]["to_class"], json!("sale_stock"));
    assert_eq!(rows[0]["from_class"], json!("rental_out"));

    let only_assets = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/inventory/class-movements?unit_kind=asset",
            &[auth(&token)],
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        only_assets.as_array().expect("array").len(),
        0,
        "no asset moves"
    );

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
