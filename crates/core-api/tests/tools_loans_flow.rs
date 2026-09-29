//! Tools checkout + toolboxes + client loans (Wave 32 lane 3, migration
//! 0104) through the REAL core-api router.
//!
//! Verifies the contract pieces downstream lanes + Wave 33 consume:
//! 1. Tools are lane-1 stock items flagged `is_tool` (serialized-only:
//!    flagging a consumable is rejected).
//! 2. Check-out/check-in: one ledger leg per physical unit
//!    (`checkout`/`checkin` kinds), the serial flipping
//!    `in_stock` → `checked_out` → `in_stock`, and a checkout line citing
//!    the leg ids.
//! 3. The single-holder invariant: one tool, one person at a time — a
//!    second checkout of the same unit is a typed 409.
//! 4. Overdue is DERIVED: a checkout past its due date reads back overdue
//!    and answers `?overdue_only=true`.
//! 5. A toolbox (kit) is checked out AS ONE transaction — every member
//!    unit on its own leg, one header.
//! 6. The where-is-it-now read model answers holder/location/status per
//!    serialized unit; availability answers counts per item + location.
//! 7. Client loans: issuing a stock-sourced device rides the ledger
//!    (`loan` leg) and instantiates a `DeviceAsset` in the client's fleet
//!    with identity keys; a later RMM feed describing the same serial
//!    CONVERGES onto that same row via the ADR 0003 dedupe path (never a
//!    twin).
//! 8. Lease reuse: an is_lease loan stamps the 0061 rental fields on the
//!    issued asset (ownership/cycle/cost/currency/start/end), and the
//!    return closes `rental_end` at the actual return date.
//! 9. Loan returns ride the ledger (`loan_return` leg) and re-arm the
//!    serial to `in_stock`; asset-sourced lines return without a location.
//!
//! Database selection follows assets_flow's contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("tools/loans flow needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([23; 32]);
    AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    }
}

async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
}

async fn send(
    app: &Router,
    method: axum::http::Method,
    uri: &str,
    headers: &[(&str, String)],
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, value.clone());
    }
    let request = match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(axum::body::Body::from(v.to_string()))
            .unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    };
    app.clone().oneshot(request).await.unwrap()
}

async fn body_json(response: axum::response::Response) -> Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap()
}

fn auth(token: &str) -> (&'static str, String) {
    ("authorization", format!("Bearer {token}"))
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Tools Co",
            "org_slug": slug,
            "email": format!("owner+ws32l3+{slug}@tools.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let value = body_json(response).await;
    value["access_token"].as_str().expect("access token").into()
}

/// The caller's (user_id, org_id) from /me.
async fn me_ids(app: &Router, token: &str) -> (String, String) {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/v1/me",
        &[auth(token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let value = body_json(response).await;
    (
        value["user_id"].as_str().unwrap().to_string(),
        value["org_id"].as_str().unwrap().to_string(),
    )
}

async fn create_location(app: &Router, token: &str, name: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/stock-locations",
        &[auth(token)],
        Some(json!({ "name": name, "kind": "warehouse" })),
    )
    .await;
    assert_eq!(response.status(), 200, "location create");
    body_json(response).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn create_item(app: &Router, token: &str, name: &str, is_tool: bool) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/stock-items",
        &[auth(token)],
        Some(json!({
            "name": name,
            "kind": "serialized",
            "is_tool": is_tool,
            "unit": "each",
            "currency": "USD",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "item create");
    body_json(response).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn receive_serial(app: &Router, token: &str, item_id: &str, location_id: &str, serial: &str) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/stock/receive",
        &[auth(token)],
        Some(json!({
            "item_id": item_id,
            "location_id": location_id,
            "qty_milli": 1000,
            "serial_numbers": [serial],
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "receive {serial}");
}

async fn serial_ids(app: &Router, token: &str, item_id: &str) -> Vec<Value> {
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/v1/stock-items/{item_id}/serials"),
        &[auth(token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    body_json(response).await.as_array().unwrap().clone()
}

async fn whereabouts(app: &Router, token: &str, query: &str) -> Vec<Value> {
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/v1/stock-serials?{query}"),
        &[auth(token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "whereabouts read");
    body_json(response).await.as_array().unwrap().clone()
}

async fn availability(app: &Router, token: &str, query: &str) -> Vec<Value> {
    let uri = if query.is_empty() {
        "/api/v1/tools/availability".to_string()
    } else {
        format!("/api/v1/tools/availability?{query}")
    };
    let response = send(app, axum::http::Method::GET, &uri, &[auth(token)], None).await;
    assert_eq!(response.status(), 200, "availability read");
    body_json(response).await.as_array().unwrap().clone()
}

/// Sums one count column across availability rows (a checked-out unit has
/// no location, so it aggregates in the location-less row).
fn count_sum(rows: &[Value], column: &str) -> i64 {
    rows.iter().map(|r| r[column].as_i64().unwrap_or(0)).sum()
}

fn past_due() -> String {
    (chrono::Utc::now() - chrono::Duration::days(2)).to_rfc3339()
}

fn future_due() -> String {
    (chrono::Utc::now() + chrono::Duration::days(3)).to_rfc3339()
}

/// Serializer-independent view of an identity key (the feed may store it
/// in its own formatting; matching normalizes both sides).
fn normalized_serial(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_uppercase())
        .collect()
}

fn future_date() -> String {
    (chrono::Utc::now() + chrono::Duration::days(30))
        .date_naive()
        .to_string()
}

fn past_date() -> String {
    (chrono::Utc::now() - chrono::Duration::days(3))
        .date_naive()
        .to_string()
}

#[tokio::test]
async fn tools_checkout_checkin_kit_single_holder_and_overdue() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("tools-{}", &Uuid::now_v7().simple().to_string()[..12]);
    let token = register_org(&app, &slug).await;
    let (user_id, _org_id) = me_ids(&app, &token).await;

    // A consumable CANNOT be flagged as a tool (serialized-only rule).
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/stock-items",
        &[auth(&token)],
        Some(json!({
            "name": "Cable box", "kind": "consumable", "is_tool": true,
            "unit": "box", "currency": "USD",
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        422,
        "flagging a consumable as a tool is a validation error"
    );

    let warehouse = create_location(&app, &token, "Main warehouse").await;
    let drill = create_item(&app, &token, "Cordless drill", true).await;
    receive_serial(&app, &token, &drill, &warehouse, "DRILL-1").await;
    receive_serial(&app, &token, &drill, &warehouse, "DRILL-2").await;
    receive_serial(&app, &token, &drill, &warehouse, "DRILL-3").await;

    // A toolbox = a kit of member tools, checked out as one.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/tool-kits",
        &[auth(&token)],
        Some(json!({ "name": "Electrical toolbox", "code": "TB-ELEC-1" })),
    )
    .await;
    assert_eq!(response.status(), 200, "kit create");
    let kit = body_json(response).await;
    let kit_id = kit["id"].as_str().unwrap().to_string();

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/tool-kits/{kit_id}/members"),
        &[auth(&token)],
        Some(json!({ "stock_item_id": drill, "qty_milli": 1000 })),
    )
    .await;
    assert_eq!(response.status(), 200, "kit member add");
    let members = body_json(response).await;
    assert_eq!(members["members"].as_array().unwrap().len(), 1);
    assert_eq!(members["members"][0]["is_tool"], json!(true));

    // Readiness before checkout: 3 in stock, 1 required → ready.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/tool-kits/{kit_id}/availability?location_id={warehouse}"),
        &[auth(&token)],
        None,
    )
    .await;
    let kit_ready = body_json(response).await;
    assert_eq!(kit_ready["ready"], json!(true));
    assert_eq!(kit_ready["members"][0]["available_units"], json!(3));

    // The toolbox goes out AS ONE transaction, already overdue (past due).
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/tool-checkouts",
        &[auth(&token)],
        Some(json!({
            "holder_user_id": user_id,
            "kit_id": kit_id,
            "from_location_id": warehouse,
            "due_at": past_due(),
            "notes": "North route van kit",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "kit checkout");
    let checkout = body_json(response).await;
    let checkout_id = checkout["checkout"]["id"].as_str().unwrap().to_string();
    assert_eq!(checkout["checkout"]["status"], json!("open"));
    assert_eq!(checkout["checkout"]["kit_id"], json!(kit_id));
    assert_eq!(
        checkout["checkout"]["overdue"],
        json!(true),
        "past due reads back overdue (derived, never stored)"
    );
    assert_eq!(checkout["lines"].as_array().unwrap().len(), 1);
    let line_id = checkout["lines"][0]["id"].as_str().unwrap().to_string();
    assert!(
        checkout["lines"][0]["movement_out_id"].is_string(),
        "each unit cites its own ledger leg"
    );

    // Where is it now: the unit is with the holder, not at a location.
    let units = whereabouts(&app, &token, "status=checked_out").await;
    assert_eq!(units.len(), 1);
    assert_eq!(units[0]["holder_user_id"], json!(user_id));
    assert_eq!(units[0]["checkout_id"], json!(checkout_id));
    assert_eq!(units[0]["location_id"], json!(null));
    assert_eq!(units[0]["is_tool"], json!(true));
    assert_eq!(units[0]["overdue"], json!(true));
    let held_serial_id = units[0]["id"].as_str().unwrap().to_string();

    // Availability now: 2 units remain available at the warehouse; the
    // third is away with the holder (no location row → org-wide counts).
    let at_warehouse = availability(&app, &token, &format!("location_id={warehouse}")).await;
    assert_eq!(count_sum(&at_warehouse, "available_units"), 2);
    let org_wide = availability(&app, &token, "").await;
    assert_eq!(count_sum(&org_wide, "available_units"), 2);
    assert_eq!(count_sum(&org_wide, "checked_out_units"), 1);

    // THE single-holder invariant: one tool, one person at a time.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/tool-checkouts",
        &[auth(&token)],
        Some(json!({
            "holder_user_id": user_id,
            "items": [{ "stock_item_id": drill, "serial_id": held_serial_id }],
            "from_location_id": warehouse,
            "due_at": future_due(),
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        409,
        "a unit already out under an open checkout refuses a second holder"
    );

    // Overdue list answers the derived state.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/tool-checkouts?overdue_only=true",
        &[auth(&token)],
        None,
    )
    .await;
    let overdue = body_json(response).await;
    assert_eq!(overdue.as_array().unwrap().len(), 1);
    assert_eq!(overdue[0]["checkout"]["id"], json!(checkout_id));

    // The kit's readiness gate: take the two remaining units out
    // individually, then the kit checkout is a typed conflict.
    let remaining = whereabouts(&app, &token, "status=in_stock&tool_only=true").await;
    assert_eq!(remaining.len(), 2, "two units still on the shelf");
    let mut extra_checkouts = Vec::new();
    for unit in &remaining {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/tool-checkouts",
            &[auth(&token)],
            Some(json!({
                "holder_user_id": user_id,
                "items": [{ "stock_item_id": drill, "serial_id": unit["id"] }],
                "from_location_id": warehouse,
                "due_at": future_due(),
            })),
        )
        .await;
        assert_eq!(response.status(), 200, "individual checkout");
        extra_checkouts.push(
            body_json(response).await["checkout"]["id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/tool-checkouts",
        &[auth(&token)],
        Some(json!({
            "holder_user_id": user_id,
            "kit_id": kit_id,
            "from_location_id": warehouse,
            "due_at": future_due(),
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        409,
        "a kit with no available member unit is a typed conflict"
    );

    // Check-in: condition + notes land on the line; the unit re-arms.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/tool-checkouts/{checkout_id}/checkin"),
        &[auth(&token)],
        Some(json!({
            "lines": [{
                "line_id": line_id,
                "return_condition": "fair",
                "return_notes": "Chuck worn",
            }],
            "notes": "Back from the north route",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "checkin");
    let checked_in = body_json(response).await;
    assert_eq!(checked_in["checkout"]["status"], json!("returned"));
    assert_eq!(checked_in["checkout"]["overdue"], json!(false));
    assert_eq!(checked_in["lines"][0]["return_condition"], json!("fair"));
    assert_eq!(checked_in["lines"][0]["return_notes"], json!("Chuck worn"));
    assert!(checked_in["lines"][0]["movement_in_id"].is_string());

    // The individually checked-out units come back too (whole-transaction
    // check-in: no line list = every outstanding unit).
    for id in &extra_checkouts {
        let response = send(
            &app,
            axum::http::Method::POST,
            &format!("/api/v1/tool-checkouts/{id}/checkin"),
            &[auth(&token)],
            Some(json!({ "idempotency_key": Uuid::now_v7().to_string() })),
        )
        .await;
        assert_eq!(response.status(), 200, "checkin {id}");
        assert_eq!(
            body_json(response).await["checkout"]["status"],
            json!("returned")
        );
    }

    // Where is it now: all three back in stock at the warehouse.
    let units = whereabouts(&app, &token, "status=in_stock&tool_only=true").await;
    assert_eq!(units.len(), 3);
    assert_eq!(units[0]["location_id"], json!(warehouse));
    assert_eq!(units[0]["holder_user_id"], json!(null));
    let org_wide = availability(&app, &token, "").await;
    assert_eq!(count_sum(&org_wide, "available_units"), 3);
    assert_eq!(count_sum(&org_wide, "checked_out_units"), 0);

    // Checking in twice is a typed conflict, not a silent no-op.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/tool-checkouts/{checkout_id}/checkin"),
        &[auth(&token)],
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 409);

    // Traceability: the ledger holds the checkout/checkin legs, both ways.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/stock-items/{drill}/movements?limit=50"),
        &[auth(&token)],
        None,
    )
    .await;
    let ledger = body_json(response).await;
    let kinds: Vec<&str> = ledger
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["kind"].as_str())
        .collect();
    assert_eq!(
        kinds,
        vec![
            "checkin", "checkin", "checkin", "checkout", "checkout", "checkout", "receive",
            "receive", "receive"
        ],
        "every checkout/checkin is a traceable ledger leg"
    );
}

#[tokio::test]
async fn client_loan_instantiates_asset_converges_via_dedupe_and_lease_fields() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("loans-{}", &Uuid::now_v7().simple().to_string()[..12]);
    let token = register_org(&app, &slug).await;

    // Client + a serialized device in stock (not a tool: loan devices are
    // not checkout-eligible tools).
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/clients",
        &[auth(&token)],
        Some(json!({ "name": "Acme Manufacturing", "email": "it@acme.test" })),
    )
    .await;
    assert_eq!(response.status(), 200, "client create");
    let client_id = body_json(response).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    let warehouse = create_location(&app, &token, "Loan depot").await;
    let phone = create_item(&app, &token, "Field phone", false).await;
    receive_serial(&app, &token, &phone, &warehouse, "SN-IPH-77").await;
    let serials = serial_ids(&app, &token, &phone).await;
    let serial_id = serials[0]["id"].as_str().unwrap().to_string();

    // Issue a LEASE: the 0061 rental fields land on the client's new asset.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/client-loans",
        &[auth(&token)],
        Some(json!({
            "client_id": client_id,
            "agreement_ref": "AGR-2026-014",
            "expected_return_at": future_date(),
            "is_lease": true,
            "lease_cycle": "monthly",
            "lease_cost_minor": 4900,
            "lease_currency": "USD",
            "items": [{ "stock_item_id": phone, "serial_id": serial_id }],
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "loan issue");
    let loan = body_json(response).await;
    let loan_id = loan["loan"]["id"].as_str().unwrap().to_string();
    assert_eq!(loan["loan"]["status"], json!("open"));
    assert_eq!(loan["loan"]["is_lease"], json!(true));
    assert_eq!(loan["loan"]["overdue"], json!(false));
    let line_id = loan["lines"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(loan["lines"][0]["from_stock"], json!(true));
    assert!(
        loan["lines"][0]["movement_out_id"].is_string(),
        "the issue leg rides the ledger"
    );
    let asset_id = loan["lines"][0]["device_asset_id"]
        .as_str()
        .expect("issue instantiates the device asset")
        .to_string();

    // The instantiated asset: identity-keyed + lease-stamped (0061 reuse).
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/assets/{asset_id}"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "asset read");
    let asset = body_json(response).await;
    assert_eq!(asset["client_id"], json!(client_id));
    assert_eq!(asset["name"], json!("Field phone"));
    assert_eq!(
        asset["identity_keys"]["serial"],
        json!("SNIPH77"),
        "normalized serial identity key for ADR 0003 matching"
    );
    assert_eq!(asset["ownership"], json!("leased"));
    assert_eq!(asset["rental_cycle"], json!("monthly"));
    assert_eq!(asset["rental_cost_minor"], json!(4900));
    assert_eq!(asset["rental_currency"], json!("USD"));
    assert_eq!(
        asset["rental_start"],
        json!(chrono::Utc::now().date_naive().to_string())
    );
    assert_eq!(asset["rental_end"], json!(future_date()));

    // The serialized unit now reads "with this client" everywhere.
    let units = whereabouts(&app, &token, "status=loaned").await;
    assert_eq!(units.len(), 1);
    assert_eq!(units[0]["client_id"], json!(client_id));
    assert_eq!(units[0]["loan_id"], json!(loan_id));
    assert_eq!(units[0]["is_tool"], json!(false));
    assert_eq!(units[0]["overdue"], json!(false));

    // The issue leg is on the append-only ledger with the client anchor.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/stock-items/{phone}/movements?limit=10"),
        &[auth(&token)],
        None,
    )
    .await;
    let ledger = body_json(response).await;
    assert_eq!(ledger[0]["kind"], json!("loan"));
    assert_eq!(ledger[0]["client_id"], json!(client_id));
    assert_eq!(ledger[0]["serial_id"], json!(serial_id));

    // ADR 0003 convergence: a later RMM feed describing the same phone
    // (serial printed differently) adopts THIS asset — never a twin.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/config-items/feed-upsert",
        &[auth(&token)],
        Some(json!({
            "source_system": "ninjaone",
            "external_id": "N-PHONE-1",
            "client_id": client_id,
            "tag": "MOB-01",
            "name": "Field phone",
            "identity_keys": { "serial": "sn iph 77" },
            "observed_fields": { "model": "iPhone 15", "os_version": "18.2" },
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "feed upsert");
    let converged = body_json(response).await;
    assert_eq!(
        converged["created_item"],
        json!(false),
        "the feed must not create a twin"
    );
    assert_eq!(converged["resolved_existing"], json!(true));
    assert_eq!(
        converged["id"],
        json!(asset_id),
        "converged onto the loaned asset"
    );
    assert_eq!(converged["fields"]["model"], json!("iPhone 15"));

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/assets?client_id={client_id}"),
        &[auth(&token)],
        None,
    )
    .await;
    let fleet = body_json(response).await;
    assert_eq!(
        fleet.as_array().unwrap().len(),
        1,
        "one physical phone = one asset row after the feed"
    );
    // The serial identity converged (formatting may be either side's form;
    // matching normalizes).
    assert_eq!(
        normalized_serial(&fleet[0]["identity_keys"]["serial"]),
        "SNIPH77"
    );

    // Return: the device rides the loan_return leg back into stock and the
    // lease closes at the ACTUAL return date.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/client-loans/{loan_id}/return"),
        &[auth(&token)],
        Some(json!({
            "to_location_id": warehouse,
            "lines": [{ "line_id": line_id, "return_condition": "good" }],
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "loan return");
    let returned = body_json(response).await;
    assert_eq!(returned["loan"]["status"], json!("returned"));
    assert_eq!(returned["lines"][0]["return_condition"], json!("good"));
    assert!(returned["lines"][0]["movement_in_id"].is_string());

    let units = whereabouts(&app, &token, &format!("status=in_stock&item_id={phone}")).await;
    assert_eq!(units.len(), 1);
    assert_eq!(units[0]["location_id"], json!(warehouse));
    assert_eq!(units[0]["client_id"], json!(null));

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/assets/{asset_id}"),
        &[auth(&token)],
        None,
    )
    .await;
    let asset = body_json(response).await;
    assert_eq!(
        asset["rental_end"],
        json!(chrono::Utc::now().date_naive().to_string()),
        "the lease closed at the actual return date"
    );

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/stock-items/{phone}/movements?limit=10"),
        &[auth(&token)],
        None,
    )
    .await;
    let ledger = body_json(response).await;
    assert_eq!(ledger[0]["kind"], json!("loan_return"));
    assert_eq!(ledger[0]["client_id"], json!(client_id));

    // Asset-sourced lines: an existing fleet device goes out again under a
    // past-due (non-lease) loan → overdue, and returns without a location.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/client-loans",
        &[auth(&token)],
        Some(json!({
            "client_id": client_id,
            "agreement_ref": "AGR-2026-015",
            // Issued a week ago, was due three days ago → overdue today.
            "out_at": (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339(),
            "expected_return_at": past_date(),
            "items": [{ "device_asset_id": asset_id }],
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "asset-sourced loan issue");
    let second = body_json(response).await;
    let second_id = second["loan"]["id"].as_str().unwrap().to_string();
    assert_eq!(second["loan"]["overdue"], json!(true));
    assert!(
        second["lines"][0]["movement_out_id"].is_null(),
        "no stock leg"
    );
    assert_eq!(second["lines"][0]["from_stock"], json!(false));

    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/client-loans?overdue_only=true",
        &[auth(&token)],
        None,
    )
    .await;
    let overdue = body_json(response).await;
    assert_eq!(overdue.as_array().unwrap().len(), 1);
    assert_eq!(overdue[0]["loan"]["id"], json!(second_id));

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/client-loans/{second_id}/return"),
        &[auth(&token)],
        Some(json!({ "idempotency_key": Uuid::now_v7().to_string() })),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "asset-sourced return needs no location"
    );
    assert_eq!(
        body_json(response).await["loan"]["status"],
        json!("returned")
    );

    // Incomplete lease terms are a typed refusal.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/client-loans",
        &[auth(&token)],
        Some(json!({
            "client_id": client_id,
            "expected_return_at": future_date(),
            "is_lease": true,
            "items": [{ "device_asset_id": asset_id }],
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        409,
        "a lease without cycle cost/currency is refused"
    );
}
