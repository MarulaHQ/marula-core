//! worker/automations — event-triggered rule evaluation over the stream.
//!
//! Purpose: consume the single domain-event stream via a durable consumer
//! group, match each ticket event against enabled automation rules (pure
//! core-domain matching on a READ-ONLY ticket snapshot), and enqueue
//! `automation.effects` jobs. Delivery is at-least-once: a job is ACKed
//! only after it is durably queued; retries belong to the queue.
//!
//! Loop safety: events whose payload carries `via_automation: true` are
//! skipped, so an effect can never re-trigger evaluation recursively.
//!
//! Non-purpose: no writes outside queue tables, no condition logic here
//! (core-domain), no effect application (queue runner → core-api).

use core_domain::automations::{
    is_automation_echo, AutomationRule, AutomationRuleRow, TicketSnapshot,
};
use core_events::{Envelope, EVENT_STREAM};
use redis::streams::{StreamReadOptions, StreamReadReply};
use redis::AsyncCommands;
use sqlx::PgPool;
use tokio::sync::watch;

/// Stable consumer name: a restarted process resumes its own pending
/// entries through the same identity. Multi-worker fan-out with claim
/// stealing lands later (XAUTOCLAIM).
const CONSUMER_GROUP: &str = "automations";
const CONSUMER_NAME: &str = "worker";
const BLOCK_MS: usize = 2_000;
const COUNT: usize = 50;

/// Extracts the ticket id + board id an envelope refers to, when it has one.
fn ticket_ids_of(envelope: &Envelope) -> Option<(uuid::Uuid, Option<uuid::Uuid>)> {
    let ticket_id = envelope
        .payload
        .get("ticket_id")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok())?;
    let board_id = envelope
        .payload
        .get("board_id")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok());
    Some((ticket_id, board_id))
}

/// Read-only snapshot of the ticket a rule would match against.
async fn load_snapshot(
    pool: &PgPool,
    org_id: uuid::Uuid,
    ticket_id: uuid::Uuid,
) -> Option<TicketSnapshot> {
    // Reads across orgs need the owner role (see config module note);
    // this query is deliberately unscoped like every other worker read.
    let row = sqlx::query_as::<
        _,
        (
            uuid::Uuid,
            uuid::Uuid,
            String,
            String,
            String,
            Option<uuid::Uuid>,
            String,
        ),
    >(
        r#"SELECT id, board_id, number, title, priority, assignee_id, category
           FROM tickets WHERE id = $1 AND org_id = $2"#,
    )
    .bind(ticket_id)
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .ok()?;
    let row = row?;

    Some(TicketSnapshot {
        ticket_id: row.0,
        org_id,
        board_id: row.1,
        number: row.2,
        title: row.3,
        priority: row.4.parse().ok()?,
        category: row.6.parse().ok()?,
        assignee_id: row.5,
    })
}

/// One matched (rule, ticket) pair ready to become a durable job.
pub struct MatchedRule {
    pub org_id: uuid::Uuid,
    pub rule_id: uuid::Uuid,
    pub ticket_id: uuid::Uuid,
    pub actions: serde_json::Value,
}

/// Matches rules for one envelope; returns jobs to enqueue.
pub async fn match_rules(pool: &PgPool, envelope: &Envelope) -> Vec<MatchedRule> {
    if is_automation_echo(&envelope.payload) {
        return Vec::new();
    }
    let rules = match core_db::automations::list_enabled_by_trigger(
        pool,
        envelope.org_id,
        &envelope.event_type,
    )
    .await
    {
        Ok(rules) if !rules.is_empty() => rules,
        _ => return Vec::new(),
    };
    // Snapshot loading is deferred until at least one rule listens —
    // the overwhelmingly common case is zero rules per event type.
    let Some((ticket_id, _board_hint)) = ticket_ids_of(envelope) else {
        return Vec::new();
    };
    let Some(snapshot) = load_snapshot(pool, envelope.org_id, ticket_id).await else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for record in rules {
        let row = AutomationRuleRow {
            id: record.id,
            org_id: record.org_id,
            board_id: record.board_id,
            name: record.name.clone(),
            trigger_event: record.trigger_event.clone(),
            conditions: record.conditions.clone(),
            actions: record.actions.clone(),
            enabled: record.enabled,
        };
        let rule = match AutomationRule::parse(&row) {
            Ok(rule) => rule,
            Err(reason) => {
                tracing::warn!(%reason, "malformed automation rule skipped");
                continue;
            }
        };
        if !rule.applies_to_board(snapshot.board_id) {
            continue;
        }
        // Deterministic conditions first (pure, cheap); semantic
        // `ai_decision` conditions evaluate ONLY when those all hold,
        // against the NEWEST recorded decision answers for the ticket
        // (stored-answers evaluation — the decision subsystem owns the
        // one-fan-out-per-trigger dispatch; missing/malformed answers
        // are no-match-with-warn, the same skip posture as a malformed
        // deterministic rule).
        let split = core_domain::automations::split_conditions(&rule.conditions);
        if !core_domain::automations::matches_all(split.deterministic.iter().copied(), &snapshot) {
            continue;
        }
        if !split.semantic.is_empty() {
            // One read per distinct referenced surface, newest answers row.
            let mut surfaces: Vec<&str> = split.semantic.iter().map(|spec| spec.surface).collect();
            surfaces.sort_unstable();
            surfaces.dedup();
            let mut answers = std::collections::HashMap::new();
            let mut answers_ok = true;
            for surface in &surfaces {
                match core_db::decision::latest_answers_for_entity(
                    pool,
                    snapshot.org_id,
                    surface,
                    "ticket",
                    snapshot.ticket_id,
                )
                .await
                {
                    Ok(Some(row)) => {
                        answers.insert(*surface, row.answers);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(
                            rule = %rule.id,
                            surface,
                            %error,
                            "ai_decision answers read failed; rule skipped-with-warn"
                        );
                        answers_ok = false;
                        break;
                    }
                }
            }
            if !answers_ok {
                continue;
            }
            let mut all_hold = true;
            for spec in &split.semantic {
                let surface = spec.surface;
                let question = spec.question;
                let Some(surface_answers) = answers.get(surface) else {
                    tracing::warn!(
                        rule = %rule.id,
                        surface,
                        question,
                        "ai_decision condition no-match: no recorded answers for surface"
                    );
                    all_hold = false;
                    break;
                };
                if !core_domain::automations::evaluate_ai_decision(
                    surface_answers,
                    surface,
                    question,
                    spec.operator,
                    spec.threshold,
                    spec.confidence_path,
                ) {
                    all_hold = false;
                    break;
                }
            }
            if !all_hold {
                continue;
            }
        }
        out.push(MatchedRule {
            org_id: envelope.org_id,
            rule_id: rule.id,
            ticket_id: snapshot.ticket_id,
            actions: serde_json::to_value(&rule.actions).unwrap_or_default(),
        });
    }
    out
}

/// Runs the automations consumer until shutdown. Redis errors reconnect
/// with backoff; malformed entries are logged, ACKed and skipped so one
/// poison message never stalls the stream.
pub async fn run_automations(pool: PgPool, redis_url: String, mut shutdown: watch::Receiver<bool>) {
    let client = match redis::Client::open(redis_url.as_str()) {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(%error, "invalid redis url; automations exiting");
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
                // so we don't replay history from before this deploy.
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
                follow_stream(&pool, &mut conn, &mut shutdown).await;
                tracing::warn!("automations stream connection ended; reconnecting");
            }
            Err(error) => {
                tracing::error!(%error, "automations redis connect failed");
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
        backoff_ms = (backoff_ms * 2).min(5_000);
    }
}

/// Follows the group. On startup it first drains THIS consumer's pending
/// entries ("0" reads the PEL), recovering work interrupted by a crash;
/// then follows live entries (">"). Returns None on shutdown.
async fn follow_stream(
    pool: &PgPool,
    conn: &mut redis::aio::MultiplexedConnection,
    shutdown: &mut watch::Receiver<bool>,
) {
    // 1. Recover pending entries owned by this consumer name.
    let options = StreamReadOptions::default().count(COUNT);
    if let Ok(reply) = conn
        .xread_options::<_, _, StreamReadReply>(&[EVENT_STREAM], &["0"], &options)
        .await
    {
        for entry in reply.keys.into_iter().flat_map(|k| k.ids) {
            handle_entry(pool, conn, &entry).await;
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
                            handle_entry(pool, conn, &entry).await;
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "automations xreadgroup failed");
                        return;
                    }
                }
            }
        }
    }
}

async fn handle_entry(
    pool: &PgPool,
    conn: &mut redis::aio::MultiplexedConnection,
    entry: &redis::streams::StreamId,
) {
    let Some(raw) = entry.get::<String>("data") else {
        tracing::warn!(entry_id = %entry.id, "stream entry missing data field");
        return;
    };
    let Ok(envelope) = serde_json::from_str::<Envelope>(&raw) else {
        tracing::warn!(entry_id = %entry.id, "unparseable event; acking to skip");
        ack(conn, &entry.id).await;
        return;
    };
    let matches = match_rules(pool, &envelope).await;
    if !matches.is_empty() {
        for m in matches {
            let job = core_db::queue::NewJob {
                org_id: m.org_id,
                kind: "automation.effects".to_string(),
                payload: serde_json::json!({
                    "org_id": m.org_id,
                    "rule_id": m.rule_id,
                    "ticket_id": m.ticket_id,
                    "actions": m.actions,
                }),
                max_attempts: 5,
            };
            match core_db::queue::enqueue(pool, &job).await {
                Ok(id) => {
                    tracing::info!(job_id = %id, rule_id = %m.rule_id, "automation effect queued")
                }
                Err(error) => {
                    // NOT acking: the group redelivers this entry after
                    // restart, so a failed enqueue retries by design.
                    tracing::error!(%error, entry_id = %entry.id, "enqueue failed; leaving unacked");
                    return;
                }
            }
        }
    }
    // Work is durably queued (or nothing matched) — safe to settle.
    ack(conn, &entry.id).await;
}

async fn ack(conn: &mut redis::aio::MultiplexedConnection, id: &str) {
    let _: Result<i64, _> = conn.xack(EVENT_STREAM, CONSUMER_GROUP, &[id]).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_events::Envelope;

    fn envelope(event_type: &str, payload: serde_json::Value) -> Envelope {
        Envelope {
            id: uuid::Uuid::now_v7(),
            event_type: event_type.into(),
            org_id: uuid::Uuid::now_v7(),
            entity: "ticket".into(),
            entity_id: None,
            occurred_at: chrono::Utc::now(),
            payload_schema_ref: "core-domain/v1".into(),
            payload,
        }
    }

    #[test]
    fn ticket_ids_extract_from_canonical_payloads() {
        let id = uuid::Uuid::now_v7();
        let board = uuid::Uuid::now_v7();
        let env = envelope(
            "ticket.created",
            serde_json::json!({ "ticket_id": id.to_string(), "board_id": board.to_string() }),
        );
        let (t, b) = ticket_ids_of(&env).expect("ids");
        assert_eq!(t, id);
        assert_eq!(b, Some(board));

        let bare = envelope(
            "ticket.updated",
            serde_json::json!({ "ticket_id": id.to_string() }),
        );
        assert_eq!(ticket_ids_of(&bare).unwrap().1, None);

        let none = envelope("org.created", serde_json::json!({}));
        assert!(ticket_ids_of(&none).is_none());
    }

    /// The loop guard is enforced BEFORE any DB work in match_rules; this
    /// pins the predicate the guard relies on.
    #[test]
    fn automation_echoes_are_detectable() {
        let echoed = envelope(
            "ticket.updated",
            serde_json::json!({ "ticket_id": uuid::Uuid::now_v7().to_string(), "via_automation": true }),
        );
        assert!(is_automation_echo(&echoed.payload));
    }
}
