//! The accountant package and its class→account mapping through the REAL
//! router.
//!
//! Pins what the handoff promises:
//! 1. Every class has an account before the org chooses one, so a package
//!    parses without a mapping meeting.
//! 2. A chosen account replaces the default and the package uses it.
//! 3. The package answers a period with its totals and stable references.
//! 4. An inverted period is a typed 422.

use crate::common::{auth, body_json, boot_with, register_org_token, send_headers as send};
use axum::Router;
use serde_json::{json, Value};
use uuid::Uuid;

async fn get(app: &Router, token: &str, path: &str) -> (u16, Value) {
    let response = send(app, axum::http::Method::GET, path, &[auth(token)], None).await;
    let status = response.status().as_u16();
    (status, body_json(response).await)
}

#[tokio::test]
async fn a_package_resolves_every_class_without_a_mapping_meeting() {
    let (_state, app) =
        boot_with(|config| config.secret_key = core_api::config::SecretKey([37; 32])).await;
    let slug = format!("acct-pkg-{}", Uuid::now_v7().simple());
    let token = register_org_token(&app, "Package Co", &slug).await;

    let (status, accounts) = get(&app, &token, "/api/v1/accounting/class-accounts").await;
    assert_eq!(status, 200);
    let classes: Vec<&str> = accounts
        .as_array()
        .expect("class rows")
        .iter()
        .map(|row| row["reporting_class"].as_str().unwrap())
        .collect();
    for class in ["inventory", "sales", "freight_in", "service_cost", "other"] {
        assert!(classes.contains(&class), "the default map has {class}");
    }
    let freight = accounts
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["reporting_class"] == json!("freight_in"))
        .expect("freight row");
    assert_eq!(freight["account_code"], json!("5100"));

    // Choose a different account and read it back.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/accounting/class-accounts",
        &[auth(&token)],
        Some(json!({
            "reporting_class": "freight_in",
            "account_code": "6100",
            "account_name": "Inbound freight",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "set a class account");

    let (status, package) = get(
        &app,
        &token,
        "/api/v1/accounting/package?from=2026-01-01&to=2026-12-31",
    )
    .await;
    assert_eq!(status, 200);
    assert!(package["base_currency"]
        .as_str()
        .is_some_and(|c| !c.is_empty()));
    let mapped = package["class_accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["reporting_class"] == json!("freight_in"))
        .expect("freight in the package map");
    assert_eq!(mapped["account_code"], json!("6100"));
    assert!(package["records"].is_array());
    assert!(package["totals_by_account"].is_array());
    assert!(package["totals_by_kind"].is_array());

    let (status, _) = get(
        &app,
        &token,
        "/api/v1/accounting/package?from=2026-12-31&to=2026-01-01",
    )
    .await;
    assert_eq!(status, 422, "an inverted period is refused");
}
