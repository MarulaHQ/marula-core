//! Client-scope vault records (Wave 44 lane 7, p44-ui-knowledge-automation)
//! driven through the REAL core-api router against a real Postgres.
//!
//! Regression for the scope-lookup bug this lane found while staging the
//! /passwords client vault: `secret_vault::create_record` resolved the owning
//! folder with `sf.org_id = $1`, but client-scope folders carry a NULL
//! `org_id` (the exactly-one-of invariant from migration 0078), so every
//! create in a client vault 404'd — the client vault could never carry a
//! record except by fork, which copies folders only.
//!
//! Proves:
//! * an org-scope record still creates (the pre-existing path, unchanged);
//! * a record creates inside a CLIENT-scope folder (forked into the org's
//!   client) when the record carries the matching client scope;
//! * the scope guard still refuses a mismatched write (an org-scope record
//!   into the client folder) rather than silently re-scoping it;
//! * the record is then READABLE back through the client-scope listing that
//!   /passwords queries.
//!
//! Non-purpose: the restricted-chain grant gate has its own lane-19 tests.

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
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("client-vault e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url.clone();
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
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
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

/// A freshly registered org (which seeds the default secret folder tree),
/// returning the owner bearer and the seeded org-scope folder id.
async fn register_org(app: &Router, slug: String) -> (String, String) {
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
    let body = body_json(response).await;
    let bearer = body["access_token"]
        .as_str()
        .or_else(|| body["token"].as_str())
        .expect("register returns an access token")
        .to_string();
    let folders = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/secrets/folders",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let folder = folders
        .as_array()
        .expect("folders list")
        .iter()
        .find(|f| f["client_id"].is_null())
        .expect("an org-scope folder exists after seeding")["id"]
        .as_str()
        .unwrap()
        .to_string();
    (bearer, folder)
}

#[tokio::test]
async fn a_record_creates_inside_a_client_scope_folder() {
    let (_state, app) = test_state().await;
    let slug = format!("client-vault-{}", Uuid::now_v7().simple());
    let (bearer, org_folder) = register_org(&app, slug).await;

    // A client of the calling org.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/clients",
        Some(&bearer),
        Some(json!({ "name": "Acme" })),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "client create must succeed: {}",
        response.status()
    );
    let client_id = body_json(response).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Fork the org folder into the client scope — the only sanctioned way a
    // client-scope category comes to exist.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/secrets/folders/{org_folder}/fork"),
        Some(&bearer),
        Some(json!({ "client_id": client_id })),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "fork must succeed: {}",
        response.status()
    );
    let forked = body_json(response).await;
    let client_folder = forked[0]["id"].as_str().unwrap().to_string();
    assert!(forked[0]["client_id"].as_str() == Some(client_id.as_str()));

    // The regression: creating a record inside the client-scope folder used
    // to 404 because the folder lookup only matched org-scope rows.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records",
        Some(&bearer),
        Some(json!({
            "client_id": client_id,
            "folder_id": client_folder,
            "kind": "password",
            "title": "Client vault login",
            "fields": [{"field_key": "secret", "value": "hunter2"}],
            "idempotency_key": format!("row-{}", Uuid::now_v7().simple()),
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "a record must create inside a client-scope folder"
    );
    let created = body_json(response).await;
    assert_eq!(created["client_id"].as_str(), Some(client_id.as_str()));
    assert_eq!(created["folder_id"].as_str(), Some(client_folder.as_str()));

    // The scope guard still refuses a mismatched write: an org-scope record
    // into the client folder must not silently re-scope or be accepted.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records",
        Some(&bearer),
        Some(json!({
            "client_id": Value::Null,
            "folder_id": client_folder,
            "kind": "password",
            "title": "Scope mismatch probe",
            "fields": [{"field_key": "secret", "value": "nope"}],
            "idempotency_key": format!("row-{}", Uuid::now_v7().simple()),
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        409,
        "an org-scope record into a client folder is refused, not re-scoped"
    );

    // The client vault's own listing (/passwords queries it) sees the record.
    let listing = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/secrets/records?folder_id={client_folder}&client_id={client_id}"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let titles: Vec<&str> = listing
        .as_array()
        .expect("records list")
        .iter()
        .filter_map(|r| r["title"].as_str())
        .collect();
    assert!(
        titles.contains(&"Client vault login"),
        "the client listing carries the created record, got {titles:?}"
    );
    assert!(
        !titles.contains(&"Scope mismatch probe"),
        "the refused record never lands"
    );
}

#[tokio::test]
async fn an_org_scope_record_still_creates() {
    let (_state, app) = test_state().await;
    let slug = format!("org-vault-{}", Uuid::now_v7().simple());
    let (bearer, org_folder) = register_org(&app, slug).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records",
        Some(&bearer),
        Some(json!({
            "client_id": Value::Null,
            "folder_id": org_folder,
            "kind": "password",
            "title": "Org vault login",
            "fields": [{"field_key": "secret", "value": "hunter2"}],
            "idempotency_key": format!("row-{}", Uuid::now_v7().simple()),
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "the org-scope create path stays green: {}",
        response.status()
    );
}
