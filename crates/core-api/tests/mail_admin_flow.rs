//! Wave 34 lane 8 (`p34-email-config-closeout`) end-to-end coverage through
//! the REAL core-api router against a real Postgres.
//!
//! Purpose: pin the mail administration surface —
//! * the honest not-configured state (transport none, typed 409 on the
//!   test-send probe, the ledger reporting nothing sent);
//! * the sealed-credential slot (presence-only responses, no material in
//!   any payload, readiness flips only once the credential is sealed);
//! * the probe riding the outbound ledger (queued row + counts);
//! * per-board outbound identities (partial writes, clears, from-address
//!   collision as a typed 409) and inbound-address collision validation
//!   (board vs board, board vs catch-all);
//! * org notification defaults (admin upsert + read) and the digest policy
//!   setting.
//!
//! Non-purpose: worker transport resolution (worker unit tests own it),
//! SQL-layer primitives (core-db suites), and the UI surfaces (vitest page
//! suites).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

struct Org {
    token: String,
}

async fn test_state() -> (AppState, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("mail-admin e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    let reporting_url = admin_url;
    let state = AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: AppState::reporting_pool_lazy(&reporting_url).expect("lazy reporting pool"),
    };
    let app = core_api::routes::build_router(state.clone());
    (state, app)
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
    token: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
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

async fn register_org(app: &Router, slug: String) -> Org {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Mail Lane Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    Org {
        token: body_json(response).await["access_token"]
            .as_str()
            .expect("access token")
            .to_string(),
    }
}

async fn first_board(app: &Router, org: &Org) -> Uuid {
    let body = body_json(
        send(app, axum::http::Method::GET, "/api/v1/boards", Some(&org.token), None).await,
    )
    .await;
    body.as_array()
        .and_then(|rows| rows.first())
        .and_then(|b| b["id"].as_str())
        .expect("seeded default board")
        .parse()
        .unwrap()
}

async fn create_board(app: &Router, org: &Org, name: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/boards",
        Some(&org.token),
        Some(json!({ "name": name, "is_default": false })),
    )
    .await;
    assert_eq!(response.status(), 200, "board create must succeed");
    body_json(response).await["id"]
        .as_str()
        .expect("board id")
        .parse()
        .unwrap()
}

async fn get_settings(app: &Router, org: &Org) -> Value {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/v1/email/settings",
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "settings read");
    body_json(response).await
}

#[tokio::test]
async fn mail_surface_stays_honest_and_the_probe_rides_the_ledger() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("mail-{}", Uuid::now_v7())).await;
    let board = first_board(&app, &org).await;

    // 1. Nothing configured: the typed not-configured state, not fake zeros.
    let settings = get_settings(&app, &org).await;
    assert_eq!(settings["outbound"]["transport"], "none");
    assert_eq!(settings["outbound"]["ready"], false);
    assert!(settings["outbound"]["not_ready_reason"]
        .as_str()
        .unwrap_or_default()
        .contains("no outbound transport"));
    assert_eq!(settings["outbound"]["credentials"]["smtp"]["configured"], false);

    // The probe refuses with a typed conflict before anything is enqueued.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/email/test-send",
        Some(&org.token),
        Some(json!({ "to": "probe@acme.test" })),
    )
    .await;
    assert_eq!(response.status(), 409, "probe on none-mode is a conflict");
    let conflict = body_json(response).await;
    assert!(conflict["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("not configured"));

    // 2. A transport selection without a credential is still not ready.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/email/outbound-settings",
        Some(&org.token),
        Some(json!({
            "transport": "smtp",
            "from_address": "support@acme.test",
            "from_name": "Acme Support",
            "reply_to": "support@acme.test",
            "smtp_host": "smtp.acme.test",
            "smtp_port": 587,
            "smtp_username": "meridian",
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let settings = body_json(response).await;
    assert_eq!(settings["outbound"]["transport"], "smtp");
    assert_eq!(settings["outbound"]["ready"], false);
    assert!(settings["outbound"]["not_ready_reason"]
        .as_str()
        .unwrap_or_default()
        .contains("password"));

    // 3. Sealing the credential flips readiness; the response carries
    //    presence only — never the material.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/email/transport-credentials",
        Some(&org.token),
        Some(json!({ "transport": "smtp", "smtp_password": "sup3r-secret-pw" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let raw = body_json(response).await;
    let rendered = raw.to_string();
    assert!(!rendered.contains("sup3r-secret-pw"), "secret must never return");
    assert_eq!(raw["smtp"]["configured"], true);

    let settings = get_settings(&app, &org).await;
    assert_eq!(settings["outbound"]["ready"], true);
    assert_eq!(settings["outbound"]["credentials"]["smtp"]["configured"], true);

    // 4. The probe now queues a real message through the ledger.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/email/test-send",
        Some(&org.token),
        Some(json!({ "to": "probe@acme.test" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let probe = body_json(response).await;
    assert_eq!(probe["transport"], "smtp");
    assert_eq!(probe["outcome"], "queued");
    assert!(probe["message_id"].as_str().is_some());

    let ledger = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/email/delivery-ledger?limit=25",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    // Registration queues its address-verification mail through the same
    // ledger, so the org total is verification + probe (and the probe is
    // the most recent row).
    assert_eq!(ledger["counts"]["queued"], 2);
    assert_eq!(ledger["counts"]["sent"], 0);
    assert_eq!(ledger["transport_configured"], true);
    let row = &ledger["rows"][0];
    assert_eq!(row["status"], "queued");
    assert_eq!(row["to_addresses"][0], "probe@acme.test");
    assert_eq!(row["from_address"], "support@acme.test");
    assert_eq!(row["attempts"], 0);

    // 5. Removing the credential returns to the honest resting state.
    let response = send(
        &app,
        axum::http::Method::DELETE,
        "/api/v1/email/transport-credentials?transport=smtp",
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["smtp"]["configured"], false);
    assert_eq!(get_settings(&app, &org).await["outbound"]["ready"], false);

    // Digest policy round-trips through the same settings endpoint.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/email/outbound-settings",
        Some(&org.token),
        Some(json!({ "digest_policy": "disabled" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["digest_policy"], "disabled");
    let _ = board;
}

#[tokio::test]
async fn board_identities_validate_and_clear_per_field() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("identity-{}", Uuid::now_v7())).await;
    let board_one = first_board(&app, &org).await;
    let board_two = create_board(&app, &org, "Field Ops").await;

    // Inbound: board addresses must not collide inside the org.
    let response = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/v1/boards/{board_one}/inbound-address"),
        Some(&org.token),
        Some(json!({ "address": "shared@acme.test" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/v1/boards/{board_two}/inbound-address"),
        Some(&org.token),
        Some(json!({ "address": "SHARED@acme.test" })),
    )
    .await;
    assert_eq!(response.status(), 409, "duplicate address is refused");
    let conflict = body_json(response).await;
    assert!(conflict["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("already in use"));

    // Outbound identity: partial writes keep untouched fields.
    let response = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/v1/boards/{board_one}/email-identity"),
        Some(&org.token),
        Some(json!({
            "from_address": "projects@acme.test",
            "reply_to": "shared@acme.test",
            "signature": "Acme Projects",
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let identity = body_json(response).await;
    assert_eq!(identity["from_address"], "projects@acme.test");
    assert_eq!(identity["from_name"], Value::Null);

    let response = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/v1/boards/{board_one}/email-identity"),
        Some(&org.token),
        Some(json!({ "from_name": "Acme Projects Team" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let identity = body_json(response).await;
    assert_eq!(identity["from_name"], "Acme Projects Team");
    assert_eq!(identity["from_address"], "projects@acme.test", "kept");

    // A second board cannot send from the same address.
    let response = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/v1/boards/{board_two}/email-identity"),
        Some(&org.token),
        Some(json!({ "from_address": "projects@acme.test" })),
    )
    .await;
    assert_eq!(response.status(), 409);
    let conflict = body_json(response).await;
    let message = conflict["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("Service Desk"),
        "conflict must name the OWNING board (the seeded default), got: {message}"
    );

    // Explicit null clears one field and leaves the rest.
    let response = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/v1/boards/{board_one}/email-identity"),
        Some(&org.token),
        Some(json!({ "from_address": null })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let identity = body_json(response).await;
    assert_eq!(identity["from_address"], Value::Null);
    assert_eq!(identity["reply_to"], "shared@acme.test");
    assert_eq!(identity["from_name"], "Acme Projects Team");

    // The settings read carries the board table with the identity applied.
    let settings = get_settings(&app, &org).await;
    let routed = settings["boards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["board_id"] == board_one.to_string())
        .expect("board listed");
    assert_eq!(routed["inbound_address"], "shared@acme.test");
    assert_eq!(routed["identity"]["reply_to"], "shared@acme.test");
}

#[tokio::test]
async fn inbound_catchall_validates_and_org_defaults_round_trip() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("inbound-{}", Uuid::now_v7())).await;
    let board = first_board(&app, &org).await;

    // A board address already owns "taken@": the catch-all refuses it.
    let response = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/v1/boards/{board}/inbound-address"),
        Some(&org.token),
        Some(json!({ "address": "taken@acme.test" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/email/inbound-settings",
        Some(&org.token),
        Some(json!({
            "catchall_address": "taken@acme.test",
            "catchall_board_id": board,
        })),
    )
    .await;
    assert_eq!(response.status(), 409, "catch-all may not shadow a board");

    // A fresh address saves and reads back with the enable state.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/email/inbound-settings",
        Some(&org.token),
        Some(json!({
            "catchall_address": "support@acme.test",
            "catchall_board_id": board,
            "enabled": true,
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let inbound = body_json(response).await;
    assert_eq!(inbound["catchall_address"], "support@acme.test");
    assert_eq!(inbound["enabled"], true);

    // Reply-by-email state derives from the same row.
    let settings = get_settings(&app, &org).await;
    assert_eq!(settings["reply_by_email"]["enabled"], true);
    assert_eq!(settings["reply_by_email"]["board_addresses"], 1);
    assert_eq!(settings["digest_policy"], "allowed");

    // Org notification defaults: admin write then read.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/notification-org-defaults",
        Some(&org.token),
        Some(json!({
            "kind": "sla_breach",
            "in_app": true,
            "email": true,
            "quiet_from": null,
            "quiet_until": null,
            "timezone": "UTC",
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["kind"], "sla_breach");

    let defaults = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/notification-org-defaults",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    let rows = defaults["defaults"].as_array().expect("defaults array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["kind"], "sla_breach");
    assert_eq!(rows[0]["email"], true);

    // An invalid quiet window is a typed 422, not a stored mistake.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/notification-org-defaults",
        Some(&org.token),
        Some(json!({
            "kind": "sla_breach",
            "in_app": true,
            "email": true,
            "quiet_from": "22:00",
            "quiet_until": null,
            "timezone": "UTC",
        })),
    )
    .await;
    assert_eq!(response.status(), 422);
}
