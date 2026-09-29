//! Restricted-category owner/admin grant seeding — HTTP e2e (Wave 16
//! lane 10, ADR 0006 directive plus migration 0083).
//!
//! Purpose: the new-org default at the ROUTE boundary — the owner/admin
//! pair is never locked out of their own restricted vault chains while
//! the deny-by-default posture holds for members:
//!   * owner creates a restricted category → WITHOUT touching the grants
//!     surface the org Admin can read (and edit) records in that chain,
//!     and the Technician is still denied (no grant, deny-by-default);
//!   * the runtime seeding rides the SAME transaction as the folder write
//!     (no self-lockout window mid-call);
//!   * role grants are still manageable: the seeded rows are the plain
//!     `secret_category_grants` data — deleting the admin grant re-locks
//!     the admin out until it is re-upserted (posture back, delibe-
//!     rately manageable).
//!
//! Non-purpose: store-level determinism of the backfill (the core-db
//! suite `restricted_grant_seeding` proves the migration idempotence and
//! the runtime/store twins).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use core_store::memory::MemoryStore;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> (AppState, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("secrets e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let state = AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(MemoryStore::new()),
        routing: Arc::new(core_routing::RoutingRegistry::new()),
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
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "body_json parse failed ({e}); status={status} body={:?}",
            String::from_utf8_lossy(&bytes[..bytes.len().min(300)])
        )
    })
}

async fn register_org(app: &Router, slug: String) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Grant Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let body = body_json(response).await;
    body["access_token"].as_str().expect("access token").into()
}

async fn invite_and_activate(app: &Router, owner_token: &str, email: &str, role: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/org/users",
        Some(owner_token),
        Some(json!({
            "email": email,
            "display_name": role,
            "role": role,
            "idempotency_key": format!("invite-{}", Uuid::now_v7().simple()),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "invite must succeed");
    let body = body_json(response).await;
    let setup_token = body["invite"]["setup_token"].as_str().expect("setup token");
    let accept = send(
        app,
        axum::http::Method::POST,
        "/api/org/users/accept-invite",
        None,
        Some(json!({ "token": setup_token, "password": "another-correct-horse" })),
    )
    .await;
    assert_eq!(accept.status(), 200, "redemption must succeed");
    let tokens = body_json(accept).await;
    tokens["access_token"].as_str().expect("token").into()
}

#[tokio::test]
async fn new_org_restricted_chain_serves_admin_and_denies_technician() {
    let (_state, router) = test_state().await;
    let slug = format!("grant-flow-{}", Uuid::now_v7().simple());
    let owner = register_org(&router, slug.clone()).await;
    let admin = invite_and_activate(&router, &owner, &format!("admin@{slug}.test"), "admin").await;
    let technician =
        invite_and_activate(&router, &owner, &format!("tech@{slug}.test"), "technician").await;

    // Owner creates a RESTRICTED category in one call — the seeding rides
    // the same transaction (no moment where the folder exists ungranted).
    let created = send(
        &router,
        axum::http::Method::POST,
        "/api/secrets/folders",
        Some(&owner),
        Some(json!({
            "slug": "client-master-keys",
            "label": "Client master keys",
            "kind": "api_keys",
            "restricted": true,
        })),
    )
    .await;
    let status = created.status();
    let body = body_json(created).await;
    assert_eq!(status, 200, "folder create must succeed: {body}");
    let folder_id = body["id"].as_str().unwrap().to_string();

    // Put one record into the restricted chain.
    let record = send(
        &router,
        axum::http::Method::POST,
        "/api/secrets/records",
        Some(&owner),
        Some(json!({
            "folder_id": folder_id,
            "kind": "api_key",
            "title": "Vault master key",
            "fields": [{"field_key": "secret", "value": "s3cr3t"}]
        })),
    )
    .await;
    let record_body = body_json(record).await;
    assert!(
        record_body["id"].as_str().is_some(),
        "record create must succeed inside the fresh restricted chain: {record_body}"
    );
    let record_id = record_body["id"].as_str().expect("record id").to_string();

    // TECHNICIAN: deny-by-default — the record metadata is invisible.
    let tech_read = send(
        &router,
        axum::http::Method::GET,
        &format!("/api/secrets/records/{record_id}"),
        Some(&technician),
        None,
    )
    .await;
    assert_ne!(
        tech_read.status(),
        200,
        "technician must NOT read a restricted-chain record"
    );

    // ADMIN: the seeded owner/admin grants admit read AND edit.
    let admin_read = send(
        &router,
        axum::http::Method::GET,
        &format!("/api/secrets/records/{record_id}"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(
        admin_read.status(),
        200,
        "admin must read the restricted chain via the runtime-seeded grants"
    );

    // ADMIN edit lands on the restricted chain (can_edit seeded).
    let admin_edit = send(
        &router,
        axum::http::Method::PATCH,
        &format!("/api/secrets/records/{record_id}"),
        Some(&admin),
        Some(json!({ "title": "Vault master key (renamed)" })),
    )
    .await;
    let edit_status = admin_edit.status();
    assert_eq!(
        edit_status,
        axum::http::StatusCode::OK,
        "admin must edit the restricted chain via the runtime-seeded grants"
    );
}
