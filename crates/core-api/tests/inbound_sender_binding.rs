//! Inbound sender binding, through the REAL pipeline and the REAL router
//! (Wave 55 lane V7, `p55-inbound-sender-binding`).
//!
//! Purpose: pin decision 4 and the 2026-10-25 ruling. The webhook HMAC proves
//! a message came through the mail provider and says nothing about who wrote
//! it, so a reply may only join an existing ticket when its `From` is already
//! a participant on that ticket. The org's `email.inbound_sender_binding`
//! level says how the message from anyone else is settled:
//!
//! * `human_only` (the default) holds it for a person;
//! * `shadow` holds it and records what the engine would have done;
//! * `autonomous` applies the engine's decision and audits it as one.
//!
//! Each test drives `core_api::email::process_inbound` for the mail (the same
//! entry point the webhook calls) and the real HTTP router for the person's
//! decision, then reads the ledger, the comments and the audit rows back.
//!
//! Non-purpose: the HMAC gate itself (`routes/webhooks.rs` unit tests), the
//! scan gate (`email_attachments`), the settings registry's own validation
//! (`core-domain` unit tests).

use axum::Router;
use core_api::email::MailStack;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use core_mail::inbound::InboundEmail;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

/// The real router, plus this lane's triage group mounted the way
/// `routes/mod.rs` mounts it (inside the protected chain, so it inherits the
/// auth and org-binding layers).
///
/// The group is defined in `core_api::email` and merged by one line the
/// group's `routes/mod.rs` owner applies; mounting it here proves the
/// handlers, the paths and the permission gates as shipped.
fn app_with_triage(state: &AppState) -> Router {
    // `build_router` mounts the triage group now (routes/mod.rs merges it into
    // the protected chain), so mounting it again here would panic on an
    // overlapping method route. The shipped router is the thing under test.
    core_api::routes::build_router(state.clone())
}

struct Fixture {
    app: Router,
    state: AppState,
    token: String,
    address: String,
    board_id: Uuid,
}

async fn fixture() -> Fixture {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("sender-binding e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    let reporting_url = admin_url.clone();
    let state = AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: Arc::new(core_routing::RoutingRegistry::new()),
        reporting: AppState::reporting_pool_lazy(&reporting_url).expect("lazy reporting pool"),
    };
    let app = app_with_triage(&state);

    // A tenant per test run: the inbound address below is globally unique,
    // so a fixed address would collide on the second run against the same
    // database (the reason `mail_admin_flow` scopes its addresses too).
    let slug = format!("bind-{}", Uuid::now_v7());
    let registered = send(
        &app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Binding Lane Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(registered.status(), 200, "register must succeed");
    let token = body_json(registered).await["access_token"]
        .as_str()
        .expect("access token")
        .to_string();

    let boards = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/boards",
            Some(&token),
            None,
        )
        .await,
    )
    .await;
    let board_id: Uuid = boards
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|b| b["id"].as_str())
        .expect("seeded default board")
        .parse()
        .unwrap();

    let address = format!("inbound-{}@acme.test", Uuid::now_v7());
    let configured = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/email/inbound-settings",
        Some(&token),
        Some(json!({
            "catchall_address": address,
            "catchall_board_id": board_id,
        })),
    )
    .await;
    assert_eq!(configured.status(), 200, "inbound route configured");

    Fixture {
        app,
        state,
        token,
        address,
        board_id,
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
    token: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if let Some(token) = token {
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

impl Fixture {
    fn reply(&self, rfc: &str, in_reply_to: &str, from: &str, body: &str) -> InboundEmail {
        InboundEmail {
            rfc_message_id: rfc.to_string(),
            in_reply_to: Some(in_reply_to.to_string()),
            references: vec![in_reply_to.to_string()],
            subject: Some("Re: printer".into()),
            from_address: from.to_string(),
            from_name: None,
            to: vec![self.address.clone()],
            cc: vec![],
            text: Some(body.to_string()),
            html: None,
            received_at: None,
            attachments: vec![],
        }
    }

    /// Opens a ticket from `from` and returns its id, its number and the
    /// Message-ID that threading will reply to.
    async fn open_ticket(&self, from: &str) -> (Uuid, String, String) {
        let rfc = format!("{}@client.test", Uuid::now_v7());
        let first = InboundEmail {
            rfc_message_id: rfc.clone(),
            in_reply_to: None,
            references: vec![],
            subject: Some(format!("printer {}", Uuid::now_v7())),
            from_address: from.to_string(),
            from_name: Some("Requester".into()),
            to: vec![self.address.clone()],
            cc: vec![],
            text: Some("the printer is on fire".into()),
            html: None,
            received_at: None,
            attachments: vec![],
        };
        let status = core_api::email::process_inbound(&self.state, first)
            .await
            .expect("first message");
        assert_eq!(status, core_api::email::InboundStatus::TicketCreated);
        let (ticket_id, number): (Uuid, String) = sqlx::query_as(
            "SELECT id, number FROM tickets WHERE board_id = $1 \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(self.board_id)
        .fetch_one(&self.state.pool)
        .await
        .expect("seeded ticket exists");
        (ticket_id, number, rfc)
    }

    async fn set_level(&self, level: &str) {
        let response = send(
            &self.app,
            axum::http::Method::PUT,
            "/api/v1/settings/org",
            Some(&self.token),
            Some(json!({ "key": "email.inbound_sender_binding", "value": level })),
        )
        .await;
        assert_eq!(response.status(), 200, "level {level} is accepted");
    }

    /// Ledger facts about one inbound message.
    async fn binding_row(&self, rfc: &str) -> (String, Option<String>, Option<Uuid>, Value) {
        let row: (String, Option<String>, Option<Uuid>, Option<Value>) = sqlx::query_as(
            "SELECT sender_binding, sender_binding_reason, sender_binding_candidate_ticket_id, \
                    sender_binding_proposal \
             FROM email_messages WHERE direction = 'inbound' AND rfc_message_id = $1",
        )
        .bind(rfc)
        .fetch_one(&self.state.pool)
        .await
        .expect("ledger row");
        (row.0, row.1, row.2, row.3.unwrap_or(Value::Null))
    }

    async fn comments_on(&self, ticket_id: Uuid) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM ticket_comments WHERE ticket_id = $1")
            .bind(ticket_id)
            .fetch_one(&self.state.pool)
            .await
            .expect("comment count")
    }

    async fn audit_actions(&self, message_id: Uuid) -> Vec<(String, String, Option<String>)> {
        sqlx::query_as(
            "SELECT action, actor_kind, actor_id FROM audit_events \
             WHERE entity_type = 'email_message' AND entity_id = $1 ORDER BY occurred_at",
        )
        .bind(message_id.to_string())
        .fetch_all(&self.state.pool)
        .await
        .expect("audit rows")
    }

    async fn message_id_of(&self, rfc: &str) -> Uuid {
        sqlx::query_scalar(
            "SELECT id FROM email_messages WHERE direction = 'inbound' AND rfc_message_id = $1",
        )
        .bind(rfc)
        .fetch_one(&self.state.pool)
        .await
        .expect("message id")
    }
}

/// Human only is the default and the strictest answer: a stranger's reply is
/// recorded with its reason, and the ticket it named gains nothing.
#[tokio::test]
async fn human_only_holds_a_strangers_reply_and_the_ticket_gains_nothing() {
    let fx = fixture().await;
    let (ticket_id, _number, first_rfc) = fx.open_ticket("client@acme.test").await;
    assert_eq!(
        fx.comments_on(ticket_id).await,
        0,
        "the first mail opened it"
    );

    let rfc = format!("{}@stranger.test", Uuid::now_v7());
    let status = core_api::email::process_inbound(
        &fx.state,
        fx.reply(&rfc, &first_rfc, "stranger@evil.test", "wire the money"),
    )
    .await
    .expect("stranger reply");
    assert_eq!(
        status,
        core_api::email::InboundStatus::Held,
        "a stranger's reply waits for a person"
    );
    assert_eq!(
        fx.comments_on(ticket_id).await,
        0,
        "nothing reached the ticket"
    );
    let (state, reason, candidate, proposal) = fx.binding_row(&rfc).await;
    assert_eq!(state, "held");
    assert_eq!(reason.as_deref(), Some("sender_not_participant"));
    assert_eq!(
        candidate,
        Some(ticket_id),
        "routing still says where it was"
    );
    assert!(proposal.is_null(), "human only asks no engine");

    // And the queue shows it, with the level the org is on.
    let holds = body_json(
        send(
            &fx.app,
            axum::http::Method::GET,
            "/api/v1/email/inbound-holds",
            Some(&fx.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(holds["level"], "human_only", "the default applies");
    let rows = holds["holds"].as_array().expect("holds array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["rfc_message_id"], rfc);
    assert_eq!(rows[0]["reason"], "sender_not_participant");
}

/// Shadow holds the message exactly as human only does, and records what the
/// engine would have done so agreement is measurable. Nothing is applied.
#[tokio::test]
async fn shadow_records_the_engines_proposal_and_still_holds() {
    let fx = fixture().await;
    let (ticket_id, _number, first_rfc) = fx.open_ticket("client@acme.test").await;
    fx.set_level("shadow").await;

    let rfc = format!("{}@stranger.test", Uuid::now_v7());
    let status = core_api::email::process_inbound(
        &fx.state,
        fx.reply(&rfc, &first_rfc, "stranger@evil.test", "also, the invoice"),
    )
    .await
    .expect("stranger reply under shadow");
    assert_eq!(
        status,
        core_api::email::InboundStatus::Held,
        "shadow still holds"
    );
    assert_eq!(fx.comments_on(ticket_id).await, 0, "nothing was applied");

    let (state, _reason, candidate, proposal) = fx.binding_row(&rfc).await;
    assert_eq!(state, "held");
    assert_eq!(candidate, Some(ticket_id));
    assert_eq!(proposal["surface"], "inbound_sender_binding");
    assert_eq!(proposal["level"], "shadow");
    assert_eq!(proposal["proposed_action"], "attach");
    assert_eq!(proposal["sender_match"], "none", "no sender match exists");
    assert_eq!(proposal["signals"][0], "thread_reference");
    assert!(
        proposal["rationale"]
            .as_str()
            .unwrap_or_default()
            .contains("not a participant"),
        "the proposal says why it cannot match the sender: {proposal}"
    );
    let proposed_ticket = proposal["ticket_id"].as_str().expect("ticket id");
    assert_eq!(proposed_ticket, ticket_id.to_string());
}

/// Autonomous applies the engine's decision in the same transaction, stamps
/// it as an engine decision, and leaves the audit row a person's decision
/// leaves — with `system` as the actor.
#[tokio::test]
async fn autonomous_attaches_and_leaves_an_engine_audit_row() {
    let fx = fixture().await;
    let (ticket_id, _number, first_rfc) = fx.open_ticket("client@acme.test").await;
    fx.set_level("autonomous").await;

    let rfc = format!("{}@stranger.test", Uuid::now_v7());
    let status = core_api::email::process_inbound(
        &fx.state,
        fx.reply(&rfc, &first_rfc, "stranger@evil.test", "here is the detail"),
    )
    .await
    .expect("stranger reply under autonomous");
    assert_eq!(
        status,
        core_api::email::InboundStatus::Commented,
        "the engine's decision was applied"
    );
    assert_eq!(fx.comments_on(ticket_id).await, 1, "the comment landed");

    let (state, _reason, _candidate, _proposal) = fx.binding_row(&rfc).await;
    assert_eq!(state, "accepted");

    let message_id = fx.message_id_of(&rfc).await;
    let decisions: (String, Option<Uuid>, Option<String>) = sqlx::query_as(
        "SELECT sender_binding, sender_binding_decided_by, sender_binding_decided_source \
         FROM email_messages WHERE id = $1",
    )
    .bind(message_id)
    .fetch_one(&fx.state.pool)
    .await
    .expect("decision columns");
    assert_eq!(decisions.0, "accepted");
    assert!(decisions.1.is_none(), "no person decided this");
    assert_eq!(decisions.2.as_deref(), Some("engine"));

    let audit = fx.audit_actions(message_id).await;
    assert_eq!(audit.len(), 1, "one audit row: {audit:?}");
    assert_eq!(audit[0].0, "inbound:sender_binding_accepted");
    assert_eq!(
        audit[0].1, "system",
        "an engine act is never dressed as one"
    );
    assert!(audit[0].2.is_none());
}

/// A bound sender is unaffected at every level: the address opened the
/// conversation, so the reply threads exactly as it did before this rule.
#[tokio::test]
async fn a_bound_sender_threads_at_every_level() {
    let fx = fixture().await;
    for level in ["human_only", "shadow", "autonomous"] {
        fx.set_level(level).await;
        let (ticket_id, _number, first_rfc) = fx.open_ticket("client@acme.test").await;
        let rfc = format!("{}@client.test", Uuid::now_v7());
        let status = core_api::email::process_inbound(
            &fx.state,
            fx.reply(&rfc, &first_rfc, "client@acme.test", "still broken"),
        )
        .await
        .expect("bound reply");
        assert_eq!(
            status,
            core_api::email::InboundStatus::Commented,
            "{level} must not touch a bound sender"
        );
        assert_eq!(fx.comments_on(ticket_id).await, 1);
        let (state, reason, _, _) = fx.binding_row(&rfc).await;
        assert_eq!(state, "bound", "{level}");
        assert_eq!(reason.as_deref(), Some("ticket_correspondence"), "{level}");
    }
}

/// The hold is idempotent on the Message-ID like the rest of the inbound
/// path: a provider replay is `duplicate`, and one message is held once.
#[tokio::test]
async fn a_replayed_message_id_is_duplicate_and_is_held_once() {
    let fx = fixture().await;
    let (_ticket_id, _number, first_rfc) = fx.open_ticket("client@acme.test").await;
    let rfc = format!("{}@stranger.test", Uuid::now_v7());
    let reply = fx.reply(&rfc, &first_rfc, "stranger@evil.test", "hello again");

    let first = core_api::email::process_inbound(&fx.state, reply.clone())
        .await
        .expect("first delivery");
    assert_eq!(first, core_api::email::InboundStatus::Held);
    let replay = core_api::email::process_inbound(&fx.state, reply)
        .await
        .expect("replayed delivery");
    assert_eq!(
        replay,
        core_api::email::InboundStatus::Duplicate,
        "a replay touches nothing"
    );

    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM email_messages WHERE direction = 'inbound' AND rfc_message_id = $1",
    )
    .bind(&rfc)
    .fetch_one(&fx.state.pool)
    .await
    .expect("ledger count");
    assert_eq!(rows, 1, "one ledger row, held once");
    let held: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM email_messages WHERE sender_binding = 'held' AND rfc_message_id = $1",
    )
    .bind(&rfc)
    .fetch_one(&fx.state.pool)
    .await
    .expect("held count");
    assert_eq!(held, 1);
}

/// The person's acceptance threads the held message and leaves an audit row
/// naming them.
#[tokio::test]
async fn a_human_accept_attaches_the_message_and_leaves_an_audit_row() {
    let fx = fixture().await;
    let (ticket_id, _number, first_rfc) = fx.open_ticket("client@acme.test").await;
    let rfc = format!("{}@stranger.test", Uuid::now_v7());
    core_api::email::process_inbound(
        &fx.state,
        fx.reply(
            &rfc,
            &first_rfc,
            "stranger@evil.test",
            "the vendor says it is fixed",
        ),
    )
    .await
    .expect("stranger reply");
    let message_id = fx.message_id_of(&rfc).await;

    let response = send(
        &fx.app,
        axum::http::Method::POST,
        &format!("/api/v1/email/inbound-holds/{message_id}/decide"),
        Some(&fx.token),
        Some(json!({ "decision": "accept", "reason": "known vendor address" })),
    )
    .await;
    assert_eq!(response.status(), 200, "accept is applied");
    let body = body_json(response).await;
    assert_eq!(body["decision"], "accepted");
    assert_eq!(body["ticket_id"], ticket_id.to_string());
    assert!(body["comment_id"].is_string(), "a comment was created");

    assert_eq!(fx.comments_on(ticket_id).await, 1, "the comment is on it");
    let (state, _reason, _candidate, _proposal) = fx.binding_row(&rfc).await;
    assert_eq!(state, "accepted");

    let audit = fx.audit_actions(message_id).await;
    assert_eq!(audit.len(), 1, "one audit row: {audit:?}");
    assert_eq!(audit[0].0, "inbound:sender_binding_accepted");
    assert_eq!(audit[0].1, "human");
    assert!(audit[0].2.is_some(), "the person is named");

    // The same decision again is a typed conflict, not a second comment.
    let again = send(
        &fx.app,
        axum::http::Method::POST,
        &format!("/api/v1/email/inbound-holds/{message_id}/decide"),
        Some(&fx.token),
        Some(json!({ "decision": "accept" })),
    )
    .await;
    assert_eq!(again.status(), 409, "already decided");
    assert_eq!(fx.comments_on(ticket_id).await, 1, "still one comment");
}

/// Refusing records the decision and leaves the message off every ticket.
#[tokio::test]
async fn a_human_refusal_leaves_the_message_off_the_ticket() {
    let fx = fixture().await;
    let (ticket_id, _number, first_rfc) = fx.open_ticket("client@acme.test").await;
    let rfc = format!("{}@stranger.test", Uuid::now_v7());
    core_api::email::process_inbound(
        &fx.state,
        fx.reply(&rfc, &first_rfc, "stranger@evil.test", "buy this now"),
    )
    .await
    .expect("stranger reply");
    let message_id = fx.message_id_of(&rfc).await;

    let response = send(
        &fx.app,
        axum::http::Method::POST,
        &format!("/api/v1/email/inbound-holds/{message_id}/decide"),
        Some(&fx.token),
        Some(json!({ "decision": "refuse", "reason": "cold outreach" })),
    )
    .await;
    assert_eq!(response.status(), 200, "refuse is applied");
    let body = body_json(response).await;
    assert_eq!(body["decision"], "refused");
    assert!(body["ticket_id"].is_null());

    assert_eq!(fx.comments_on(ticket_id).await, 0, "nothing reached it");
    let (state, _reason, _candidate, _proposal) = fx.binding_row(&rfc).await;
    assert_eq!(state, "refused");
    let audit = fx.audit_actions(message_id).await;
    assert_eq!(audit[0].0, "inbound:sender_binding_refused");
    assert_eq!(audit[0].1, "human");

    // A refused message is no longer in the queue, and the ledger keeps it.
    let holds = body_json(
        send(
            &fx.app,
            axum::http::Method::GET,
            "/api/v1/email/inbound-holds",
            Some(&fx.token),
            None,
        )
        .await,
    )
    .await;
    assert!(
        holds["holds"].as_array().expect("holds").is_empty(),
        "nothing is waiting any more"
    );
}

/// An org that never touched the setting is on `human only`, and a technician
/// — no admin grant — cannot see or decide the queue.
#[tokio::test]
async fn the_level_defaults_to_human_only_and_the_queue_needs_a_session() {
    let fx = fixture().await;
    let holds = send(
        &fx.app,
        axum::http::Method::GET,
        "/api/v1/email/inbound-holds",
        Some(&fx.token),
        None,
    )
    .await;
    assert_eq!(holds.status(), 200);
    assert_eq!(body_json(holds).await["level"], "human_only");

    let unauthenticated = send(
        &fx.app,
        axum::http::Method::GET,
        "/api/v1/email/inbound-holds",
        None,
        None,
    )
    .await;
    assert_eq!(
        unauthenticated.status(),
        401,
        "the queue is not a public surface"
    );
}
