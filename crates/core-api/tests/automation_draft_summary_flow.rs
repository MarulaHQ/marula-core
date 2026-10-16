//! Automation draft-summary flow over the REAL core-api router (Wave 43
//! lane 17b, C1).
//!
//! Purpose: the rule builder's live summary leg — one streamed sentence of
//! what the draft does, over the shared admission path and the shared
//! transport seam. Frames (`summary_started` → `summary_delta`(s) →
//! `summary_completed`) are the contract the client renders from, the
//! summary is NOT persisted anywhere, and AI admission refusals ride the
//! existing lane-1 edge with no second gate.
//!
//! Non-purpose: the summary's real wording (no provider call —
//! `ScriptedTransport`, the BYOK-injection posture), the admin surface
//! (`sla_admin`), and C2's prose leg (lane 17's `workflow_authoring_flow`).
//!
//! `MARULA_TEST_DATABASE_URL` (or `DATABASE_URL`) must point at a booted
//! stack (see `scripts/e2e-stack.sh`).

use axum::Router;
use core_api::routes::chat_runner as ChatRunner;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> (PgPool, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("automation draft summary e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    let reporting_url = admin_url.clone();
    let state = AppState {
        pool: pool.clone(),
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: AppState::reporting_pool_lazy(&reporting_url).expect("lazy reporting pool"),
    };
    let app = core_api::routes::build_router(state.clone());
    (pool, app)
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

/// The whole SSE body as text. The handler spawns the stream and returns;
/// collecting the body runs it to completion, which is what a client does.
async fn sse_text(response: axum::response::Response) -> String {
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|c| c.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .to_string();
    assert_eq!(
        content_type, "text/event-stream",
        "the draft summary is an SSE stream, not a JSON body: {content_type:?}"
    );
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec()
        .iter()
        .map(|b| *b as char)
        .collect()
}

/// The body's `event: name\ndata: {..}` pairs, as the client parses them.
fn frames(text: &str) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for event in text.split("\n\n") {
        if event.trim().is_empty() {
            continue;
        }
        let mut name = String::new();
        let mut data = String::new();
        for line in event.lines() {
            if let Some(rest) = line.strip_prefix("event: ") {
                name = rest.trim().to_string();
            }
            if let Some(rest) = line.strip_prefix("data: ") {
                data.push_str(rest.trim());
            }
        }
        if name.is_empty() {
            continue;
        }
        out.push((name, serde_json::from_str(&data).unwrap_or(Value::Null)));
    }
    out
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
            "org_name": "Draft Summary Co",
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

async fn me_ids(app: &Router, bearer: &str) -> (Uuid, Uuid) {
    let response = send(app, axum::http::Method::GET, "/api/me", Some(bearer), None).await;
    assert_eq!(response.status(), 200, "auth/me must resolve");
    let me = body_json(response).await;
    (
        me["user_id"].as_str().unwrap().parse().unwrap(),
        me["org_id"].as_str().unwrap().parse().unwrap(),
    )
}

/// Catalog + org credential + surface grant + master switch: the complete
/// admit kit for the owner on `general_chat`. The summary is a core surface,
/// so the seeded `automation_writing` PLUGIN surface is deliberately NOT
/// what this leg rides.
async fn seed_admit_kit(app: &Router, owner: &str, user_id: Uuid, hex: &str) {
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/settings/org",
        Some(owner),
        Some(json!({ "key": "ai.features_enabled", "value": true })),
    )
    .await;
    assert_eq!(response.status(), 200, "master switch on");
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/settings/ai/catalog",
        Some(owner),
        Some(json!({
            "provider": "openai",
            "model": "marula-test-chat",
            "label": "Summary Test",
            "capabilities": ["chat", "completion"],
            "cost_weight": 1,
            "idempotency_key": format!("cat-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "catalog seed");
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/settings/ai/credentials",
        Some(owner),
        Some(json!({
            "provider": "openai",
            "kind": "api_key",
            "secret": "sk-summary-test-secret",
            "label": "org openai",
            "idempotency_key": format!("cred-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "credential seed");
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/settings/ai/grants",
        Some(owner),
        Some(json!({
            "surface_key": "general_chat",
            "grantee_kind": "user",
            "user_id": user_id.to_string(),
            "idempotency_key": format!("grant-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "surface grant");
}

fn draft() -> Value {
    json!({
        "surface_key": "general_chat",
        "name": "Route unassigned work",
        "trigger_event": "ticket.created",
        "subject_kind": "ticket",
        "scope": "Support",
        "conditions_text": "it has no assignee",
        "actions_text": ["assign to Alice"],
        "incomplete_conditions": 0
    })
}

async fn summarize(app: &Router, bearer: &str, body: Value) -> axum::response::Response {
    send(
        app,
        axum::http::Method::POST,
        "/api/v1/automation-rules/draft-summary",
        Some(bearer),
        Some(body),
    )
    .await
}

#[tokio::test]
async fn a_summarized_draft_streams_its_frames_and_persists_nothing() {
    let (pool, app) = test_state().await;
    let owner = register_org(&app, &format!("summary-{}", Uuid::now_v7().simple())).await;
    let (user_id, org_id) = me_ids(&app, &owner).await;
    seed_admit_kit(&app, &owner, user_id, "summary-happy").await;
    // The scripted transport inherits the DEFAULT stream: one whole-turn
    // fragment. That is the degradation a non-streaming transport owes the
    // contract, not a special case.
    ChatRunner::seed_transport_for_org(
        org_id,
        Arc::new(ChatRunner::ScriptedTransport::new(vec![
            ChatRunner::ModelTurn {
                content: "Creates a ticket and pings the on-call people.".into(),
                tool_calls: Vec::new(),
                prompt_tokens: 120,
                completion_tokens: 24,
            },
        ])),
    );

    let response = summarize(&app, &owner, draft()).await;
    assert_eq!(
        response.status(),
        200,
        "the summary must stream: {response:?}"
    );
    let text = sse_text(response).await;
    let parsed = frames(&text);

    let names: Vec<&str> = parsed.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        vec!["summary_started", "summary_delta", "summary_completed"],
        "a non-streaming transport still owes the full frame contract: {text}"
    );
    // Provenance rides `summary_started`.
    let (_, started) = parsed.first().expect("started").clone();
    assert_eq!(started["model"], json!("marula-test-chat"));
    assert_eq!(started["provider"], json!("openai"));
    // Delta carries the fragment key the chat runner's assistant_delta uses.
    let (_, delta) = parsed.get(1).expect("delta").clone();
    assert_eq!(
        delta["delta"],
        json!("Creates a ticket and pings the on-call people.")
    );
    // Completed carries the whole sentence and the usage the turn returned.
    let (_, completed) = parsed.last().expect("completed").clone();
    assert_eq!(
        completed["text"],
        json!("Creates a ticket and pings the on-call people.")
    );
    assert_eq!(completed["usage"]["prompt_tokens"], json!(120));
    assert_eq!(completed["usage"]["completion_tokens"], json!(24));

    // The summary is a summary: no rule, no transcript, no draft row. The
    // usage ledger row is the only durable fact.
    let rules: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM automation_rules WHERE org_id = $1")
        .bind(org_id)
        .fetch_one(&pool)
        .await
        .expect("rule count");
    assert_eq!(rules, 0, "summarising a draft never writes a rule");
    let (surface, prompt, completion): (String, i32, i32) = sqlx::query_as(
        "SELECT surface, prompt_tokens, completion_tokens FROM ai_usage_ledger WHERE org_id = $1",
    )
    .bind(org_id)
    .fetch_one(&pool)
    .await
    .expect("usage row");
    assert_eq!(surface, "general_chat");
    assert_eq!(prompt, 120);
    assert_eq!(completion, 24);
}

#[tokio::test]
async fn ai_switch_off_and_no_grant_are_typed_refusals_before_any_stream() {
    let (_, app) = test_state().await;
    let owner = register_org(&app, &format!("summary-{}", Uuid::now_v7().simple())).await;
    let (user_id, _) = me_ids(&app, &owner).await;
    // Master switch off: the admission edge is the gate, and this module
    // adds no second one. The body carrys a machine reason under `error`,
    // which is what a client branches on.
    let response = summarize(&app, &owner, draft()).await;
    assert_eq!(response.status(), 403, "a switch that is OFF refuses typed");
    let refused = body_json(response).await;
    assert_eq!(
        refused["error"]["reason"],
        json!("ai_features_disabled"),
        "{refused}"
    );
    assert_eq!(refused["error"]["code"], json!(403), "{refused}");

    // Switch on, no grant: still refused, still typed.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/org",
        Some(&owner),
        Some(json!({ "key": "ai.features_enabled", "value": true })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = promptless_summarize(&app, &owner, user_id).await;
    assert_eq!(
        response.status(),
        403,
        "no grant on the surface refuses typed"
    );
}

/// Same request, no admit kit — isolates the no-grant refusal.
async fn promptless_summarize(
    app: &Router,
    bearer: &str,
    user_id: Uuid,
) -> axum::response::Response {
    let mut body = draft();
    body["actions_text"] = json!(["assign to Alice"]);
    let _ = user_id;
    summarize(app, bearer, body).await
}

#[tokio::test]
async fn an_empty_trigger_is_a_validation_error_not_a_model_call() {
    let (_, app) = test_state().await;
    let owner = register_org(&app, &format!("summary-{}", Uuid::now_v7().simple())).await;
    let (user_id, _) = me_ids(&app, &owner).await;
    seed_admit_kit(&app, &owner, user_id, "summary-empty").await;
    let mut body = draft();
    body["trigger_event"] = json!("   ");
    ChatRunner::seed_transport_for_org(
        Uuid::now_v7(),
        Arc::new(ChatRunner::ScriptedTransport::new(Vec::new())),
    );
    let response = summarize(&app, &owner, body).await;
    assert_eq!(
        response.status(),
        422,
        "an unnamed trigger is a validation error, not a model ask"
    );
    // The transport was never reached: nothing else needed it.
}
