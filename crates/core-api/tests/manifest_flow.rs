//! Customization-manifest endpoint flow (TASKLIST U5) through the REAL
//! core-api router.
//!
//! Purpose: the admin-gated GET /api/org/manifest contract over HTTP —
//! 1. Owner round-trip: sections populated through the PUBLIC API surface
//!    (webhook, API key, org setting, custom field) come back faithfully
//!    in the canonical envelope.
//! 2. No-secret leakage through the HTTP path: the webhook signing secret
//!    (shown once at creation) and the raw API key (shown once) must never
//!    appear in any later dump, nor must any secret-shaped field.
//! 3. Authz: technicians are rejected (403); unauthenticated requests are
//!    rejected (401).
//! 4. Dual-mount parity: /api/v1 (canonical) and /api (legacy alias) serve
//!    the same handler.
//!
//! Non-purpose: loader-level ordering/determinism coverage (owned by
//! core-db/tests/manifest_queries.rs; the golden stability contract lives
//! there). Database selection follows settings_flow's contract:
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
        .expect("manifest e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env();
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    // Deterministic at-rest key so secret sealing round-trips in-process.
    config.secret_key = core_api::config::SecretKey([13; 32]);
    AppState {
        pool,
        config: Arc::new(config),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&Config::from_env())),
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
            "org_name": "Manifest Co",
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

/// Creates a second user through core_db with the builtin technician role,
/// then logs in over HTTP so the 403 is asserted against a REAL principal.
async fn provision_technician(state: &AppState, app: &Router, org_id: Uuid, email: &str) -> String {
    let password_hash = core_api::auth::password::hash("tech-password-1234").expect("hash");
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    let tech_role: Uuid =
        sqlx::query_scalar("SELECT id FROM roles WHERE org_id = $1 AND builtin = 'technician'")
            .bind(org_id)
            .fetch_one(&mut *tx)
            .await
            .expect("technician role exists from registration");
    let user_id = Uuid::now_v7();
    core_db::users::create_user(
        &mut *tx,
        user_id,
        org_id,
        email,
        "Tech User",
        &password_hash,
        "UTC",
    )
    .await
    .expect("user insert");
    core_db::users::assign_role(&mut *tx, user_id, tech_role)
        .await
        .expect("role assign");
    tx.commit().await.expect("commit");

    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/login",
        &[],
        Some(json!({ "email": email, "password": "tech-password-1234" })),
    )
    .await;
    assert_eq!(response.status(), 200, "technician login must succeed");
    body_json(response).await["access_token"].as_str().unwrap().to_string()
}

/// Populates sections through the PUBLIC API and returns the one-time
/// secrets (webhook signing secret, raw API key) for the leakage checks.
async fn populate_via_api(
    app: &Router,
    token: &str,
) -> (String, String, Value) {
    let bearer = [auth(token)];

    // Webhook with a filter; the signing secret is returned EXACTLY ONCE.
    let webhook_secret = {
        let response = send(
            app,
            axum::http::Method::POST,
            "/api/webhooks",
            &bearer,
            Some(json!({
                "url": "https://hooks.example.test/u5-manifest",
                "event_types": ["ticket.created", "ticket.closed"]
            })),
        )
        .await;
        assert_eq!(response.status(), 201, "webhook create must succeed");
        body_json(response).await["secret"].as_str().unwrap().to_string()
    };

    // API key with scopes; raw material returned EXACTLY ONCE.
    let raw_key = {
        let response = send(
            app,
            axum::http::Method::POST,
            "/api/api-keys",
            &bearer,
            Some(json!({
                "name": "Export Checker",
                "scopes": ["tickets:read", "clients:read"]
            })),
        )
        .await;
        assert_eq!(response.status(), 201, "api-key create must succeed");
        body_json(response).await["raw_key"].as_str().unwrap().to_string()
    };

    // One org-scope registry override (a REAL declared key).
    let setting = {
        let response = send(
            app,
            axum::http::Method::PUT,
            "/api/settings/org",
            &bearer,
            Some(json!({
                "key": "general.eod_hour",
                "value": 20
            })),
        )
        .await;
        assert_eq!(response.status(), 200, "org setting put must succeed");
        body_json(response).await
    };

    // One custom field definition on the default board.
    let board_id = {
        let response = send(app, axum::http::Method::GET, "/api/boards", &bearer, None).await;
        assert_eq!(response.status(), 200);
        let boards = body_json(response).await;
        let arr = boards.as_array().expect("boards array");
        assert!(!arr.is_empty(), "registration seeds a default board");
        arr[0]["id"].as_str().unwrap().to_string()
    };
    {
        let response = send(
            app,
            axum::http::Method::POST,
            &format!("/api/boards/{board_id}/fields"),
            &bearer,
            Some(json!({
                "name": "Impact",
                "kind": "select",
                "options": ["minor", "major", "critical"],
                "required": false,
                "sort_order": 3
            })),
        )
        .await;
        assert_eq!(response.status(), 200, "field def create must succeed");
    }

    (webhook_secret, raw_key, setting)
}

#[tokio::test]
async fn owner_manifest_round_trip_and_no_secret_leakage() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("manifest-flow-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    let org_id = {
        let response = send(&app, axum::http::Method::GET, "/api/v1/me", &[auth(&token)], None)
            .await;
        assert_eq!(response.status(), 200);
        body_json(response).await["org_id"].as_str().unwrap().to_string()
    };

    let (webhook_secret, raw_key, setting) = populate_via_api(&app, &token).await;

    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/org/manifest",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "owner must read the manifest");
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        "application/json",
        "the manifest is application/json"
    );
    let manifest = body_json(response).await;

    assert_eq!(manifest["manifest_version"], 1);
    assert_eq!(
        manifest["org_id"].as_str().unwrap(),
        org_id,
        "the envelope names the requesting principal's org"
    );

    // Every section is present (never absent keys), populated ones round-trip.
    let sections = &manifest["sections"];
    for section in [
        "custom_fields",
        "boards",
        "automation_rules",
        "webhooks",
        "api_keys",
        "settings",
        "sla",
    ] {
        assert!(
            sections.get(section).is_some(),
            "section {section} must always be present"
        );
    }

    let hooks = sections["webhooks"].as_array().unwrap();
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0]["url"], "https://hooks.example.test/u5-manifest");
    assert_eq!(
        hooks[0]["event_types"],
        json!(["ticket.created", "ticket.closed"]),
        "webhook filters round-trip"
    );

    let keys = sections["api_keys"].as_array().unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0]["name"], "Export Checker");
    assert_eq!(keys[0]["scopes"], json!(["tickets:read", "clients:read"]));

    let settings = sections["settings"].as_array().unwrap();
    assert_eq!(settings.len(), 1, "only the touched org key is overridden");
    assert_eq!(settings[0]["key"], "general.eod_hour");
    assert_eq!(settings[0]["value"], 20);
    assert_eq!(
        setting["value"], 20,
        "the setting API's own read-back agrees with the manifest"
    );

    let fields = sections["custom_fields"].as_array().unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0]["name"], "Impact");
    assert_eq!(fields[0]["kind"], "select");
    assert_eq!(fields[0]["config"]["options"], json!(["minor", "major", "critical"]));

    // The defaults-first baseline: default board with category-covered
    // statuses and the seeded default SLA policy.
    let boards = sections["boards"].as_array().unwrap();
    assert_eq!(boards.len(), 1);
    let categories: Vec<&str> = boards[0]["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["category"].as_str().unwrap())
        .collect();
    assert_eq!(categories, vec!["open", "in_progress", "waiting", "closed"]);
    assert_eq!(sections["sla"]["policies"].as_array().unwrap().len(), 1);

    // NO secrets anywhere in the dump: not the once-shown webhook signing
    // secret, not the once-shown raw API key, and no secret-shaped fields.
    let serialized = serde_json::to_string(&manifest).unwrap();
    assert!(!serialized.contains(&webhook_secret), "webhook signing secret leaked");
    assert!(!serialized.contains(&raw_key), "raw API key leaked");
    assert!(!serialized.contains("secret_encrypted"));
    assert!(!serialized.contains("key_hash"));
}

#[tokio::test]
async fn technician_is_rejected_from_the_manifest() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("manifest-authz-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    let org_id: Uuid = {
        let response = send(&app, axum::http::Method::GET, "/api/v1/me", &[auth(&token)], None)
            .await;
        body_json(response).await["org_id"]
            .as_str()
            .unwrap()
            .to_string()
            .parse()
            .unwrap()
    };
    let tech_token = provision_technician(
        &state,
        &app,
        org_id,
        &format!("tech@{slug}.test"),
    )
    .await;

    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/org/manifest",
        &[auth(&tech_token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 403, "non-admin principals are rejected");
    let body = body_json(response).await;
    assert!(
        body["error"].is_object() || body.is_object(),
        "rejections use the JSON error shape, got: {body}"
    );
}

#[tokio::test]
async fn unauthenticated_manifest_request_is_rejected() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);

    let response = send(&app, axum::http::Method::GET, "/api/v1/org/manifest", &[], None).await;
    assert_eq!(response.status(), 401);
    let body = body_string(response).await;
    assert!(!body.is_empty(), "auth middleware answers with a JSON body, not a bare 404");
}

#[tokio::test]
async fn manifest_is_served_identically_on_both_mounts() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("manifest-dual-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    let canonical = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/org/manifest",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(canonical.status(), 200);
    let canonical = body_json(canonical).await;

    let alias = send(
        &app,
        axum::http::Method::GET,
        "/api/org/manifest",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(alias.status(), 200);
    let alias = body_json(alias).await;

    // Identical modulo the dump stamp (each call generates its own).
    let mut a = canonical.clone();
    let mut b = alias.clone();
    a["generated_at"] = json!(null);
    b["generated_at"] = json!(null);
    assert_eq!(a, b, "canonical and legacy mounts serve the same handler");
}
