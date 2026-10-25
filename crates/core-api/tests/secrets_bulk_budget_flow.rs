//! Bulk vault ingestion pays its write budget PER ROW (Wave 54 lane S4b).
//!
//! `bulk_ingest` used to admit a whole chunk on one write-budget seat, so a
//! caller could write up to `BULK_CHUNK_CAP` sealed records against a budget
//! that promised one write per seat, and the org's noisy-neighbor floor
//! bounded requests rather than rows. The fix charges one seat per row and
//! decides the request's whole total in ONE verdict: a chunk that does not
//! fit is refused with the existing typed 429 before any row is sealed.
//!
//! Isolated in its own test binary, like `secrets_budget_flow`: the budget
//! limit and the pacer arrive through process-global env vars.

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> (AppState, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("bulk budget e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&admin_url)
        .await
        .expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let state = AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    };
    let app = core_api::routes::build_router(state.clone());
    (state, app)
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
            "org_name": "Bulk Budget Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    body_json(response).await
}

async fn org_folder(app: &Router, bearer: &str) -> Uuid {
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
    folders.as_array().expect("folders list")[0]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

fn chunk(folder_id: Uuid, rows: usize) -> Value {
    let records: Vec<Value> = (0..rows)
        .map(|i| {
            json!({
                "title": format!("bulk row {i}"),
                "fields": [],
                "idempotency_key": format!("row-{}", Uuid::now_v7().simple()),
            })
        })
        .collect();
    json!({
        "folder_id": folder_id,
        "kind": "password",
        "records": records,
    })
}

async fn stored_records(state: &AppState, org_id: Uuid, user_id: Uuid) -> i64 {
    let mut tx = state.pool.begin().await.expect("count tx");
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .expect("org GUC");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM secret_records WHERE org_id = $1")
        .bind(org_id)
        .fetch_one(&mut *tx)
        .await
        .expect("secret_records count");
    tx.rollback().await.expect("count rollback");
    count
}

#[tokio::test]
async fn a_bulk_chunk_pays_one_seat_per_row() {
    // Four seats per window, pacer off: a 3-row chunk leaves one seat, and
    // the 2-row chunk that follows cannot fit.
    std::env::set_var("MARULA_VAULT_WRITE_BUDGET_PER_MIN", "4");
    std::env::set_var("MARULA_VAULT_BULK_ROWS_PER_SEC", "0");
    let (state, app) = test_state().await;
    assert_eq!(state.config.vault_write_budget_per_min, 4);

    let org = register_org(&app, format!("vaultbulk-{}", Uuid::now_v7().simple())).await;
    let bearer = org["access_token"].as_str().unwrap().to_string();
    let me = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/me",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let org_id: Uuid = me["org_id"].as_str().unwrap().parse().unwrap();
    let user_id: Uuid = me["user_id"].as_str().unwrap().parse().unwrap();
    let folder_id = org_folder(&app, &bearer).await;

    // Three rows cost three seats and land.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records/bulk",
        Some(&bearer),
        Some(chunk(folder_id, 3)),
    )
    .await;
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, 200, "a chunk inside the budget lands: {body}");
    assert_eq!(body["created"], 3, "every row of the chunk is applied");
    assert_eq!(stored_records(&state, org_id, user_id).await, 3);

    // Two more rows need five seats in a four-seat window: refused whole.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records/bulk",
        Some(&bearer),
        Some(chunk(folder_id, 2)),
    )
    .await;
    assert_eq!(
        response.status(),
        axum::http::StatusCode::TOO_MANY_REQUESTS,
        "a chunk that does not fit the remaining budget is refused"
    );
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    assert!(retry_after.is_some(), "the typed 429 carries Retry-After");
    let problem = body_json(response).await;
    assert_eq!(problem["error"]["reason"], json!("rate_limited"));
    assert_eq!(
        stored_records(&state, org_id, user_id).await,
        3,
        "the refused chunk wrote nothing — the verdict is all-or-nothing"
    );

    std::env::remove_var("MARULA_VAULT_WRITE_BUDGET_PER_MIN");
    std::env::remove_var("MARULA_VAULT_BULK_ROWS_PER_SEC");
}
