//! The accountant package and its class→account mapping through the REAL
//! router.
//!
//! Pins what the handoff promises:
//! 1. Every class has an account before the org chooses one, so a package
//!    parses without a mapping meeting — except `cogs`, which holds.
//! 2. A chosen account replaces the default and the package uses it.
//! 3. The package answers a period with its totals and stable references.
//! 4. An inverted period is a typed 422.
//! 5. Vendor bills, stock valuation and e-waste dispositions are in the
//!    package, and an unmapped class holds it with a typed 409 naming the
//!    classes instead of posting to a guessed account.

use crate::common::{auth, body_json, boot_with, register_org_token, send_headers as send};
use axum::Router;
use serde_json::{json, Value};
use uuid::Uuid;

async fn get(app: &Router, token: &str, path: &str) -> (u16, Value) {
    let response = send(app, axum::http::Method::GET, path, &[auth(token)], None).await;
    let status = response.status().as_u16();
    (status, body_json(response).await)
}

/// A mutation the test expects to be accepted; prints the body when it is not.
async fn mutate(
    app: &Router,
    token: &str,
    method: axum::http::Method,
    path: &str,
    body: Value,
) -> (u16, Value) {
    let response = send(app, method, path, &[auth(token)], Some(body)).await;
    let status = response.status().as_u16();
    (status, body_json(response).await)
}

fn key() -> String {
    Uuid::now_v7().to_string()
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
    for class in [
        "inventory",
        "sales",
        "freight_in",
        "duty",
        "service_cost",
        "rental_expense",
        "ewaste_cost",
        "overhead",
        "other",
    ] {
        assert!(classes.contains(&class), "the default map has {class}");
    }
    assert!(
        !classes.contains(&"cogs"),
        "cost of goods sold has no default; it holds until the org maps it"
    );
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

/// Places an order for one stock line, receives it in full, closes the order,
/// then records and posts its bill. Returns the purchase order id.
#[allow(clippy::too_many_arguments)]
async fn receive_and_bill(
    app: &Router,
    token: &str,
    vendor_id: &str,
    item_id: &str,
    location_id: &str,
    acquired_for: &str,
    qty_milli: i64,
    unit_cost_minor: i64,
    bill_number: &str,
) -> String {
    let (status, created) = mutate(
        app,
        token,
        axum::http::Method::POST,
        "/api/v1/purchase-orders",
        json!({
            "vendor_id": vendor_id,
            "acquired_for": acquired_for,
            "lines": [{
                "stock_item_id": item_id,
                "description": format!("Line for {acquired_for}"),
                "qty_ordered_milli": qty_milli,
                "unit_cost_minor": unit_cost_minor,
            }],
            "idempotency_key": key(),
        }),
    )
    .await;
    assert_eq!(status, 200, "create purchase order: {created}");
    let order_id = created["order"]["id"]
        .as_str()
        .expect("order id")
        .to_string();
    let line_id = created["lines"][0]["id"]
        .as_str()
        .expect("order line id")
        .to_string();

    let (status, placed) = mutate(
        app,
        token,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order_id}/order"),
        json!({ "idempotency_key": key() }),
    )
    .await;
    assert_eq!(status, 200, "place order: {placed}");

    let (status, received) = mutate(
        app,
        token,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order_id}/receive"),
        json!({
            "location_id": location_id,
            "lines": [{ "line_id": line_id, "qty_milli": qty_milli }],
            "idempotency_key": key(),
        }),
    )
    .await;
    assert_eq!(status, 200, "receive: {received}");

    let (status, closed) = mutate(
        app,
        token,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order_id}/close"),
        json!({ "idempotency_key": key() }),
    )
    .await;
    assert_eq!(status, 200, "close: {closed}");

    let (status, bill) = mutate(
        app,
        token,
        axum::http::Method::PUT,
        &format!("/api/v1/purchase-orders/{order_id}/bill"),
        json!({ "bill_number": bill_number, "idempotency_key": key() }),
    )
    .await;
    assert_eq!(status, 200, "record bill: {bill}");

    let (status, posted) = mutate(
        app,
        token,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order_id}/bill/post"),
        json!({ "idempotency_key": key() }),
    )
    .await;
    assert_eq!(status, 200, "post bill: {posted}");
    order_id
}

#[tokio::test]
async fn a_package_carries_bills_valuation_and_ewaste_and_holds_on_an_unmapped_class() {
    let (_state, app) =
        boot_with(|config| config.secret_key = core_api::config::SecretKey([38; 32])).await;
    let slug = format!("acct-kinds-{}", Uuid::now_v7().simple());
    let token = register_org_token(&app, "Handoff Co", &slug).await;

    // A client for the e-waste custody.
    let (status, client) = mutate(
        &app,
        &token,
        axum::http::Method::POST,
        "/api/v1/clients",
        json!({ "name": "Birch Legal", "idempotency_key": key() }),
    )
    .await;
    assert_eq!(status, 200, "create client: {client}");
    let client_id = client["id"].as_str().expect("client id").to_string();

    let (status, vendor) = mutate(
        &app,
        &token,
        axum::http::Method::POST,
        "/api/v1/vendors",
        json!({
            "name": "Parts Depot",
            "default_currency": "USD",
            "idempotency_key": key(),
        }),
    )
    .await;
    assert_eq!(status, 200, "create vendor: {vendor}");
    let vendor_id = vendor["id"].as_str().expect("vendor id").to_string();

    let (status, item) = mutate(
        &app,
        &token,
        axum::http::Method::POST,
        "/api/v1/stock-items",
        json!({
            "name": "Patch cable box",
            "kind": "consumable",
            "unit": "each",
            "currency": "USD",
            "idempotency_key": key(),
        }),
    )
    .await;
    assert_eq!(status, 200, "create stock item: {item}");
    let item_id = item["id"].as_str().expect("item id").to_string();

    let (status, location) = mutate(
        &app,
        &token,
        axum::http::Method::POST,
        "/api/v1/stock-locations",
        json!({ "name": "Warehouse", "kind": "warehouse", "idempotency_key": key() }),
    )
    .await;
    assert_eq!(status, 200, "create location: {location}");
    let location_id = location["id"].as_str().expect("location id").to_string();

    // Ten units into stock at 1500, then five more at 2000: the last receipt is
    // the cost basis, so fifteen units value at 15 × 2000.
    receive_and_bill(
        &app,
        &token,
        &vendor_id,
        &item_id,
        &location_id,
        "sale_stock",
        10_000,
        1_500,
        "VB-1001",
    )
    .await;
    // A consumable bought for a project reports as cost of goods sold, whose
    // account the org has to name.
    receive_and_bill(
        &app,
        &token,
        &vendor_id,
        &item_id,
        &location_id,
        "project_consumable",
        5_000,
        2_000,
        "VB-1002",
    )
    .await;

    // A disposed lot: the custody fact, at zero money.
    let today = chrono::Utc::now().date_naive().to_string();
    let (status, intake) = mutate(
        &app,
        &token,
        axum::http::Method::POST,
        "/api/v1/ewaste/intakes",
        json!({
            "client_id": client_id,
            "reference": "EW-126",
            "received_by_name": "Dana",
            "idempotency_key": key(),
        }),
    )
    .await;
    assert_eq!(status, 200, "create intake: {intake}");
    let intake_id = intake["intake"]["id"]
        .as_str()
        .expect("intake id")
        .to_string();
    let (status, lined) = mutate(
        &app,
        &token,
        axum::http::Method::POST,
        &format!("/api/v1/ewaste/intakes/{intake_id}/lines"),
        json!({
            "line_kind": "lot",
            "category": "Laptops",
            "weight_g": 2_500,
            "quantity": 2,
            "idempotency_key": key(),
        }),
    )
    .await;
    assert_eq!(status, 200, "add line: {lined}");
    let line_id = lined["lines"][0]["id"]
        .as_str()
        .expect("intake line id")
        .to_string();
    let (status, disposed) = mutate(
        &app,
        &token,
        axum::http::Method::POST,
        &format!("/api/v1/ewaste/intakes/{intake_id}/dispositions"),
        json!({
            "line_id": line_id,
            "pathway": "recycled",
            "partner_name": "Recycle Co",
            "disposed_at": today,
            "weight_g": 2_500,
            "certificate_reference": "CERT-126",
            "idempotency_key": key(),
        }),
    )
    .await;
    assert_eq!(status, 200, "record disposition: {disposed}");

    // `cogs` has no default, so the package holds and names the class rather
    // than posting a real period to a guessed account.
    let (status, held) = get(
        &app,
        &token,
        "/api/v1/accounting/package?from=2020-01-01&to=2035-12-31",
    )
    .await;
    assert_eq!(status, 409, "the package holds: {held}");
    assert_eq!(held["error"]["reason"], json!("account_mapping_missing"));
    assert_eq!(held["error"]["classes"], json!(["cogs"]));

    // The CSV form is the same period and the same hold.
    let held_csv = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/accounting/package.csv?from=2020-01-01&to=2035-12-31",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(held_csv.status().as_u16(), 409, "the CSV form holds too");
    let held_csv = body_json(held_csv).await;
    assert_eq!(
        held_csv["error"]["reason"],
        json!("account_mapping_missing")
    );

    // Map it, then the same period hands over cleanly.
    let (status, mapped) = mutate(
        &app,
        &token,
        axum::http::Method::PUT,
        "/api/v1/accounting/class-accounts",
        json!({
            "reporting_class": "cogs",
            "account_code": "7000",
            "account_name": "Cost of goods sold",
        }),
    )
    .await;
    assert_eq!(status, 200, "map cogs: {mapped}");

    let (status, package) = get(
        &app,
        &token,
        "/api/v1/accounting/package?from=2020-01-01&to=2035-12-31",
    )
    .await;
    assert_eq!(status, 200, "the mapped package: {package}");
    let records = package["records"].as_array().expect("records");

    let bills: Vec<&Value> = records
        .iter()
        .filter(|r| r["kind"] == json!("vendor_bill"))
        .collect();
    assert_eq!(bills.len(), 2, "both posted bills are in the package");
    let goods = bills
        .iter()
        .find(|r| r["reporting_class"] == json!("inventory"))
        .expect("the stock bill reports as inventory");
    assert_eq!(goods["account_code"], json!("1200"));
    assert_eq!(goods["amount_minor"], json!(15_000));
    assert_eq!(goods["document_ref"], json!("VB-1001"));
    assert!(goods["stable_ref"]
        .as_str()
        .is_some_and(|s| s.starts_with("purchase_bill:")));
    let project = bills
        .iter()
        .find(|r| r["reporting_class"] == json!("cogs"))
        .expect("the project consumable reports as cogs");
    assert_eq!(project["account_code"], json!("7000"));

    let valuation = records
        .iter()
        .find(|r| r["kind"] == json!("stock_item"))
        .expect("a stock valuation row");
    assert_eq!(valuation["reporting_class"], json!("inventory"));
    assert_eq!(valuation["account_code"], json!("1200"));
    assert_eq!(valuation["currency"], json!("USD"));
    assert_eq!(
        valuation["amount_minor"],
        json!(30_000),
        "15 units at the last receipt cost of 2000"
    );
    assert_eq!(valuation["occurred_on"], json!("2035-12-31"));
    assert!(valuation["stable_ref"]
        .as_str()
        .is_some_and(|s| s.starts_with("stock_item:")));

    let ewaste = records
        .iter()
        .find(|r| r["kind"] == json!("ewaste_disposition"))
        .expect("an e-waste disposition row");
    assert_eq!(ewaste["reporting_class"], json!("ewaste_cost"));
    assert_eq!(ewaste["account_code"], json!("5500"));
    assert_eq!(ewaste["amount_minor"], json!(0));
    assert_eq!(ewaste["client_id"], json!(client_id));
    assert_eq!(ewaste["document_ref"], json!("CERT-126"));
    assert!(ewaste["memo"]
        .as_str()
        .is_some_and(|m| m.contains("recycled") && m.contains("2500 g")));
    assert!(ewaste["stable_ref"]
        .as_str()
        .is_some_and(|s| s.starts_with("ewaste_disposition:")));

    let inventory_total = package["totals_by_account"]
        .as_array()
        .expect("account totals")
        .iter()
        .find(|t| t["account_code"] == json!("1200"))
        .expect("inventory total");
    assert_eq!(inventory_total["base_total_minor"], json!(45_000));

    // The CSV form carries the same rows, the same references and the same
    // account choices as the JSON package.
    let csv_response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/accounting/package.csv?from=2020-01-01&to=2035-12-31",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(csv_response.status().as_u16(), 200, "the CSV package");
    let content_type = csv_response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(content_type.starts_with("text/csv"), "{content_type}");
    let bytes = axum::body::to_bytes(csv_response.into_body(), usize::MAX)
        .await
        .expect("csv body");
    let csv = String::from_utf8(bytes.to_vec()).expect("utf-8 csv");
    assert!(csv.starts_with(
        "kind,stable_ref,document_ref,occurred_on,client_id,currency,amount_minor,\
         base_amount_minor,reporting_class,account_code,memo\n"
    ));
    assert_eq!(
        csv.lines().count(),
        1 + records.len(),
        "one line per record"
    );
    assert!(csv.contains("purchase_bill:"));
    assert!(csv.contains("ewaste_disposition:"));
    assert!(
        csv.lines().any(|line| line.starts_with("vendor_bill,")
            && line.contains(",cogs,")
            && line.contains(",7000,")),
        "the mapped cogs row is in the CSV"
    );
}
