//! Label templates + label rendering + scan resolution (Wave 32 lane 5,
//! migration 0105) through the REAL core-api router.
//!
//! Verifies the contract pieces downstream surfaces + Wave 33 consume:
//!
//! 1. Template CRUD: typed purposes, token validation per purpose (a token
//!    outside the purpose's set is a typed 422, never a silently dropped
//!    line), size presets resolving to millimetres, and ONE active default
//!    per (org, purpose) — a second default clears the first in the same
//!    commit, and archiving the default frees the slot.
//! 2. Draft preview: deterministic sample values, QR as inline SVG, and the
//!    same label geometry the print path renders (one renderer).
//! 3. Entity render: values resolved from the real entity, the QR payload
//!    equal to the canonical deep link (`marula://stock-serial/<id>`),
//!    and the ZPL carrying the Code128 + QR commands.
//! 4. Scan resolution semantics: deep links resolve by id; bare codes match
//!    serial → SKU/part → location code → asset tag in that fixed order,
//!    case-insensitively; ambiguous codes come back as candidates (never a
//!    guess); unknown codes are honestly `unknown`.
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
        .expect("labels/scan flow needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([31; 32]);
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

/// Unique per run so the suite is re-runnable against a persistent DB.
fn unique_slug(prefix: &str) -> String {
    format!("{prefix}-{}", &Uuid::now_v7().simple().to_string()[..12])
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Labels Co",
            "org_slug": slug,
            "email": format!("owner+ws32l5+{slug}@labels.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let value = body_json(response).await;
    value["access_token"].as_str().expect("access token").into()
}

async fn create_location(app: &Router, token: &str, name: &str, code: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/stock-locations",
        &[auth(token)],
        Some(json!({ "name": name, "kind": "warehouse", "code": code })),
    )
    .await;
    assert_eq!(response.status(), 200, "location create");
    body_json(response).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn create_item(
    app: &Router,
    token: &str,
    name: &str,
    kind: &str,
    sku: Option<&str>,
    part_number: Option<&str>,
    is_tool: bool,
) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/stock-items",
        &[auth(token)],
        Some(json!({
            "name": name,
            "kind": kind,
            "sku": sku,
            "part_number": part_number,
            "is_tool": is_tool,
            "unit": "each",
            "currency": "USD",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "item create: {name}");
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

async fn create_template(app: &Router, token: &str, body: Value) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/label-templates",
        &[auth(token)],
        Some(body),
    )
    .await;
    let status = response.status();
    let value = body_json(response).await;
    assert_eq!(status, 200, "template create: {value}");
    value
}

async fn resolve_scan(app: &Router, token: &str, text: &str) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/scan/resolve",
        &[auth(token)],
        Some(json!({ "text": text })),
    )
    .await;
    assert_eq!(response.status(), 200, "scan resolve: {text}");
    body_json(response).await
}

fn template_body(purpose: &str, name: &str, fields: Value) -> Value {
    json!({
        "name": name,
        "purpose": purpose,
        "size_preset": "zebra_2x1",
        "dpi": 203,
        "barcode_token": "serial",
        "include_qr": true,
        "fields": fields,
        "is_default": true,
        "idempotency_key": Uuid::now_v7().to_string(),
    })
}

#[tokio::test]
async fn label_templates_crud_preview_render_and_single_default() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let token = register_org(&app, &unique_slug("labels-crud")).await;

    // A real serialized unit to render.
    let location = create_location(&app, &token, "Shelf A-3", "A-3").await;
    let tool = create_item(
        &app,
        &token,
        "Torque wrench 1/2in",
        "serialized",
        Some("TW-12"),
        None,
        true,
    )
    .await;
    receive_serial(&app, &token, &tool, &location, "TW-000123").await;
    let serial = serial_ids(&app, &token, &tool).await[0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Create: the first serialized-unit template claims the default slot.
    let first = create_template(
        &app,
        &token,
        template_body(
            "serialized_unit",
            "Unit 2×1",
            json!(["name", "serial", "location"]),
        ),
    )
    .await;
    assert_eq!(first["width_mm"], 50.8, "preset resolves to mm");
    assert_eq!(first["height_mm"], 25.4);
    assert_eq!(first["is_default"], true);
    assert_eq!(first["fields"], json!(["name", "serial", "location"]));

    // A token outside the purpose's set is a typed 422 with the token named.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/label-templates",
        &[auth(&token)],
        Some(template_body("stock_location", "Bad", json!(["serial"]))),
    )
    .await;
    assert_eq!(response.status(), 422, "purpose/token matrix enforced");
    let error = body_json(response).await;
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("serial"),
        "the rejection names the offending token: {error}"
    );

    // A second default for the same purpose clears the first, in one commit.
    let second = create_template(
        &app,
        &token,
        template_body("serialized_unit", "Unit 3×1", json!(["name", "serial"])),
    )
    .await;
    let list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/label-templates?purpose=serialized_unit",
            &[auth(&token)],
            None,
        )
        .await,
    )
    .await;
    let rows = list.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter().filter(|r| r["is_default"] == true).count(),
        1,
        "exactly one default: {list}"
    );
    assert_eq!(
        rows.iter().find(|r| r["is_default"] == true).unwrap()["id"],
        second["id"]
    );

    // PATCH: change the size preset; a custom size without dims is refused.
    let patched = body_json(
        send(
            &app,
            axum::http::Method::PATCH,
            &format!("/api/v1/label-templates/{}", second["id"].as_str().unwrap()),
            &[auth(&token)],
            Some(json!({
                "size_preset": "custom",
                "idempotency_key": Uuid::now_v7().to_string(),
            })),
        )
        .await,
    )
    .await;
    assert_eq!(
        patched["error"]["code"], 422,
        "custom without dimensions must be refused: {patched}"
    );
    let patched = body_json(
        send(
            &app,
            axum::http::Method::PATCH,
            &format!("/api/v1/label-templates/{}", second["id"].as_str().unwrap()),
            &[auth(&token)],
            Some(json!({
                "size_preset": "custom",
                "width_mm": 60.0,
                "height_mm": 30.0,
                "idempotency_key": Uuid::now_v7().to_string(),
            })),
        )
        .await,
    )
    .await;
    assert_eq!(patched["width_mm"], 60.0);
    assert_eq!(patched["height_mm"], 30.0);

    // Draft preview: deterministic sample values + a real QR SVG + ZPL.
    let preview = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/label-templates/preview",
            &[auth(&token)],
            Some(json!({
                "purpose": "serialized_unit",
                "size_preset": "zebra_2x1",
                "barcode_token": "serial",
                "include_qr": true,
                "fields": ["name", "serial", "location"],
            })),
        )
        .await,
    )
    .await;
    assert_eq!(preview["sample"], true);
    assert!(
        preview["qr_svg"]
            .as_str()
            .unwrap_or_default()
            .contains("<svg"),
        "QR renders as inline SVG: {preview}"
    );
    assert!(
        preview["code128_svg"]
            .as_str()
            .unwrap_or_default()
            .contains("<rect"),
        "Code128 renders as an inline SVG bar band: {preview}"
    );
    let sample_serial = preview["label"]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["token"] == "serial")
        .unwrap()["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(sample_serial, "TW-000123", "server-owned sample values");
    let zpl = preview["zpl"].as_str().unwrap();
    assert!(zpl.starts_with("^XA"), "zpl header");
    assert!(zpl.contains("^BQN,2,"), "qr command present");
    assert!(zpl.ends_with("^XZ\n"));

    // Real entity render: values resolve and the QR carries the deep link.
    let rendered = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/labels/render",
            &[auth(&token)],
            Some(json!({
                "template_id": first["id"],
                "entity_id": serial,
            })),
        )
        .await,
    )
    .await;
    assert_eq!(rendered["sample"], false);
    assert_eq!(
        rendered["entity"]["deep_link"],
        format!("marula://stock-serial/{serial}")
    );
    assert_eq!(
        rendered["entity"]["display_name"],
        "Torque wrench 1/2in · TW-000123"
    );
    let texts: Vec<String> = rendered["label"]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["text"].as_str().unwrap().to_string())
        .collect();
    assert!(
        texts.contains(&"TW-000123".to_string()),
        "serial line: {texts:?}"
    );
    assert!(
        texts.contains(&"Shelf A-3".to_string()),
        "location line: {texts:?}"
    );
    assert_eq!(
        rendered["label"]["barcode"]["data"], "TW-000123",
        "Code128 encodes the serial"
    );
    assert!(
        rendered["zpl"].as_str().unwrap().contains("^BCN,"),
        "code128 command present"
    );

    // Archive the default: the slot frees and the row leaves the active list.
    let archived = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!(
                "/api/v1/label-templates/{}/archive",
                second["id"].as_str().unwrap()
            ),
            &[auth(&token)],
            None,
        )
        .await,
    )
    .await;
    assert_eq!(archived["active"], false);
    assert_eq!(archived["is_default"], false);
    let list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/label-templates?purpose=serialized_unit",
            &[auth(&token)],
            None,
        )
        .await,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 1, "archived rows drop out");
}

#[tokio::test]
async fn scan_resolution_is_typed_deterministic_and_never_a_guess() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let token = register_org(&app, &unique_slug("labels-scan")).await;

    let location = create_location(&app, &token, "Shelf A-3", "A-3").await;
    let item = create_item(
        &app,
        &token,
        "Cat6 patch cable (1 m)",
        "consumable",
        Some("C6-1M"),
        Some("PN-20431"),
        false,
    )
    .await;
    let tool = create_item(
        &app,
        &token,
        "Torque wrench 1/2in",
        "serialized",
        Some("TW-12"),
        None,
        true,
    )
    .await;
    receive_serial(&app, &token, &tool, &location, "TW-000123").await;
    let serial = serial_ids(&app, &token, &tool).await[0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // 1. SKU, case-insensitive.
    let hit = resolve_scan(&app, &token, "c6-1m").await;
    assert_eq!(hit["kind"], "stock_item");
    assert_eq!(hit["entity_id"], item);
    assert_eq!(hit["stock_item_id"], item);
    assert_eq!(hit["item_kind"], "consumable");
    assert_eq!(hit["display_name"], "Cat6 patch cable (1 m)");
    assert_eq!(hit["deep_link"], format!("marula://stock-item/{item}"));
    assert_eq!(hit["candidates"], json!([]));

    // 2. Part number.
    let hit = resolve_scan(&app, &token, "PN-20431").await;
    assert_eq!(hit["kind"], "stock_item");
    assert_eq!(hit["entity_id"], item);

    // 3. Serial number beats everything below it.
    let hit = resolve_scan(&app, &token, "tw-000123").await;
    assert_eq!(hit["kind"], "serialized_unit");
    assert_eq!(hit["serial_id"], serial);
    assert_eq!(hit["stock_item_id"], tool);
    assert_eq!(hit["is_tool"], true);
    assert_eq!(hit["item_kind"], "serialized");
    assert_eq!(hit["serial_status"], "in_stock");
    assert_eq!(hit["deep_link"], format!("marula://stock-serial/{serial}"));

    // 4. Location code.
    let hit = resolve_scan(&app, &token, "A-3").await;
    assert_eq!(hit["kind"], "stock_location");
    assert_eq!(hit["location_id"], location);
    assert_eq!(
        hit["deep_link"],
        format!("marula://stock-location/{location}")
    );

    // 5. Deep links resolve by id (the label round-trip).
    let hit = resolve_scan(&app, &token, &format!("marula://stock-serial/{serial}")).await;
    assert_eq!(hit["kind"], "serialized_unit");
    assert_eq!(hit["serial_id"], serial);
    let hit = resolve_scan(&app, &token, &format!("marula://stock-item/{item}")).await;
    assert_eq!(hit["kind"], "stock_item");
    assert_eq!(hit["entity_id"], item);

    // 6. Device asset tag (a client-fleet asset).
    let client = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/clients",
            &[auth(&token)],
            Some(json!({ "name": "Acme Dental", "idempotency_key": Uuid::now_v7().to_string() })),
        )
        .await,
    )
    .await;
    let client_id = client["id"].as_str().unwrap();
    let asset = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/v1/clients/{client_id}/assets"),
            &[auth(&token)],
            Some(json!({ "tag": "EXCH-01", "name": "ThinkPad T14", "kind": "device" })),
        )
        .await,
    )
    .await;
    let asset_id = asset["id"].as_str().unwrap();
    let hit = resolve_scan(&app, &token, "exch-01").await;
    assert_eq!(hit["kind"], "device_asset");
    assert_eq!(hit["device_asset_id"], asset_id);
    assert_eq!(hit["secondary"], "EXCH-01");
    assert_eq!(
        hit["deep_link"],
        format!("marula://device-asset/{asset_id}")
    );

    // 7. Ambiguity: two items sharing a part number come back as candidates.
    let _twin = create_item(
        &app,
        &token,
        "Cat6 patch cable (2 m)",
        "consumable",
        Some("C6-2M"),
        Some("PN-20431"),
        false,
    )
    .await;
    let hit = resolve_scan(&app, &token, "PN-20431").await;
    assert_eq!(hit["kind"], "stock_item");
    assert_eq!(hit["entity_id"], Value::Null, "no silent pick: {hit}");
    assert_eq!(hit["candidates"].as_array().unwrap().len(), 2);
    assert!(hit["deep_link"].is_null());

    // 7b. Toolbox stencil (Wave 33 lane 2). Kit matching is the LAST code
    //     step, so codes that already mean a serial/SKU/location/asset keep
    //     their meaning; the kit resolves case-insensitively with its
    //     canonical deep link.
    let kit = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/tool-kits",
            &[auth(&token)],
            Some(json!({
                "name": "Field network kit",
                "code": "KIT-FIELD-1",
                "idempotency_key": Uuid::now_v7().to_string(),
            })),
        )
        .await,
    )
    .await;
    let kit_id = kit["id"].as_str().unwrap();
    let hit = resolve_scan(&app, &token, "kit-field-1").await;
    assert_eq!(hit["kind"], "tool_kit");
    assert_eq!(hit["entity_id"], kit_id);
    assert_eq!(hit["display_name"], "Field network kit");
    assert_eq!(hit["secondary"], "KIT-FIELD-1");
    assert_eq!(hit["deep_link"], format!("marula://tool-kit/{kit_id}"));
    assert_eq!(hit["is_tool"], false);
    let hit = resolve_scan(&app, &token, &format!("marula://tool-kit/{kit_id}")).await;
    assert_eq!(hit["kind"], "tool_kit");
    assert_eq!(hit["entity_id"], kit_id);

    // 8. Unknown stays unknown, with the input echoed.
    let hit = resolve_scan(&app, &token, "NOPE-404").await;
    assert_eq!(hit["kind"], "unknown");
    assert_eq!(hit["input"], "NOPE-404");
    assert_eq!(hit["entity_id"], Value::Null);
    assert_eq!(hit["candidates"], json!([]));

    // 9. A device-asset label renders with the tag + a nil deep link craft
    //    guard: the render permission follows the asset module, and the
    //    values carry the tag token.
    let template = create_template(
        &app,
        &token,
        json!({
            "name": "Asset tag",
            "purpose": "device_asset",
            "size_preset": "dymo_multipurpose",
            "barcode_token": "tag",
            "include_qr": true,
            "fields": ["name", "tag"],
            "is_default": true,
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(template["width_mm"], 57.0);
    let rendered = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/labels/render",
            &[auth(&token)],
            Some(json!({ "template_id": template["id"], "entity_id": asset_id })),
        )
        .await,
    )
    .await;
    assert_eq!(
        rendered["entity"]["deep_link"],
        format!("marula://device-asset/{asset_id}")
    );
    assert_eq!(rendered["label"]["barcode"]["data"], "EXCH-01");
    assert_eq!(rendered["label"]["purpose"], "device_asset");
}
