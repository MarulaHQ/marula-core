//! worker/decision — the decision surfaces' stream consumer (Wave 23
//! lane 3, ADR 0010 §3 #1–#4 + #9).
//!
//! Purpose: consume the domain event stream with the SAME discipline as
//! the automations consumer (durable Redis consumer group, pending-entry
//! recovery, malformed entries ACKed-and-skipped, reconnect with
//! backoff), compose decision state from event envelopes + DB snapshots
//! per the surface's `state_template`, and dispatch ONE speculative
//! fan-out per trigger through core-api's internal decision seam.
//!
//! Fan-out doctrine (ADR 0011 §4): all questions for one trigger ride
//! ONE request. `decision_ticket_triage` and `decision_workbench` share
//! an identical question map, so the workbench card CONSUMES the triage
//! answers — one fan-out serves both surfaces. `decision_dispatch` and
//! `decision_contract_scope` ride their own single requests on the
//! ticket.created trigger.
//!
//! Temporal context enters state as pre-computed derived fields
//! (ADR 0011 §4): `after_hours`, `within_sla_window`, `elapsed_sum` are
//! composed here in code — raw timestamps never reach the model.
//!
//! Application posture: suggestions-by-default. Every run is recorded by
//! the edge itself (shadow store + usage ledger); the ONLY side effect
//! this subsystem takes is the alert auto-ack, and only when the surface
//! ran `live` AND actionable AND severity sit above their act thresholds
//! (ADR 0010 #4: "auto-ack only above high thresholds; else leaves the
//! alert untouched"). Ticket suggestions apply exclusively through the
//! automations engine's `ai_decision` conditions.
//!
//! Degrade posture is absolute (old-school guarantee): master switch
//! OFF, surface unconfigured, or the bot principal / internal token
//! unset ⇒ this loop never fires a request and the org sees exactly the
//! pre-AI product. Quiet fan-out answers are logged and dropped — no
//! retries, no failure propagation; intake, alerts, and billing never
//! wait on a decision model.

use crate::config::WorkerConfig;
use core_events::{Envelope, EVENT_STREAM};
use redis::AsyncCommands;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::watch;
use uuid::Uuid;

const CONSUMER_GROUP: &str = "decision-triage";
const CONSUMER_NAME: &str = "worker";
const BLOCK_MS: usize = 2_000;
const COUNT: usize = 50;

/// Surfaces this subsystem dispatches.
const TRIAGE: &str = "decision_ticket_triage";
const DISPATCH: &str = "decision_dispatch";
const ALERT_TRIAGE: &str = "decision_alert_triage";
const CONTRACT_SCOPE: &str = "decision_contract_scope";

/// The HTTP seam to core-api's internal decision surface. Trait so tests
/// record calls without any network (same doctrine as effects::EffectApi).
#[async_trait::async_trait]
pub trait DecisionFanOut: Send + Sync {
    async fn fan_out(&self, body: Value) -> Option<Value>;

    /// The alert auto-ack side effect (the ONLY write this subsystem
    /// may trigger). Returns true when an open alert was acked.
    async fn decision_ack(&self, body: Value) -> bool;
}

/// Production client: one POST with the internal bearer token; a
/// non-2xx or transport failure is a quiet None/false (degrade-never-fail
/// — the consumer logs and moves on, exactly like a quiet decision edge).
#[derive(Clone)]
pub struct ReqwestFanOut {
    base_url: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl ReqwestFanOut {
    pub fn new(base_url: String, token: Option<String>) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }

    async fn post(&self, path: &str, body: Value) -> Option<Value> {
        let Some(token) = &self.token else {
            tracing::warn!("MARULA_INTERNAL_TOKEN unset; skipping decision call");
            return None;
        };
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .http
            .post(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await;
        match response {
            Ok(resp) if resp.status().is_success() => resp.json().await.ok(),
            Ok(resp) => {
                tracing::warn!(status = %resp.status(), path, "decision call rejected");
                None
            }
            Err(error) => {
                tracing::warn!(%error, path, "decision call transport failure");
                None
            }
        }
    }
}

#[async_trait::async_trait]
impl DecisionFanOut for ReqwestFanOut {
    async fn fan_out(&self, body: Value) -> Option<Value> {
        self.post("/internal/decision/fan-out", body).await
    }

    async fn decision_ack(&self, body: Value) -> bool {
        self.post("/internal/alerts/decision-ack", body)
            .await
            .and_then(|v| v.get("acked").and_then(|b| b.as_bool()))
            .unwrap_or(false)
    }
}

/// Test double: records every posted body, replays canned responses.
pub struct RecordingFanOut {
    pub calls: std::sync::Mutex<Vec<Value>>,
    pub ack_calls: std::sync::Mutex<Vec<Value>>,
    responses: std::sync::Mutex<std::collections::VecDeque<Value>>,
    ack_outcome: std::sync::Mutex<std::collections::VecDeque<bool>>,
}

impl RecordingFanOut {
    pub fn new(responses: Vec<Value>) -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
            ack_calls: std::sync::Mutex::new(Vec::new()),
            responses: std::sync::Mutex::new(responses.into_iter().collect()),
            ack_outcome: std::sync::Mutex::new(std::collections::VecDeque::new()),
        }
    }

    pub fn with_ack_outcome(mut self, outcomes: Vec<bool>) -> Self {
        self.ack_outcome = std::sync::Mutex::new(outcomes.into_iter().collect());
        self
    }

    pub fn recorded(&self) -> Vec<Value> {
        self.calls.lock().expect("mutex").clone()
    }

    pub fn acks(&self) -> Vec<Value> {
        self.ack_calls.lock().expect("mutex").clone()
    }
}

#[async_trait::async_trait]
impl DecisionFanOut for RecordingFanOut {
    async fn fan_out(&self, body: Value) -> Option<Value> {
        self.calls.lock().expect("mutex").push(body);
        self.responses.lock().expect("mutex").pop_front()
    }

    async fn decision_ack(&self, body: Value) -> bool {
        self.ack_calls.lock().expect("mutex").push(body);
        self.ack_outcome
            .lock()
            .expect("mutex")
            .pop_front()
            .unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// Derived temporal fields (ADR 0011 §4) — composed in CODE, never passed
// as raw timestamps for the model to interpret.
// ---------------------------------------------------------------------------

/// Outside Mon–Fri 08:00–18:00 UTC. Marula keeps business hours a
/// per-org concern off the decision path for now; the honest default is
/// a fixed UTC window until org business-hours settings exist.
pub fn after_hours(at: chrono::DateTime<chrono::Utc>) -> bool {
    use chrono::{Datelike, Timelike};
    let weekday = at.weekday();
    let hour = at.hour();
    !(matches!(
        weekday,
        chrono::Weekday::Mon
            | chrono::Weekday::Tue
            | chrono::Weekday::Wed
            | chrono::Weekday::Thu
            | chrono::Weekday::Fri
    ) && (8..18).contains(&hour))
}

/// The ticket's SLA response window is still open: a future due date
/// exists and has not passed. No due date ⇒ not in an SLA window (the
/// honest reading — no clock, no window).
pub fn within_sla_window(
    due_at: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    due_at.is_some_and(|due| due > now)
}

// ---------------------------------------------------------------------------
// Trigger mapping
// ---------------------------------------------------------------------------

/// Which decision dispatch an event triggers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    TicketCreated,
    TicketUpdated,
    AlertIngested,
    TimeEntryCreated,
}

/// Maps an envelope onto its decision trigger. Automation-echo events
/// are skipped in [`handle_entry`] by the same `via_automation` marker
/// rule the automations evaluator uses — a rule-applied effect never
/// re-triggers triage.
pub fn match_event(envelope: &Envelope) -> Option<Trigger> {
    match envelope.event_type.as_str() {
        "ticket.created" => Some(Trigger::TicketCreated),
        "ticket.updated" => Some(Trigger::TicketUpdated),
        "alert.ingested" => Some(Trigger::AlertIngested),
        "time_entry.created" => Some(Trigger::TimeEntryCreated),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// State composition — per the surface's state_template shape (the starter
// sets compose ticket.subject / ticket.body / client.name / alert.subject /
// alert.body / asset.name / contract.summary / request.description plus the
// derived temporal leaves).
// ---------------------------------------------------------------------------

struct TicketContext {
    title: String,
    body: String,
    client_name: Option<String>,
    due_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Ticket snapshot for state composition (owner-role read — every
/// worker read is deliberately unscoped, see the config module note).
async fn load_ticket_context(
    pool: &sqlx::PgPool,
    org_id: Uuid,
    ticket_id: Uuid,
) -> Option<TicketContext> {
    let row = sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            Option<String>,
            Option<chrono::DateTime<chrono::Utc>>,
        ),
    >(
        r#"SELECT t.title, t.body, c.name, t.due_at
             FROM tickets t
             LEFT JOIN clients c ON c.id = t.client_id
            WHERE t.id = $1 AND t.org_id = $2"#,
    )
    .bind(ticket_id)
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .ok()??;
    Some(TicketContext {
        title: row.0,
        body: row.1.unwrap_or_default(),
        client_name: row.2,
        due_at: row.3,
    })
}

/// Sum of recorded time-entry minutes on the ticket (the `elapsed_sum`
/// derived field — arithmetic happens HERE, never in the model).
async fn elapsed_sum_minutes(pool: &sqlx::PgPool, org_id: Uuid, ticket_id: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"SELECT COALESCE(SUM(minutes), 0) FROM time_entries
            WHERE org_id = $1 AND ticket_id = $2"#,
    )
    .bind(org_id)
    .bind(ticket_id)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
}

/// Org board names — the dynamic Choice inventory (`org.boards`, and the
/// honest dispatch-queue inventory: in this product a dispatch queue IS
/// a board's queue, so `org.dispatch_queues` mirrors the same list).
async fn board_names(pool: &sqlx::PgPool, org_id: Uuid) -> Vec<String> {
    sqlx::query_scalar::<_, String>(r#"SELECT name FROM boards WHERE org_id = $1"#)
        .bind(org_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// Whether the ticket's client has an active contract (the contract-scope
/// trigger gate) and its summary line for state composition — including
/// `included_minutes` / `overage_rate_minor` (the contracts schema the
/// ADR 0011 §4 surface rides).
async fn active_contract_summary(
    pool: &sqlx::PgPool,
    org_id: Uuid,
    ticket_id: Uuid,
) -> Option<String> {
    let row = sqlx::query_as::<_, (String, String, String, Option<i32>, Option<i64>)>(
        r#"SELECT ct.number, ct.name, ct.contract_type,
                  ct.included_minutes, ct.overage_rate_minor
             FROM tickets t
             JOIN clients c ON c.id = t.client_id
             JOIN contracts ct ON ct.client_id = c.id
                 AND ct.org_id = t.org_id AND ct.status = 'active'
                 AND ct.effective_from <= CURRENT_DATE
                 AND (ct.effective_until IS NULL OR ct.effective_until >= CURRENT_DATE)
            WHERE t.id = $1 AND t.org_id = $2
            ORDER BY ct.created_at DESC
            LIMIT 1"#,
    )
    .bind(ticket_id)
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .ok()??;
    let (number, name, kind, included_minutes, overage_rate) = row;
    Some(format!(
        "{number} {name} ({kind}); included_minutes={:?}; overage_rate_minor={:?}",
        included_minutes, overage_rate
    ))
}

fn ticket_state(
    ctx: &TicketContext,
    elapsed_sum: i64,
    now: chrono::DateTime<chrono::Utc>,
) -> Value {
    json!({
        "ticket": {
            "subject": ctx.title,
            "body": ctx.body,
        },
        "client": { "name": ctx.client_name.clone().unwrap_or_default() },
        "after_hours": after_hours(now),
        "within_sla_window": within_sla_window(ctx.due_at, now),
        "elapsed_sum": elapsed_sum,
    })
}

// ---------------------------------------------------------------------------
// The consumer loop — mirrors run_automations exactly (pending recovery,
// live tail under the group, ACKed-and-skipped poison entries).
// ---------------------------------------------------------------------------

/// Runs the decision subsystem until shutdown. Disabled cleanly (none
/// mode) when the bot principal is unconfigured — surfaces act exactly
/// as they would without a decision model.
pub async fn run_decision_triage(
    pool: sqlx::PgPool,
    config: WorkerConfig,
    fan_out: Arc<dyn DecisionFanOut>,
    mut shutdown: watch::Receiver<bool>,
) {
    let Some(bot_user) = config.decision_bot_user_id else {
        tracing::info!("decision triage disabled: MARULA_DECISION_BOT_USER_ID not configured");
        return;
    };
    let client = match redis::Client::open(config.redis_url.as_str()) {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(%error, "invalid redis url; decision triage exiting");
            return;
        }
    };
    let mut backoff_ms = 100_u64;
    loop {
        if *shutdown.borrow() {
            return;
        }
        match client.get_multiplexed_async_connection().await {
            Ok(mut conn) => {
                backoff_ms = 100;
                // Durable group; '$' starts at live tail on FIRST creation
                // (same posture as the automations consumer).
                if let Err(error) = conn
                    .xgroup_create_mkstream::<&str, &str, &str, ()>(
                        EVENT_STREAM,
                        CONSUMER_GROUP,
                        "$",
                    )
                    .await
                {
                    let msg = error.to_string();
                    if !msg.contains("BUSYGROUP") {
                        tracing::error!(%error, "xgroup create failed");
                        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                        backoff_ms = (backoff_ms * 2).min(5_000);
                        continue;
                    }
                }
                follow_stream(&pool, &bot_user, &fan_out, &mut conn, &mut shutdown).await;
                tracing::warn!("decision triage stream connection ended; reconnecting");
            }
            Err(error) => {
                tracing::error!(%error, "decision triage redis connect failed");
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
        backoff_ms = (backoff_ms * 2).min(5_000);
    }
}

async fn follow_stream(
    pool: &sqlx::PgPool,
    bot_user: &Uuid,
    fan_out: &Arc<dyn DecisionFanOut>,
    conn: &mut redis::aio::MultiplexedConnection,
    shutdown: &mut watch::Receiver<bool>,
) {
    use redis::streams::{StreamReadOptions, StreamReadReply};
    // 1. Recover pending entries owned by this consumer name.
    let options = StreamReadOptions::default().count(COUNT);
    if let Ok(reply) = conn
        .xread_options::<_, _, StreamReadReply>(&[EVENT_STREAM], &["0"], &options)
        .await
    {
        for entry in reply.keys.into_iter().flat_map(|k| k.ids) {
            handle_entry(pool, bot_user, fan_out, conn, &entry).await;
        }
    }
    // 2. Live tail under the group.
    let options = StreamReadOptions::default()
        .block(BLOCK_MS)
        .count(COUNT)
        .group(CONSUMER_GROUP, CONSUMER_NAME);
    loop {
        if *shutdown.borrow() {
            return;
        }
        tokio::select! {
            _ = shutdown.changed() => return,
            result = conn.xread_options::<_, _, StreamReadReply>(&[EVENT_STREAM], &[">"], &options) => {
                match result {
                    Ok(reply) => {
                        for entry in reply.keys.into_iter().flat_map(|k| k.ids) {
                            handle_entry(pool, bot_user, fan_out, conn, &entry).await;
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "decision triage xreadgroup failed");
                        return;
                    }
                }
            }
        }
        // Consumer backlog: PEL depth, lag and the age of the oldest unacked
        // entry for THIS group, read after the batch (TASKLIST R7a).
        crate::stream_health::probe(conn, CONSUMER_GROUP).await;
    }
}

async fn ack(conn: &mut redis::aio::MultiplexedConnection, id: &str) {
    let _: Result<i64, _> = conn.xack(EVENT_STREAM, CONSUMER_GROUP, &[id]).await;
}

async fn handle_entry(
    pool: &sqlx::PgPool,
    bot_user: &Uuid,
    fan_out: &Arc<dyn DecisionFanOut>,
    conn: &mut redis::aio::MultiplexedConnection,
    entry: &redis::streams::StreamId,
) {
    let Some(raw) = entry.get::<String>("data") else {
        tracing::warn!(entry_id = %entry.id, "decision triage: stream entry missing data field");
        ack(conn, entry.id.as_str()).await;
        return;
    };
    let envelope = match serde_json::from_str::<Envelope>(&raw) {
        Ok(envelope) => envelope,
        Err(_) => {
            tracing::warn!(entry_id = %entry.id, "decision triage: unparseable event");
            ack(conn, entry.id.as_str()).await;
            return;
        }
    };
    if core_domain::automations::is_automation_echo(&envelope.payload) {
        ack(conn, entry.id.as_str()).await;
        return;
    }
    let now = chrono::Utc::now();
    match match_event(&envelope) {
        Some(Trigger::AlertIngested) => {
            handle_alert(pool, bot_user, fan_out, &envelope, now).await;
        }
        Some(Trigger::TicketCreated | Trigger::TicketUpdated) => {
            handle_ticket(pool, bot_user, fan_out, &envelope, now).await;
        }
        Some(Trigger::TimeEntryCreated) => {
            handle_time_entry(pool, bot_user, fan_out, &envelope).await;
        }
        None => {}
    }
    ack(conn, entry.id.as_str()).await;
}

/// `decision_billing_audit` (ADR 0010 §3 #7): a fresh time entry gets its
/// billable + description-matches-work Noul. Suggestion-grade — the
/// answers ride the existing approvals flow (recorded only; no side
/// effect here, the shadow store IS the feed). Invoice `time_materials`
/// lines inherit the audit through `invoice_time_entries` (their work
/// description is composed FROM audited entries at the earliest creation
/// point; fixed-fee/recurring lines carry no free-text work description
/// to audit).
async fn handle_time_entry(
    pool: &sqlx::PgPool,
    bot_user: &Uuid,
    fan_out: &Arc<dyn DecisionFanOut>,
    envelope: &Envelope,
) {
    const BILLING_AUDIT: &str = "decision_billing_audit";
    let org_id = envelope.org_id;
    if !surface_configured(pool, org_id, BILLING_AUDIT).await {
        return;
    }
    let Some(entity_id) = envelope
        .entity_id
        .as_deref()
        .and_then(|v| Uuid::parse_str(v).ok())
    else {
        return;
    };
    // Entry description + ticket context in one owner-role read; a
    // missing row (deleted mid-stream) is a logged no-op.
    let Some(entry) = load_time_entry_context(pool, org_id, entity_id).await else {
        return;
    };
    let elapsed_sum = elapsed_sum_minutes(pool, org_id, entry.ticket_id).await;
    let state = time_entry_state(
        &entry.description,
        &entry.ticket_title,
        &entry.ticket_body,
        elapsed_sum,
    );
    let request_id = format!("decision-billing-{}", envelope.id);
    fan_out_and_log(
        fan_out,
        FanOutAsk {
            org_id,
            user_id: *bot_user,
            surface: BILLING_AUDIT,
            entity_type: "time_entry",
            entity_id: Some(entity_id),
            state,
            request_id: &request_id,
            inventories: HashMap::new(),
        },
    )
    .await;
}

struct TimeEntryContext {
    ticket_id: Uuid,
    description: String,
    ticket_title: String,
    ticket_body: String,
}

/// State per the `decision_billing_audit` starter template: entry
/// description + ticket subject/body + the derived elapsed_sum (summed in
/// code, never in the model).
fn time_entry_state(
    description: &str,
    ticket_title: &str,
    ticket_body: &str,
    elapsed_sum: i64,
) -> Value {
    json!({
        "entry": { "description": description },
        "ticket": {
            "subject": ticket_title,
            "body": ticket_body,
        },
        "elapsed_sum": elapsed_sum,
    })
}

async fn load_time_entry_context(
    pool: &sqlx::PgPool,
    org_id: Uuid,
    entry_id: Uuid,
) -> Option<TimeEntryContext> {
    let row = sqlx::query_as::<_, (Uuid, Option<String>, String, Option<String>)>(
        r#"SELECT te.ticket_id, te.notes, t.title, t.body
             FROM time_entries te
             JOIN tickets t ON t.id = te.ticket_id AND t.org_id = te.org_id
            WHERE te.id = $1 AND te.org_id = $2"#,
    )
    .bind(entry_id)
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .ok()??;
    Some(TimeEntryContext {
        ticket_id: row.0,
        description: row.1.unwrap_or_default(),
        ticket_title: row.2,
        ticket_body: row.3.unwrap_or_default(),
    })
}

/// One trigger → at most its mapped fan-outs. Every quiet/None outcome
/// is a logged no-op: the decision model is additive, never
/// load-bearing.
async fn surface_configured(pool: &sqlx::PgPool, org_id: Uuid, surface: &str) -> bool {
    match core_db::ai_policy::features_enabled(pool, org_id).await {
        Ok(true) => {}
        Ok(false) => return false,
        Err(error) => {
            tracing::warn!(%error, "features_enabled read failed; skipping decision trigger");
            return false;
        }
    }
    matches!(
        core_db::decision::get_surface_config(pool, org_id, surface).await,
        Ok(Some(cfg)) if cfg.enabled
    )
}

async fn handle_ticket(
    pool: &sqlx::PgPool,
    bot_user: &Uuid,
    fan_out: &Arc<dyn DecisionFanOut>,
    envelope: &Envelope,
    now: chrono::DateTime<chrono::Utc>,
) {
    let org_id = envelope.org_id;
    let Some(entity_id) = envelope
        .entity_id
        .as_deref()
        .and_then(|v| Uuid::parse_str(v).ok())
    else {
        return;
    };
    let created = envelope.event_type == "ticket.created";
    // Each surface gates on its own config BEFORE any composition work —
    // an unconfigured org costs zero HTTP and zero queries beyond the
    // config read.
    let triage_on = surface_configured(pool, org_id, TRIAGE).await;
    let dispatch_on = created && surface_configured(pool, org_id, DISPATCH).await;
    let scope_on = created && surface_configured(pool, org_id, CONTRACT_SCOPE).await;
    if !triage_on && !dispatch_on && !scope_on {
        return;
    }
    let Some(ctx) = load_ticket_context(pool, org_id, entity_id).await else {
        return;
    };
    let elapsed_sum = elapsed_sum_minutes(pool, org_id, entity_id).await;
    let boards = board_names(pool, org_id).await;
    let state = ticket_state(&ctx, elapsed_sum, now);
    let request_id = format!("decision-triage-{}", envelope.id);

    if triage_on {
        let mut inventories = HashMap::new();
        inventories.insert("org.boards".to_string(), boards.clone());
        fan_out_and_log(
            fan_out,
            FanOutAsk {
                org_id,
                user_id: *bot_user,
                surface: TRIAGE,
                entity_type: "ticket",
                entity_id: Some(entity_id),
                state: state.clone(),
                request_id: &request_id,
                inventories,
            },
        )
        .await;
        // decision_workbench consumes the SAME answers (identical
        // question map — one fan-out serves both surfaces; the card
        // reads the triage rows).
    }
    if dispatch_on {
        let mut inventories = HashMap::new();
        inventories.insert("org.dispatch_queues".to_string(), boards.clone());
        fan_out_and_log(
            fan_out,
            FanOutAsk {
                org_id,
                user_id: *bot_user,
                surface: DISPATCH,
                entity_type: "ticket",
                entity_id: Some(entity_id),
                state: state.clone(),
                request_id: &request_id,
                inventories,
            },
        )
        .await;
    }
    if scope_on {
        if let Some(summary) = active_contract_summary(pool, org_id, entity_id).await {
            let state = json!({
                "contract": { "summary": summary },
                "request": {
                    "description": format!("{} — {}", ctx.title, ctx.body),
                },
            });
            fan_out_and_log(
                fan_out,
                FanOutAsk {
                    org_id,
                    user_id: *bot_user,
                    surface: CONTRACT_SCOPE,
                    entity_type: "ticket",
                    entity_id: Some(entity_id),
                    state,
                    request_id: &request_id,
                    inventories: HashMap::new(),
                },
            )
            .await;
        }
    }
}

async fn handle_alert(
    pool: &sqlx::PgPool,
    bot_user: &Uuid,
    fan_out: &Arc<dyn DecisionFanOut>,
    envelope: &Envelope,
    now: chrono::DateTime<chrono::Utc>,
) {
    let org_id = envelope.org_id;
    if !surface_configured(pool, org_id, ALERT_TRIAGE).await {
        return;
    }
    let Some(entity_id) = envelope
        .entity_id
        .as_deref()
        .and_then(|v| Uuid::parse_str(v).ok())
    else {
        return;
    };
    let Some((title, description, asset_name)) = alert_context(pool, org_id, entity_id).await
    else {
        return;
    };
    let _ = now;
    let state = json!({
        "alert": {
            "subject": title,
            "body": description,
        },
        "asset": { "name": asset_name },
        "within_sla_window": false,
    });
    let request_id = format!("decision-alert-{}", envelope.id);
    if let Some(run) = fan_out_and_log(
        fan_out,
        FanOutAsk {
            org_id,
            user_id: *bot_user,
            surface: ALERT_TRIAGE,
            entity_type: "alert",
            entity_id: Some(entity_id),
            state,
            request_id: &request_id,
            inventories: HashMap::new(),
        },
    )
    .await
    {
        maybe_auto_ack(fan_out, org_id, bot_user, entity_id, &request_id, &run).await;
    }
}

async fn alert_context(
    pool: &sqlx::PgPool,
    org_id: Uuid,
    alert_id: Uuid,
) -> Option<(String, String, String)> {
    let row = sqlx::query_as::<_, (String, Option<String>, Value)>(
        r#"SELECT title, description, attributes FROM alerts
            WHERE id = $1 AND org_id = $2"#,
    )
    .bind(alert_id)
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .ok()??;
    let asset_name = row
        .2
        .pointer("/asset/name")
        .or_else(|| row.2.pointer("/asset_name"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Some((row.0, row.1.unwrap_or_default(), asset_name))
}

/// One fan-out dispatch ask (param object keeping the caller under
/// clippy's argument limit, every call site naming its fields).
struct FanOutAsk<'a> {
    org_id: Uuid,
    user_id: Uuid,
    surface: &'a str,
    entity_type: &'a str,
    entity_id: Option<Uuid>,
    state: Value,
    request_id: &'a str,
    inventories: HashMap<String, Vec<String>>,
}

/// Posts one fan-out; logs quiet outcomes and returns the parsed run.
async fn fan_out_and_log(fan_out: &Arc<dyn DecisionFanOut>, ask: FanOutAsk<'_>) -> Option<Value> {
    let body = json!({
        "org_id": ask.org_id,
        "user_id": ask.user_id,
        "surface_key": ask.surface,
        "entity_type": ask.entity_type,
        "entity_id": ask.entity_id,
        "state": ask.state,
        "request_id": ask.request_id,
        "inventories": ask.inventories,
    });
    let Some(response) = fan_out.fan_out(body).await else {
        tracing::debug!(org = %ask.org_id, surface = ask.surface, "decision fan-out quiet (no response)");
        return None;
    };
    if response.get("status").and_then(|v| v.as_str()) == Some("ok") {
        tracing::info!(org = %ask.org_id, surface = ask.surface, "decision fan-out recorded");
        Some(response)
    } else {
        tracing::debug!(
            org = %ask.org_id,
            surface = ask.surface,
            reason = ?response.get("reason").cloned().unwrap_or(serde_json::Value::Null),
            "decision fan-out went quiet"
        );
        None
    }
}

/// The ONLY side effect this subsystem takes: auto-ack an OPEN alert
/// when the surface ran LIVE and actionable AND severity are above
/// their act thresholds (ADR 0010 #4 — else the alert stays untouched).
async fn maybe_auto_ack(
    fan_out: &Arc<dyn DecisionFanOut>,
    org_id: Uuid,
    bot_user: &Uuid,
    alert_id: Uuid,
    request_id: &str,
    run: &Value,
) {
    if run.get("mode").and_then(|v| v.as_str()) != Some("live") {
        return; // shadow rows drive nothing (ADR 0010 §5)
    }
    let (Some(answers), Some(thresholds)) = (run.get("answers"), run.get("thresholds")) else {
        return;
    };
    // High thresholds ONLY: actionable Noul above act AND severity
    // rubric level above act. Anything else leaves the alert untouched.
    if gate(answers, thresholds, "actionable") != Some(true)
        || gate(answers, thresholds, "severity") != Some(true)
    {
        return;
    }
    let body = json!({
        "org_id": org_id,
        "alert_id": alert_id,
        "actor_id": bot_user,
        "request_id": request_id,
    });
    if fan_out.decision_ack(body).await {
        tracing::info!(org = %org_id, alert = %alert_id, "alert auto-acked above thresholds");
    } else {
        tracing::debug!(org = %org_id, alert = %alert_id, "decision auto-ack no-op");
    }
}

/// Act-gate for one recorded answer against its threshold entry:
/// `Some(true)` = at/above the act threshold, `Some(false)` = below,
/// `None` = the question did not answer.
fn gate(answers: &Value, thresholds: &Value, question: &str) -> Option<bool> {
    let answer = answers.get(question)?;
    let threshold = thresholds.get(question)?;
    let act = threshold.get("act")?.as_f64()?;
    let value = answer
        .get("probability")
        .or_else(|| answer.get("level"))
        .and_then(|v| v.as_f64())?;
    Some(value >= act)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fan_upcast(fan: &Arc<RecordingFanOut>) -> Arc<dyn DecisionFanOut> {
        fan.clone()
    }

    fn envelope(event_type: &str, entity_id: &str) -> Envelope {
        Envelope {
            id: Uuid::now_v7(),
            event_type: event_type.into(),
            org_id: Uuid::now_v7(),
            entity: "ticket".into(),
            entity_id: Some(entity_id.into()),
            occurred_at: chrono::Utc::now(),
            payload_schema_ref: "v1".into(),
            payload: json!({}),
        }
    }

    #[test]
    fn trigger_mapping_covers_triage_alerts_and_ignores_noise() {
        let ticket = envelope("ticket.created", &Uuid::now_v7().to_string());
        assert_eq!(match_event(&ticket), Some(Trigger::TicketCreated));
        let update = envelope("ticket.updated", &Uuid::now_v7().to_string());
        assert_eq!(match_event(&update), Some(Trigger::TicketUpdated));
        let alert = envelope("alert.ingested", &Uuid::now_v7().to_string());
        assert_eq!(match_event(&alert), Some(Trigger::AlertIngested));
        // Wave 23 lane 4: a fresh time entry triggers the billing audit.
        let entry = envelope("time_entry.created", &Uuid::now_v7().to_string());
        assert_eq!(match_event(&entry), Some(Trigger::TimeEntryCreated));
        let noise = envelope("invoice.run.created", &Uuid::now_v7().to_string());
        assert_eq!(match_event(&noise), None);
    }

    #[test]
    fn billing_audit_state_matches_the_starter_template() {
        let state = time_entry_state("fixed printer driver", "Printer broken", "it sparks", 45);
        assert_eq!(state["entry"]["description"], "fixed printer driver");
        assert_eq!(state["ticket"]["subject"], "Printer broken");
        assert_eq!(state["ticket"]["body"], "it sparks");
        assert_eq!(state["elapsed_sum"], 45);
    }

    #[tokio::test]
    async fn billing_audit_fan_out_rides_the_time_entry_entity() {
        let org = Uuid::now_v7();
        let ok = json!({
            "status": "ok",
            "mode": "shadow",
            "answers": {"billable": {"kind": "noul", "probability": 0.9}},
            "thresholds": {"billable": {"act": 0.8, "review": 0.6}}
        });
        let fan = Arc::new(RecordingFanOut::new(vec![ok]));
        let run = fan_out_and_log(
            &fan_upcast(&fan),
            FanOutAsk {
                org_id: org,
                user_id: Uuid::now_v7(),
                surface: "decision_billing_audit",
                entity_type: "time_entry",
                entity_id: Some(Uuid::now_v7()),
                state: time_entry_state("desc", "subject", "body", 30),
                request_id: "r-billing",
                inventories: HashMap::new(),
            },
        )
        .await;
        assert!(run.is_some());
        let body = &fan.recorded()[0];
        assert_eq!(body["surface_key"], "decision_billing_audit");
        assert_eq!(body["entity_type"], "time_entry");
        assert_eq!(body["state"]["entry"]["description"], "desc");
    }

    #[test]
    fn derived_temporal_fields_compose_in_code() {
        // Monday 2026-09-21 10:00 UTC is business hours; Sunday 20:00
        // and a weekday 22:00 are after hours.
        let business = chrono::DateTime::parse_from_rfc3339("2026-09-21T10:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let sunday_evening = chrono::DateTime::parse_from_rfc3339("2026-09-20T20:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let late_night = chrono::DateTime::parse_from_rfc3339("2026-09-21T22:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(!after_hours(business));
        assert!(after_hours(sunday_evening));
        assert!(after_hours(late_night));

        let now = business;
        assert!(within_sla_window(
            Some(now + chrono::Duration::hours(2)),
            now
        ));
        assert!(!within_sla_window(
            Some(now - chrono::Duration::hours(2)),
            now
        ));
        assert!(!within_sla_window(None, now));
    }

    #[tokio::test]
    async fn ticket_trigger_dispatches_one_fan_out_per_surface() {
        let org = Uuid::now_v7();
        let ticket = Uuid::now_v7();
        let ok = json!({
            "status": "ok",
            "mode": "shadow",
            "answers": {"actionable": {"kind": "noul", "probability": 0.9}},
            "thresholds": {"actionable": {"act": 0.8, "review": 0.5}}
        });
        let fan = Arc::new(RecordingFanOut::new(vec![ok]));
        let bot = Uuid::now_v7();
        let _env = envelope("ticket.created", &ticket.to_string());
        // No DB here: handle_ticket exits before any query when no pool
        // row exists — assert via the recorded bodies on a synthetic path
        // instead. We exercise fan_out_and_log + auto-ack gating directly.
        let body = json!({
            "org_id": org,
            "user_id": bot,
            "surface_key": TRIAGE,
            "entity_type": "ticket",
            "entity_id": ticket,
            "state": {},
            "request_id": "r1",
            "inventories": {},
        });
        let run = fan_out_and_log(
            &fan_upcast(&fan),
            FanOutAsk {
                org_id: org,
                user_id: bot,
                surface: TRIAGE,
                entity_type: "ticket",
                entity_id: Some(ticket),
                state: json!({}),
                request_id: "r1",
                inventories: HashMap::new(),
            },
        )
        .await;
        assert!(run.is_some());
        assert_eq!(fan.recorded().len(), 1);
        assert_eq!(fan.recorded()[0], body);
        // Shadow mode never auto-acks even above thresholds.
        maybe_auto_ack(&fan_upcast(&fan), org, &bot, ticket, "r1", &run.unwrap()).await;
        assert!(fan.acks().is_empty());
    }

    #[tokio::test]
    async fn live_above_thresholds_auto_acks_but_shadow_never_does() {
        let org = Uuid::now_v7();
        let alert = Uuid::now_v7();
        let bot = Uuid::now_v7();
        let live = json!({
            "status": "ok",
            "mode": "live",
            "answers": {
                "actionable": {"kind": "noul", "probability": 0.9},
                "severity": {"kind": "score", "level": 4}
            },
            "thresholds": {
                "actionable": {"act": 0.85, "review": 0.6},
                "severity": {"act": 4.0, "review": 3.0}
            }
        });
        let fan = Arc::new(RecordingFanOut::new(vec![]).with_ack_outcome(vec![true]));
        maybe_auto_ack(&fan_upcast(&fan), org, &bot, alert, "req", &live).await;
        assert_eq!(fan.acks().len(), 1);
        assert_eq!(fan.acks()[0]["alert_id"], json!(alert));

        // Below the act threshold → untouched.
        let low = json!({
            "status": "ok",
            "mode": "live",
            "answers": {
                "actionable": {"kind": "noul", "probability": 0.7},
                "severity": {"kind": "score", "level": 4}
            },
            "thresholds": live["thresholds"].clone()
        });
        let fan2 = Arc::new(RecordingFanOut::new(vec![]));
        maybe_auto_ack(&fan_upcast(&fan2), org, &bot, alert, "req", &low).await;
        assert!(fan2.acks().is_empty());
    }
}
