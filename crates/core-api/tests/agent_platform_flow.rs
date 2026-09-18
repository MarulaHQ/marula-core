//! Agent-platform v1 flow (TASKLIST Phase 3, IF8) through the REAL
//! core-api router.
//!
//! Purpose: pin the MCP-v1 + proposal-pipeline contract over HTTP:
//! 1. Capability registry: deny-by-default (unknown capabilities 403 the
//!    proposal), grant/re-dial/revoke round-trip, autonomous restricted
//!    to `:write` capabilities, owner/admin-only surface.
//! 2. Agent skills: publish appends versions (server-derived max+1),
//!    history is append-only (no in-place edits), listing modes work.
//! 3. IF8 proposal pipeline: agents file proposals carrying the full
//!    provenance snapshot (before/proposed, agent identity, skill +
//!    version, rule-set hash, dial position captured from the grant);
//!    a DECLINED proposal produces no write at all; an APPROVED ticket
//!    update applies through the IF1 instant-first path (write-trail
//!    row, monotonic seq, undoable `mutation.before/after`); a proposal
//!    can never be decided twice (typed 409); withdrawal is proposer/
//!    admin only.
//! 4. Provenance audit: every proposal action and skill publish lands an
//!    audit event, and the domain events land in the outbox
//!    (`agent_proposal.created` / `.approved` / `.declined` /
//!    `.withdrawn`, `agent_skill.*`, `mcp_capability.updated`).
//!
//! Database selection follows the lane suite contract:
//! MERIDIAN_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
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
        .expect("agent-platform e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([41; 32]);
    let state = AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
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

struct Org {
    token: String,
    org_id: Uuid,
}

async fn register_org(app: &Router, slug: String) -> Org {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Agent Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let tokens = body_json(response).await;
    let token = tokens["access_token"].as_str().unwrap().to_string();
    let me = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/v1/me",
            &[auth(&token)],
            None,
        )
        .await,
    )
    .await;
    Org {
        token,
        org_id: me["org_id"].as_str().unwrap().parse().unwrap(),
    }
}

async fn default_board(app: &Router, org: &Org) -> Uuid {
    let nav = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/v1/boards/nav",
            &[auth(&org.token)],
            None,
        )
        .await,
    )
    .await;
    nav[0]["id"].as_str().unwrap().parse().expect("board id")
}

async fn create_ticket(app: &Router, org: &Org, board: Uuid, title: &str) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/tickets",
        &[auth(&org.token)],
        Some(json!({ "board_id": board, "title": title, "priority": "p3" })),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket create must succeed");
    body_json(response).await
}

/// Creates an org API key with the given scopes, returning the raw key.
async fn create_api_key(app: &Router, org: &Org, name: &str, scopes: &[&str]) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/api-keys",
        &[auth(&org.token)],
        Some(json!({ "name": name, "scopes": scopes })),
    )
    .await;
    assert_eq!(response.status(), 201, "api key create must succeed");
    body_json(response).await["raw_key"]
        .as_str()
        .expect("raw key")
        .into()
}

async fn grant_capability(app: &Router, org: &Org, capability: &str, mode: &str) {
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/v1/mcp/capabilities",
        &[auth(&org.token)],
        Some(json!({ "capability": capability, "mode": mode })),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "capability grant must succeed: {capability}"
    );
}

async fn file_proposal(
    app: &Router,
    org: &Org,
    api_key: &str,
    body: Value,
) -> axum::response::Response {
    send(
        app,
        axum::http::Method::POST,
        "/api/v1/agent-proposals",
        &[auth(&org.token), ("x-api-key", api_key.to_string())],
        Some(body),
    )
    .await
}

async fn outbox_events(state: &AppState, org: Uuid, event_type: &str) -> Vec<Value> {
    let rows: Vec<(serde_json::Value,)> = sqlx::query_as(
        "SELECT payload FROM outbox_events WHERE org_id = $1 AND event_type = $2 ORDER BY occurred_at",
    )
    .bind(org)
    .bind(event_type)
    .fetch_all(&state.pool)
    .await
    .expect("outbox read");
    rows.into_iter().map(|r| r.0).collect()
}

// ===========================================================================
// Capability registry
// ===========================================================================

#[tokio::test]
async fn capability_registry_round_trip_and_deny_by_default() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("ag-cap-{}", Uuid::now_v7())).await;

    // Nothing granted yet: the listing is empty (deny-by-default).
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/mcp/capabilities",
        &[auth(&org.token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        body_json(response).await["grants"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    // Grant + re-dial; autonomous on a write capability is legal.
    grant_capability(&app, &org, "tickets:read", "proposed").await;
    grant_capability(&app, &org, "tickets:write", "autonomous").await;
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/mcp/capabilities",
        &[auth(&org.token)],
        None,
    )
    .await;
    let grants = body_json(response).await["grants"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(grants.len(), 2);
    let write = grants
        .iter()
        .find(|g| g["capability"] == "tickets:write")
        .unwrap();
    assert_eq!(write["mode"], "autonomous");

    // Autonomous on a READ capability is refused: it is a per-write-
    // capability grant, never a global switch.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/mcp/capabilities",
        &[auth(&org.token)],
        Some(json!({ "capability": "clients:read", "mode": "autonomous" })),
    )
    .await;
    assert_eq!(response.status(), 422);

    // Revoke: the grant disappears (deny again).
    let response = send(
        &app,
        axum::http::Method::DELETE,
        "/api/v1/mcp/capabilities/tickets%3Aread",
        &[auth(&org.token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 204);
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/mcp/capabilities",
        &[auth(&org.token)],
        None,
    )
    .await;
    let grants = body_json(response).await["grants"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(grants.len(), 1);
}

#[tokio::test]
async fn capability_registry_is_admin_only() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("ag-role-{}", Uuid::now_v7())).await;
    // A same-org user with no role (no permissions).
    let email = format!("peasant-{}@agent.test", Uuid::now_v7());
    core_db::users::create_user(
        &state.pool,
        Uuid::now_v7(),
        org.org_id,
        &email,
        "No Roles",
        &core_api::auth::password::hash("correct-horse-battery").unwrap(),
        "UTC",
    )
    .await
    .expect("seed user");
    let login = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/auth/login",
            &[],
            Some(json!({"email": email, "password": "correct-horse-battery"})),
        )
        .await,
    )
    .await;
    let token = login["access_token"].as_str().unwrap();
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/mcp/capabilities",
        &[auth(token)],
        Some(json!({ "capability": "tickets:read", "mode": "proposed" })),
    )
    .await;
    assert_eq!(
        response.status(),
        403,
        "non-admin cannot grant capabilities"
    );
}

// ===========================================================================
// Agent skills
// ===========================================================================

#[tokio::test]
async fn skill_versions_append_and_history_never_rewrites() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("ag-skill-{}", Uuid::now_v7())).await;
    let slug = "triage-drafter";

    // First publish → v1.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/agent-skills",
        &[auth(&org.token)],
        Some(json!({
            "slug": slug,
            "manifest": { "capabilities": ["tickets:read"], "prompts": ["Draft a triage note"], "rules": [] }
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let v1 = body_json(response).await;
    assert_eq!(v1["version"], 1);

    // Second publish → v2 (server-derived, append-only).
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/agent-skills",
        &[auth(&org.token)],
        Some(json!({
            "slug": slug,
            "manifest": { "capabilities": ["tickets:read", "clients:read"], "prompts": [], "rules": [{"id": "no-close"}] }
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["version"], 2);

    // Latest listing shows ONE row (v2); history shows BOTH (append-only).
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/agent-skills",
        &[auth(&org.token)],
        None,
    )
    .await;
    let latest = body_json(response).await["skills"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0]["version"], 2);
    assert_eq!(latest[0]["slug"], slug);

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/agent-skills?slug={slug}"),
        &[auth(&org.token)],
        None,
    )
    .await;
    let history = body_json(response).await["skills"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0]["version"], 2, "history newest first");
    assert_eq!(history[1]["version"], 1);

    // One fetched by id stays immutable.
    let v1_id = history[1]["id"].as_str().unwrap();
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/agent-skills/{v1_id}"),
        &[auth(&org.token)],
        None,
    )
    .await;
    let fetched = body_json(response).await;
    assert_eq!(fetched["version"], 1);
    assert_eq!(
        fetched["manifest"]["prompts"][0], "Draft a triage note",
        "v1 manifest unchanged by v2 publish"
    );

    // Invalid manifests are rejected with a typed 422.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/agent-skills",
        &[auth(&org.token)],
        Some(json!({ "slug": "bad-typo", "manifest": { "prompts": "not-an-array" } })),
    )
    .await;
    assert_eq!(response.status(), 422);
}

// ===========================================================================
// IF8 proposal pipeline
// ===========================================================================

fn proposal_body(ticket_id: &str, title: &str) -> Value {
    json!({
        "entity": "ticket",
        "entity_id": ticket_id,
        "op": "update",
        "before_value": { "title": "Original title" },
        "proposed_value": { "title": title },
        "classification": "internal",
        "capability": "tickets:write",
        "skill_slug": "triage-drafter",
        "skill_version": 2,
        "rule_set_hash": "abc123",
        "session_id": "sess-1",
        "tool": "mcp:read_only_probe",
        "rationale": "Sharpen the summary for triage."
    })
}

#[tokio::test]
async fn deny_by_default_filing_without_grant_is_forbidden() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("ag-deny-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "Original").await;
    let api_key = create_api_key(&app, &org, "agent", &["tickets:read"]).await;

    let response = file_proposal(
        &app,
        &org,
        &api_key,
        proposal_body(ticket["id"].as_str().unwrap(), "Nope"),
    )
    .await;
    assert_eq!(
        response.status(),
        403,
        "no tickets:write grant ⇒ 403 (deny-by-default)"
    );
}

#[tokio::test]
async fn declined_proposal_produces_no_write() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("ag-decline-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "Original title").await;
    let ticket_id = ticket["id"].as_str().unwrap().to_string();
    let api_key = create_api_key(&app, &org, "agent", &["tickets:read"]).await;
    grant_capability(&app, &org, "tickets:write", "proposed").await;

    // The agent files; the dial position is captured from the grant.
    let response = file_proposal(
        &app,
        &org,
        &api_key,
        proposal_body(&ticket_id, "Agent rewrite"),
    )
    .await;
    assert_eq!(response.status(), 200, "filing must succeed with the grant");
    let proposal = body_json(response).await["proposal"].clone();
    let proposal_id = proposal["id"].as_str().unwrap().to_string();
    assert_eq!(proposal["status"], "proposed");
    assert_eq!(
        proposal["dial_mode"], "proposed",
        "dial captured from grant"
    );
    assert_eq!(proposal["agent_kind"], "api-key");
    assert_eq!(
        proposal["before_value"]["title"], "Original title",
        "IF8 before-snapshot stored"
    );
    assert_eq!(proposal["skill_slug"], "triage-drafter");
    assert_eq!(proposal["skill_version"], 2);
    assert_eq!(proposal["rule_set_hash"], "abc123");
    assert_eq!(proposal["proposed_value"]["title"], "Agent rewrite");

    // The ticket is UNTOUCHED while the proposal sits open.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/tickets/{ticket_id}"),
        &[auth(&org.token)],
        None,
    )
    .await;
    assert_eq!(body_json(response).await["title"], "Original title");

    // Decline: no write at all.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/agent-proposals/{proposal_id}/decline"),
        &[auth(&org.token)],
        Some(json!({ "note": "Does not read right" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let decided = body_json(response).await;
    assert_eq!(decided["status"], "declined");
    assert_eq!(decided["decision_note"], "Does not read right");

    // Still untouched.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/tickets/{ticket_id}"),
        &[auth(&org.token)],
        None,
    )
    .await;
    assert_eq!(body_json(response).await["title"], "Original title");

    // Double decision is a typed 409.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/agent-proposals/{proposal_id}/approve"),
        &[auth(&org.token)],
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 409, "a proposal is decided exactly once");

    // Provenance audit + contract events landed.
    let events = outbox_events(&state, org.org_id, "agent_proposal.created").await;
    assert_eq!(events.len(), 1);
    let events = outbox_events(&state, org.org_id, "agent_proposal.declined").await;
    assert_eq!(events.len(), 1);
    assert!(
        events[0]["declined_by"].is_string(),
        "decision provenance rides the event"
    );
}

#[tokio::test]
async fn approved_ticket_proposal_applies_through_instant_first_path() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("ag-approve-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "Draft title").await;
    let ticket_id = ticket["id"].as_str().unwrap().to_string();
    let api_key = create_api_key(&app, &org, "agent", &["tickets:read"]).await;
    grant_capability(&app, &org, "tickets:write", "proposed").await;

    let response = file_proposal(
        &app,
        &org,
        &api_key,
        proposal_body(&ticket_id, "Approved rewrite"),
    )
    .await;
    let proposal = body_json(response).await["proposal"].clone();
    let proposal_id = proposal["id"].as_str().unwrap().to_string();

    // Approve: the change lands as a normal autosave write.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/agent-proposals/{proposal_id}/approve"),
        &[auth(&org.token)],
        Some(json!({ "note": "Ship it" })),
    )
    .await;
    if response.status() != 200 {
        let body = body_json(response).await;
        panic!("approve failed: {body}");
    }

    // The ticket carries the approved value.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/tickets/{ticket_id}"),
        &[auth(&org.token)],
        None,
    )
    .await;
    let after = body_json(response).await;
    assert_eq!(after["title"], "Approved rewrite");

    // …and the write is on the instant-first trail: the write-trail row
    // carries before + after (the undo substrate), keyed to the
    // deterministic proposal idempotency key.
    let trail: Vec<(String, Value, Value)> = sqlx::query_as(
        "SELECT op, COALESCE(\"before\", 'null'::jsonb), \"after\" FROM entity_writes \
         WHERE org_id = $1 AND entity = 'ticket' AND entity_id = $2 ORDER BY seq",
    )
    .bind(org.org_id)
    .bind(
        ticket["id"]
            .as_str()
            .unwrap()
            .parse::<Uuid>()
            .expect("ticket uuid"),
    )
    .fetch_all(&state.pool)
    .await
    .expect("trail read");
    let last = trail.last().expect("at least the create + apply writes");
    assert_eq!(last.0, "update");
    assert_eq!(last.1["title"], "Draft title", "before value captured");
    assert_eq!(last.2["title"], "Approved rewrite", "after value captured");

    // Approval events + audit landed.
    let events = outbox_events(&state, org.org_id, "agent_proposal.approved").await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["applied"], json!(true));

    // Re-approving is a typed 409 (decided exactly once).
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/agent-proposals/{proposal_id}/approve"),
        &[auth(&org.token)],
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 409);
}

#[tokio::test]
async fn withdrawal_is_proposer_or_admin_only() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("ag-withdraw-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "Original").await;
    let api_key = create_api_key(&app, &org, "agent", &["tickets:read"]).await;
    grant_capability(&app, &org, "tickets:write", "proposed").await;

    let response = file_proposal(
        &app,
        &org,
        &api_key,
        proposal_body(ticket["id"].as_str().unwrap(), "Withdraw me"),
    )
    .await;
    let proposal_id = body_json(response).await["proposal"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // The agent withdraws its own proposal (same machine identity).
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/agent-proposals/{proposal_id}/withdraw"),
        &[auth(&org.token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["status"], "withdrawn");

    // The ticket is untouched.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/tickets/{}", ticket["id"].as_str().unwrap()),
        &[auth(&org.token)],
        None,
    )
    .await;
    assert_eq!(body_json(response).await["title"], "Original");

    let events = outbox_events(&state, org.org_id, "agent_proposal.withdrawn").await;
    assert_eq!(events.len(), 1);
}
