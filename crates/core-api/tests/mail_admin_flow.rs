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

/// Inbound addresses — a board's `inbound_address` AND an org's
/// `catchall_address` — are GLOBALLY unique, and that is load-bearing
/// rather than incidental:
///
///   * migration 0014 creates `idx_boards_inbound_address` and
///     `idx_email_inbound_catchall_addr` as single-column unique indexes and
///     gives the reason: "globally unique for the same org-resolution
///     reason";
///   * `core_db::emails::resolve_inbound_route` looks an address up with NO
///     org predicate and RETURNS the `org_id` it found —
///     `SELECT org_id, id FROM boards WHERE lower(inbound_address) = ANY($1)
///     LIMIT 1` — because inbound mail carries no org context. The org is
///     derived FROM the address. Two orgs holding one address would make that
///     `LIMIT 1` resolve to a different org on different deliveries.
///
/// So a FIXED inbound address makes this suite non-idempotent: the second run
/// collides with the first run's row, in a different org, and reports a 409
/// where a 200 is expected. Every inbound address below is therefore scoped to
/// `run`, which preserves what these tests actually assert (in-org collisions,
/// case-insensitivity, catch-all-shadows-board) while letting the suite be
/// re-run against the same database.
///
/// `from_address`, `reply_to` and `smtp_host` are NOT globally unique and stay
/// fixed literals.
async fn test_state() -> (AppState, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("mail-admin e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    let reporting_url = admin_url;
    let state = AppState {
        plugin_pool: Default::default(),
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
        send(
            app,
            axum::http::Method::GET,
            "/api/v1/boards",
            Some(&org.token),
            None,
        )
        .await,
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
    assert_eq!(
        settings["outbound"]["credentials"]["smtp"]["configured"],
        false
    );

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
            "smtp_username": "marula",
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
    assert!(
        !rendered.contains("sup3r-secret-pw"),
        "secret must never return"
    );
    assert_eq!(raw["smtp"]["configured"], true);

    let settings = get_settings(&app, &org).await;
    assert_eq!(settings["outbound"]["ready"], true);
    assert_eq!(
        settings["outbound"]["credentials"]["smtp"]["configured"],
        true
    );

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

    // A board's inbound address is GLOBALLY unique, not per-org — see
    // migration 0014 and `resolve_inbound_route`: inbound mail carries no
    // org context, so the org is derived FROM the address
    // (`SELECT org_id, id FROM boards WHERE lower(inbound_address) = ANY($1)
    // LIMIT 1`). Two orgs claiming one address would make that `LIMIT 1`
    // non-deterministic, so the constraint is global on purpose.
    //
    // Consequence for a test: a FIXED address makes the suite
    // non-idempotent, because the second run of the suite collides with
    // the first run's row. The address is therefore scoped to this run,
    // which preserves exactly what this test is about — a case-insensitive
    // collision INSIDE one org — while letting the suite be re-run against
    // the same database.
    let run = Uuid::now_v7().simple();
    let shared = format!("shared-{run}@acme.test");

    // Inbound: board addresses must not collide inside the org.
    let response = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/v1/boards/{board_one}/inbound-address"),
        Some(&org.token),
        Some(json!({ "address": shared })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/v1/boards/{board_two}/inbound-address"),
        Some(&org.token),
        // Only the local part is upper-cased: the collision is
        // case-INSENSITIVE and that is the property worth pinning, while
        // leaving the domain alone keeps the fixture readable and avoids
        // implying that the domain is matched case-sensitively.
        Some(json!({ "address": format!("SHARED-{run}@acme.test") })),
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
    assert_eq!(routed["inbound_address"], shared);
    assert_eq!(routed["identity"]["reply_to"], "shared@acme.test");
}

#[tokio::test]
async fn inbound_catchall_validates_and_org_defaults_round_trip() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("inbound-{}", Uuid::now_v7())).await;
    let board = first_board(&app, &org).await;
    let run = Uuid::now_v7().simple();
    // The board claims `taken` first, so the catch-all below can be shown
    // refusing to shadow it.
    let taken = format!("taken-{run}@acme.test");
    let support = format!("support-{run}@acme.test");

    // A board address already owns "taken@": the catch-all refuses it.
    let response = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/v1/boards/{board}/inbound-address"),
        Some(&org.token),
        Some(json!({ "address": taken })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/email/inbound-settings",
        Some(&org.token),
        Some(json!({
            "catchall_address": taken,
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
            "catchall_address": support,
            "catchall_board_id": board,
            "enabled": true,
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let inbound = body_json(response).await;
    assert_eq!(inbound["catchall_address"], support);
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
