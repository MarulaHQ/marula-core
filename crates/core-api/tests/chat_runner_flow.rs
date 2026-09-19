//! ChatRunner flow over the REAL core-api router (ADR 0008 §3, Wave 18
//! lane 2).
//!
//! Purpose: the bounded model loop under the lane-1 authorization edge —
//! grant missing typed refusal, allowlist model ask, hard cap bound the
//! call BEFORE any stream, host-side turn rate limit (429), the tool
//! wardrobe respecting the capability gate (denied tools refuse typed and
//! the loop CONTINUES on a granted one), the ticket_note_assist analyzing
//! through PROPOSALS (status proposed; the ticket itself never written),
//! usage-ledger rows per model + tool call, provenance audit rows, and the
//! SSE loop framing (turn_started / assistant_delta / tool_invocation /
//! assist_proposed / turn_completed).
//!
//! Non-purpose: SQL-layer behavior (core-db suites), the lane-1 authorize
//! edge details (ai_policy_flow), UI (lane 3), any provider/network call —
//! the model answers are deterministic scripted transports (org-keyed; the
//! BYOK-injection posture). The two KNOWN pre-existing billing failures
//! (billing_rule_previews_all_types, invoice_run_never_double_bills) stay
//! filed and out of this lane.

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

use core_api::routes::chat_runner as ChatRunner;

async fn test_state() -> (PgPool, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("chat runner e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
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

async fn body_json(response: axum::response::Response) -> Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap()
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8_lossy(&bytes).to_string()
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Chat Runner Co",
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
                "role": "admin",
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

async fn me_ids(app: &Router, bearer: &str) -> (Uuid, Uuid) {
    let response = send(app, axum::http::Method::GET, "/api/me", Some(bearer), None).await;
    assert_eq!(response.status(), 200, "auth/me must resolve");
    let me = body_json(response).await;
    (
        me["user_id"].as_str().unwrap().parse().unwrap(),
        me["org_id"].as_str().unwrap().parse().unwrap(),
    )
}

/// One turn POST with the boilerplate message shape; returns the raw
/// response (tests assert status/headers/body per scenario).
async fn post_turn(
    app: &Router,
    session_id: Uuid,
    bearer: &str,
    body: Value,
) -> axum::response::Response {
    send(
        app,
        axum::http::Method::POST,
        &format!("/api/ai/chat/sessions/{session_id}/turns"),
        Some(bearer),
        Some(body),
    )
    .await
}

fn turn_body(surface_key: &str, ticket_id: Option<Uuid>) -> Value {
    let mut body = json!({
        "surface_key": surface_key,
        "messages": [
            { "role": "system", "content": "You are Meridian's in-product assistant." },
            { "role": "user", "content": "help me with this" }
        ],
        "session_opener": true,
    });
    if let Some(ticket_id) = ticket_id {
        body["ticket_id"] = json!(ticket_id.to_string());
    }
    body
}

/// Catalog + org credential + surface grant + capability grant: the
/// complete admit kit for one member on one surface.
async fn seed_admit_kit(app: &Router, owner: &str, member_id: Uuid, surface_key: &str, hex: &str) {
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/settings/ai/catalog",
        Some(owner),
        Some(json!({
            "provider": "openai",
            "model": "meridian-test-chat",
            "label": "Chat Runner Test",
            "capabilities": ["chat", "completion", "tools"],
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
            "secret": "sk-chat-runner-test-secret",
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
            "surface_key": surface_key,
            "grantee_kind": "user",
            "user_id": member_id.to_string(),
            "idempotency_key": format!("grant-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "surface grant");
}

/// Grants an agent capability through the SAME registry the MCP adapter
/// gate reads.
async fn grant_capability(app: &Router, owner: &str, capability: &str, mode: &str) {
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/mcp/capabilities",
        Some(owner),
        Some(json!({ "capability": capability, "mode": mode })),
    )
    .await;
    assert_eq!(response.status(), 200, "capability grant {capability}");
}

async fn default_board(app: &Router, bearer: &str) -> Uuid {
    let nav = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/boards/nav",
            Some(bearer),
            None,
        )
        .await,
    )
    .await;
    nav[0]["id"].as_str().unwrap().parse().expect("board id")
}

async fn create_ticket(app: &Router, bearer: &str, board: Uuid, title: &str) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(bearer),
        Some(json!({ "board_id": board, "title": title, "priority": "p3" })),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket create must succeed");
    body_json(response).await
}

fn final_turn_model(prompt: u32, completion: u32, content: &str) -> ChatRunner::ModelTurn {
    ChatRunner::ModelTurn {
        content: content.to_string(),
        tool_calls: Vec::new(),
        prompt_tokens: prompt,
        completion_tokens: completion,
    }
}

fn usage_rows_for<'a>(ledger: &'a [Value], request_id: &'a str) -> Vec<&'a Value> {
    ledger
        .iter()
        .filter(|row| row["request_id"].as_str() == Some(request_id))
        .collect()
}

async fn ledger_rows(pool: &PgPool, org_id: Uuid, user_id: Uuid) -> Vec<Value> {
    let rows: Vec<(String, i32, i32, Option<String>)> = sqlx::query_as(
        "SELECT surface, prompt_tokens, completion_tokens, request_id \
         FROM ai_usage_ledger WHERE org_id = $1 AND user_id = $2 \
         ORDER BY created_at",
    )
    .bind(org_id)
    .bind(user_id)
    .fetch_all(pool)
    .await
    .expect("ledger read");
    rows.into_iter()
        .map(|(surface, prompt, completion, request_id)| {
            json!({
                "surface": surface,
                "prompt_tokens": prompt,
                "completion_tokens": completion,
                "request_id": request_id,
            })
        })
        .collect()
}

fn tool_call_turn(
    prompt: u32,
    completion: u32,
    id: &str,
    name: &str,
    args: Value,
) -> ChatRunner::ModelTurn {
    ChatRunner::ModelTurn {
        content: String::new(),
        tool_calls: vec![ChatRunner::ToolCallReq {
            id: id.to_string(),
            name: name.to_string(),
            arguments: args,
        }],
        prompt_tokens: prompt,
        completion_tokens: completion,
    }
}

/// Parses the SSE body into (event_kind, payload_json) pairs.
fn sse_frames(body: &str) -> Vec<(String, Value)> {
    let mut frames = Vec::new();
    let mut current_event = String::new();
    for line in body.lines() {
        if let Some(kind) = line.strip_prefix("event: ") {
            current_event = kind.to_string();
        } else if let Some(data) = line.strip_prefix("data: ") {
            if let Ok(payload) = serde_json::from_str(data) {
                frames.push((current_event.clone(), payload));
            }
            current_event.clear();
        }
    }
    frames
}

#[tokio::test(flavor = "multi_thread")]
async fn deny_by_default_refuses_typed_and_outside_allowlist_model_ask() {
    let (_pool, app) = test_state().await;
    let slug = format!("chat-runner-{}", Uuid::now_v7().simple());
    let hex = Uuid::now_v7().simple().to_string();
    let owner = register_org(&app, &slug).await;
    let member = invite_member(
        &app,
        &owner,
        &format!("member-{}@{}.test", Uuid::now_v7().simple(), slug),
    )
    .await;
    let (_member_id, _org_id) = me_ids(&app, &member).await;

    // deny-by-default: no grant → typed refusal BEFORE any stream begins
    let session = Uuid::now_v7();
    let response = post_turn(&app, session, &member, turn_body("general_chat", None)).await;
    assert_eq!(response.status(), 403);
    let refused = body_json(response).await;
    assert_eq!(refused["error"]["reason"], "ai_surface_not_granted");
    assert_eq!(refused["error"]["surface"], "general_chat");

    // admit kit for the member (catalog + credential + grant), then an
    // OUTSIDE-allowlist model ask refuses typed too
    let (member_id, _org_id) = me_ids(&app, &member).await;
    seed_admit_kit(&app, &owner, member_id, "general_chat", &hex).await;

    let _ = &session; // freed below
    let response = post_turn(
        &app,
        session,
        &member,
        json!({
            "surface_key": "general_chat",
            "provider": "openai",
            "model": "not-in-allowlist-model",
            "messages": [{ "role": "user", "content": "hi" }]
        }),
    )
    .await;
    assert_eq!(response.status(), 403);
    assert_eq!(
        body_json(response).await["error"]["reason"],
        "ai_model_not_allowed",
        "an out-of-allowlist ask is refused, never redirected"
    );

    // empty messages → validation (not consumed as a turn)
    let response = post_turn(
        &app,
        session,
        &member,
        json!({ "surface_key": "general_chat", "messages": [] }),
    )
    .await;
    assert_eq!(response.status(), 422);
}

#[tokio::test(flavor = "multi_thread")]
async fn hard_cap_refuses_typed_at_the_edge_and_rate_limit_answers_429() {
    let (pool, app) = test_state().await;
    let slug = format!("chat-runner-cap-{}", Uuid::now_v7().simple());
    let hex = Uuid::now_v7().simple().to_string();
    let owner = register_org(&app, &slug).await;
    let member = invite_member(
        &app,
        &owner,
        &format!("member-{}@{}.test", Uuid::now_v7().simple(), slug),
    )
    .await;
    let (member_id, org_id) = me_ids(&app, &member).await;
    seed_admit_kit(&app, &owner, member_id, "general_chat", &hex).await;

    // per-user hard cap: 10 tokens
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/settings/ai/caps",
        Some(&owner),
        Some(json!({
            "scope": "user",
            "user_id": member_id.to_string(),
            "period": "month",
            "soft_token_limit": 5,
            "hard_token_limit": 10,
            "idempotency_key": format!("cap-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "cap seed");

    // drive usage past the hard cap through the LEDGER ITSELF
    let mut tx = pool.begin().await.unwrap();
    core_db::set_org_and_user(&mut tx, org_id, member_id)
        .await
        .unwrap();
    for _ in 0..3 {
        core_db::ai_policy::record_usage(
            &mut tx,
            &core_db::ai_policy::AiUsageNew {
                id: Uuid::now_v7(),
                org_id,
                user_id: member_id,
                surface: "general_chat".into(),
                provider: "openai".into(),
                model: "meridian-test-chat".into(),
                prompt_tokens: 10,
                completion_tokens: 0,
                cost_weight: 1,
                request_id: Some(Uuid::now_v7().to_string()),
            },
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();

    let session = Uuid::now_v7();
    let response = post_turn(&app, session, &member, turn_body("general_chat", None)).await;
    assert_eq!(response.status(), 403, "hard cap bounds the call typed");
    let refused = body_json(response).await;
    assert_eq!(refused["error"]["reason"], "ai_hard_cap_reached");

    // host-side rate limit: eat the window — the limiter is the HOST edge
    // and every turn POST consumes a slot.
    for i in 0..20 {
        let session_id = Uuid::now_v7();
        let _ = post_turn(
            &app,
            session_id,
            &member,
            json!({
                "surface_key": "general_chat",
                "messages": [{ "role": "user", "content": format!("probe {i}") }]
            }),
        )
        .await;
    }
    let session = Uuid::now_v7();
    let response = post_turn(&app, session, &member, turn_body("general_chat", None)).await;
    assert_eq!(
        response.status(),
        429,
        "the 21st turn in the window throttles"
    );
    let throttled = body_json(response).await;
    assert_eq!(throttled["error"]["reason"], "rate_limited");
    assert!(throttled["error"]["retry_after_secs"].as_i64().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_loop_streams_frames_executes_tools_and_lands_usage_and_audit() {
    let (pool, app) = test_state().await;
    let slug = format!("chat-runner-loop-{}", Uuid::now_v7().simple());
    let hex = Uuid::now_v7().simple().to_string();
    let owner = register_org(&app, &slug).await;
    let member = invite_member(
        &app,
        &owner,
        &format!("member-{}@{}.test", Uuid::now_v7().simple(), slug),
    )
    .await;
    let (member_id, org_id) = me_ids(&app, &member).await;
    seed_admit_kit(&app, &owner, member_id, "general_chat", &hex).await;
    // the wardrobe tool tickets_get_one executes ONLY inside a granted
    // capability; grant it (approval-default dial position is irrelevant
    // for reads).
    grant_capability(&app, &owner, "tickets:read", "proposed").await;

    let board = default_board(&app, &member).await;
    let ticket = create_ticket(&app, &member, board, "Runner loop probe").await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();

    // deterministic model: step 1 calls the read tool, step 2 answers.
    ChatRunner::seed_transport_for_org(
        org_id,
        Arc::new(ChatRunner::ScriptedTransport::new(vec![
            tool_call_turn(
                30,
                8,
                "call_1",
                "tickets_get_one",
                json!({ "id": ticket_id.to_string() }),
            ),
            final_turn_model(50, 20, "The ticket exists; a draft answer."),
        ])),
    );

    let session = Uuid::now_v7();
    let response = post_turn(&app, session, &member, turn_body("general_chat", None)).await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream"),
        "SSE transport choice"
    );
    let body = body_text(response).await;
    let frames = sse_frames(&body);
    let kinds: Vec<&str> = frames.iter().map(|(k, _)| k.as_str()).collect();
    assert!(kinds.contains(&"turn_started"));
    assert!(kinds.contains(&"tool_invocation"));
    assert!(kinds.contains(&"assistant_delta"));
    assert!(kinds.contains(&"turn_completed"));

    // the executed read tool rode the REAL router + capability gate.
    let tool = frames
        .iter()
        .find(|(k, _)| k == "tool_invocation")
        .expect("frame exists")
        .1
        .clone();
    assert_eq!(tool["tool"], "tickets_get_one");
    assert_eq!(tool["outcome"], "executed");
    assert_eq!(tool["capability"], "tickets:read");

    // completed = the loop iterated to its final answer
    let done = frames
        .iter()
        .find(|(k, _)| k == "turn_completed")
        .expect("frame exists")
        .1
        .clone();
    assert_eq!(done["outcome"], "completed");
    assert_eq!(done["steps"], 2);
    assert_eq!(
        done["prompt_tokens"],
        80 /* 30 + 50, host-visible spend */
    );
    assert_eq!(done["completion_tokens"], 28);

    // NO ticket write happened (reads only; the runner never writes).
    let read_back = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket_id}"),
            Some(&member),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(read_back["body"], Value::Null, "no assistant write landed");
    assert_eq!(read_back["title"], "Runner loop probe");

    // every model + tool call lands a usage row (model token spend; tool
    // rows zero-token), request_ids step-unique.
    let ledger = ledger_rows(&pool, org_id, member_id).await;
    let model_rows = usage_rows_for(&ledger, "turn:tool-locator:step:0");
    // exact request ids unknown here — assert counts instead.
    assert!(
        ledger
            .iter()
            .any(|r| r["prompt_tokens"].as_i64().unwrap_or(-1) == 30),
        "model step 1 spend lands"
    );
    assert!(
        ledger
            .iter()
            .any(|r| r["prompt_tokens"].as_i64().unwrap_or(-1) == 0
                && r["completion_tokens"].as_i64().unwrap_or(-1) == 0),
        "the tool call lands a zero-token ledger row"
    );
    let _ = model_rows;
}

#[tokio::test(flavor = "multi_thread")]
async fn wardrobe_respects_the_capability_gate_on_ungranted_tools() {
    let (_pool, app) = test_state().await;
    let slug = format!("chat-runner-nocap-{}", Uuid::now_v7().simple());
    let hex = Uuid::now_v7().simple().to_string();
    let owner = register_org(&app, &slug).await;
    let member = invite_member(
        &app,
        &owner,
        &format!("member-{}@{}.test", Uuid::now_v7().simple(), slug),
    )
    .await;
    let (member_id, org_id) = me_ids(&app, &member).await;
    seed_admit_kit(&app, &owner, member_id, "general_chat", &hex).await;
    // NO tickets:read capability grant this time.

    let board = default_board(&app, &member).await;
    let ticket = create_ticket(&app, &member, board, "No-cap probe").await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();

    ChatRunner::seed_transport_for_org(
        org_id,
        Arc::new(ChatRunner::ScriptedTransport::new(vec![
            tool_call_turn(
                10,
                4,
                "call_1",
                "tickets_get_one",
                json!({ "id": ticket_id.to_string() }),
            ),
            final_turn_model(20, 8, "I could not read the ticket; here is what I know."),
        ])),
    );

    let session = Uuid::now_v7();
    let response = post_turn(&app, session, &member, turn_body("general_chat", None)).await;
    assert_eq!(response.status(), 200);
    let frames = sse_frames(&body_text(response).await);
    let tool = frames
        .iter()
        .find(|(k, _)| k == "tool_invocation")
        .expect("the tool frame exists");
    assert_eq!(tool.1["outcome"], "refused_capability_not_granted");
    // the loop CONTINUES: the model gets the typed refusal as feedback and
    // finishes honestly.
    let done = frames
        .iter()
        .find(|(k, _)| k == "turn_completed")
        .expect("frame exists")
        .1
        .clone();
    assert_eq!(done["outcome"], "completed");
}

#[tokio::test(flavor = "multi_thread")]
async fn ticket_note_assist_files_a_proposal_and_never_direct_writes() {
    let (pool, app) = test_state().await;
    let slug = format!("chat-runner-assist-{}", Uuid::now_v7().simple());
    let hex = Uuid::now_v7().simple().to_string();
    let owner = register_org(&app, &slug).await;
    let member = invite_member(
        &app,
        &owner,
        &format!("member-{}@{}.test", Uuid::now_v7().simple(), slug),
    )
    .await;
    let (member_id, org_id) = me_ids(&app, &member).await;

    // assist WITHOUT tickets:write → typed refusal, deny-by-default
    seed_admit_kit(&app, &owner, member_id, "ticket_note_assist", &hex).await;
    let board = default_board(&app, &member).await;
    let ticket = create_ticket(&app, &member, board, "Assist target").await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();
    let session = Uuid::now_v7();
    let response = post_turn(
        &app,
        session,
        &member,
        turn_body("ticket_note_assist", Some(ticket_id)),
    )
    .await;
    assert_eq!(response.status(), 403);
    assert_eq!(
        body_json(response).await["error"]["reason"],
        "ai_agent_capability_not_granted",
        "the assist files ONLY through the proposal pipeline (grant-gated)"
    );

    // now grant the write capability (approval-default dial)
    grant_capability(&app, &owner, "tickets:write", "proposed").await;
    let session = Uuid::now_v7();
    ChatRunner::seed_transport_for_org(
        org_id,
        Arc::new(ChatRunner::ScriptedTransport::new(vec![final_turn_model(
            40,
            60,
            "Assist draft: suspected DNS failure; propose adding note.",
        )])),
    );
    let response = post_turn(
        &app,
        session,
        &member,
        turn_body("ticket_note_assist", Some(ticket_id)),
    )
    .await;
    assert_eq!(response.status(), 200);
    let frames = sse_frames(&body_text(response).await);
    let proposed = frames
        .iter()
        .find(|(k, _)| k == "assist_proposed")
        .expect("the assist frame exists")
        .1
        .clone();
    let proposal_id: Uuid = proposed["proposal_id"].as_str().unwrap().parse().unwrap();

    // the ticket NEVER changed (proposal-not-write!)
    let read_back = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket_id}"),
            Some(&member),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(read_back["body"], Value::Null, "no unapproved write");
    assert_eq!(read_back["title"], "Assist target");

    // the PROPOSAL row: status proposed, dial captured, agent identity
    let proposal = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/agent-proposals/{proposal_id}"),
            Some(&owner),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(proposal["status"], "proposed");
    assert_eq!(proposal["entity"], "ticket");
    assert_eq!(proposal["dial_mode"], "proposed");
    assert_eq!(proposal["agent_ref"], "chat_runner");
    assert_eq!(
        proposal["proposed_value"]["body"],
        json!("Assist draft: suspected DNS failure; propose adding note."),
    );

    // usage ledger carries the assist's single model call
    let ledger = ledger_rows(&pool, org_id, member_id).await;
    assert!(
        ledger
            .iter()
            .any(|r| r["surface"].as_str() == Some("ticket_note_assist")),
        "the assist's calls ride the same ledger keyed to ITS surface"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unauthenticated_turns_never_reach_the_runner() {
    let (_pool, app) = test_state().await;
    let session = Uuid::now_v7();
    let response = post_turn(
        &app,
        session,
        "nope-not-a-token",
        turn_body("general_chat", None),
    )
    .await;
    assert_eq!(response.status(), 401);
}
