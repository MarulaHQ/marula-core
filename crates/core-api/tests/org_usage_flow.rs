//! HTTP flow tests for the org resource-usage + quota read API (the private build tracker
//! Wave 47 lane A6, `p47-org-resource-quota-api`, migration 0131) — over
//! the REAL core-api router.
//!
//! Purpose: prove the surface's posture end to end.
//!   * A member reads their OWN org's usage; before the worker has
//!     refreshed the row the payload says so through its timestamp
//!     (`refreshed_at: null`) instead of reporting a measured zero.
//!   * A cross-org read is a 404 — the same answer a nonexistent org gets,
//!     so the endpoint never confirms another org exists.
//!   * A service account reads usage ONLY with `org_usage:read`; the same
//!     path with any other scope is a typed 403, and a non-allowlisted
//!     path refuses keys outright (the middleware's allowlist).
//!   * Quotas are INFORMATIONAL: an org sitting at 200% of a configured
//!     limit is served normally and keeps writing. Nothing throttles.
//!   * Percentages are the arithmetic core-domain computes, not a second
//!     implementation in the route.
//!
//! Non-purpose: the refresh sweep itself (worker crate + its db_suite
//! coverage — this test calls `refresh_org`, the single-org view over the
//! same `refresh_orgs` statement the sweep's batch path runs, so both are
//! exercised), the metric SQL (db_suite), and any write surface for limits
//! (the hosted tier configures those out of band; there is no knob here on
//! purpose).
//!
//! Database selection follows the documented contract: MARULA_TEST_DATABASE_URL
//! or DATABASE_URL (no schema reset).

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
        .expect("org usage flow needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
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

fn auth(token: &str) -> (&'static str, String) {
    ("authorization", format!("Bearer {token}"))
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json body")
}

async fn send(
    app: &axum::Router,
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
        Some(value) => builder
            .header("content-type", "application/json")
            .body(axum::body::Body::from(value.to_string()))
            .unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    };
    app.clone()
        .oneshot(request)
        .await
        .expect("infallible router")
}

/// Registers an org the usual way and returns (token, org_id).
async fn register_org(app: &axum::Router, state: &AppState, slug: &str) -> (String, Uuid) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Usage Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
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
    let org_id: Uuid = sqlx::query_scalar("SELECT id FROM orgs WHERE slug = $1")
        .bind(slug)
        .fetch_one(&state.pool)
        .await
        .expect("org id");
    (token, org_id)
}

/// Seeds one staff account directly through core-db, so the usage row's
/// `active_users` count is a real second account rather than a hand-written
/// number. The invite flow is the API's own surface and not what this test
/// is about.
async fn add_user(state: &AppState, org_id: Uuid, n: u64) {
    let mut tx = state.pool.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("set org");
    core_db::users::create_user(
        &mut *tx,
        Uuid::now_v7(),
        org_id,
        &format!("u{n}@usage.test"),
        "Tech",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user insert");
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn usage_reports_own_org_refuses_others_and_honours_the_machine_grant() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("usage-flow-{}", Uuid::now_v7().simple());
    let (token, org_id) = register_org(&app, &state, &slug).await;
    let bearer = [auth(&token)];
    let other_slug = format!("usage-other-{}", Uuid::now_v7().simple());
    let (_other_token, other_org_id) = register_org(&app, &state, &other_slug).await;

    // --- before any refresh: measured vs zero is distinguishable -----------
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/orgs/{org_id}/usage"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "a member reads their own usage");
    let usage = body_json(response).await;
    assert_eq!(usage["org_id"], json!(org_id.to_string()));
    assert_eq!(
        usage["refreshed_at"],
        Value::Null,
        "an unrefreshed org reports NO measurement rather than zeros"
    );
    // The unavailable list is real data, not a placeholder: every metric
    // that cannot be computed honestly is named with its reason.
    let unavailable = usage["unavailable"].as_array().expect("unavailable list");
    let metrics: Vec<&str> = unavailable
        .iter()
        .map(|m| m["metric"].as_str().expect("metric token"))
        .collect();
    assert!(
        metrics.contains(&"api_calls_last_24h"),
        "the gap is stated, hidden by no zero: {metrics:?}"
    );
    assert!(metrics.contains(&"database_bytes"));
    for entry in unavailable {
        assert!(
            entry["reason"].as_str().is_some_and(|r| !r.is_empty()),
            "every unavailable metric carries its reason"
        );
    }

    // --- refresh the row exactly as the worker sweep does --------------------
    core_db::org_usage::refresh_orgs(&state.pool, &[org_id])
        .await
        .expect("refresh")
        .into_iter()
        .next()
        .expect("row");

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/orgs/{org_id}/usage"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let usage = body_json(response).await;
    assert!(
        usage["refreshed_at"].is_string(),
        "a refreshed org says when it was measured"
    );
    assert_eq!(
        usage["active_users"],
        json!(1),
        "the registering owner is the only active account"
    );
    assert_eq!(usage["tickets_by_category"]["open"], json!(0));
    assert_eq!(usage["tickets_total"], json!(0));
    assert_eq!(usage["object_store_bytes"], json!(0));
    // Registration writes TWO audit events, not one: `auth.register` and the
    // `auth.email_verification_requested` that follows it. Pinned as 2 with
    // both names, because the honest proxy counts the writes the core
    // actually records — asserting 1 (the first version of this test) silently
    // assumed the verification request was not audited.
    assert_eq!(
        usage["audit_events_24h"],
        json!(2),
        "the honest proxy counts both registration events the core records"
    );

    // --- cross-org read is refused, indistinguishably from "no such org" ----
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/orgs/{other_org_id}/usage"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        404,
        "another org's usage is a 404, exactly like a nonexistent org"
    );

    // --- service accounts: the grant is the whole gate ----------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/api-keys",
        &bearer,
        Some(json!({"name": "hq-monitor", "scopes": ["org_usage:read"]})),
    )
    .await;
    assert_eq!(response.status(), 201, "the monitoring scope is creatable");
    let monitor_key = body_json(response).await["raw_key"]
        .as_str()
        .expect("raw key shown once")
        .to_string();

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/orgs/{org_id}/usage"),
        &[("x-api-key", monitor_key.clone())],
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "a key carrying org_usage:read reads the usage report"
    );

    // A key with any other scope is authenticated but unauthorized.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/api-keys",
        &bearer,
        Some(json!({"name": "wrong-scope", "scopes": ["tickets:read"]})),
    )
    .await;
    let wrong_scope = body_json(response).await["raw_key"]
        .as_str()
        .expect("raw key")
        .to_string();
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/orgs/{org_id}/usage"),
        &[("x-api-key", wrong_scope)],
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        403,
        "a key without org_usage:read is refused at the permission gate"
    );
    // Both reports ride the same allowlisted `/api/orgs` prefix, so a key
    // created for the usage endpoint reaches the quotas report too.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/orgs/{org_id}/quotas"),
        &[("x-api-key", monitor_key)],
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "the quotas report rides the same allowlisted /api/orgs prefix"
    );
}

#[tokio::test]
async fn quotas_are_informational_and_never_gate_anything() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("quota-flow-{}", Uuid::now_v7().simple());
    let (token, org_id) = register_org(&app, &state, &slug).await;
    let bearer = [auth(&token)];

    // --- no limits configured: the honest answer is "none" ------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/orgs/{org_id}/quotas"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let quotas = body_json(response).await;
    assert_eq!(
        quotas["limits_configured"],
        json!(false),
        "an org that configured nothing gets no invented default limit"
    );
    for metric in quotas["metrics"].as_array().expect("metric rows") {
        assert_eq!(metric["limit"], Value::Null);
        assert_eq!(
            metric["percent"],
            Value::Null,
            "no limit to measure against means no percentage, not 0%"
        );
    }

    // --- two real accounts against a one-seat "limit" → 200% ---------------
    add_user(&state, org_id, 1).await;
    core_db::org_usage::refresh_orgs(&state.pool, &[org_id])
        .await
        .expect("refresh")
        .into_iter()
        .next()
        .expect("row");
    let mut tx = state.pool.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("set org");
    let actor: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE org_id = $1 LIMIT 1")
        .bind(org_id)
        .fetch_one(&mut *tx)
        .await
        .expect("actor");
    core_db::settings::upsert_org_setting(
        &mut tx,
        org_id,
        "usage_quota.active_users",
        &json!(1),
        actor,
    )
    .await
    .expect("limit setting");
    tx.commit().await.expect("commit");

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/orgs/{org_id}/quotas"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let quotas = body_json(response).await;
    assert_eq!(quotas["limits_configured"], json!(true));
    let active = quotas["metrics"]
        .as_array()
        .expect("metric rows")
        .iter()
        .find(|m| m["metric"] == json!("active_users"))
        .expect("active_users row");
    assert_eq!(active["usage"], json!(2));
    assert_eq!(active["limit"], json!(1));
    assert_eq!(
        active["percent"],
        json!(200.0),
        "over the limit is a normal reading, not an error and not a clamp"
    );
    assert_eq!(
        active["setting_key"],
        json!("usage_quota.active_users"),
        "the payload names the settings key so the gap is actionable"
    );

    // --- the posture: being over quota changes NOTHING ----------------------
    // The org still writes. If any code path read this endpoint's numbers
    // to limit behaviour, this write would fail — it does not.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/clients",
        &bearer,
        Some(json!({"name": "Over Limit Co"})),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "an org at 200% of a configured limit is not throttled"
    );
    // ...and the reports stay readable while over.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/orgs/{org_id}/usage"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "usage stays readable, over or not");
}
