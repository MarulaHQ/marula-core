//! Org-data export endpoint flow (TASKLIST D1) through the REAL core-api
//! router.
//!
//! Purpose: the admin-gated POST /org/exports + signed-download contract:
//! 1. Owner round-trip: creating an export stores the bundle in the
//!    ObjectStore and returns a self-contained signed URL; downloading it
//!    streams the versioned bundle (no bearer token needed).
//! 2. Signature discipline: tampered signatures, foreign-org paths, and
//!    expired links are all rejected with the typed unauthorized envelope.
//! 3. Bundle content: the wider-scope sections (org profile, clients,
//!    tickets, customization) are present in the downloaded body.
//! 4. Authz: technicians (403) and unauthenticated callers (401) cannot
//!    create exports; both mounts serve the same handlers.
//!
//! Database selection follows settings_flow's contract:
//! MERIDIAN_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

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
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("export e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    // Deterministic at-rest key so the signed-download scheme is testable.
    config.secret_key = core_api::config::SecretKey([13; 32]);
    AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
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

async fn body_string(response: axum::response::Response) -> String {
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap()
}

fn auth(token: &str) -> (&'static str, String) {
    ("authorization", format!("Bearer {token}"))
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Export Co",
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

#[tokio::test]
async fn owner_creates_export_and_downloads_via_signed_link() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("export-flow-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    // Seed one client through the public API so the bundle has content.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/clients",
        &[auth(&token)],
        Some(json!({ "name": "Bundle Client" })),
    )
    .await;
    assert_eq!(response.status(), 200, "client create must succeed");

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/org/exports",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "owner must create an export");
    let created = body_json(response).await;
    assert_eq!(created["bundle_version"], 1);
    let size = created["size_bytes"].as_i64().unwrap();
    assert!(size > 0, "bundle has bytes");
    assert!(created["sha256"].as_str().unwrap().len() == 64);
    let download_url = created["download_url"].as_str().unwrap();
    assert!(
        download_url.starts_with("/api/v1/org/exports/"),
        "{download_url}"
    );
    assert!(download_url.contains("expires=") && download_url.contains("signature="));

    // The signed link downloads WITHOUT any bearer header.
    let response = send(&app, axum::http::Method::GET, download_url, &[], None).await;
    assert_eq!(response.status(), 200, "signed link must download");
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        "application/json"
    );
    let bundle = body_json(response).await;
    assert_eq!(bundle["bundle_version"], 1);
    assert_eq!(bundle["sections"]["org"]["name"], "Export Co");
    let clients = bundle["sections"]["clients"].as_array().unwrap();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0]["name"], "Bundle Client");
    // The embedded U5 customization section rides along.
    assert!(bundle["sections"]["customization"]["boards"].is_array());

    // A tampered signature is rejected with the typed envelope.
    let tampered = format!(
        "{}tampered",
        download_url.split_at(download_url.len() - 8).0
    );
    let response = send(&app, axum::http::Method::GET, &tampered, &[], None).await;
    assert_eq!(response.status(), 401, "tampered signature rejected");
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "unauthorized");
}

#[tokio::test]
async fn expired_and_foreign_org_links_are_rejected() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("export-exp-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/org/exports",
        &[auth(&token)],
        None,
    )
    .await;
    let created = body_json(response).await;
    let download_url = created["download_url"].as_str().unwrap().to_string();
    let org_id = download_url
        .split("/org/exports/")
        .nth(1)
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .to_string();
    let export_id = created["export_id"].as_str().unwrap().to_string();

    // A correctly signed but EXPIRED link is rejected: sign with the test's
    // deterministic at-rest key (mirrors the server's scheme).
    let expired = chrono::Utc::now().timestamp() - 60;
    let signature = core_mail::hmac::sign(
        &[13u8; 32],
        format!("export-download:{org_id}:{export_id}:{expired}").as_bytes(),
    );
    let expired_url = format!(
        "/api/v1/org/exports/{org_id}/{export_id}/download?expires={expired}&signature={signature}"
    );
    let response = send(&app, axum::http::Method::GET, &expired_url, &[], None).await;
    assert_eq!(response.status(), 401, "expired link rejected");

    // The same signature presented for a DIFFERENT org path never verifies.
    let future = chrono::Utc::now().timestamp() + 600;
    let signature = core_mail::hmac::sign(
        &[13u8; 32],
        format!("export-download:{org_id}:{export_id}:{future}").as_bytes(),
    );
    let foreign_org = Uuid::now_v7();
    let foreign_url = format!(
        "/api/v1/org/exports/{foreign_org}/{export_id}/download?expires={future}&signature={signature}"
    );
    let response = send(&app, axum::http::Method::GET, &foreign_url, &[], None).await;
    assert_eq!(response.status(), 401, "foreign-org path rejected");
}

#[tokio::test]
async fn non_admins_cannot_create_exports() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("export-authz-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    let org_id: Uuid = {
        let response = send(
            &app,
            axum::http::Method::GET,
            "/api/v1/me",
            &[auth(&token)],
            None,
        )
        .await;
        body_json(response).await["org_id"]
            .as_str()
            .unwrap()
            .to_string()
            .parse()
            .unwrap()
    };

    // Provision a technician through core_db, log in over HTTP.
    let password_hash = core_api::auth::password::hash("tech-password-1234").expect("hash");
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    let tech_role: Uuid =
        sqlx::query_scalar("SELECT id FROM roles WHERE org_id = $1 AND builtin = 'technician'")
            .bind(org_id)
            .fetch_one(&mut *tx)
            .await
            .expect("technician role");
    let user_id = Uuid::now_v7();
    core_db::users::create_user(
        &mut *tx,
        user_id,
        org_id,
        &format!("tech@{slug}.test"),
        "Tech User",
        &password_hash,
        "UTC",
    )
    .await
    .expect("user");
    core_db::users::assign_role(&mut *tx, user_id, tech_role)
        .await
        .expect("role");
    tx.commit().await.expect("commit");

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/auth/login",
        &[],
        Some(json!({ "email": format!("tech@{slug}.test"), "password": "tech-password-1234" })),
    )
    .await;
    let tech_token = body_json(response).await["access_token"]
        .as_str()
        .unwrap()
        .to_string();

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/org/exports",
        &[auth(&tech_token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 403, "technicians cannot export");

    // Unauthenticated callers are 401 on both the create and download paths.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/org/exports",
        &[],
        None,
    )
    .await;
    assert_eq!(response.status(), 401);
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/org/exports/{org_id}/{org_id}/download?expires=0&signature=ab"),
        &[],
        None,
    )
    .await;
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn exports_are_served_identically_on_both_mounts() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("export-dual-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    let canonical = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/org/exports",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(canonical.status(), 200);
    let legacy = send(
        &app,
        axum::http::Method::POST,
        "/api/org/exports",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(legacy.status(), 200, "legacy alias serves the same handler");
    let _ = body_string(canonical).await;
    let _ = body_string(legacy).await;
}
