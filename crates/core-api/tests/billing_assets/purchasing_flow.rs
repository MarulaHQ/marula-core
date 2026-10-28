//! Purchasing + vendors flow (Wave 32 lane 2, migration 0103) through the
//! REAL core-api router.
//!
//! Verifies the contract pieces downstream lanes + Wave 33 consume:
//! 1. The PO lifecycle: draft → ordered → partially_received → received →
//!    closed | cancelled, with wrong moves refused as typed 409s.
//! 2. Receiving posts ONE lane-1 `stock_movements` row per line at the
//!    chosen location and stamps `movement_id` on the receipt line — the
//!    ledger stays the traceability record (on-hand moves accordingly).
//! 3. Typed receipt states, never silent: over-receipt refused (409)
//!    without `allow_over_receipt`, then persisted as `receipt_state=over`;
//!    under-receipt closes with `close_kind=short`.
//! 4. Serialized lines post one ledger row + one receipt line per serial.
//! 5. Reseller deeplinks resolve per vendor / PO / line, https-only.
//! 6. The bill hook: recorded, posted (ADR 0002 lock), and rolled into the
//!    project cost rollup.
//! 7. Internal-org assets ride the DeviceAsset lifecycle with client_id
//!    NULL (scope=internal_org), never a fake client.
//! 8. Cross-org isolation: another org's reads come back empty.
//!
//! Database selection follows stock_flow's contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use crate::common::{auth, body_json, send_headers as send, state_with};
use axum::Router;
use core_api::state::AppState;
use serde_json::{json, Value};
use uuid::Uuid;

async fn test_state() -> AppState {
    state_with(|config| config.secret_key = core_api::config::SecretKey([23; 32])).await
}

async fn register_org(app: &Router, slug: &str) -> String {
    crate::common::register(
        app,
        json!({
            "org_name": "Purchasing Co",
            "org_slug": slug,
            "email": format!("owner+ws32b+{slug}@purchasing.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        }),
    )
    .await["access_token"]
        .as_str()
        .expect("access token")
        .into()
}

fn key() -> String {
    Uuid::now_v7().to_string()
}

fn id_of(value: &Value, pointer: &str) -> Uuid {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing id at {pointer}: {value}"))
        .parse()
        .expect("uuid")
}

/// Creates a consumable stock item and a warehouse; returns (item, location).
async fn seed_stock(app: &Router, token: &str, name: &str) -> (Uuid, Uuid) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/stock-items",
        &[auth(token)],
        Some(json!({
            "name": name,
            "kind": "consumable",
            "unit": "each",
            "currency": "USD",
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "create stock item");
    let item_id = id_of(&body_json(response).await, "/id");
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/stock-locations",
        &[auth(token)],
        Some(json!({
            "name": format!("Warehouse {name}"),
            "kind": "warehouse",
            "code": "WH1",
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "create warehouse");
    let location_id = id_of(&body_json(response).await, "/id");
    (item_id, location_id)
}

#[allow(clippy::too_many_arguments)]
async fn create_order(
    app: &Router,
    token: &str,
    vendor_id: Uuid,
    project_id: Option<Uuid>,
    lines: Value,
) -> (Uuid, Value) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/purchase-orders",
        &[auth(token)],
        Some(json!({
            "vendor_id": vendor_id,
            "project_id": project_id,
            "lines": lines,
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "create purchase order");
    let body = body_json(response).await;
    (id_of(&body, "/order/id"), body)
}

#[tokio::test]
async fn purchasing_lifecycle_partial_over_under_receipt_rollup_and_deeplinks() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("purch-{}", &Uuid::now_v7().simple().to_string()[..12]);
    let token = register_org(&app, &slug).await;

    // A project with a budget (the bill rolls against it).
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/projects",
        &[auth(&token)],
        Some(json!({
            "name": "Network refresh",
            "budget_minor": 500_000,
            "currency": "USD",
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "create project");
    let project_id = id_of(&body_json(response).await, "/id");

    // Vendor with a typed contact + an order-portal deeplink.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/vendors",
        &[auth(&token)],
        Some(json!({
            "name": "CDW Canada",
            "code": "CDW",
            "email": "orders@cdw.test",
            "account_number": "ACCT-7788",
            "default_currency": "USD",
            "contacts": [
                { "name": "Sales desk", "email": "sales@cdw.test", "is_primary": true }
            ],
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "create vendor");
    let vendor = body_json(response).await;
    let vendor_id = id_of(&vendor, "/id");
    assert_eq!(vendor["code"], "CDW");

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/vendors/{vendor_id}"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "vendor detail");
    let detail = body_json(response).await;
    assert_eq!(detail["contacts"].as_array().unwrap().len(), 1);
    assert_eq!(detail["contacts"][0]["name"], "Sales desk");

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/vendors/{vendor_id}/links"),
        &[auth(&token)],
        Some(json!({
            "kind": "order_portal",
            "url": "https://reseller.cdw.test/orders",
            "label": "CDW order desk",
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "vendor portal link");

    // http:// is rejected: deeplinks must be https.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/vendors/{vendor_id}/links"),
        &[auth(&token)],
        Some(json!({
            "kind": "order_portal",
            "url": "http://reseller.cdw.test/orders",
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 422, "http deeplink refused");

    let (item_a, warehouse) = seed_stock(&app, &token, "Patch cable box").await;
    let (item_b, _) = seed_stock(&app, &token, "Keystone jacks").await;

    // PO with two lines: A 10 boxes @ 1200, B 5 boxes @ 2000 (milli).
    let (order_id, created) = create_order(
        &app,
        &token,
        vendor_id,
        Some(project_id),
        json!([
            { "stock_item_id": item_a, "description": "Patch cable box", "qty_ordered_milli": 10_000, "unit_cost_minor": 1_200 },
            { "stock_item_id": item_b, "description": "Keystone jacks", "qty_ordered_milli": 5_000, "unit_cost_minor": 2_000 },
        ]),
    )
    .await;
    assert_eq!(created["order"]["status"], "draft");
    assert!(!created["order"]["number"].as_str().unwrap().is_empty());
    assert_eq!(created["totals"]["ordered_minor"], 22_000);
    assert_eq!(created["order"]["project_id"], project_id.to_string());
    let line_a = id_of(&created, "/lines/0/id");
    let line_b = id_of(&created, "/lines/1/id");

    // Per-line tracking link (the parcel page for line A) + a PO-level link.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order_id}/links"),
        &[auth(&token)],
        Some(json!({
            "kind": "tracking",
            "url": "https://tracking.ups.test/1Z999",
            "carrier": "UPS",
            "line_id": line_a,
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "line tracking link");
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order_id}/links"),
        &[auth(&token)],
        Some(json!({
            "kind": "order_portal",
            "url": "https://reseller.cdw.test/orders/88213",
            "vendor_reference": "CDW-88213",
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "order portal link");

    // Deeplinks resolve vendor → PO → line, in that order.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/purchase-orders/{order_id}/deeplinks"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "deeplinks");
    let deeplinks = body_json(response).await;
    let links = deeplinks["links"].as_array().unwrap();
    assert_eq!(links.len(), 3);
    assert_eq!(links[0]["subject"], "vendor");
    assert_eq!(links[1]["subject"], "purchase_order");
    assert_eq!(links[2]["subject"], "purchase_order_line");
    assert_eq!(links[2]["line_id"], line_a.to_string());
    assert_eq!(links[2]["carrier"], "UPS");
    assert_eq!(deeplinks["vendor"]["account_number"], "ACCT-7788");

    // Draft cannot receive, only draft lines are editable.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order_id}/receive"),
        &[auth(&token)],
        Some(json!({
            "location_id": warehouse,
            "lines": [{ "line_id": line_a, "qty_milli": 1_000 }],
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 409, "draft cannot receive");

    // Place the order.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order_id}/order"),
        &[auth(&token)],
        Some(json!({ "idempotency_key": key() })),
    )
    .await;
    assert_eq!(response.status(), 200, "order it");
    let placed = body_json(response).await;
    assert_eq!(placed["order"]["status"], "ordered");

    // Partial receipt: A 4000, B 5000 → partially_received; A partial,
    // B complete; one ledger row per receipt line at the warehouse.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order_id}/receive"),
        &[auth(&token)],
        Some(json!({
            "location_id": warehouse,
            "carrier": "UPS",
            "tracking_ref": "1Z999",
            "lines": [
                { "line_id": line_a, "qty_milli": 4_000 },
                { "line_id": line_b, "qty_milli": 5_000 }
            ],
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "partial receipt");
    let received = body_json(response).await;
    assert_eq!(received["order"]["order"]["status"], "partially_received");
    assert_eq!(received["order"]["lines"][0]["receipt_state"], "partial");
    assert_eq!(received["order"]["lines"][1]["receipt_state"], "complete");
    let receipt_lines = received["receipt"]["lines"].as_array().unwrap();
    assert_eq!(receipt_lines.len(), 2);
    assert!(
        receipt_lines.iter().all(|l| l["movement_id"].is_string()),
        "every receipt line stamps its ledger movement"
    );

    // The ledger knows: on-hand at the warehouse = 4000 of item A.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/stock/on-hand?item_id={item_a}"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "on-hand read");
    let on_hand = body_json(response).await;
    let rows = on_hand.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["qty_milli"], 4_000);

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/stock-items/{item_a}/movements"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "movement ledger read");
    let movements = body_json(response).await;
    let mv = &movements.as_array().unwrap()[0];
    assert_eq!(mv["kind"], "receive");
    assert_eq!(mv["qty_milli"], 4_000);
    assert_eq!(mv["to_location_id"], warehouse.to_string());
    assert_eq!(mv["carrier"], "UPS");

    // Over-receipt without acknowledgement is a typed 409…
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order_id}/receive"),
        &[auth(&token)],
        Some(json!({
            "location_id": warehouse,
            "lines": [{ "line_id": line_a, "qty_milli": 7_000 }],
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 409, "over-receipt refused without ack");

    // …and lands as the typed `over` state when acknowledged.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order_id}/receive"),
        &[auth(&token)],
        Some(json!({
            "location_id": warehouse,
            "allow_over_receipt": true,
            "lines": [{ "line_id": line_a, "qty_milli": 7_000 }],
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "over-receipt accepted with ack");
    let over = body_json(response).await;
    assert_eq!(over["order"]["order"]["status"], "received");
    assert_eq!(over["order"]["lines"][0]["receipt_state"], "over");
    // Received total: A 11_000 milli × 1200 = 13_200; B 5_000 × 2000 = 10_000.
    assert_eq!(over["order"]["totals"]["received_minor"], 23_200);

    // Close: every line complete/over → full close.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order_id}/close"),
        &[auth(&token)],
        Some(json!({ "idempotency_key": key() })),
    )
    .await;
    assert_eq!(response.status(), 200, "close");
    let closed = body_json(response).await;
    assert_eq!(closed["order"]["status"], "closed");
    assert_eq!(closed["order"]["close_kind"], "full");

    // Bill: recorded with the received total, then posted (rollup moment).
    let response = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/v1/purchase-orders/{order_id}/bill"),
        &[auth(&token)],
        Some(json!({
            "bill_number": "INV-55621",
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "record bill");
    let bill = body_json(response).await;
    assert_eq!(bill["bill"]["amount_minor"], 23_200);
    assert!(bill["bill"]["posted_at"].is_null());

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order_id}/bill/post"),
        &[auth(&token)],
        Some(json!({ "idempotency_key": key() })),
    )
    .await;
    assert_eq!(response.status(), 200, "post bill");
    let posted = body_json(response).await;
    assert!(!posted["bill"]["posted_at"].is_null());

    // The project cost rollup carries the posted bill.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/projects/{project_id}/cost-rollup"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "cost rollup");
    let rollup = body_json(response).await;
    assert_eq!(rollup["po_bill_base_minor"], 23_200);
    assert_eq!(rollup["budget_minor"], 500_000);
    assert_eq!(rollup["bills"].as_array().unwrap().len(), 1);

    // Under-receipt: close with unreceived quantity → close_kind = short.
    let (order2, _) = create_order(
        &app,
        &token,
        vendor_id,
        None,
        json!([
            { "stock_item_id": item_a, "description": "Patch cable box", "qty_ordered_milli": 10_000, "unit_cost_minor": 1_200 },
        ]),
    )
    .await;
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order2}/order"),
        &[auth(&token)],
        Some(json!({ "idempotency_key": key() })),
    )
    .await;
    assert_eq!(response.status(), 200, "order 2");
    let body = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/v1/purchase-orders/{order2}"),
            &[auth(&token)],
            None,
        )
        .await,
    )
    .await;
    let line2 = id_of(&body, "/lines/0/id");
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order2}/receive"),
        &[auth(&token)],
        Some(json!({
            "location_id": warehouse,
            "lines": [{ "line_id": line2, "qty_milli": 6_000 }],
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "under receipt");
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order2}/close"),
        &[auth(&token)],
        Some(json!({ "idempotency_key": key() })),
    )
    .await;
    assert_eq!(response.status(), 200, "close short");
    let short = body_json(response).await;
    assert_eq!(short["order"]["status"], "closed");
    assert_eq!(short["order"]["close_kind"], "short");

    // An order that received goods cannot be cancelled (close short is
    // the honest move) and a cancelled order is terminal.
    let (order3, _) = create_order(
        &app,
        &token,
        vendor_id,
        None,
        json!([
            { "stock_item_id": item_a, "description": "Patch cable box", "qty_ordered_milli": 1_000, "unit_cost_minor": 1_200 },
        ]),
    )
    .await;
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order3}/cancel"),
        &[auth(&token)],
        Some(json!({ "reason": "duplicate", "idempotency_key": key() })),
    )
    .await;
    assert_eq!(response.status(), 200, "cancel a draft");
    let cancelled = body_json(response).await;
    assert_eq!(cancelled["order"]["status"], "cancelled");
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order3}/order"),
        &[auth(&token)],
        Some(json!({ "idempotency_key": key() })),
    )
    .await;
    assert_eq!(response.status(), 409, "cancelled is terminal");

    // Cross-org isolation: a second org sees none of this.
    let slug2 = format!("purch2-{}", &Uuid::now_v7().simple().to_string()[..12]);
    let token2 = register_org(&app, &slug2).await;
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/vendors",
        &[auth(&token2)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "cross-org vendor list");
    assert!(body_json(response).await.as_array().unwrap().is_empty());
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/purchase-orders/{order_id}"),
        &[auth(&token2)],
        None,
    )
    .await;
    assert_eq!(response.status(), 404, "cross-org order is invisible");
}

#[tokio::test]
async fn serialized_po_receipt_posts_one_ledger_row_per_serial() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("purchs-{}", &Uuid::now_v7().simple().to_string()[..12]);
    let token = register_org(&app, &slug).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/vendors",
        &[auth(&token)],
        Some(json!({ "name": "Ingram Micro", "idempotency_key": key() })),
    )
    .await;
    assert_eq!(response.status(), 200, "vendor");
    let vendor_id = id_of(&body_json(response).await, "/id");

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/stock-items",
        &[auth(&token)],
        Some(json!({
            "name": "SSD 2.5\" 1TB",
            "kind": "serialized",
            "unit": "each",
            "currency": "USD",
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "serialized item");
    let item_id = id_of(&body_json(response).await, "/id");

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/stock-locations",
        &[auth(&token)],
        Some(json!({
            "name": "Bench shelf",
            "kind": "shelf",
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "shelf");
    let location_id = id_of(&body_json(response).await, "/id");

    let (order_id, created) = create_order(
        &app,
        &token,
        vendor_id,
        None,
        json!([
            { "stock_item_id": item_id, "description": "SSD 1TB", "qty_ordered_milli": 2_000, "unit_cost_minor": 8_500 },
        ]),
    )
    .await;
    let line_id = id_of(&created, "/lines/0/id");
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order_id}/order"),
        &[auth(&token)],
        Some(json!({ "idempotency_key": key() })),
    )
    .await;
    assert_eq!(response.status(), 200, "order it");

    // Quantity must equal 1000 × serial count.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order_id}/receive"),
        &[auth(&token)],
        Some(json!({
            "location_id": location_id,
            "lines": [{
                "line_id": line_id,
                "qty_milli": 1_000,
                "serial_numbers": ["SN-A", "SN-B"]
            }],
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 422, "qty must match the serial count");

    // Duplicate serials in one receipt are refused.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order_id}/receive"),
        &[auth(&token)],
        Some(json!({
            "location_id": location_id,
            "lines": [{
                "line_id": line_id,
                "qty_milli": 2_000,
                "serial_numbers": ["SN-A", "sn-a"]
            }],
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 422, "duplicate serial refused");

    // The real receipt: two units → two ledger rows, two receipt lines.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/purchase-orders/{order_id}/receive"),
        &[auth(&token)],
        Some(json!({
            "location_id": location_id,
            "lines": [{
                "line_id": line_id,
                "qty_milli": 2_000,
                "serial_numbers": ["SN-A", "SN-B"]
            }],
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "serialized receipt");
    let body = body_json(response).await;
    assert_eq!(body["order"]["order"]["status"], "received");
    assert_eq!(body["order"]["lines"][0]["receipt_state"], "complete");
    let receipt_lines = body["receipt"]["lines"].as_array().unwrap();
    assert_eq!(receipt_lines.len(), 2);
    assert!(receipt_lines.iter().all(|l| l["qty_milli"] == 1_000));
    let movement_ids: std::collections::HashSet<&str> = receipt_lines
        .iter()
        .map(|l| l["movement_id"].as_str().unwrap())
        .collect();
    assert_eq!(movement_ids.len(), 2, "one distinct ledger row per serial");

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/stock-items/{item_id}/serials"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "serials read");
    let serials = body_json(response).await;
    assert_eq!(serials.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn internal_org_assets_ride_the_device_asset_lifecycle() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("purcha-{}", &Uuid::now_v7().simple().to_string()[..12]);
    let token = register_org(&app, &slug).await;

    // The MSP's own gear: scope=internal_org, no client.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/assets",
        &[auth(&token)],
        Some(json!({
            "tag": "NOC-SW-01",
            "name": "NOC core switch",
            "kind": "network",
            "scope": "internal_org",
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "internal-org asset create");
    let asset = body_json(response).await;
    assert_eq!(asset["scope"], "internal_org");
    assert!(asset["client_id"].is_null(), "no fake client record");
    let asset_id = id_of(&asset, "/id");

    // The lifecycle seam still applies (name/status patch).
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/assets/{asset_id}"),
        &[auth(&token)],
        Some(json!({ "status": "maintenance", "idempotency_key": key() })),
    )
    .await;
    assert_eq!(response.status(), 200, "asset lifecycle patch");

    // Scope coherence is a typed 422, not a constraint crash.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/assets",
        &[auth(&token)],
        Some(json!({
            "tag": "NOC-SW-02",
            "name": "NOC spare switch",
            "scope": "internal_org",
            "client_id": Uuid::now_v7().to_string(),
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        422,
        "internal-org assets carry no client"
    );
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/assets",
        &[auth(&token)],
        Some(json!({
            "tag": "NOC-SW-03",
            "name": "No scope, no client",
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 422, "client scope needs a client");

    // A real client asset still works and the scope filter separates them.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/clients",
        &[auth(&token)],
        Some(json!({ "name": "Acme", "idempotency_key": key() })),
    )
    .await;
    assert_eq!(response.status(), 200, "client");
    let client_id = id_of(&body_json(response).await, "/id");
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/assets",
        &[auth(&token)],
        Some(json!({
            "tag": "ACM-FW-01",
            "name": "Acme firewall",
            "client_id": client_id,
            "idempotency_key": key(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "client asset create");
    let client_asset = body_json(response).await;
    assert_eq!(client_asset["scope"], "client");

    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/assets?scope=internal_org",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "scope filter");
    let internal = body_json(response).await;
    let internal_ids: Vec<String> = internal
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_string())
        .collect();
    assert!(internal_ids.contains(&asset_id.to_string()));
    assert!(!internal_ids.contains(&client_asset["id"].as_str().unwrap().to_string()));

    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/assets?scope=client",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "client scope filter");
    let clients = body_json(response).await;
    let client_ids: Vec<String> = clients
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_string())
        .collect();
    assert!(client_ids.contains(&client_asset["id"].as_str().unwrap().to_string()));
    assert!(!client_ids.contains(&asset_id.to_string()));
}

/// The list carries each order's own totals, and withholds them from a caller
/// who cannot see financials — the same gate the order detail obeys.
///
/// The list is the surface a buyer scans all day, so "no money access" has to
/// read as *withheld* rather than as zero. If this gate ever regressed to
/// "always sum", a technician's screen would quietly start carrying the org's
/// order values.
#[tokio::test]
async fn order_list_totals_follow_the_financials_gate() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("polist-{}", &Uuid::now_v7().simple().to_string()[..12]);
    let token = register_org(&app, &slug).await;

    let vendor = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/vendors",
            &[auth(&token)],
            Some(json!({
                "name": "Gate Vendor",
                "default_currency": "USD",
                "idempotency_key": key(),
            })),
        )
        .await,
    )
    .await;
    let vendor_id = vendor["id"].as_str().unwrap().to_string();

    let order = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/purchase-orders",
            &[auth(&token)],
            Some(json!({
                "vendor_id": vendor_id,
                "number": "PO-GATE-1",
                "currency": "USD",
                "idempotency_key": key(),
            })),
        )
        .await,
    )
    .await;
    let order_id = order["order"]["id"].as_str().unwrap().to_string();

    // Two lines, so the row's total is real money and not a rounding artefact.
    for (description, qty_ordered_milli, unit_cost_minor) in [
        ("Rack rails", 2_000i64, 12_500i64),
        ("Patch panel", 1_000, 40_000),
    ] {
        let response = send(
            &app,
            axum::http::Method::POST,
            &format!("/api/v1/purchase-orders/{order_id}/lines"),
            &[auth(&token)],
            Some(json!({
                "description": description,
                "qty_ordered_milli": qty_ordered_milli,
                "unit_cost_minor": unit_cost_minor,
                "idempotency_key": key(),
            })),
        )
        .await;
        assert_eq!(response.status(), 200, "line {description} must be added");
    }
    let expected_ordered = 2_000 * 12_500 / 1_000 + 1_000 * 40_000 / 1_000;

    // The owner may see financials: the row carries the rollup…
    let list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/purchase-orders",
            &[auth(&token)],
            None,
        )
        .await,
    )
    .await;
    let row = list
        .as_array()
        .expect("a list")
        .iter()
        .find(|row| row["id"].as_str() == Some(order_id.as_str()))
        .expect("the order is in its own list")
        .clone();
    assert_eq!(
        row["totals"]["ordered_minor"].as_i64(),
        Some(expected_ordered),
        "the list row is priced from its lines"
    );
    assert_eq!(
        row["totals"]["received_minor"].as_i64(),
        Some(0),
        "nothing is received yet"
    );
    assert_eq!(
        row["number"].as_str(),
        Some("PO-GATE-1"),
        "the order's own fields stay at the top level — the summary is additive"
    );

    // …and it is the same figure the order's own detail reports.
    let detail = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/v1/purchase-orders/{order_id}"),
            &[auth(&token)],
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        row["totals"]["ordered_minor"].as_i64(),
        detail["totals"]["ordered_minor"].as_i64(),
        "a row's value and the order's own total cannot disagree"
    );

    // A Read Only member may VIEW commercial but holds no financial permission.
    let email = format!("gate-{}@purchasing.test", Uuid::now_v7());
    let invite = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/org/users",
            &[auth(&token)],
            Some(json!({
                "email": email,
                "display_name": "Read Only Member",
                "role": "read_only",
            })),
        )
        .await,
    )
    .await;
    let setup_token = invite["invite"]["setup_token"]
        .as_str()
        .expect("an invite carries a setup token")
        .to_string();
    let accept = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/org/users/accept-invite",
            &[],
            Some(json!({
                "token": setup_token,
                "password": "another-correct-horse-battery",
            })),
        )
        .await,
    )
    .await;
    let limited = accept["access_token"].as_str().unwrap().to_string();

    let limited_list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/purchase-orders",
            &[auth(&limited)],
            None,
        )
        .await,
    )
    .await;
    let limited_row = limited_list
        .as_array()
        .expect("a list")
        .iter()
        .find(|row| row["id"].as_str() == Some(order_id.as_str()))
        .expect("the order is still listed")
        .clone();
    assert_eq!(
        limited_row["number"].as_str(),
        Some("PO-GATE-1"),
        "commercial view is enough to see the order itself"
    );
    assert!(
        limited_row["totals"].is_null(),
        "without ViewFinancials the totals are withheld, never zeroed: {}",
        limited_row["totals"]
    );
}
