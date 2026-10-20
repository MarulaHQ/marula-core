//! Error-envelope conformance (Wave 13 lane 1, the private build tracker error-message pass).
//!
//! Purpose: prove the API leg of the error-message pass end-to-end on the
//! real router — every failure is the IF3 typed envelope with a
//! machine-readable `reason` from the registry, and the internal-leak
//! paths stay closed:
//!
//! 1. org-slug collision at register → 409 `org_slug_taken` with a usable
//!    `suggestion` (never the raw `orgs_slug_key` constraint name).
//! 2. dev-seed against an already-seeded demo org → 409 `already_seeded`
//!    (the ops page branches on the CODE, not the prose).
//! 3. malformed JSON body → 400 envelope, raw serde diagnostics
//!    sanitized away.
//!
//! Database contract follows dev_surface/settings_flow:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;

async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
}

async fn test_state(dev_tools: bool) -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("error-envelope e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url;
    config.dev_tools = dev_tools;
    AppState {
        pool,
        reporting: core_api::state::AppState::reporting_pool_lazy(&config.database_url)
            .expect("reporting pool"),
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
    }
}

async fn post_json(app: &Router, uri: &str, body: String) -> axum::response::Response {
    let request = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body))
        .unwrap();
    app.clone().oneshot(request).await.unwrap()
}

async fn body_json(response: axum::response::Response) -> (u16, Value) {
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).expect("JSON body"))
}

/// Registering the same slug twice must return the typed 409 with a
/// suggested available slug — and the raw constraint name must never
/// appear anywhere in the response.
#[tokio::test]
async fn org_slug_collision_is_typed_with_suggestion() {
    let state = test_state(false).await;
    let app = core_api::routes::build_router(state);
    let slug = format!("p13-envelope-{}", uuid::Uuid::now_v7().simple());
    async fn register(app: &Router, slug: String) -> axum::response::Response {
        let payload = json!({
            "org_name": "Envelope Co",
            "org_slug": slug,
            "email": "owner@envelope.test",
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })
        .to_string();
        post_json(app, "/api/auth/register", payload).await
    }

    let (status, body) = body_json(register(&app, slug.clone()).await).await;
    assert_eq!(status, 200, "first registration must succeed: {body}");

    let (status, body) = body_json(register(&app, slug.clone()).await).await;
    assert_eq!(status, 409, "collision must be a typed 409: {body}");
    assert_eq!(body["error"]["reason"], "org_slug_taken");
    assert_eq!(
        body["error"]["suggestion"],
        format!("{slug}-2"),
        "suggestion must be the first free -2 suffix"
    );
    let text = serde_json::to_string(&body).unwrap();
    assert!(
        !text.contains("orgs_slug_key") && !text.contains("constraint"),
        "constraint name leaked into the response: {text}"
    );

    // The suggestion must actually be usable — register with it.
    let (status, body) = body_json(register(&app, format!("{slug}-2")).await).await;
    assert_eq!(status, 200, "suggested slug must be registerable: {body}");
}

/// Malformed JSON bodies reject through the envelope — never raw serde
/// text with internal type names / parse positions.
#[tokio::test]
async fn malformed_json_body_is_typed_and_sanitized() {
    let state = test_state(false).await;
    let app = core_api::routes::build_router(state);
    let (status, body) =
        body_json(post_json(&app, "/api/auth/login", "{\"not\":\"closed".to_string()).await).await;
    assert_eq!(status, 400);
    assert_eq!(body["error"]["reason"], "validation_failed");
    let text = serde_json::to_string(&body).unwrap();
    assert!(
        !text.contains("serde") && !text.contains("line ") && !text.contains("LoginRequest"),
        "serde diagnostics leaked into the response: {text}"
    );
}

/// Dev-seed against an already-seeded demo org must return the typed
/// `already_seeded` reason (gate on: the routes exist in-process).
#[tokio::test]
async fn dev_seed_already_seeded_is_typed() {
    let state = test_state(true).await;
    let app = core_api::routes::build_router(state);

    // First seed (or the org is already there from a previous run —
    // either way, a second NON-forced seed must hit the typed 409).
    let (first_status, _) = body_json(post_json(&app, "/api/dev/seed", "".to_string()).await).await;
    let (status, body) = body_json(post_json(&app, "/api/dev/seed", "".to_string()).await).await;
    assert_eq!(
        status, 409,
        "second non-forced seed must be a typed 409 (first was {first_status}): {body}"
    );
    assert_eq!(body["error"]["reason"], "already_seeded");
}
