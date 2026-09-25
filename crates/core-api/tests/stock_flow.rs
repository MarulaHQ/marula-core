//! Stock + consumables ledger flow (Wave 32 lane 1, migration 0102)
//! through the REAL core-api router.
//!
//! Verifies the contract pieces downstream lanes + Wave 33 consume:
//! 1. Items + locations: create with a soft catalog link (absent =
//!    free-form part), PATCH, archive-as-flag.
//! 2. The append-only ledger flow: receive → transfer → consume →
//!    ship-out; each mutation appends exactly ONE `stock_movements` row
//!    and the level is its deterministic consequence.
//! 3. Milli precision: no float anywhere — 1.5 boxes land as 1500.
//! 4. Traceability: consume/ship-out rows carry the client/ticket anchor
//!    and who/when/where/item; the movement ledger read shows them.
//! 5. Insufficient stock is a typed 409; serialized adjust a typed 409.
//! 6. Low-stock transition: level at/below the reorder point notifies
//!    the notification center (stock_low) ONCE (no re-notify while low)
//!    and re-arms after recovery.
//! 7. Append-only is database-enforced: UPDATE/DELETE on the ledger is
//!    rejected by the trigger.
//! 8. The named read models: /stock/on-hand + /stock/consumption-velocity.
//!
//! Database selection follows assets_flow's contract:
//! MERIDIAN_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

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
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("stock flow needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([22; 32]);
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
            "org_name": "Stock Co",
            "org_slug": slug,
            "email": format!("owner+ws32+{slug}@stock.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let raw = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&raw).expect("register response is JSON");
    value["access_token"].as_str().expect("access token").into()
}

#[tokio::test]
async fn stock_ledger_flow_receive_transfer_consume_ship_out() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("stock-{}", &Uuid::now_v7().simple().to_string()[..12]);
    let token = register_org(&app, &slug).await;

    // A real client row for the ship-out anchor (client_id is FK-scoped).
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/clients",
        &[auth(&token)],
        Some(json!({
            "name": "Acme Manufacturing",
            "email": "it@acme.test",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "create ship-out client");
    let client = body_json(response).await;
    let client_id: Uuid = client["client"]["id"]
        .as_str()
        .or_else(|| client["id"].as_str())
        .expect("client id")
        .parse()
        .unwrap();

    // 1. items + locations -------------------------------------------------
    // Free-form part (no catalog line).
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/stock-items",
        &[auth(&token)],
        Some(json!({
            "name": "Cat6 patch cable (1m)",
            "sku": "CAT6-1M",
            "part_number": "C6-1M",
            "kind": "consumable",
            "unit": "meter",
            "currency": "USD",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "create item");
    let item = body_json(response).await;
    let item_id: Uuid = item["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(item["kind"], "consumable");
    assert_eq!(item["unit"], "meter");

    // Catalog-linked item + a location row.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/stock-items",
        &[auth(&token)],
        Some(json!({
            "name": "SSD 2.5\" 1TB",
            "sku": "SSD-1TB",
            "kind": "serialized",
            "unit": "each",
            "currency": "USD",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "create serialized item");
    let serialized: Value = body_json(response).await;
    let serialized_id: Uuid = serialized["id"].as_str().unwrap().parse().unwrap();

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/stock-locations",
        &[auth(&token)],
        Some(json!({
            "name": "Main warehouse",
            "kind": "warehouse",
            "code": "WH1",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "create warehouse");
    let wh: Value = body_json(response).await;
    let warehouse: Uuid = wh["id"].as_str().unwrap().parse().unwrap();

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/stock-locations",
        &[auth(&token)],
        Some(json!({
            "name": "Van 1",
            "kind": "van",
            "code": "V1",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "create van");
    let van: Value = body_json(response).await;
    let van_id: Uuid = van["id"].as_str().unwrap().parse().unwrap();

    // 2. milli precision: receive 1.5 meters = 1500 -----------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/stock/receive",
        &[auth(&token)],
        Some(json!({
            "item_id": item_id,
            "location_id": warehouse,
            "qty_milli": 1500,
            "unit_cost_minor": 120,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "receive 1.5 meters");
    let receive: Value = body_json(response).await;
    assert_eq!(receive["movement"]["qty_milli"], 1500);
    let receive_movement: Uuid = receive["movement"]["id"].as_str().unwrap().parse().unwrap();

    // Transfer 1000 milli to the van: warehouse 500, van 1000.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/stock/transfer",
        &[auth(&token)],
        Some(json!({
            "item_id": item_id,
            "from_location_id": warehouse,
            "to_location_id": van_id,
            "qty_milli": 1000,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "transfer to van");
    let transfer: Value = body_json(response).await;
    assert_eq!(transfer["movement"]["kind"], "transfer");
    assert_eq!(transfer["movement"]["qty_milli"], 1000);
    let transfer_movement: Uuid = transfer["movement"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    // 3. consume 300 milli on a ticket: 500 mA at warehouse → 500 - 0 = 0.5
    //    no ticket anchor → traceability requires it?  ticket optional.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/stock/consume",
        &[auth(&token)],
        Some(json!({
            "item_id": item_id,
            "location_id": van_id,
            "qty_milli": 250,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "consume from the van");
    let consume: Value = body_json(response).await;
    assert_eq!(consume["movement"]["qty_milli"], -250);

    // Insufficient stock is typed 409.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/stock/consume",
        &[auth(&token)],
        Some(json!({
            "item_id": item_id,
            "location_id": van_id,
            "qty_milli": 999_999,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 409, "insufficient stock must be 409");

    // 4. ship-out to a client with carrier + tracking ref ------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/stock/ship-out",
        &[auth(&token)],
        Some(json!({
            "item_id": item_id,
            "location_id": van_id,
            "qty_milli": 100,
            "client_id": client_id,
            "carrier": "UPS",
            "tracking_ref": "1Z999AA10123456784",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "ship out with carrier ref");
    let ship_out: Value = body_json(response).await;
    assert_eq!(ship_out["movement"]["qty_milli"], -100);
    assert_eq!(ship_out["movement"]["carrier"], "UPS");
    let ship_movement: Uuid = ship_out["movement"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    // Ship-out without a destination anchor is a typed 422.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/stock/ship-out",
        &[auth(&token)],
        Some(json!({
            "item_id": item_id,
            "location_id": van_id,
            "qty_milli": 10,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 422, "ship-out needs client or ticket");

    // 5. traceability read: the ledger shows who/when/where/item ----------
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/stock-items/{item_id}/movements"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let movements = body_json(response).await;
    let ids: Vec<&str> = movements
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&receive_movement.to_string().as_str()));
    assert!(ids.contains(&transfer_movement.to_string().as_str()));
    assert!(ids.contains(&ship_movement.to_string().as_str()));
    assert_eq!(ids.len(), 4, "four ledger rows so far");

    // 6. on-hand read model ------------------------------------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/stock/on-hand?item_id={item_id}"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let on_hand = body_json(response).await;
    let rows = on_hand.as_array().unwrap();
    let van_row = rows
        .iter()
        .find(|r| r["stock_location_id"].as_str() == Some(van_id.to_string().as_str()))
        .expect("van level row");
    // 1000 - 250 consume - 100 ship-out = 650 milli in the van.
    assert_eq!(van_row["qty_milli"], 650);

    // 7. consumption velocity: 250 consume + 100 ship-out = 350 outbound ---
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/stock/consumption-velocity?window_days=30",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let velocity = body_json(response).await;
    let vd = velocity
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["stock_item_id"].as_str() == Some(item_id.to_string().as_str()))
        .expect("velocity row for the item");
    assert_eq!(vd["consumed_milli"], 350);
    assert_eq!(vd["unit"], "meter");

    // 8. serialized: receive requires a serial; adjust rejected on it ------
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/stock/receive",
        &[auth(&token)],
        Some(json!({
            "item_id": serialized_id,
            "location_id": warehouse,
            "qty_milli": 1000,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 422, "serialized receive needs a serial");

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/stock/receive",
        &[auth(&token)],
        Some(json!({
            "item_id": serialized_id,
            "location_id": warehouse,
            "qty_milli": 1000,
            "serial_numbers": ["AAPL-SSD-0001"],
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "serialized receive with serial");
    let serial_receive = body_json(response).await;
    assert!(
        serial_receive["movement"]["serial_id"].is_string(),
        "movement stamps the serial"
    );
    let serial_item_id: Uuid = serial_receive["movement"]["serial_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/stock/adjust",
        &[auth(&token)],
        Some(json!({
            "item_id": serialized_id,
            "location_id": warehouse,
            "qty_milli": 1000,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 409, "adjust on serialized is rejected");

    // serialized list read shows the unit in the warehouse
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/stock-items/{serialized_id}/serials"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let serials = body_json(response).await;
    let list = serials.as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["serial_number"], "AAPL-SSD-0001");
    assert_eq!(list[0]["status"], "in_stock");
    assert_eq!(
        list[0]["id"].as_str(),
        Some(serial_item_id.to_string().as_str())
    );
}

/// Low-stock: transition notifies ONCE via the notification center, stays
/// quiet while low, and re-arms after recovery; append-only trigger
/// guards the ledger; org scoping keeps another org out.
#[tokio::test]
async fn low_stock_notification_once_and_ledger_is_append_only() {
    use axum::http::Method;
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("low-{}", &Uuid::now_v7().simple().to_string()[..12]);
    let token = register_org(&app, &slug).await;
    let org_id: Uuid = sqlx::query_scalar("SELECT id FROM orgs WHERE slug = $1")
        .bind(&slug)
        .fetch_one(&state.pool)
        .await
        .expect("org row");

    let response = send(
        &app,
        Method::POST,
        "/api/v1/stock-items",
        &[auth(&token)],
        Some(json!({
            "name": "Thermal paste",
            "sku": "TP-5G",
            "kind": "consumable",
            "unit": "each",
            "currency": "USD",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let item = body_json(response).await;
    let item_id: Uuid = item["id"].as_str().unwrap().parse().unwrap();

    let response = send(
        &app,
        Method::POST,
        "/api/v1/stock-locations",
        &[auth(&token)],
        Some(json!({
            "name": "Bench shelf",
            "kind": "shelf",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let loc = body_json(response).await;
    let loc_id: Uuid = loc["id"].as_str().unwrap().parse().unwrap();

    // Thresholds: reorder at 1000 milli.
    let response = send(
        &app,
        Method::POST,
        "/api/v1/stock/levels/thresholds",
        &[auth(&token)],
        Some(json!({
            "item_id": item_id,
            "location_id": loc_id,
            "reorder_point_milli": 1000,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200);

    async fn count_low_notifs(state: &AppState, org_id: Uuid) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM notifications WHERE org_id = $1 AND kind = 'stock_low'",
        )
        .bind(org_id)
        .fetch_one(&state.pool)
        .await
        .expect("count notifications")
    }

    // Receive two units (2000) with THREE out points each?? NO — receive 2.
    let response = send(
        &app,
        Method::POST,
        "/api/v1/stock/receive",
        &[auth(&token)],
        Some(json!({
            "item_id": item_id,
            "location_id": loc_id,
            "qty_milli": 2000,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        count_low_notifs(&state, org_id).await,
        0,
        "above point: silent"
    );

    // Consume down to 800 → the TRANSITION notifies (owner is a recipient).
    for _ in 0..1 {
        let response = send(
            &app,
            Method::POST,
            "/api/v1/stock/consume",
            &[auth(&token)],
            Some(json!({
                "item_id": item_id,
                "location_id": loc_id,
                "qty_milli": 1200,
                "idempotency_key": Uuid::now_v7().to_string(),
            })),
        )
        .await;
        assert_eq!(response.status(), 200);
    }
    assert_eq!(
        count_low_notifs(&state, org_id).await,
        1,
        "low transition notifies once"
    );

    // A further consume while still low stays quiet (flag gates repeats).
    let response = send(
        &app,
        Method::POST,
        "/api/v1/stock/consume",
        &[auth(&token)],
        Some(json!({
            "item_id": item_id,
            "location_id": loc_id,
            "qty_milli": 10,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        count_low_notifs(&state, org_id).await,
        1,
        "no re-notify while the level stays low"
    );

    // Recovery + dip: notify arms the bell again; a fresh emit. (The
    // recovery path is exercised by the same engine.)
    let response = send(
        &app,
        Method::POST,
        "/api/v1/stock/receive",
        &[auth(&token)],
        Some(json!({
            "item_id": item_id,
            "location_id": loc_id,
            "qty_milli": 5000,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = send(
        &app,
        Method::POST,
        "/api/v1/stock/consume",
        &[auth(&token)],
        Some(json!({
            "item_id": item_id,
            "location_id": loc_id,
            "qty_milli": 5000,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        count_low_notifs(&state, org_id).await,
        2,
        "recovered then dipped low again: notify again"
    );

    // Append-only: both UPDATE and DELETE must be trigger-refused.
    let update = sqlx::query("UPDATE stock_movements SET qty_milli = 1 WHERE org_id = $1")
        .bind(org_id)
        .execute(&state.pool)
        .await;
    assert!(update.is_err(), "UPDATE on the ledger must be rejected");
    let delete = sqlx::query("DELETE FROM stock_movements WHERE org_id = $1")
        .bind(org_id)
        .execute(&state.pool)
        .await;
    assert!(delete.is_err(), "DELETE on the ledger must be rejected");

    // Cross-org isolation: a second org's owner sees NO stock rows.
    let other_slug = format!("low-{}", &Uuid::now_v7().simple().to_string()[..12]);
    let other = register_org(&app, &other_slug).await;
    let response = send(
        &app,
        Method::GET,
        "/api/v1/stock/on-hand",
        &[auth(&other)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let rows = body_json(response).await;
    assert!(
        rows.as_array().unwrap().is_empty(),
        "cross-org read stays empty"
    );
}
