//! Entitlement artifact adoption, end to end against a real database
//! (RFC 0023): the DB path the unit tests cannot cover — RLS, the atomic
//! monotonic guard, and the two handlers over HTTP.
//!
//! What only this test can prove: that the migration applies, that the guard
//! refuses a stale revision under a real transaction, that a replay answers
//! `unchanged` without writing, that an artifact signed for another tenant is
//! refused at the org boundary, and that a caller cannot address another org's
//! path.
//!
//! Needs `MARULA_TEST_DATABASE_URL` or `DATABASE_URL`, like its neighbours.

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use ed25519_dalek::Signer as _;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const SCHEMA: &str = "marula.entitlement/v1";

async fn test_state() -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("entitlement e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&admin_url)
        .await
        .expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([13; 32]);
    AppState {
        plugin_pool: Default::default(),
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

/// Registers an org with a slug unique to this run, so repeated runs against a
/// shared test database do not collide. The register answer carries tokens but
/// not the org id, so the id is read back the way the neighbouring flow tests
/// do.
async fn register_org(app: &Router, state: &AppState) -> (String, Uuid) {
    let slug = format!("ent{}", Uuid::now_v7().simple());
    let email = format!("owner@{slug}.test");
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Entitlement Co",
            "org_slug": slug,
            "email": email,
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let token = body_json(response).await["access_token"]
        .as_str()
        .expect("access token")
        .to_string();
    let row: (Uuid,) = sqlx::query_as("SELECT org_id FROM users WHERE email = $1")
        .bind(&email)
        .fetch_one(&state.pool)
        .await
        .expect("org lookup for the registered user");
    (token, row.0)
}

/// Adds a verification key to the operator registry — the same table the CLI
/// writes — and returns the signing half.
async fn trust_key(state: &AppState, key_id: &str) -> ed25519_dalek::SigningKey {
    let signing = ed25519_dalek::SigningKey::from_bytes(&[0x5a; 32]);
    sqlx::query(
        "INSERT INTO plugin_signing_keys (id, key_id, public_key, label)
         VALUES ($1, $2, $3, 'entitlement e2e')
         ON CONFLICT (key_id) DO NOTHING",
    )
    .bind(Uuid::now_v7())
    .bind(key_id)
    .bind(hex::encode(signing.verifying_key().as_bytes()))
    .execute(&state.pool)
    .await
    .expect("insert signing key");
    signing
}

/// Mints the request the issuer would send: the artifact with a payload
/// digest, signed over its canonical JSON.
fn signed_request(
    org_id: Uuid,
    revision: i64,
    payload: Value,
    key: &ed25519_dalek::SigningKey,
) -> Value {
    let payload_sha256 =
        plugin_host::distribution::manifest_sha256(&payload).expect("payload is canonical json");
    let artifact = json!({
        "schema": SCHEMA,
        "org_id": org_id.to_string(),
        "revision": revision,
        "issued_at": "2026-10-27T00:00:00Z",
        "payload": payload,
        "payload_sha256": payload_sha256,
    });
    let digest_hex =
        plugin_host::distribution::manifest_sha256(&artifact).expect("artifact is canonical json");
    let digest: [u8; 32] = hex::decode(&digest_hex).unwrap().try_into().unwrap();
    json!({
        "artifact": artifact,
        "signature": { "key_id": "claimed", "sig": hex::encode(key.sign(&digest).to_bytes()) },
    })
}

async fn adopt(
    app: &Router,
    token: &str,
    org_id: Uuid,
    body: Value,
) -> (axum::http::StatusCode, Value) {
    let response = send(
        app,
        axum::http::Method::PUT,
        &format!("/api/v1/orgs/{org_id}/entitlements"),
        &[auth(token)],
        Some(body),
    )
    .await;
    let status = response.status();
    (status, body_json(response).await)
}

#[tokio::test]
async fn adoption_is_monotonic_and_reports_what_it_holds() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let key_id = format!("e2e-{}", Uuid::now_v7().simple());
    let key = trust_key(&state, &key_id).await;
    let (token, org_id) = register_org(&app, &state).await;

    // Nothing is held yet, and "nothing held" is a null, not revision zero.
    let held = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/orgs/{org_id}/entitlements"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(held.status(), 200, "read must succeed");
    assert!(body_json(held).await["entitlement"].is_null());

    // First adoption.
    let (status, body) = adopt(
        &app,
        &token,
        org_id,
        signed_request(org_id, 7, json!({ "seats": 25 }), &key),
    )
    .await;
    assert_eq!(status, 200, "first adoption: {body}");
    assert_eq!(body["outcome"], "adopted");
    assert_eq!(body["entitlement"]["revision"], 7);
    assert_eq!(body["entitlement"]["payload"]["seats"], 25);

    // A replay of the same revision and content changes nothing.
    let (status, body) = adopt(
        &app,
        &token,
        org_id,
        signed_request(org_id, 7, json!({ "seats": 25 }), &key),
    )
    .await;
    assert_eq!(status, 200, "replay: {body}");
    assert_eq!(body["outcome"], "unchanged");

    // Same revision, different content: the issuer contradicted itself.
    let (status, body) = adopt(
        &app,
        &token,
        org_id,
        signed_request(org_id, 7, json!({ "seats": 500 }), &key),
    )
    .await;
    assert_eq!(status, 409, "same revision, new content: {body}");

    // A lower revision is older news.
    let (status, body) = adopt(
        &app,
        &token,
        org_id,
        signed_request(org_id, 6, json!({ "seats": 1 }), &key),
    )
    .await;
    assert_eq!(status, 409, "stale revision: {body}");
    assert!(
        body.to_string().contains("stale"),
        "the refusal must say it is stale: {body}"
    );

    // A higher revision replaces, and the read agrees with the write.
    let (status, body) = adopt(
        &app,
        &token,
        org_id,
        signed_request(org_id, 8, json!({ "seats": 30 }), &key),
    )
    .await;
    assert_eq!(status, 200, "newer revision: {body}");
    assert_eq!(body["outcome"], "adopted");
    let held = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/orgs/{org_id}/entitlements"),
        &[auth(&token)],
        None,
    )
    .await;
    let held = body_json(held).await;
    assert_eq!(held["entitlement"]["revision"], 8);
    assert_eq!(held["entitlement"]["payload"]["seats"], 30);
    // The verifying key is reported, not the identity the issuer claimed.
    assert_eq!(held["entitlement"]["key_id"], key_id);
}

#[tokio::test]
async fn refusals_never_write_and_never_cross_a_tenant() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let key_id = format!("e2e-{}", Uuid::now_v7().simple());
    let key = trust_key(&state, &key_id).await;
    let (token, org_id) = register_org(&app, &state).await;
    let (other_token, other_org) = register_org(&app, &state).await;

    // A payload edited after signing.
    let mut tampered = signed_request(org_id, 1, json!({ "seats": 1 }), &key);
    tampered["artifact"]["payload"] = json!({ "seats": 9999 });
    let (status, body) = adopt(&app, &token, org_id, tampered).await;
    assert_eq!(status, 422, "tampered payload: {body}");
    assert!(
        body.to_string().contains("payload_digest_mismatch"),
        "{body}"
    );

    // Another tenant's artifact, validly signed, adopted into this org's path.
    let (status, body) = adopt(
        &app,
        &token,
        org_id,
        signed_request(other_org, 1, json!({ "seats": 1 }), &key),
    )
    .await;
    assert_eq!(status, 422, "foreign artifact: {body}");
    assert!(body.to_string().contains("org_mismatch"), "{body}");

    // A caller with a valid token cannot address another org's path.
    let (status, body) = adopt(
        &app,
        &other_token,
        org_id,
        signed_request(org_id, 1, json!({ "seats": 1 }), &key),
    )
    .await;
    assert_eq!(status, 404, "cross-org path: {body}");

    // None of the refusals wrote anything.
    let held = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/orgs/{org_id}/entitlements"),
        &[auth(&token)],
        None,
    )
    .await;
    assert!(
        body_json(held).await["entitlement"].is_null(),
        "a refused artifact must leave the org holding nothing"
    );
}
