//! §4.6 plugin surface registry e2e (Wave 34 lane 4).
//!
//! Covers the declarative surface chain over the REAL router: a manifest
//! with a `surface_profile` publishes (wasm-kind coherence), the install
//! adopts with a grant SUBSET, the surface registrations derive per
//! section from (manifest, grants), the list endpoints expose only
//! enabled-install entries, grant widening registers more sections,
//! disabling the install hides everything, and another org sees nothing
//! (org isolation).

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
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("surface e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage holds with plugin surface tables");
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
            "org_name": "Surface Co",
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

fn surface_manifest() -> Value {
    json!({
        "description": "Reference surface plugin",
        "capabilities": [
            "ui.widgets", "automation.actions", "reports.render", "fields.validate"
        ],
        "surface_profile": {
            "widgets": [
                {
                    "id": "escalation_policy",
                    "title": "Escalation policy",
                    "slot": "ticket_pane",
                    "kind": "list",
                    "props_schema": {
                        "type": "object",
                        "properties": {"Tier 1": {"type": "string", "maxLength": 80}}
                    },
                    "default_props": {"Tier 1": "Board owner, within 2 hours"}
                }
            ],
            "actions": [
                {
                    "id": "triage_note",
                    "label": "Triage note",
                    "description": "Adds the triage note the rule configures.",
                    "effect": "add_internal_comment",
                    "params_schema": {
                        "type": "object",
                        "properties": {"body": {"type": "string", "maxLength": 500}}
                    }
                }
            ],
            "render_kinds": [
                {"id": "gauges", "label": "Gauges", "description": "Gauge rows."}
            ],
            "field_validators": [
                {
                    "id": "asset_tag",
                    "label": "Asset tag",
                    "schema": {
                        "type": "object",
                        "properties": {"value": {"type": "string", "maxLength": 32}}
                    }
                }
            ]
        }
    })
}

async fn get_entries(app: &Router, token: &str, path: &str) -> Vec<Value> {
    let response = send(app, axum::http::Method::GET, path, Some(token), None).await;
    assert_eq!(response.status(), 200, "surface list must succeed");
    body_json(response).await["entries"]
        .as_array()
        .expect("entries array")
        .clone()
}

#[tokio::test]
async fn plugin_surface_registry_flow() {
    let (state, _router) = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("surface-flow-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, slug.clone()).await;
    let other = register_org(&app, format!("other-{slug}")).await;

    // --- publish a wasm manifest with a full surface_profile ----------------
    let entry_slug = format!("surface-plugin-{}", Uuid::now_v7().simple());
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/catalog",
        Some(&owner),
        Some(json!({
            "slug": entry_slug,
            "name": "Surface Plugin",
            "description": "Reference surface plugin",
            "publisher": "meridian",
            "version": "1.0.0",
            "manifest": surface_manifest(),
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "publish must succeed");

    // --- adopt with a grant SUBSET: widgets only ----------------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/installed",
        Some(&owner),
        Some(json!({
            "slug": entry_slug,
            "granted_capabilities": ["ui.widgets"],
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "install must succeed");
    let install_id = body_json(response).await["id"]
        .as_str()
        .expect("install id")
        .to_string();

    // Entries derive even while disabled, but lists expose nothing until
    // the install enables (deny-by-default renders nowhere).
    let entries = get_entries(&app, &owner, "/api/plugins/surface/widgets").await;
    assert!(entries.is_empty(), "disabled installs list nothing");
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/plugins/installed/{install_id}"),
        Some(&owner),
        Some(json!({"status": "enabled"})),
    )
    .await;
    assert_eq!(response.status(), 200, "enable must succeed");

    // --- widgets register; ungranted sections stay empty --------------------
    let entries = get_entries(&app, &owner, "/api/plugins/surface/widgets").await;
    assert_eq!(entries.len(), 1, "the granted widget section registers");
    assert_eq!(
        entries[0]["entry_key"],
        json!(format!("plugin.{entry_slug}.escalation_policy"))
    );
    assert_eq!(entries[0]["definition"]["kind"], json!("list"));
    assert_eq!(entries[0]["definition"]["slot"], json!("ticket_pane"));
    let actions = get_entries(&app, &owner, "/api/plugins/surface/automation-actions").await;
    assert!(actions.is_empty(), "ungranted sections register nothing");
    let validators = get_entries(&app, &owner, "/api/plugins/surface/field-validators").await;
    assert!(validators.is_empty(), "ungranted sections register nothing");

    // --- org isolation: another org sees nothing ----------------------------
    let other_entries = get_entries(&app, &other, "/api/plugins/surface/widgets").await;
    assert!(other_entries.is_empty(), "entries are org-scoped");

    // --- widening grants registers the new sections --------------------------
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/plugins/installed/{install_id}"),
        Some(&owner),
        Some(json!({
            "granted_capabilities": [
                "ui.widgets", "automation.actions", "reports.render", "fields.validate"
            ],
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "grant widening must succeed");
    let actions = get_entries(&app, &owner, "/api/plugins/surface/automation-actions").await;
    assert_eq!(actions.len(), 1, "the action section registers after widening");
    assert_eq!(
        actions[0]["definition"]["effect"],
        json!("add_internal_comment")
    );
    let validators = get_entries(&app, &owner, "/api/plugins/surface/field-validators").await;
    assert_eq!(validators.len(), 1, "the validator section registers");
    let renders = get_entries(&app, &owner, "/api/plugins/surface/report-renders").await;
    assert_eq!(renders.len(), 1, "the render-kind section registers");

    // --- disabling the install hides every registration ----------------------
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/plugins/installed/{install_id}"),
        Some(&owner),
        Some(json!({"status": "disabled"})),
    )
    .await;
    assert_eq!(response.status(), 200, "disable must succeed");
    let entries = get_entries(&app, &owner, "/api/plugins/surface/widgets").await;
    assert!(entries.is_empty(), "disabled installs render nowhere");
    let actions = get_entries(&app, &owner, "/api/plugins/surface/automation-actions").await;
    assert!(actions.is_empty(), "disabled installs render nowhere");
}
