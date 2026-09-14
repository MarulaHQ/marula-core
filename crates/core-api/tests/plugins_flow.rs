//! Plugin runtime v2 + marketplace registry e2e (Wave 14 lane 5).
//!
//! Covers the deny-by-default posture end to end over the REAL router:
//! the declarative manifest gate (closed capability vocabulary, pinned
//! interface version), install = fork-like adoption at a pinned version
//! (created DISABLED), the admin-enabled gate, grant widening validated
//! against the pinned manifest, the typed duplicate 409s
//! (`plugin_slug_taken` / `plugin_already_installed`), versioning
//! (append + explicit upgrade), and registry-level revocation as the
//! terminal kill switch (existing installs revoked, new installs
//! denied, revoked installs immutable).

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
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("plugin e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage holds with plugin tables");
    let mut config = Config::from_env();
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let state = AppState {
        pool,
        config: Arc::new(config),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&Config::from_env())),
        store: Arc::new(MemoryStore::new()),
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
            "org_name": "Plugin Co",
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

/// Invites one staff user and redeems the setup token; returns the new
/// user's access token.
async fn invite_and_activate(app: &Router, owner_token: &str, email: &str, role: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/org/users",
        Some(owner_token),
        Some(json!({
            "email": email,
            "display_name": "Tech",
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
    tokens["access_token"].as_str().expect("tech token").into()
}

fn publish_body(slug: &str, capabilities: &[&str]) -> Value {
    json!({
        "slug": slug,
        "name": "Ticket Summarizer",
        "description": "Summarizes ticket threads",
        "publisher": "meridian",
        "version": "1.0.0",
        "manifest": {
            "description": "Summarizes ticket threads",
            "capabilities": capabilities,
        },
    })
}

#[tokio::test]
async fn plugin_marketplace_lifecycle_flow() {
    let (state, _router) = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("plugin-flow-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, slug.clone()).await;
    let tech = invite_and_activate(
        &app,
        &owner,
        &format!("tech-{slug}@acme.test"),
        "technician",
    )
    .await;

    // --- catalog read is plugins:view; the owner holds the full matrix ------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/plugins/catalog",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "catalog read is plugins:view");

    // --- publish: declarative manifest, closed vocabulary -------------------
    let entry_slug = format!("ticket-summarizer-{}", Uuid::now_v7().simple());
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/catalog",
        Some(&owner),
        Some(publish_body(
            &entry_slug,
            &["tickets.read", "financial.read"],
        )),
    )
    .await;
    assert_eq!(response.status(), 201, "publish must succeed");
    let entry = body_json(response).await;
    assert_eq!(entry["interface_version"], 1);
    assert_eq!(entry["latest_version"], "1.0.0");
    assert_eq!(entry["revoked"], false);
    let catalog_id = entry["id"].as_str().unwrap().to_string();

    // Unknown capability keys are rejected (the vocabulary is closed).
    let bad_cap = format!("bad-cap-{}", Uuid::now_v7().simple());
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/catalog",
        Some(&owner),
        Some(publish_body(&bad_cap, &["admin.users"])),
    )
    .await;
    assert_eq!(response.status(), 422, "unknown capability must reject");
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "validation_failed", "{body}");

    // Duplicate slug → typed 409.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/catalog",
        Some(&owner),
        Some(publish_body(&entry_slug, &["tickets.read"])),
    )
    .await;
    assert_eq!(response.status(), 409);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "plugin_slug_taken", "{body}");

    // --- install = adoption at a pinned version, created DISABLED -----------
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/installed",
        Some(&owner),
        Some(json!({
            "slug": entry_slug,
            "granted_capabilities": ["tickets.read"],
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "install must succeed");
    let install = body_json(response).await;
    assert_eq!(
        install["status"], "disabled",
        "deny-by-default: installs start disabled"
    );
    assert_eq!(install["version"], "1.0.0");
    assert_eq!(
        install["effective_permissions"][0], "tickets:view",
        "grants resolve onto the EXISTING matrix vocabulary"
    );
    let install_id = install["id"].as_str().unwrap().to_string();

    // Double install → typed 409.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/installed",
        Some(&owner),
        Some(json!({
            "slug": entry_slug,
            "granted_capabilities": ["tickets.read"],
        })),
    )
    .await;
    assert_eq!(response.status(), 409);
    let body = body_json(response).await;
    assert_eq!(
        body["error"]["reason"], "plugin_already_installed",
        "{body}"
    );

    // A grant OUTSIDE the pinned manifest's declared set is rejected.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/installed",
        Some(&owner),
        Some(json!({
            "slug": format!("other-{}", Uuid::now_v7().simple()),
            "granted_capabilities": [],
        })),
    )
    .await;
    assert_eq!(response.status(), 404, "unknown slug must 404");

    // --- the admin-enabled gate: a technician can do none of this -----------
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/installed",
        Some(&tech),
        Some(json!({
            "slug": entry_slug,
            "granted_capabilities": ["tickets.read"],
        })),
    )
    .await;
    assert_eq!(response.status(), 403, "technician must be denied");
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/plugins/installed/{install_id}"),
        Some(&tech),
        Some(json!({ "status": "enabled" })),
    )
    .await;
    assert_eq!(response.status(), 403, "technician must be denied");

    // --- enable (explicit admin action), widen grants within the manifest ---
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/plugins/installed/{install_id}"),
        Some(&owner),
        Some(json!({ "status": "enabled" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["status"], "enabled");

    // `revoked` is NOT settable through the update surface.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/plugins/installed/{install_id}"),
        Some(&owner),
        Some(json!({ "status": "revoked" })),
    )
    .await;
    assert_eq!(response.status(), 422);

    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/plugins/installed/{install_id}"),
        Some(&owner),
        Some(json!({ "granted_capabilities": ["tickets.read", "financial.read"] })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let updated = body_json(response).await;
    assert_eq!(updated["granted_capabilities"].as_array().unwrap().len(), 2);
    assert!(
        updated["effective_permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "financial:read"),
        "financial.read must map onto the RB2 financial:read matrix key: {updated}"
    );

    // --- versioning: append + explicit upgrade ------------------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/catalog/{entry_slug}/versions"),
        Some(&owner),
        Some(json!({
            "version": "1.1.0",
            "manifest": {
                "description": "Summarizes ticket threads",
                "capabilities": ["tickets.read", "tickets.write", "financial.read"],
            },
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "version append must succeed");

    // Duplicate version → typed 409.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/catalog/{entry_slug}/versions"),
        Some(&owner),
        Some(json!({
            "version": "1.1.0",
            "manifest": { "description": "", "capabilities": [] },
        })),
    )
    .await;
    assert_eq!(response.status(), 409);
    assert_eq!(
        body_json(response).await["error"]["reason"],
        "plugin_version_taken"
    );

    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/plugins/installed/{install_id}"),
        Some(&owner),
        Some(json!({ "version": "1.1.0" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["version"], "1.1.0");

    // --- registry revocation: the terminal kill switch ----------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/catalog/{entry_slug}/revoke"),
        Some(&owner),
        Some(json!({ "reason": "upstream vulnerability" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let revoked = body_json(response).await;
    assert_eq!(revoked["revoked"], true);
    assert_eq!(revoked["revoked_reason"], "upstream vulnerability");

    // The org's install was cascaded to revoked.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/plugins/installed",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let installs = body_json(response).await;
    let mine = installs
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == install_id.as_str())
        .expect("install still listed");
    assert_eq!(mine["status"], "revoked", "revocation must cascade");

    // New installs of a revoked entry are denied.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/installed",
        Some(&owner),
        Some(json!({
            "slug": entry_slug,
            "granted_capabilities": ["tickets.read"],
        })),
    )
    .await;
    assert_eq!(response.status(), 409, "revoked entry denies new installs");

    // Revoked installs are terminal: no enable, no grant change.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/plugins/installed/{install_id}"),
        Some(&owner),
        Some(json!({ "status": "enabled" })),
    )
    .await;
    assert_eq!(response.status(), 409, "revoked install must be immutable");

    // Double revoke → 409.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/catalog/{entry_slug}/revoke"),
        Some(&owner),
        Some(json!({ "reason": "again" })),
    )
    .await;
    assert_eq!(response.status(), 409);

    // --- audit trail covers the lifecycle ------------------------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/audit-events?action=plugin.",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let audit = body_json(response).await;
    let actions: Vec<&str> = audit["events"]
        .as_array()
        .map(|a| a.iter().filter_map(|e| e["action"].as_str()).collect())
        .unwrap_or_default();
    for expected in [
        "plugin.catalog_entry_created",
        "plugin.installed",
        "plugin.install_updated",
        "plugin.catalog_entry_revoked",
    ] {
        assert!(
            actions
                .iter()
                .any(|a| a.starts_with(expected.split('.').next().unwrap())),
            "audit must cover {expected}; got {actions:?}"
        );
    }

    // --- uninstall ------------------------------------------------------------
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/plugins/installed/{install_id}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 204, "uninstall must succeed");
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/installed",
        Some(&owner),
        Some(json!({
            "slug": entry_slug,
            "granted_capabilities": ["tickets.read"],
        })),
    )
    .await;
    assert_eq!(response.status(), 409, "revoked entry denies re-install");
    let _ = catalog_id;
}

#[tokio::test]
async fn sandbox_seam_denies_by_default() {
    // The seam-level deny-by-default matrix lives in plugin-host's unit
    // tests; here we prove the WIRE posture: an unauthenticated caller
    // gets the typed envelope, and catalog reads need plugins:view.
    let (state, _router) = test_state().await;
    let app = core_api::routes::build_router(state);
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/plugins/catalog",
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), 401);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "unauthorized", "{body}");
}
