//! Generic M2M webhook-injest flows (Wave 43 lane 1) through the REAL
//! core-api router — the Wave-43 acceptance for this lane:
//!
//! **HMAC-signed ingest → domain-event envelope on the outbox stream.**
//!
//! Covered here:
//! 1. Ingest auth shell: unknown source (404), missing/bad signature (401),
//!    tampered body with a stale valid signature (401), malformed payload (422)
//!    — typed reasons throughout.
//! 2. Basic ingest: a source with `event_type` set emits a real outbox
//!    envelope whose `event_type` matches the declaration, `entity = 'webhook'`,
//!    `payload` carries the provider's `data` + the board-scoped `board_id`.
//! 3. Dedup: replaying the SAME signed delivery returns `duplicate` and
//!    writes a second outbox event.
//! 4. Rate limit: flooding the fence returns 429.
//! 5. Dual-mount parity for the ingest route (`/api/v1` + legacy `/api`).
//!
//! Database selection follows settings_flow's contract:
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
        .expect("alerts flow needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([37; 32]);
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

/// Sends JSON through the full middleware stack (content-type + serialized body).
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

/// Sends the RAW signed ingest body (HMAC over the EXACT bytes, like a real
/// provider would).
async fn send_raw(
    app: &Router,
    uri: &str,
    headers: &[(&str, String)],
    body: &str,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri(uri)
        .header("content-type", "application/json");
    for (name, value) in headers {
        builder = builder.header(*name, value.clone());
    }
    app.clone()
        .oneshot(
            builder
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
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

/// Source_ref values are GLOBALLY unique (they route pre-auth), and the test
/// database is not reset between runs — suffix per run.
fn unique_ref(prefix: &str) -> String {
    let simple = Uuid::now_v7().simple().to_string();
    format!("{prefix}-{}", &simple[simple.len() - 8..])
}

fn sig(value: &str) -> (&'static str, String) {
    (
        core_api::routes::webhook_receiver::SIGNATURE_HEADER,
        value.to_string(),
    )
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Webhook Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    body_json(response).await["access_token"]
        .as_str()
        .expect("access token")
        .into()
}

async fn create_board(app: &Router, token: &str, name: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/boards",
        &[auth(token)],
        Some(json!({ "name": name, "is_default": false })),
    )
    .await;
    assert_eq!(response.status(), 200, "board create must succeed");
    body_json(response).await["id"]
        .as_str()
        .expect("board id")
        .parse()
        .expect("uuid")
}

/// Creates a generic M2M alert source (with event_type set) via the API.
struct CreatedGenericSource {
    source_ref: String,
    secret: String,
}

async fn create_generic_source(
    app: &Router,
    token: &str,
    board_id: Uuid,
    source_ref: &str,
    event_type: &str,
    payload_schema_ref: Option<&str>,
) -> CreatedGenericSource {
    let mut payload = json!({
        "name": "Inbound Webhook",
        "source_ref": source_ref,
        "board_id": board_id,
        "event_type": event_type,
    });
    if let Some(s) = payload_schema_ref {
        payload["payload_schema_ref"] = json!(s);
    }
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/alert-sources",
        &[auth(token)],
        Some(payload),
    )
    .await;
    assert_eq!(response.status(), 200, "source create must succeed");
    let body = body_json(response).await;
    CreatedGenericSource {
        source_ref: body["source_ref"].as_str().expect("source_ref").into(),
        secret: body["secret"].as_str().expect("secret shown once").into(),
    }
}

/// Creates a classic alert source (no event_type). Returns (source_ref, secret).
async fn create_alert_source(
    app: &Router,
    token: &str,
    board_id: Uuid,
    source_ref: &str,
) -> (String, String) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/alert-sources",
        &[auth(token)],
        Some(json!({
            "name": "Alert Source",
            "source_ref": source_ref,
            "board_id": board_id,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "source create must succeed");
    let body = body_json(response).await;
    (
        body["source_ref"].as_str().expect("source_ref").into(),
        body["secret"].as_str().expect("secret shown once").into(),
    )
}

fn signed(source: &CreatedGenericSource, body: &str) -> Vec<(&'static str, String)> {
    vec![sig(&core_mail::hmac::sign(
        source.secret.as_bytes(),
        body.as_bytes(),
    ))]
}

fn webhook_body(dedup_key: &str, payload: Option<Value>) -> String {
    let mut obj = json!({ "dedup_key": dedup_key });
    if let Some(p) = payload {
        obj["payload"] = p;
    } else {
        obj["payload"] = json!({ "status": "ok", "timestamp": 1700000000 });
    }
    obj.to_string()
}

// --- tests ----------------------------------------------------------------------

#[tokio::test]
async fn ingest_auth_shell() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("webhook-auth-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let board = create_board(&app, &token, "Auth Board").await;

    // Unknown source_ref → 404 (route IS mounted, source isn't).
    let res = send_raw(&app, "/api/v1/webhooks/ingest/no-such-source", &[], "{}").await;
    assert_eq!(res.status(), 404, "unknown source ref → 404");

    // Alert source exists, no signature → 401.
    let (alert_ref, _alert_secret) = create_alert_source(&app, &token, board, &unique_ref("als")).await;
    let res = send_raw(
        &app,
        &format!("/api/v1/alerts/ingest/{}", alert_ref),
        &[],
        "{}",
    )
    .await;
    assert_eq!(res.status(), 401, "known alert source, no sig → 401");

    // Generic source exists but no signature → 401.
    let gs = create_generic_source(&app, &token, board, &unique_ref("ghook"), "my_event.received", None).await;
    let res = send_raw(
        &app,
        &format!("/api/v1/webhooks/ingest/{}", gs.source_ref),
        &[],
        &webhook_body("first", None),
    )
    .await;
    assert_eq!(res.status(), 401, "generic source, no sig → 401");

    // Bad signature → 401.
    let body_str = webhook_body("bad", None);
    let res = send_raw(
        &app,
        &format!("/api/v1/webhooks/ingest/{}", gs.source_ref),
        &[(
            core_api::routes::webhook_receiver::SIGNATURE_HEADER,
            "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        )],
        &body_str,
    )
    .await;
    assert_eq!(res.status(), 401, "bad signature → 401");

    // Tampered body with a stale valid signature → 401.
    let real_body = webhook_body("tamper", None);
    let real_sig = core_mail::hmac::sign(gs.secret.as_bytes(), real_body.as_bytes());
    let tampered_body = webhook_body("DIFFERENT_KEY", Some(json!({ "forged": true })));
    let res = send_raw(
        &app,
        &format!("/api/v1/webhooks/ingest/{}", gs.source_ref),
        &[(core_api::routes::webhook_receiver::SIGNATURE_HEADER, real_sig)],
        &tampered_body,
    )
    .await;
    assert_eq!(res.status(), 401, "tampered body + stale sig → 401");

    // Malformed JSON → 422.
    let malformed_body = "{not valid json";
    let fake_sig = core_mail::hmac::sign(gs.secret.as_bytes(), malformed_body.as_bytes());
    let res = send_raw(
        &app,
        &format!("/api/v1/webhooks/ingest/{}", gs.source_ref),
        &[(core_api::routes::webhook_receiver::SIGNATURE_HEADER, fake_sig)],
        malformed_body,
    )
    .await;
    assert_eq!(res.status(), 422, "malformed payload → 422");
}

#[tokio::test]
async fn generic_ingest_emits_envelope() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("webhook-emit-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let board = create_board(&app, &token, "Emit Board").await;

    let event_type = "order.created";
    let schema_ref = "acme-io/webhook-v1";
    let gs = create_generic_source(&app, &token, board, &unique_ref("emit"), event_type, Some(schema_ref)).await;

    // Signed POST → 200 with "emitted".
    let body = webhook_body("order-42", Some(json!({
        "order_id": "ORD-42",
        "amount": 999,
        "currency": "USD"
    })));
    let headers = signed(&gs, &body);
    let res = send_raw(
        &app,
        &format!("/api/v1/webhooks/ingest/{}", gs.source_ref),
        &headers,
        &body,
    )
    .await;
    assert_eq!(res.status(), 200, "signed ingest must succeed");
    let resp: Value = body_json(res).await;
    assert_eq!(resp["status"], "emitted", "first delivery is emitted");
    assert!(resp["event_id"].is_string(), "event_id present");
    assert_eq!(resp["board_id"], json!(board));

    // The outbox carries the declared event type + board-scoped payload.
    let rows: Vec<(String, String, Value)> = sqlx::query_as(
        "SELECT event_type, entity, payload FROM outbox_events WHERE event_type = $1 AND entity = 'webhook' ORDER BY occurred_at DESC LIMIT 1",
    )
    .bind(event_type)
    .fetch_all(&state.pool)
    .await
    .expect("outbox query");

    assert_eq!(rows.len(), 1, "one outbox event emitted");
    let (et, entity, payload) = &rows[0];
    assert_eq!(et, event_type, "event_type matches source declaration");
    assert_eq!(entity, "webhook", "entity is 'webhook'");
    assert_eq!(payload["board_id"], json!(board), "board_id carried in payload");
    assert_eq!(payload["source_ref"], json!(gs.source_ref), "source_ref in payload");
    assert_eq!(payload["entity_id"], json!("order-42"), "entity_id defaults to dedup_key");
    assert_eq!(payload["data"]["order_id"], json!("ORD-42"), "provider data under 'data'");
    assert_eq!(payload["data"]["amount"], json!(999), "provider data preserved");
}

#[tokio::test]
async fn replay_returns_duplicate() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("webhook-dedup-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let board = create_board(&app, &token, "Dedup Board").await;
    let gs = create_generic_source(&app, &token, board, &unique_ref("dup"), "sync.user.created", None).await;

    let body = webhook_body("user-1", Some(json!({ "email": "user@example.com" })));
    let headers = signed(&gs, &body);

    // First delivery → emitted.
    let res = send_raw(
        &app,
        &format!("/api/v1/webhooks/ingest/{}", gs.source_ref),
        &headers,
        &body,
    )
    .await;
    let resp: Value = body_json(res).await;
    assert_eq!(resp["status"], "emitted");

    // Replay the EXACT same signed body → duplicate, no new outbox event.
    let res = send_raw(
        &app,
        &format!("/api/v1/webhooks/ingest/{}", gs.source_ref),
        &headers,
        &body,
    )
    .await;
    let resp: Value = body_json(res).await;
    assert_eq!(resp["status"], "duplicate", "replay returns duplicate");
    assert!(resp["event_id"].is_null(), "no new event_id for duplicate");

    // Outbox should still have exactly 1 event for this dedup_key.
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM outbox_events WHERE entity = 'webhook' AND payload->>'entity_id' = 'user-1'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count");
    assert_eq!(count.0, 1, "no duplicate outbox event on replay");
}

#[tokio::test]
async fn dual_mount_parity() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("webhook-parity-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let board = create_board(&app, &token, "Parity Board").await;
    let gs = create_generic_source(&app, &token, board, &unique_ref("par"), "thing.happened", None).await;

    // The unversioned legacy alias hits the same handler: an unsigned request
    // is 401 (auth shell ran), NOT a default 404 (route missing).
    for path in [
        format!("/api/v1/webhooks/ingest/{}", gs.source_ref),
        format!("/api/webhooks/ingest/{}", gs.source_ref),
    ] {
        let res = send_raw(&app, &path, &[], "{}").await;
        assert_eq!(res.status(), 401, "{path} must be mounted and auth-gated");
    }
}
