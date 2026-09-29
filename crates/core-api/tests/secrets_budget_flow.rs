//! A1 per-org vault write-budget admission control (Wave 16 lane 9),
//! isolated in its own test binary so the process-global budget
//! statics + env override never race the other secrets-flow tests.

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
        .expect("secrets budget e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
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
            "org_name": "Budget Co",
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
    let list = folders.as_array().expect("folders list");
    list[0]["id"].as_str().unwrap().parse().unwrap()
}

#[tokio::test]
async fn vault_write_budget_returns_typed_429() {
    // A tiny budget: the test org gets 2 mutation seats per window.
    std::env::set_var("MARULA_VAULT_WRITE_BUDGET_PER_MIN", "2");
    let (state, app) = test_state().await;
    // The static bucket may carry residue from earlier tests in this
    // process; that is fine — what matters is that BREACHING the budget
    // yields the typed 429 with a Retry-After. Drive mutations until the
    // budget refuses, then assert the typed shape (bounded rounds).
    let org = register_org(&app, format!("vault3-{}", Uuid::now_v7().simple())).await;
    let bearer = org["access_token"].as_str().unwrap().to_string();
    let folder_id = org_folder(&app, &bearer).await;
    assert!(state.config.vault_write_budget_per_min >= 2);

    let mut saw_429 = false;
    for i in 0..10 {
        let body = json!({
            "folder_id": folder_id,
            "kind": "password",
            "title": format!("budget probe {i}"),
            "fields": [],
            "idempotency_key": format!("probe-{}", Uuid::now_v7().simple()),
        });
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/secrets/records",
            Some(&bearer),
            Some(body),
        )
        .await;
        if response.status() == axum::http::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let problem = body_json(response).await;
            assert_eq!(problem["error"]["reason"], json!("rate_limited"));
            assert!(retry_after.is_some(), "429 must carry Retry-After");
            saw_429 = true;
            break;
        }
        assert_eq!(
            response.status(),
            200,
            "pre-budget mutations succeed: {}",
            body_json(response).await
        );
    }
    assert!(saw_429, "the org budget must eventually refuse");
    std::env::remove_var("MARULA_VAULT_WRITE_BUDGET_PER_MIN");
}
