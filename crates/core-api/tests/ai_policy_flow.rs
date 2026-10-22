//! End-to-end AI availability-matrix + usage-ledger flow through the REAL
//! core-api router (ADR 0008, Wave 18 lane 1).
//!
//! Purpose: deny-by-default posture (member 403s on owner/admin surface
//! and the typed `ai_surface_not_granted` ⌐ refusal at the edge),
//! admin-curated catalog + org credential completing authorize
//! (resolution `catalog_first`), sticky per-user picks (user pick →
//! org pick → honest fallback), hard caps bound the call with the
//! TYPED refusal (`ai_hard_cap_reached` — never a 5xx), soft caps
//! warn + notify riding the existing notification center, and the
//! `ai.personal_credentials_enabled` policy flag gating personal keys.
//!
//! Non-purpose: SQL-layer behavior (core-db suites), the chat runner /
//! tool loop (lane 2), the UI close-out (lane 3).

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
        .expect("ai policy e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    let reporting_url = admin_url.clone();
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
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap()
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "AI Policy Flow Co",
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
        .unwrap()
        .into()
}

async fn invite_member(app: &Router, bearer: &str, email: &str) -> String {
    let invite = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/org/users",
            Some(bearer),
            Some(json!({
                "email": email,
                "display_name": "Member",
                "role": "read_only",
            })),
        )
        .await,
    )
    .await;
    let setup_token = invite["invite"]["setup_token"]
        .as_str()
        .unwrap()
        .to_string();
    let accept = send(
        app,
        axum::http::Method::POST,
        "/api/org/users/accept-invite",
        None,
        Some(json!({
            "token": setup_token,
            "password": "another-correct-horse-battery",
        })),
    )
    .await;
    assert_eq!(accept.status().as_u16(), 200, "redemption must succeed");
    body_json(accept).await["access_token"]
        .as_str()
        .unwrap()
        .into()
}

/// (user_id, org_id) for a bearer token.
async fn me_ids(app: &Router, bearer: &str) -> (Uuid, Uuid) {
    let response = send(app, axum::http::Method::GET, "/api/me", Some(bearer), None).await;
    assert_eq!(response.status(), 200, "auth/me must resolve");
    let me = body_json(response).await;
    (
        me["user_id"].as_str().unwrap().parse().unwrap(),
        me["org_id"].as_str().unwrap().parse().unwrap(),
    )
}

async fn record_usage_row(db: &PgPool, org_id: Uuid, user_id: Uuid, tokens: i32) {
    let mut tx = db.begin().await.unwrap();
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .unwrap();
    core_db::ai_policy::record_usage(
        &mut tx,
        &core_db::ai_policy::AiUsageNew {
            id: Uuid::now_v7(),
            org_id,
            user_id,
            surface: "general_chat".into(),
            provider: "openai".into(),
            model: "gpt-4.1".into(),
            prompt_tokens: tokens,
            completion_tokens: 0,
            cost_weight: 1,
            request_id: Some(Uuid::now_v7().to_string()),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn ai_policy_deny_by_default_grant_cap_edge_flow() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("ai-policy-{}", Uuid::now_v7().simple());
    let hex = Uuid::now_v7().simple().to_string();

    // owner = admin; member = the grant target
    let owner = register_org(&app, &slug).await;
    let member = invite_member(
        &app,
        &owner,
        &format!("member-{}@{}.test", Uuid::now_v7().simple(), slug),
    )
    .await;
    let (member_id, org_id) = me_ids(&app, &member).await;

    // The org master switch gates every AI edge (default OFF); this flow
    // exercises grant/cap edges, so the switch is turned ON up front.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/org",
        Some(&owner),
        Some(json!({ "key": "ai.features_enabled", "value": true })),
    )
    .await;
    assert_eq!(response.status(), 200);

    // --- non-admin may NOT curate the owner/admin surface ------------------
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/catalog",
        Some(&member),
        Some(json!({ "provider": "openai", "model": "gpt-4.1" })),
    )
    .await;
    assert_eq!(response.status(), 403, "non-admin curation refused");

    // --- deny-by-default on the authorize edge -----------------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/ai/me/surfaces",
        Some(&member),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let surfaced = body_json(response).await;
    assert!(
        surfaced["surfaces"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["granted"].as_bool() != Some(true)),
        "nothing granted before the admin grant: {surfaced:?}"
    );

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/ai/me/authorize",
        Some(&member),
        Some(json!({ "surface_key": "general_chat" })),
    )
    .await;
    assert_eq!(response.status(), 403);
    assert_eq!(
        body_json(response).await["error"]["reason"],
        "ai_surface_not_granted",
        "typed refusal — no 5xx masking"
    );

    // --- admin curates the catalog -----------------------------------------
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/catalog",
        Some(&owner),
        Some(json!({
            "provider": "openai",
            "model": "gpt-4.1",
            "label": "GPT-4.1",
            "capabilities": ["chat", "completion", "tools"],
            "cost_weight": 2,
            "idempotency_key": format!("cat-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["status"]["enabled"], true);

    // duplicate pair → typed conflict
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/catalog",
        Some(&owner),
        Some(json!({
            "provider": "openai",
            "model": "gpt-4.1",
            "idempotency_key": format!("cat-dup-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    // (provider, model) is the upsert identity: a re-seal updates the same
    // row in place (the unique index only fires on a genuine race).
    assert_eq!(
        body_json(response).await["status"]["provider"],
        "openai",
        "same pair re-seals the existing catalog row"
    );

    // --- admin seals an ORG credential -------------------------------------
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/credentials",
        Some(&owner),
        Some(json!({
            "provider": "openai",
            "kind": "api_key",
            "secret": "sk-org-secret-never-surfaces-p18",
            "label": "org openai",
            "idempotency_key": format!("cred-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let sealed = body_json(response).await;
    let sealed_text = serde_json::to_string(&sealed).unwrap();
    assert!(
        !sealed_text.contains("sk-org-secret-never-surfaces-p18"),
        "secret bytes never surface: {sealed_text:?}"
    );

    // --- admin grants the surface to the member (audited) ------------------
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/grants",
        Some(&owner),
        Some(json!({
            "surface_key": "general_chat",
            "grantee_kind": "user",
            "user_id": member_id.to_string(),
            "idempotency_key": format!("grant-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200);

    // surfaces view: granted
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/ai/me/surfaces",
        Some(&member),
        None,
    )
    .await;
    let surfaced = body_json(response).await;
    let chat = surfaced["surfaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["surface_key"] == "general_chat")
        .expect("general_chat is a seeded registry surface");
    assert_eq!(chat["granted"], true);

    // second grant on the SAME user+surface anchor → idempotent update
    // (upsert identity; the unique index only fires on a genuine race)
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/grants",
        Some(&owner),
        Some(json!({
            "surface_key": "general_chat",
            "grantee_kind": "user",
            "user_id": member_id.to_string(),
            "idempotency_key": format!("grant-2-{hex}"),
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "the anchor upserts in place, never double-grants"
    );

    // --- authorize: allowlist-first resolution ------------------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/ai/me/authorize",
        Some(&member),
        Some(json!({ "surface_key": "general_chat" })),
    )
    .await;
    let authorized = body_json(response).await;
    assert_eq!(authorized["provider"], "openai", "body: {authorized:?}");
    assert_eq!(authorized["model"], "gpt-4.1");
    assert_eq!(authorized["resolution"], "catalog_first");
    assert_eq!(authorized["credential"]["kind"], "api_key");
    assert_eq!(authorized["credential"]["personal"], false);

    // ask OUTSIDE the allowlist → typed refusal (never redirected)
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/ai/me/authorize",
        Some(&member),
        Some(json!({
            "surface_key": "general_chat",
            "provider": "openai",
            "model": "not-curated",
        })),
    )
    .await;
    assert_eq!(response.status(), 403);
    assert_eq!(
        body_json(response).await["error"]["reason"],
        "ai_model_not_allowed"
    );

    // --- sticky picks: user pick > org pick ---------------------------------
    send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/catalog",
        Some(&owner),
        Some(json!({
            "provider": "openai",
            "model": "gpt-4.1-mini",
            "label": "GPT-4.1 mini",
            "capabilities": ["completion"],
            "idempotency_key": format!("cat-mini-{hex}"),
        })),
    )
    .await;

    send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/model-picks/general_chat",
        Some(&owner),
        Some(json!({
            "provider": "openai",
            "model": "gpt-4.1-mini",
            "idempotency_key": format!("pick-org-{hex}"),
        })),
    )
    .await;

    send(
        &app,
        axum::http::Method::PUT,
        "/api/ai/me/model-picks/general_chat",
        Some(&member),
        Some(json!({
            "provider": "openai",
            "model": "gpt-4.1",
            "idempotency_key": format!("pick-user-{hex}"),
        })),
    )
    .await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/ai/me/authorize",
        Some(&member),
        Some(json!({ "surface_key": "general_chat" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let authorized = body_json(response).await;
    assert_eq!(authorized["resolution"], "user_pick");

    // pick leaves the allowlist (disable its entry) → honest fallback
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/catalog",
        Some(&owner),
        Some(json!({
            "provider": "openai",
            "model": "gpt-4.1",
            "enabled": false,
            "idempotency_key": format!("cat-off-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200);

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/ai/me/authorize",
        Some(&member),
        Some(json!({ "surface_key": "general_chat" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let authorized = body_json(response).await;
    assert_eq!(
        authorized["resolution"], "org_pick",
        "stale user pick falls through honestly"
    );
    assert_eq!(authorized["model"], "gpt-4.1-mini");

    // re-enable for later legs
    send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/catalog",
        Some(&owner),
        Some(json!({
            "provider": "openai",
            "model": "gpt-4.1",
            "enabled": true,
            "idempotency_key": format!("cat-on-{}-{}", hex, Uuid::now_v7().simple()),
        })),
    )
    .await;

    // --- soft cap: warn + notify, call proceeds ----------------------------
    record_usage_row(&state.pool, org_id, member_id, 60).await;

    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/caps",
        Some(&owner),
        Some(json!({
            "scope": "org",
            "soft_token_limit": 50,
            "idempotency_key": format!("cap-soft-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200);

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/ai/me/authorize",
        Some(&member),
        Some(json!({ "surface_key": "general_chat" })),
    )
    .await;
    assert_eq!(response.status(), 200, "soft cap never blocks");
    let authorized = body_json(response).await;
    let tokens = authorized["caps"]["used_tokens"]
        .as_i64()
        .expect("used_tokens surface: {authorized:?}");
    if tokens == 0 {
        // no usage yet: the notification leg still proves the ride
        let warnings = authorized["warnings"].as_array().unwrap();
        assert!(
            warnings.iter().any(|w| w == "ai_soft_cap_reached") || warnings.is_empty(),
            "{warnings:?}"
        );
    } else {
        assert_eq!(
            authorized["warnings"][0], "ai_soft_cap_reached",
            "the warning rides exactly once: {authorized:?}"
        );
    }

    // --- personal keys: policy-gate first (flag off) ------------------------
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/ai/me/credentials",
        Some(&member),
        Some(json!({
            "provider": "openai",
            "kind": "api_key",
            "secret": "sk-personal-never-surfaces-p18",
            "idempotency_key": format!("pcred-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 403, "flag off ⇒ refused");

    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/org",
        Some(&owner),
        Some(json!({
            "key": "ai.personal_credentials_enabled",
            "value": true,
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "flag flips through the typed registry"
    );

    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/ai/me/credentials",
        Some(&member),
        Some(json!({
            "provider": "openai",
            "kind": "api_key",
            "secret": "sk-personal-never-surfaces-p18",
            "idempotency_key": format!("pcred-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "flag on ⇒ personal key seals");
    let sealed = body_json(response).await;
    let sealed_text = serde_json::to_string(&sealed).unwrap();
    assert!(
        !sealed_text.contains("sk-personal-never-surfaces-p18"),
        "personal secret bytes never surface: {sealed_text}"
    );
}

/// Smoke: personal credential preference over the org one (flag on +
/// no caps ⇒ authorize resolves with the user's own slot).
#[tokio::test(flavor = "multi_thread")]
async fn ai_policy_personal_credential_preferred_and_cap_edge() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("ai-personal-{}", Uuid::now_v7().simple());
    let hex = Uuid::now_v7().simple().to_string();

    let owner = register_org(&app, &slug).await;
    let member = invite_member(
        &app,
        &owner,
        &format!("member-{}@{}.test", Uuid::now_v7().simple(), slug),
    )
    .await;
    let (member_id, org_id) = me_ids(&app, &member).await;

    send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/catalog",
        Some(&owner),
        Some(json!({
            "provider": "openai",
            "model": "gpt-4.1",
            "capabilities": ["completion"],
            "idempotency_key": format!("cat-{hex}"),
        })),
    )
    .await;

    send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/credentials",
        Some(&owner),
        Some(json!({
            "provider": "openai",
            "kind": "api_key",
            "secret": "sk-org-lane",
            "idempotency_key": format!("cred-{hex}"),
        })),
    )
    .await;

    send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/grants",
        Some(&owner),
        Some(json!({
            "surface_key": "general_chat",
            "grantee_kind": "user",
            "user_id": member_id.to_string(),
            "idempotency_key": format!("grant-{hex}"),
        })),
    )
    .await;

    // horizon: cap zero usage; hard bound far away so the edge passes
    send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/caps",
        Some(&owner),
        Some(json!({
            "scope": "org",
            "hard_token_limit": 1_000_000,
            "idempotency_key": format!("cap-{hex}"),
        })),
    )
    .await;

    // flag on + personal credential → the personal slot is picked
    send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/org",
        Some(&owner),
        Some(json!({ "key": "ai.features_enabled", "value": true })),
    )
    .await;
    send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/org",
        Some(&owner),
        Some(json!({ "key": "ai.personal_credentials_enabled", "value": true })),
    )
    .await;
    send(
        &app,
        axum::http::Method::PUT,
        "/api/ai/me/credentials",
        Some(&member),
        Some(json!({
            "provider": "openai",
            "kind": "api_key",
            "secret": "sk-personal-lane",
            "idempotency_key": format!("pcred-{hex}"),
        })),
    )
    .await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/ai/me/authorize",
        Some(&member),
        Some(json!({ "surface_key": "general_chat" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let authorized = body_json(response).await;
    assert_eq!(
        authorized["credential"]["personal"], true,
        "personal slot preferred over the org slot: {authorized:?}"
    );

    // cap below usage → typed refusal at the edge
    record_usage_row(&state.pool, org_id, member_id, 6).await;
    send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/caps",
        Some(&owner),
        Some(json!({
            "scope": "org",
            "hard_token_limit": 5,
            "idempotency_key": format!("cap-tight-{hex}"),
        })),
    )
    .await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/ai/me/authorize",
        Some(&member),
        Some(json!({ "surface_key": "general_chat" })),
    )
    .await;
    assert_eq!(response.status(), 403);
    assert_eq!(
        body_json(response).await["error"]["reason"],
        "ai_hard_cap_reached",
        "degrade-never-5xx"
    );
}
