//! End-to-end time-entry approval flows (Wave 14 lane 1) driven through
//! the REAL core-api router against a real Postgres.
//!
//! Purpose: prove the submit → review → approve/reject workflow over HTTP
//! (typed rejection reason codes with a closed vocabulary, reopen →
//! resubmit, bulk decisions with per-item outcomes, the approver queue
//! with per-approver decision history, the append-only decision trail),
//! the invoicing lock (an entry consumed by `invoice_time_entries` refuses
//! every mutation with the typed `time_entry_locked` reason), approval
//! carried into billing reads (unapproved entries never reach the invoice
//! preview), and RBAC (`special:approve_time` gates decisions).
//!
//! Non-purpose: pure state-machine math (core-domain unit tests), event
//! wire shapes (core-events golden conformance).

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
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("approvals e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    let state = AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&admin_url)
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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!("body_json parse failed ({e}); last SEND lines above identify the endpoint")
    })
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Approvals Co",
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
        .to_string()
}

async fn me(app: &Router, bearer: &str) -> Value {
    body_json(send(app, axum::http::Method::GET, "/api/me", Some(bearer), None).await).await
}

async fn default_board(app: &Router, bearer: &str) -> String {
    let boards = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/boards",
            Some(bearer),
            None,
        )
        .await,
    )
    .await;
    boards
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["is_default"] == true)
        .expect("default board")["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn create_ticket(app: &Router, bearer: &str) -> Value {
    let board_id = default_board(app, bearer).await;
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(bearer),
        Some(json!({
            "board_id": board_id,
            "title": "Approval fixture ticket",
            "priority": "p3",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket create must succeed");
    body_json(response).await
}

async fn create_entry(app: &Router, bearer: &str, ticket_id: &str, hours_ago: i64) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/time-entries",
        Some(bearer),
        Some(json!({
            "ticket_id": ticket_id,
            "started_at": (chrono::Utc::now() - chrono::Duration::hours(hours_ago)),
            "ended_at": (chrono::Utc::now() - chrono::Duration::hours(hours_ago)
                + chrono::Duration::minutes(60)),
            "billable": true,
            "notes": "worked on the thing",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "entry create must succeed");
    let body = body_json(response).await;
    body["id"].as_str().unwrap().to_string()
}

async fn submit_entry(app: &Router, bearer: &str, entry_id: &str) -> (u16, Value) {
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/time-entries/{entry_id}/submit"),
        Some(bearer),
        Some(json!({ "note": "for review" })),
    )
    .await;
    let status = response.status().as_u16();
    (status, body_json(response).await)
}

async fn fix_entry(app: &Router, bearer: &str, entry_id: &str, body: Value) -> (u16, Value) {
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/time-entries/{entry_id}/fix"),
        Some(bearer),
        Some(body),
    )
    .await;
    let status = response.status().as_u16();
    (status, body_json(response).await)
}

async fn decide(app: &Router, bearer: &str, body: Value) -> (u16, Value) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/time-entries/approvals/decide",
        Some(bearer),
        Some(body),
    )
    .await;
    let status = response.status().as_u16();
    (status, body_json(response).await)
}

/// Invites one member with a preset role and redeems the invite; returns
/// their access token.
async fn invite_with_role(app: &Router, owner: &str, role: &str) -> String {
    let email = format!("{role}-{}@test.local", Uuid::now_v7().simple());
    let invite = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/org/users",
            Some(owner),
            Some(json!({
                "email": email,
                "display_name": role,
                "role": role,
            })),
        )
        .await,
    )
    .await;
    let setup_token = invite["invite"]["setup_token"]
        .as_str()
        .expect("setup token")
        .to_string();
    let accept = send(
        app,
        axum::http::Method::POST,
        "/api/org/users/accept-invite",
        None,
        Some(json!({ "token": setup_token, "password": "another-correct-horse" })),
    )
    .await;
    assert_eq!(
        accept.status().as_u16(),
        200,
        "redemption must succeed: {role}"
    );
    body_json(accept).await["access_token"]
        .as_str()
        .expect("access token")
        .to_string()
}

#[tokio::test]
async fn submit_approve_lifecycle_with_queue_history_and_bulk() {
    let (_state, app) = test_state().await;
    let bearer = register_org(&app, &format!("appr-{}", Uuid::now_v7().simple())).await;
    let ticket = create_ticket(&app, &bearer).await;
    let ticket_id = ticket["id"].as_str().unwrap().to_string();

    let e1 = create_entry(&app, &bearer, &ticket_id, 5).await;
    let e2 = create_entry(&app, &bearer, &ticket_id, 6).await;
    let e3 = create_entry(&app, &bearer, &ticket_id, 7).await;

    // A fresh entry is draft.
    let entries = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/time-entries?ticket_id={ticket_id}"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(entries[0]["approval_status"], json!("draft"));

    // Submit two; the third stays draft (proves the queue filter).
    let (status, body) = submit_entry(&app, &bearer, &e1).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["approval_status"], json!("submitted"));
    let (status, _) = submit_entry(&app, &bearer, &e2).await;
    assert_eq!(status, 200);
    // Double-submit is a typed 422, not a silent replay.
    let (status, body) = submit_entry(&app, &bearer, &e1).await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["error"]["reason"], json!("validation_failed"));

    // The queue (default: submitted) holds exactly the two.
    let queue = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/time-entries/approvals/queue",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let ids: Vec<&str> = queue
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["entry"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 2, "{queue}");
    assert!(ids.contains(&e1.as_str()) && ids.contains(&e2.as_str()));

    // Reject e1 with a typed reason from the closed vocabulary.
    let (status, body) = decide(
        &app,
        &bearer,
        json!({ "entry_ids": [e1], "decision": "reject", "reason_code": "missing_notes" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["results"][0]["outcome"], json!("applied"));
    assert_eq!(
        body["results"][0]["entry"]["approval_status"],
        json!("rejected")
    );
    assert_eq!(
        body["results"][0]["entry"]["approval_reason_code"],
        json!("missing_notes")
    );

    // A rejection without a typed reason code is a 422 naming the contract.
    let (status, body) = decide(
        &app,
        &bearer,
        json!({ "entry_ids": [e2], "decision": "reject" }),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    // Unknown vocabulary is a 422 listing the closed set.
    let (status, body) = decide(
        &app,
        &bearer,
        json!({ "entry_ids": [e2], "decision": "reject", "reason_code": "made_up" }),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("missing_notes"));

    // Approve e2 (bulk shape: two ids, one applies, one is not in queue).
    let (status, body) = decide(
        &app,
        &bearer,
        json!({ "entry_ids": [e2, e3], "decision": "approve" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let outcomes: Vec<(&str, &str)> = body["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["entry_id"].as_str().unwrap(),
                r["outcome"].as_str().unwrap(),
            )
        })
        .collect();
    assert!(outcomes.contains(&(e2.as_str(), "applied")), "{body}");
    assert!(outcomes.contains(&(e3.as_str(), "not_in_queue")), "{body}");
    assert_eq!(
        body["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["entry_id"] == json!(e2))
            .unwrap()["entry"]["approval_status"],
        json!("approved")
    );

    // Reopen the rejected entry → draft, then resubmit.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/time-entries/{e1}/reopen"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(response.status().as_u16(), 200);
    let body = body_json(response).await;
    assert_eq!(body["approval_status"], json!("draft"));
    assert_eq!(body["approval_reason_code"], json!(null));
    let (status, body) = submit_entry(&app, &bearer, &e1).await;
    assert_eq!(status, 200, "{body}");

    // The decision trail is append-only and seq-ordered, every step kept.
    let history = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/time-entries/{e1}/approvals"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let seq: Vec<(i64, &str)> = history
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["seq"].as_i64().unwrap(), r["action"].as_str().unwrap()))
        .collect();
    assert_eq!(
        seq,
        vec![
            (1, "submitted"),
            (2, "rejected"),
            (3, "reopened"),
            (4, "submitted")
        ],
        "{history}"
    );
    assert_eq!(history[1]["reason_code"], json!("missing_notes"));
}

#[tokio::test]
async fn locked_after_invoicing_and_billing_reads_only_see_approved() {
    let (state, app) = test_state().await;
    let bearer = register_org(&app, &format!("appr-lock-{}", Uuid::now_v7().simple())).await;
    let me = me(&app, &bearer).await;
    let org_id: Uuid = me["org_id"].as_str().unwrap().parse().unwrap();
    let user_id: Uuid = me["user_id"].as_str().unwrap().parse().unwrap();
    let ticket = create_ticket(&app, &bearer).await;
    let ticket_id = ticket["id"].as_str().unwrap().to_string();

    // One APPROVED entry and one DRAFT entry on the same ticket.
    let approved = create_entry(&app, &bearer, &ticket_id, 8).await;
    let _draft = create_entry(&app, &bearer, &ticket_id, 9).await;
    let (status, body) = submit_entry(&app, &bearer, &approved).await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = decide(
        &app,
        &bearer,
        json!({ "entry_ids": [approved], "decision": "approve" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    // Fixture rows for the lock/consumption mechanics (the invoice-run
    // HTTP flow is out of scope here): client + contract + invoice +
    // one invoice_time_entries link consuming the APPROVED entry.
    let client_id = Uuid::now_v7();
    let contract_id = Uuid::now_v7();
    sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, 'Lock Co')")
        .bind(client_id)
        .bind(org_id)
        .execute(&state.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE tickets SET client_id = $1 WHERE id = $2 AND org_id = $3")
        .bind(client_id)
        .bind(ticket["id"].as_str().unwrap().parse::<Uuid>().unwrap())
        .bind(org_id)
        .execute(&state.pool)
        .await
        .unwrap();
    let today = chrono::Utc::now().date_naive();
    // create_entry stamps entries at `now - 8h`; before 08:00 UTC the entry
    // lands on YESTERDAY's date. The contract must already be effective on
    // that date or the billing engine (correctly) clips the approved entry
    // out of the preview — a midnight-crossing fixture flake, not an engine
    // bug. Effective-from of today-1 guarantees the entry date always falls
    // inside [effective_from, …] regardless of execution hour.
    sqlx::query(
        "INSERT INTO contracts (id, org_id, client_id, number, name, contract_type, status, currency, effective_from)
         VALUES ($1, $2, $3, 'C-LOCK-1', 'Lock contract', 'time_and_materials', 'active', 'USD', $4)",
    )
    .bind(contract_id)
    .bind(org_id)
    .bind(client_id)
    .bind(today - chrono::Duration::days(1))
    .bind(contract_id)
    .bind(org_id)
    .bind(client_id)
    .bind(today)
    .execute(&state.pool)
    .await
    .unwrap();

    // Billing reads carry approval: the preview consumes ONLY the approved
    // entry, never the draft.
    let from = (today - chrono::Duration::days(30)).to_string();
    let to = (today + chrono::Duration::days(1)).to_string();
    let preview = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/contracts/{contract_id}/billing-preview?from={from}&to={to}"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let labor_minutes: i64 = preview["lines"]
        .as_array()
        .map(|ls| {
            ls.iter()
                .filter(|l| l["kind"] == json!("time_materials"))
                .map(|l| l["quantity_minutes"].as_i64().unwrap_or(0))
                .sum()
        })
        .unwrap_or(0);
    assert_eq!(
        labor_minutes, 60,
        "preview must price only APPROVED time: {preview}"
    );

    // Consume the approved entry with an invoice → the invoicing lock.
    let invoice_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO invoices (id, org_id, client_id, contract_id, number, status, currency, issued_at, due_on, total_minor, created_by)
         VALUES ($1, $2, $3, $4, 'INV-LOCK-1', 'issued', 'USD', $5, $5, 0, $6)",
    )
    .bind(invoice_id)
    .bind(org_id)
    .bind(client_id)
    .bind(contract_id)
    .bind(today)
    .bind(user_id)
    .execute(&state.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO invoice_time_entries (id, org_id, invoice_id, time_entry_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(invoice_id)
    .bind(approved.parse::<Uuid>().unwrap())
    .execute(&state.pool)
    .await
    .unwrap();

    // Every mutation on the invoiced entry refuses with time_entry_locked.
    let (status, body) = submit_entry(&app, &bearer, &approved).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["reason"], json!("time_entry_locked"));
    assert_eq!(body["error"]["entry_id"], json!(approved));
    let (status, body) = decide(
        &app,
        &bearer,
        json!({ "entry_ids": [approved], "decision": "approve" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["results"][0]["outcome"], json!("locked"));
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/time-entries/{approved}/reopen"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(response.status().as_u16(), 409);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], json!("time_entry_locked"));

    // The queue reports the lock flag so approvers see frozen rows.
    let queue = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/time-entries/approvals/queue?status=approved",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let item = queue
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["entry"]["id"] == json!(approved))
        .expect("approved entry in approved queue");
    assert_eq!(item["locked"], json!(true));
}

#[tokio::test]
async fn decision_gating_and_per_approver_history() {
    let (_state, app) = test_state().await;
    let bearer = register_org(&app, &format!("appr-rbac-{}", Uuid::now_v7().simple())).await;
    let ticket = create_ticket(&app, &bearer).await;
    let ticket_id = ticket["id"].as_str().unwrap().to_string();
    let e1 = create_entry(&app, &bearer, &ticket_id, 4).await;
    let (status, body) = submit_entry(&app, &bearer, &e1).await;
    assert_eq!(status, 200, "{body}");

    // Invite a read-only member; a member WITHOUT special:approve_time
    // gets a typed 403 on the decision surface.
    let email = format!("tech-{}@test.local", Uuid::now_v7().simple());
    let invite = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/org/users",
            Some(&bearer),
            Some(json!({
                "email": email,
                "display_name": "Tech",
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
        &app,
        axum::http::Method::POST,
        "/api/org/users/accept-invite",
        None,
        Some(json!({ "token": setup_token, "password": "another-correct-horse" })),
    )
    .await;
    assert_eq!(accept.status().as_u16(), 200, "redemption must succeed");
    let tech_tokens = body_json(accept).await;
    let tech_bearer = tech_tokens["access_token"].as_str().unwrap().to_string();
    let (status, body) = decide(
        &app,
        &tech_bearer,
        json!({ "entry_ids": [e1], "decision": "approve" }),
    )
    .await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["error"]["reason"], json!("forbidden"));

    // Per-approver decision history: the owner decided nothing yet, so
    // decided_by=owner on the queue is empty until a decision lands.
    let owner_id = me(&app, &bearer).await["user_id"]
        .as_str()
        .unwrap()
        .to_string();
    let queue = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/time-entries/approvals/queue?status=submitted&decided_by={owner_id}"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(queue.as_array().unwrap().len(), 0, "{queue}");
}

/// Write-gate symmetry (Wave 54): `submit` refuses someone else's entry, so
/// `decide` refuses your own. A senior tech holds `special:approve_time` and
/// still cannot approve the entry they submitted; the same bulk call applies
/// the peer's entry, and an owner keeps the override.
#[tokio::test]
async fn an_approver_cannot_decide_their_own_entry_but_an_owner_can() {
    let (_state, app) = test_state().await;
    let owner = register_org(&app, &format!("appr-own-{}", Uuid::now_v7().simple())).await;
    let ticket = create_ticket(&app, &owner).await;
    let ticket_id = ticket["id"].as_str().unwrap().to_string();
    let approver = invite_with_role(&app, &owner, "senior_tech").await;
    let peer = invite_with_role(&app, &owner, "technician").await;

    // Both log time against the same ticket and submit it for review.
    let own = create_entry(&app, &approver, &ticket_id, 3).await;
    let theirs = create_entry(&app, &peer, &ticket_id, 4).await;
    let (status, body) = submit_entry(&app, &approver, &own).await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = submit_entry(&app, &peer, &theirs).await;
    assert_eq!(status, 200, "{body}");

    // One bulk call mixes both: the peer's entry applies, the approver's own
    // comes back as the typed own_entry outcome and is never transitioned.
    let (status, body) = decide(
        &app,
        &approver,
        json!({ "entry_ids": [own, theirs], "decision": "approve" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let results = body["results"].as_array().expect("results");
    let row = |id: &str| {
        results
            .iter()
            .find(|r| r["entry_id"] == json!(id))
            .unwrap_or_else(|| panic!("no row for {id}: {body}"))
    };
    assert_eq!(row(&theirs)["outcome"], json!("applied"), "{body}");
    assert_eq!(row(&own)["outcome"], json!("own_entry"), "{body}");
    assert!(
        row(&own)["entry"].is_null(),
        "a refused row carries no snapshot: {body}"
    );

    // The entry is untouched: still submitted, still undecided.
    let queue = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/time-entries/approvals/queue",
            Some(&approver),
            None,
        )
        .await,
    )
    .await;
    let queued = queue
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["entry"]["id"] == json!(own))
        .unwrap_or_else(|| panic!("own entry must stay in the queue: {queue}"));
    assert_eq!(queued["entry"]["approval_status"], json!("submitted"));

    // The owner keeps the override: the same entry decides fine.
    let (status, body) = decide(
        &app,
        &owner,
        json!({ "entry_ids": [own], "decision": "approve" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["results"][0]["outcome"], json!("applied"), "{body}");
    assert_eq!(
        body["results"][0]["entry"]["approval_status"],
        json!("approved")
    );

    // The verification workspace is the other decision write surface; it
    // carries the same predicate. Only owner/admin can reach its approve
    // branch through the shipped presets (it needs `manage_billing` too), so
    // what this proves is the override: the owner decides their own entry
    // there as well, and the disposition lands.
    let own2 = create_entry(&app, &owner, &ticket_id, 8).await;
    let (status, body) = submit_entry(&app, &owner, &own2).await;
    assert_eq!(status, 200, "{body}");
    let applied = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/billing/verification/entries/{own2}/action"),
        Some(&owner),
        Some(json!({
            "action": "approve",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(applied.status().as_u16(), 200, "owner override applies");
    let applied = body_json(applied).await;
    assert_eq!(applied["entry_id"], json!(own2));
    assert_eq!(applied["disposition"], json!("approved"), "{applied}");
}

/// The one-click verdict fix (Wave 41 lane 8): an approver applies a fix to a
/// queued entry, the change lands on the append-only trail, the entry STAYS in
/// the queue, and the guard rails (a raise, a draft, a locked entry) each
/// refuse with their own typed reason.
#[tokio::test]
async fn verdict_fix_applies_to_a_queued_entry_and_stays_in_the_queue() {
    let (_state, app) = test_state().await;
    let bearer = register_org(&app, &format!("fix-{}", Uuid::now_v7().simple())).await;
    let ticket = create_ticket(&app, &bearer).await;
    let ticket_id = ticket["id"].as_str().unwrap().to_string();
    let entry_id = create_entry(&app, &bearer, &ticket_id, 5).await;
    let (status, _) = submit_entry(&app, &bearer, &entry_id).await;
    assert_eq!(status, 200);

    // The description fix writes the text it was handed.
    let (status, body) = fix_entry(
        &app,
        &bearer,
        &entry_id,
        json!({
            "idempotency_key": Uuid::now_v7().to_string(),
            "description": "Replaced the failed drive and confirmed the array rebuilt.",
        }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["notes"],
        json!("Replaced the failed drive and confirmed the array rebuilt.")
    );
    // The point of the whole thing: a fix clears an objection, it does not
    // approve. The entry is still waiting on a person.
    assert_eq!(body["approval_status"], json!("submitted"), "{body}");
    assert_eq!(body["mutation"]["after"]["notes"], body["notes"]);

    // A trim lowers the minutes.
    let (status, body) = fix_entry(
        &app,
        &bearer,
        &entry_id,
        json!({
            "idempotency_key": Uuid::now_v7().to_string(),
            "minutes": 30,
        }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["minutes"], json!(30));

    // An approver tidies a double-claim; raising what a client owes is not
    // theirs to do from a queue, so the write is refused.
    let (status, body) = fix_entry(
        &app,
        &bearer,
        &entry_id,
        json!({
            "idempotency_key": Uuid::now_v7().to_string(),
            "minutes": 240,
        }),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["error"]["reason"], json!("validation_failed"));

    // Trimming to nothing is a rejection, not a fix.
    let (status, body) = fix_entry(
        &app,
        &bearer,
        &entry_id,
        json!({
            "idempotency_key": Uuid::now_v7().to_string(),
            "minutes": 0,
        }),
    )
    .await;
    assert_eq!(status, 422, "{body}");

    // A fix with nothing in it changes nothing, and says so.
    let (status, body) = fix_entry(
        &app,
        &bearer,
        &entry_id,
        json!({ "idempotency_key": Uuid::now_v7().to_string() }),
    )
    .await;
    assert_eq!(status, 422, "{body}");

    // The trail records both fixes, in order, on the same append-only log the
    // decisions use.
    let trail = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/time-entries/{entry_id}/approvals"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let fixed: Vec<&Value> = trail
        .as_array()
        .expect("a trail array")
        .iter()
        .filter(|r| r["action"] == json!("fixed"))
        .collect();
    assert_eq!(fixed.len(), 2, "{trail}");
    assert!(
        fixed[0]["note"]
            .as_str()
            .unwrap_or("")
            .contains("description:"),
        "the trail says what changed: {fixed:?}"
    );
    assert!(
        fixed[1]["note"].as_str().unwrap_or("").contains("60 → 30"),
        "the trail says the minutes it moved: {fixed:?}"
    );

    // A draft is out of scope: the fix only applies to the queue.
    let draft_id = create_entry(&app, &bearer, &ticket_id, 6).await;
    let (status, body) = fix_entry(
        &app,
        &bearer,
        &draft_id,
        json!({
            "idempotency_key": Uuid::now_v7().to_string(),
            "description": "nope",
        }),
    )
    .await;
    assert_eq!(status, 422, "{body}");
}

/// The same fix on an entry an invoice already consumed is refused with the
/// typed lock reason — the lock that protects an issued document from any
/// later edit, fix or otherwise.
#[tokio::test]
async fn verdict_fix_refuses_an_invoice_locked_entry() {
    let (state, app) = test_state().await;
    let bearer = register_org(&app, &format!("fixlock-{}", Uuid::now_v7().simple())).await;
    let ticket = create_ticket(&app, &bearer).await;
    let ticket_id = ticket["id"].as_str().unwrap().to_string();
    let entry_id = create_entry(&app, &bearer, &ticket_id, 5).await;
    let (status, _) = submit_entry(&app, &bearer, &entry_id).await;
    assert_eq!(status, 200);
    let (status, _) = decide(
        &app,
        &bearer,
        json!({ "entry_ids": [entry_id], "decision": "approve" }),
    )
    .await;
    assert_eq!(status, 200);

    // The lock comes from `invoice_time_entries`, so the fixture writes that
    // link directly (as the sibling lock test does) rather than driving a whole
    // invoice run for the sake of one row.
    let me = me(&app, &bearer).await;
    let org_id: Uuid = me["org_id"].as_str().unwrap().parse().unwrap();
    let client_id = Uuid::now_v7();
    let contract_id = Uuid::now_v7();
    let invoice_id = Uuid::now_v7();
    let today = chrono::Utc::now().date_naive();
    sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, 'Fix Co')")
        .bind(client_id)
        .bind(org_id)
        .execute(&state.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO contracts (id, org_id, client_id, number, name, contract_type, status, currency, effective_from)
         VALUES ($1, $2, $3, 'C-FIX-1', 'Fix contract', 'time_and_materials', 'active', 'USD', $4)",
    )
    .bind(contract_id)
    .bind(org_id)
    .bind(client_id)
    .bind(today - chrono::Duration::days(1))
    .execute(&state.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO invoices (id, org_id, client_id, contract_id, number, status, currency,
                               issued_at, due_on, total_minor, total_paid_minor)
         VALUES ($1, $2, $3, $4, 'INV-FIX-1', 'issued', 'USD', $5, $6, 6000, 0)",
    )
    .bind(invoice_id)
    .bind(org_id)
    .bind(client_id)
    .bind(contract_id)
    .bind(today)
    .bind(today + chrono::Duration::days(30))
    .execute(&state.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO invoice_time_entries (id, org_id, invoice_id, time_entry_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(invoice_id)
    .bind(entry_id.parse::<Uuid>().unwrap())
    .execute(&state.pool)
    .await
    .unwrap();

    let (status, body) = fix_entry(
        &app,
        &bearer,
        &entry_id,
        json!({
            "idempotency_key": Uuid::now_v7().to_string(),
            "description": "late edit",
        }),
    )
    .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["reason"], json!("time_entry_locked"));
}
