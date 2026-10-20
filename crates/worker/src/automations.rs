//! worker/automations — event-triggered rule evaluation over the stream.
//!
//! Purpose: consume the single domain-event stream via a durable consumer
//! group, match each event against enabled automation rules (pure
//! core-domain matching on a read-only SUBJECT), and enqueue
//! `automation.effects` jobs. Delivery is at-least-once: a job is ACKed
//! only after it is durably queued; retries belong to the queue.
//!
//! The subject is `(ticket_id: Option, board_id)` resolved from the
//! envelope (Wave 43 lane 3). A ticket event carries the read-only ticket
//! snapshot every deterministic condition reads; a ticket-less event
//! (lane 1's inbound receiver emits those) carries only the board, and
//! board-subject rules are the ones that may fire on it. Ticket-fact
//! conditions degrade to no-match-with-warn there rather than pretending.
//!
//! Loop safety: events whose payload carries `via_automation: true` are
//! skipped, so an effect can never re-trigger evaluation recursively — the
//! marker rides the payload, so both subject shapes are guarded alike.
//!
//! Non-purpose: no writes outside queue tables, no condition logic here
//! (core-domain), no effect application (queue runner → core-api).

use crate::reconnect::Reconnect;
use core_domain::automations::{
    is_automation_echo, AutomationRule, AutomationRuleRow, AutomationSubject,
    AutomationSubjectKind, TicketSnapshot,
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

/// What an envelope says about its ticket subject.
#[derive(Debug, Clone, Copy, PartialEq)]
enum TicketRef {
    /// No `ticket_id` key at all — a ticket-less envelope.
    Absent,
    /// A parseable id; the snapshot loader resolves it.
    Present(uuid::Uuid),
    /// A `ticket_id` that is not a uuid. Refused rather than reinterpreted
    /// as "no ticket": a broken ticket reference is a data problem, and
    /// silently widening the subject to the board would hide it.
    Malformed,
}

fn ticket_ref_of(envelope: &Envelope) -> TicketRef {
    match envelope.payload.get("ticket_id") {
        None => TicketRef::Absent,
        Some(value) => value
            .as_str()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .map_or(TicketRef::Malformed, TicketRef::Present),
    }
}

/// The board id the envelope carries, when it carries one that parses.
fn board_id_of(envelope: &Envelope) -> Option<uuid::Uuid> {
    envelope
        .payload
        .get("board_id")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
}

/// The subject one envelope resolves to inside one org: the ticket it names
/// (with its read-only snapshot), or a ticket-less board scope.
enum ResolvedSubject {
    Ticket(TicketSnapshot),
    /// A board-scoped envelope with no ticket. `board_id` may itself be
    /// absent — an org-wide envelope.
    Ticketless {
        board_id: Option<uuid::Uuid>,
    },
}

impl ResolvedSubject {
    fn board_id(&self) -> Option<uuid::Uuid> {
        match self {
            Self::Ticket(snapshot) => Some(snapshot.board_id),
            Self::Ticketless { board_id } => *board_id,
        }
    }

    fn ticket_id(&self) -> Option<uuid::Uuid> {
        match self {
            Self::Ticket(snapshot) => Some(snapshot.ticket_id),
            Self::Ticketless { .. } => None,
        }
    }

    fn domain(&self) -> AutomationSubject<'_> {
        match self {
            Self::Ticket(snapshot) => AutomationSubject::Ticket(snapshot),
            Self::Ticketless { board_id } => AutomationSubject::Ticketless {
                board_id: *board_id,
            },
        }
    }
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

/// True when the named board resolves inside this org. A ticket-less
/// envelope whose board does NOT resolve is not a scope that widens to
/// org-wide rules — it resolves to no subject at all, the same posture as a
/// ticket whose snapshot failed to load.
async fn board_resolves(pool: &PgPool, org_id: uuid::Uuid, board_id: uuid::Uuid) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM boards WHERE id = $1 AND org_id = $2)",
    )
    .bind(board_id)
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or(false)
}

/// Resolves the envelope's subject. Snapshot/board loading is deferred until
/// at least one rule listens — the overwhelmingly common case is zero rules
/// per event type.
async fn resolve_subject(
    pool: &PgPool,
    org_id: uuid::Uuid,
    envelope: &Envelope,
) -> Option<ResolvedSubject> {
    match ticket_ref_of(envelope) {
        TicketRef::Present(ticket_id) => {
            let snapshot = load_snapshot(pool, org_id, ticket_id).await;
            if snapshot.is_none() {
                tracing::warn!(
                    ticket_id = %ticket_id,
                    "ticket snapshot unreadable; rule evaluation skipped"
                );
            }
            snapshot.map(ResolvedSubject::Ticket)
        }
        TicketRef::Malformed => {
            tracing::warn!("payload carries an unparseable ticket_id; rule evaluation skipped");
            None
        }
        TicketRef::Absent => match board_id_of(envelope) {
            Some(board_id) if board_resolves(pool, org_id, board_id).await => {
                Some(ResolvedSubject::Ticketless {
                    board_id: Some(board_id),
                })
            }
            Some(board_id) => {
                tracing::warn!(
                    board_id = %board_id,
                    "ticket-less envelope names a board outside this org; rule evaluation skipped"
                );
                None
            }
            None => Some(ResolvedSubject::Ticketless { board_id: None }),
        },
    }
}

/// One matched (rule, subject) pair ready to become a durable job.
pub struct MatchedRule {
    pub org_id: uuid::Uuid,
    pub rule_id: uuid::Uuid,
    /// `None` for a board-subject match — there is no ticket to act on.
    pub ticket_id: Option<uuid::Uuid>,
    /// The board the match is scoped to, when the subject carried one.
    pub board_id: Option<uuid::Uuid>,
    pub actions: serde_json::Value,
}

/// The `automation.effects` job payload for one match. `ticket_id` is null
/// on a board-subject match; the effect applier branches on exactly that.
fn effect_payload(match_: &MatchedRule) -> serde_json::Value {
    serde_json::json!({
        "org_id": match_.org_id,
        "rule_id": match_.rule_id,
        "ticket_id": match_.ticket_id,
        "board_id": match_.board_id,
        "actions": match_.actions,
    })
}

/// Evaluates one rule's conditions against one subject. `true` when every
/// condition holds.
///
/// Deterministic conditions are pure and cheap; the semantic `ai_decision`
/// conditions evaluate ONLY when those all hold, against the NEWEST
/// recorded decision answers for the ticket (stored-answers evaluation —
/// the decision subsystem owns the one-fan-out-per-trigger dispatch;
/// missing/malformed answers are no-match-with-warn, the same skip posture
/// as a malformed deterministic rule).
///
/// CSAT score conditions evaluate against the EVENT PAYLOAD score (Wave 34
/// lane 2); a `csat_score_below` condition on any other trigger event is a
/// no-match by construction.
///
/// On a ticket-less subject every ticket-fact predicate degrades to
/// no-match, and the rule says so: one warn names each degrading condition,
/// so a board rule that can never fire is visible instead of silent.
async fn conditions_hold(
    pool: &PgPool,
    rule: &AutomationRule,
    subject: &ResolvedSubject,
    envelope: &Envelope,
) -> bool {
    let score = envelope
        .payload
        .get("score")
        .and_then(|v| v.as_i64())
        .map(|v| v as i16);
    if !core_domain::automations::csat_score_holds(&rule.conditions, score) {
        return false;
    }
    let split = core_domain::automations::split_conditions(&rule.conditions);
    let snapshot = match subject {
        ResolvedSubject::Ticket(snapshot) => Some(snapshot),
        ResolvedSubject::Ticketless { board_id } => {
            for condition in split.deterministic.iter().filter(|c| c.requires_ticket()) {
                tracing::warn!(
                    rule = %rule.id,
                    board_id = ?board_id,
                    condition = ?condition,
                    "ticket-less subject: condition needs a ticket, no-match"
                );
            }
            if !split.semantic.is_empty() {
                tracing::warn!(
                    rule = %rule.id,
                    board_id = ?board_id,
                    "ai_decision condition no-match: ticket-less subject has no ticket"
                );
                return false;
            }
            None
        }
    };
    if !core_domain::automations::matches_all(split.deterministic.iter().copied(), subject.domain())
    {
        return false;
    }
    if !split.semantic.is_empty() {
        let snapshot = snapshot.expect("semantic path requires a ticket subject");
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
            return false;
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
            return false;
        }
    }
    true
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
    let Some(subject) = resolve_subject(pool, envelope.org_id, envelope).await else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for record in rules {
        let row = AutomationRuleRow {
            id: record.id,
            org_id: record.org_id,
            board_id: record.board_id,
            subject_kind: record.subject_kind,
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
        // Board scope is checked over the subject's board, which a
        // ticket-less envelope may not carry at all.
        if !rule.applies_to_board(subject.board_id()) {
            continue;
        }
        // Subject kind gates the envelope shape, symmetrically: a ticket
        // rule fires only on an envelope that resolved a ticket, a board
        // rule only on a ticket-less one. A rule's subject declares what it
        // consumes, so nothing fires "by accident" on the other shape.
        let subject_fits = match rule.subject_kind {
            AutomationSubjectKind::Ticket => subject.ticket_id().is_some(),
            AutomationSubjectKind::Board => subject.ticket_id().is_none(),
        };
        if !subject_fits {
            continue;
        }
        if !conditions_hold(pool, &rule, &subject, envelope).await {
            continue;
        }
        out.push(MatchedRule {
            org_id: envelope.org_id,
            rule_id: rule.id,
            ticket_id: subject.ticket_id(),
            board_id: subject.board_id(),
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
    let mut backoff = Reconnect::consumer();
    loop {
        if *shutdown.borrow() {
            return;
        }
        match client.get_multiplexed_async_connection().await {
            Ok(mut conn) => {
                backoff.reset();
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
                        backoff.sleep().await;
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
        backoff.sleep().await;
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
                payload: effect_payload(&m),
                max_attempts: crate::queue_runner::queue_max_attempts(),
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
    fn ticket_ref_and_board_extract_from_canonical_payloads() {
        let id = uuid::Uuid::now_v7();
        let board = uuid::Uuid::now_v7();
        let both = envelope(
            "ticket.created",
            serde_json::json!({ "ticket_id": id.to_string(), "board_id": board.to_string() }),
        );
        assert_eq!(ticket_ref_of(&both), TicketRef::Present(id));
        assert_eq!(board_id_of(&both), Some(board));

        let bare = envelope(
            "ticket.updated",
            serde_json::json!({ "ticket_id": id.to_string() }),
        );
        assert_eq!(ticket_ref_of(&bare), TicketRef::Present(id));
        assert_eq!(board_id_of(&bare), None);

        // Lane 1's inbound receiver emits exactly this shape: a board and
        // no ticket at all.
        let ticketless = envelope(
            "webhook.received",
            serde_json::json!({ "board_id": board.to_string(), "data": {} }),
        );
        assert_eq!(ticket_ref_of(&ticketless), TicketRef::Absent);
        assert_eq!(board_id_of(&ticketless), Some(board));

        let none = envelope("org.created", serde_json::json!({}));
        assert_eq!(ticket_ref_of(&none), TicketRef::Absent);
        assert_eq!(board_id_of(&none), None);

        // A broken ticket reference is refused, never read as "no ticket".
        let broken = envelope(
            "ticket.created",
            serde_json::json!({ "ticket_id": "not-a-uuid", "board_id": board.to_string() }),
        );
        assert_eq!(ticket_ref_of(&broken), TicketRef::Malformed);
    }

    /// The loop guard is enforced BEFORE any DB work in match_rules; this
    /// pins the predicate the guard relies on, on BOTH subject shapes.
    #[test]
    fn automation_echoes_are_detectable() {
        let echoed = envelope(
            "ticket.updated",
            serde_json::json!({
                "ticket_id": uuid::Uuid::now_v7().to_string(),
                "via_automation": true,
            }),
        );
        assert!(is_automation_echo(&echoed.payload));

        // The ticket-less path is guarded identically: an effect emitted by
        // a board-subject rule carries the marker, so it cannot re-trigger
        // its own rule. The marker rides the payload, not the subject, so
        // the guard needs no branch for the new path.
        let ticketless_echo = envelope(
            "webhook.received",
            serde_json::json!({
                "board_id": uuid::Uuid::now_v7().to_string(),
                "via_automation": true,
            }),
        );
        assert!(is_automation_echo(&ticketless_echo.payload));
        assert_eq!(ticket_ref_of(&ticketless_echo), TicketRef::Absent);

        // A genuine inbound delivery is NOT an echo and must still match.
        let inbound = envelope(
            "webhook.received",
            serde_json::json!({ "board_id": uuid::Uuid::now_v7().to_string(), "data": {} }),
        );
        assert!(!is_automation_echo(&inbound.payload));
    }

    /// The job payload carries the subject: a null `ticket_id` plus the
    /// board on a ticket-less match, and the ticket on a ticket match.
    #[test]
    fn effect_payload_carries_the_subject() {
        let org = uuid::Uuid::now_v7();
        let board = uuid::Uuid::now_v7();
        let ticketless = MatchedRule {
            org_id: org,
            rule_id: uuid::Uuid::now_v7(),
            ticket_id: None,
            board_id: Some(board),
            actions: serde_json::json!([{ "type": "notify_users" }]),
        };
        let payload = effect_payload(&ticketless);
        assert!(payload["ticket_id"].is_null(), "no ticket to act on");
        assert_eq!(payload["board_id"], serde_json::json!(board.to_string()));
        assert_eq!(payload["org_id"], serde_json::json!(org.to_string()));

        let ticket = uuid::Uuid::now_v7();
        let ticketed = MatchedRule {
            ticket_id: Some(ticket),
            board_id: Some(board),
            ..ticketless
        };
        let payload = effect_payload(&ticketed);
        assert_eq!(payload["ticket_id"], serde_json::json!(ticket.to_string()));
    }
}
