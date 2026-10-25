//! Money-path flow for Wave 54 lane S3c (`p54-money-and-roles`), driven
//! through the REAL core-api router against a real Postgres.
//!
//! Purpose: prove an SLA service credit is priced only in the contract's
//! currency. The write path refuses a mismatch with the same typed 422 its
//! sibling credit-note write uses, and an evaluation refuses a credit that
//! predates that guard (naming the credit) instead of pricing it in the wrong
//! currency or dropping it silently.
//!
//! Non-purpose: engine arithmetic (core-billing unit tests, including the
//! FEFO block draw), the credit-note surface, and the invoice run pipeline.

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
        .expect("money e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    let state = AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&admin_url)
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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Money Co",
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
        .to_string()
}

/// One active T&M contract in `currency`, seeded directly so the test owns
/// the contract currency the credit must match.
async fn seed_contract(state: &AppState, slug: &str, currency: &str) -> Uuid {
    let org_id: Uuid = sqlx::query_scalar("SELECT id FROM orgs WHERE slug = $1")
        .bind(slug)
        .fetch_one(&state.pool)
        .await
        .expect("org row");
    let client_id = Uuid::now_v7();
    let contract_id = Uuid::now_v7();
    sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, 'Money Client')")
        .bind(client_id)
        .bind(org_id)
        .execute(&state.pool)
        .await
        .expect("client row");
    sqlx::query(
        "INSERT INTO contracts (id, org_id, client_id, number, name, contract_type, status, \
         currency, hourly_rate_minor, effective_from) \
         VALUES ($1, $2, $3, $4, 'Money T&M', 'time_and_materials', 'active', $5, 15000, $6)",
    )
    .bind(contract_id)
    .bind(org_id)
    .bind(client_id)
    .bind(format!(
        "C-MONEY-{}",
        &contract_id.simple().to_string()[..8]
    ))
    .bind(currency)
    .bind(chrono::Utc::now().date_naive() - chrono::Duration::days(30))
    .execute(&state.pool)
    .await
    .expect("contract row");
    contract_id
}

#[tokio::test]
async fn sla_credit_in_another_currency_is_refused_with_the_typed_answer() {
    let (state, app) = test_state().await;
    let slug = format!("money-fx-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let contract_id = seed_contract(&state, &slug, "EUR").await;
    let today = chrono::Utc::now().date_naive();

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/contracts/{contract_id}/sla-credits"),
        Some(&owner),
        Some(json!({
            "reason": "resolution target breached by 2d",
            "amount_minor": 15_000,
            "currency": "USD",
            "applies_on": today.to_string(),
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        422,
        "a USD credit on a EUR contract is a validation failure"
    );
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "validation_failed");
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("USD") && message.contains("EUR"),
        "the refusal names both currencies: {message}"
    );

    // Nothing was stored: a refused write leaves no row the engine could
    // later price in the wrong currency.
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM contract_sla_credits WHERE contract_id = $1")
            .bind(contract_id)
            .fetch_one(&state.pool)
            .await
            .expect("credit count");
    assert_eq!(stored, 0, "the refused credit must not be stored");
}

#[tokio::test]
async fn sla_credit_in_the_contract_currency_prices_as_a_negative_line() {
    let (state, app) = test_state().await;
    let slug = format!("money-same-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let contract_id = seed_contract(&state, &slug, "EUR").await;
    let today = chrono::Utc::now().date_naive();

    // Lower case on the wire, the contract's code on the row: the currency
    // is normalized, so the comparison is by code and not by spelling.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/contracts/{contract_id}/sla-credits"),
        Some(&owner),
        Some(json!({
            "reason": "resolution target breached by 2d",
            "amount_minor": 15_000,
            "currency": "eur",
            "applies_on": today.to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "a same-currency credit is accepted");
    let credit = body_json(response).await;
    assert_eq!(credit["currency"], "EUR", "stored in the contract's code");

    let from = (today - chrono::Duration::days(30)).to_string();
    let to = today.to_string();
    let preview = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/contracts/{contract_id}/billing-preview?from={from}&to={to}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(preview.status(), 200, "preview must succeed");
    let evaluation = body_json(preview).await;
    let credits: Vec<&Value> = evaluation["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .filter(|l| l["kind"] == "sla_credit")
        .collect();
    assert_eq!(credits.len(), 1, "the credit prices once: {evaluation}");
    assert_eq!(credits[0]["amount_minor"], -15_000, "a credit is negative");
}

#[tokio::test]
async fn legacy_mismatched_credit_refuses_the_evaluation_and_names_the_credit() {
    let (state, app) = test_state().await;
    let slug = format!("money-legacy-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let contract_id = seed_contract(&state, &slug, "EUR").await;
    let today = chrono::Utc::now().date_naive();

    // A row written BEFORE the write-path guard: insert it straight into the
    // table, which is the only way to reach this state now.
    let credit_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO contract_sla_credits (id, org_id, contract_id, reason, amount_minor, \
         currency, applies_on, status) \
         SELECT $1, c.org_id, c.id, 'legacy USD credit', 15000, 'USD', $2, 'pending' \
         FROM contracts c WHERE c.id = $3",
    )
    .bind(credit_id)
    .bind(today)
    .bind(contract_id)
    .execute(&state.pool)
    .await
    .expect("legacy credit row");

    let from = (today - chrono::Duration::days(30)).to_string();
    let to = today.to_string();
    let preview = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/contracts/{contract_id}/billing-preview?from={from}&to={to}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(
        preview.status(),
        422,
        "the evaluation refuses a credit it cannot price honestly"
    );
    let body = body_json(preview).await;
    assert_eq!(body["error"]["reason"], "validation_failed");
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains(&credit_id.to_string()),
        "the refusal names the offending credit: {message}"
    );
    assert!(
        message.contains("USD") && message.contains("EUR"),
        "the refusal names both currencies: {message}"
    );
}
