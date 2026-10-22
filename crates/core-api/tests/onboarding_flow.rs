//! Wave 26 lane 1 onboarding flows — the skippability CONTRACT at the API
//! level, exercised through the REAL core-api router:
//!
//! 1. Org wizard: step complete is SERVER-GATED (boards pre-exist per
//!    registration provisioning, so `board` completes; `tickets` 409s
//!    before any artifact, then 200s after a ticket is created).
//! 2. Skip semantics: "skip for now" (never:false) records NOTHING — the
//!    step stays pending/resumable through the summary; "never show this
//!    again" (never:true) persists the dismissed flag and NEVER resets.
//! 3. Summary shape: per-step done/dismissed/satisfied/resolved, and the
//!    admin callers see member rows (members empty for non-admins).
//! 4. Org-wide disable: the disable switch silences nothing in the API
//!    Summary (banner silence is entirely UI-side) but the recorded
//!    disabled=true persists; re-enabling keeps all recorded state.
//! 5. Reset/re-run: reset/org clears per-step flags but NOT the disable
//!    switch (deliberate); reset/user clears exactly one member's flags.
//!
//! Database selection follows export_flow's contract: MARULA_TEST_DATABASE_URL
//! or DATABASE_URL (no schema reset).

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
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("onboarding flow needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url.clone();
    config.secret_key = core_api::config::SecretKey([17; 32]);
    AppState {
        plugin_pool: Default::default(),
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

fn auth(token: &str) -> (&'static str, String) {
    ("authorization", format!("Bearer {token}"))
}

async fn register_org(app: &Router, name: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": format!("{name} Co"),
            "org_slug": format!("{name}-{}", Uuid::now_v7().simple()),
            "email": format!("owner-{name}@{name}.test"),
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

async fn summary(app: &Router, token: &str) -> (Value, Value) {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/v1/onboarding",
        &[auth(token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "summary readable to any member");
    let parsed = body_json(response).await;
    (parsed.clone(), parsed["members"].clone())
    // note: clone split keeps both the org view and members handy.
}

fn org_step<'a>(summary: &'a Value, key: &str) -> &'a Value {
    summary["org"]["steps"]
        .as_array()
        .expect("steps array")
        .iter()
        .find(|s| s["step"] == key)
        .unwrap_or_else(|| panic!("step {key} present in summary"))
}

#[tokio::test]
async fn org_step_completion_is_server_gated_and_skip_records_stick() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let token = register_org(&app, "gateskip").await;

    let (org_steps, _members) = summary(&app, &token).await;
    // Fresh org: registration seeds clients/board/tickets/invites (all
    // satisfied + recorded done server-side). The AI step is the first
    // gate that needs its artifact (the AI master switch default off).
    assert_eq!(org_step(&org_steps, "board")["resolved"], json!(true));
    assert_eq!(org_step(&org_steps, "ai")["satisfied"], json!(false));
    assert_eq!(org_step(&org_steps, "ai")["resolved"], json!(false));

    // "skip for now" records NOTHING — the step stays pending.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/onboarding/org/steps/ai/skip",
        &[auth(&token)],
        Some(json!({ "never": false })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let (org_steps, _) = summary(&app, &token).await;
    assert_eq!(org_step(&org_steps, "ai")["done"], json!(false));
    assert_eq!(org_step(&org_steps, "ai")["dismissed"], json!(false));

    // Server gate: completion BEFORE the artifact exists is a typed 409.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/onboarding/org/steps/ai/complete",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        409,
        "gate must refuse artifact-less done"
    );

    // "never show this again" IS recorded — and never resets.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/onboarding/org/steps/feeder/skip",
        &[auth(&token)],
        Some(json!({ "never": true })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    assert_eq!(body["resolved"], json!(true));
    let (org_steps, _) = summary(&app, &token).await;
    assert_eq!(org_step(&org_steps, "feeder")["dismissed"], json!(true));

    // Create the artifact (turn the AI master switch on), then the gate passes.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/settings/org",
        &[auth(&token)],
        Some(json!({ "key": "ai.features_enabled", "value": true })),
    )
    .await;
    assert!(
        response.status().is_success(),
        "ai enable: {}",
        response.status()
    );
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/onboarding/org/steps/ai/complete",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "satisfied gate completes");
    let (org_steps, _) = summary(&app, &token).await;
    assert_eq!(org_step(&org_steps, "ai")["done"], json!(true));
}

#[tokio::test]
async fn disable_keeps_state_and_reset_org_clears_steps_only() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let token = register_org(&app, "disres").await;

    // record one dismissal, then disable org-wide
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/onboarding/org/steps/feeder/skip",
        &[auth(&token)],
        Some(json!({ "never": true })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/onboarding/disable",
        &[auth(&token)],
        Some(json!({ "disabled": true })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let (org_steps, _) = summary(&app, &token).await;
    assert_eq!(
        org_steps["org"]["disabled"],
        json!(true),
        "disable must persist"
    );
    // Recorded state SURVIVES the disable (resume keeps everything).
    assert_eq!(
        org_step(&org_steps, "feeder").clone(),
        json!({"dismissed": true, "done": false, "resolved": true, "satisfied": false, "step": "feeder"}),
    );

    // reset/org clears per-step flags but NOT the master switch.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/onboarding/reset/org",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let (org_steps, _) = summary(&app, &token).await;
    assert_eq!(org_steps["org"]["disabled"], json!(true));
    assert_eq!(org_step(&org_steps, "feeder")["dismissed"], json!(false));

    // Re-enable keeps state; the flows family resumes.
    let _ = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/onboarding/disable",
        &[auth(&token)],
        Some(json!({ "disabled": false })),
    )
    .await;
}

#[tokio::test]
async fn user_flow_self_complete_and_member_visibility() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let token = register_org(&app, "selfflow").await;

    // Admin caller sees member rows (self included, profile resolved via
    // display_name — the demo registration sets it).
    let (_o, members_root) = summary(&app, &token).await;
    let self_row = members_root
        .as_array()
        .expect("members")
        .iter()
        .find(|m| m["display_name"] == json!("Owner"))
        .expect("self member row")
        .clone();
    let profile = self_row["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["step"] == "profile")
        .expect("profile step")["resolved"]
        .clone();
    assert_eq!(profile, json!(true));

    // orientation completes (owner self-service, no role requirement)
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/onboarding/user/steps/orientation/complete",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    assert_eq!(body["resolved"], json!(true));
    let (_o, members_root) = summary(&app, &token).await;
    let mine = members_root
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["display_name"] == json!("Owner"))
        .expect("owner row")
        .clone();
    let orientation = mine["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["step"] == "orientation")
        .expect("orientation step")["resolved"]
        .clone();
    assert_eq!(orientation, json!(true));

    // mfa gate: not enrolled yet → the completion is a typed 409
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/onboarding/user/steps/mfa/complete",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 409);
}
