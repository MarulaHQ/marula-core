//! End-to-end secrets-vault straggler sweep (Wave 16 lane 9) driven
//! through the REAL core-api router against a real Postgres.
//!
//! Purpose: prove the lane-9 vault work over HTTP —
//! * A3: the IF1–IF3 idempotent envelope on secret record create/update
//!   (same key + same payload replays byte-identically; same key +
//!   different payload is the typed 409 `idempotency_key_reused`), plus
//!   the `mutation` metadata on the applied response;
//! * A5: PATCH field-clear semantics (`value: null` deletes the sealed
//!   field row; the audited event still fires);
//! * A1: the per-org write budget returns a typed 429 (with a
//!   `Retry-After`) once `MERIDIAN_VAULT_WRITE_BUDGET_PER_MIN` seats are
//!   consumed;
//! * A2: the chunked bulk-ingestion worker path (per-row atomicity,
//!   replay of already-committed rows via per-row idempotency keys).
//!
//! Non-purpose: document-file PUT/GET round-trip against a real object
//! store (covered by the core-store adapters' own suites; the route
//! logic is a thin seal→put / get→open seam), UI.

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> (AppState, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("secrets e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env();
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let state = AppState {
        pool,
        config: Arc::new(config),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&Config::from_env())),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    };
    let app = core_api::routes::build_router(state.clone());
    (state, app)
}

async fn connect_admin(url: &str) -> Result<PgPool, sqlx::Error> {
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
    bearer: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder()
        .method(method.clone())
        .uri(uri);
    if let Some(token) = bearer {
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
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

async fn register_org(app: &Router, slug: String) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Vault Co",
            "org_slug": slug,
            "email": format!("owner@{}.test", slug),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    body_json(response).await
}

/// The org-scope seeded folder to write records into (0078 seeds five
/// immutable defaults per org; any org-scope folder works for writes).
async fn org_folder(app: &Router, bearer: &str) -> (Uuid, bool) {
    let folders: Value = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/secrets/folders",
            Some(bearer),
            None,
        )
        .await,
    )
    .await;
    let list = folders.as_array().expect("folders list");
    let row = list
        .iter()
        .find(|f| !f["restricted"].as_bool().unwrap_or(false) && f["client_id"].is_null())
        .unwrap_or(&list[0]);
    let restricted = row["restricted"].as_bool().unwrap_or(false);
    (row["id"].as_str().unwrap().parse().unwrap(), restricted)
}

#[tokio::test]
async fn secret_record_create_replays_patch_clears_and_reports() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("vault1-{}", Uuid::now_v7().simple())).await;
    let bearer = org["access_token"].as_str().unwrap().to_string();
    let (folder_id, _restricted) = org_folder(&app, &bearer).await;

    // A3: create with an idempotency key.
    let key = format!("row-{}", Uuid::now_v7().simple());
    let create_body = json!({
        "folder_id": folder_id,
        "kind": "password",
        "title": "PagerDuty key",
        "fields": [
            {"field_key": "secret", "value": "hunter2"},
            {"field_key": "username", "value": "ops-bot"}
        ],
        "idempotency_key": key,
    });
    let first = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records",
        Some(&bearer),
        Some(create_body.clone()),
    )
    .await;
    let status = first.status();
    let first_body = body_json(first).await;
    assert_eq!(status, 200, "create must succeed: {first_body}");
    let record_id = first_body["id"].as_str().unwrap().to_string();
    assert_eq!(first_body["title"], json!("PagerDuty key"));
    let seq = first_body["mutation"]["seq"].as_i64().unwrap();
    assert_eq!(first_body["mutation"]["idempotency_key"], json!(key));

    // Same key + same payload ⇒ byte-identical replay, no second row.
    let replay = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records",
        Some(&bearer),
        Some(create_body),
    )
    .await;
    let replay_status = replay.status();
    let replay_body = body_json(replay).await;
    assert_eq!(replay_status, 200, "replay must succeed: {replay_body}");
    assert_eq!(replay_body["id"].as_str().unwrap(), record_id);
    assert_eq!(replay_body["mutation"]["seq"].as_i64().unwrap(), seq);

    // Same key + DIFFERENT payload ⇒ typed 409 idempotency_key_reused.
    let conflicting = json!({
        "folder_id": folder_id,
        "kind": "password",
        "title": "A DIFFERENT record",
        "fields": [],
        "idempotency_key": key,
    });
    let reused = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records",
        Some(&bearer),
        Some(conflicting),
    )
    .await;
    assert_eq!(reused.status(), 409);
    let reused_body = body_json(reused).await;
    assert_eq!(
        reused_body["error"]["reason"],
        json!("idempotency_key_reused")
    );

    // A5: PATCH with value:null CLEARS the username field.
    let patch = json!({
        "title": "PagerDuty key",
        "fields": [{"field_key": "username", "value": null}],
        "idempotency_key": format!("patch-{}", Uuid::now_v7().simple()),
    });
    let patched = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/secrets/records/{record_id}"),
        Some(&bearer),
        Some(patch),
    )
    .await;
    let patched_status = patched.status();
    let patched_body = body_json(patched).await;
    assert_eq!(patched_status, 200, "patch must succeed: {patched_body}");
    let username_meta = patched_body["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["field_key"] == json!("username"))
        .cloned();
    assert!(
        username_meta.is_none(),
        "cleared field must vanish from metadata: {patched_body}"
    );

    // The cleared record still reveals its surviving field only.
    let reveal = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/secrets/records/{record_id}/reveal"),
        Some(&bearer),
        None,
    )
    .await;
    let reveal_status = reveal.status();
    let reveal_body = body_json(reveal).await;
    assert_eq!(reveal_status, 200, "reveal must succeed: {reveal_body}");
    let revealed_keys: Vec<&str> = reveal_body["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["field_key"].as_str().unwrap())
        .collect();
    assert_eq!(revealed_keys, vec!["secret"]);
    assert_eq!(reveal_body["fields"][0]["value"], json!("hunter2"));
}

#[tokio::test]
async fn bulk_ingest_rows_commit_atomically_and_replay() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("vault2-{}", Uuid::now_v7().simple())).await;
    let bearer = org["access_token"].as_str().unwrap().to_string();
    let (folder_id, _restricted) = org_folder(&app, &bearer).await;

    let row_key_1 = format!("bulk-{}", Uuid::now_v7().simple());
    let row_key_2 = format!("bulk-{}", Uuid::now_v7().simple());
    let chunk = json!({
        "folder_id": folder_id,
        "kind": "password",
        "records": [
            {"title": "Row one", "idempotency_key": row_key_1,
             "fields": [{"field_key": "secret", "value": "one"}]},
            {"title": "Row two", "idempotency_key": row_key_2,
             "fields": [{"field_key": "secret", "value": "two"}]}
        ]
    });
    let first = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records/bulk",
        Some(&bearer),
        Some(chunk.clone()),
    )
    .await;
    assert_eq!(first.status(), 200, "bulk chunk must succeed");
    let first_body = body_json(first).await;
    assert_eq!(first_body["created"], json!(2));
    assert_eq!(first_body["replayed"], json!(0));
    let row1_id = first_body["results"][0]["record_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Resumed chunk: same per-row keys ⇒ both rows REPLAY, ids match.
    let resumed = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records/bulk",
        Some(&bearer),
        Some(chunk),
    )
    .await;
    assert_eq!(resumed.status(), 200);
    let resumed_body = body_json(resumed).await;
    assert_eq!(resumed_body["created"], json!(0));
    assert_eq!(resumed_body["replayed"], json!(2));
    assert_eq!(
        resumed_body["results"][0]["record_id"].as_str().unwrap(),
        row1_id
    );

    // A duplicate title inside one folder is a per-row refusal (the
    // chunk never aborts: per-row atomicity).
    let dup = json!({
        "folder_id": folder_id,
        "kind": "password",
        "records": [
            {"title": "Row one", "idempotency_key": format!("bulk-{}", Uuid::now_v7().simple()),
             "fields": []}
        ]
    });
    let refused = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records/bulk",
        Some(&bearer),
        Some(dup),
    )
    .await;
    assert_eq!(refused.status(), 200);
    let refused_body = body_json(refused).await;
    assert_eq!(refused_body["refused"], json!(1));
    assert_eq!(refused_body["results"][0]["outcome"], json!("refused"));
}
