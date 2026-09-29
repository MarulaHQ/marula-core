//! worker/webhook_out — signed webhook delivery for the plugin runtime.
//!
//! Purpose: two halves. The STREAM CONSUMER subscribes to the domain-event
//! stream via a durable consumer group, matches each envelope against the
//! org's enabled webhook subscriptions, and enqueues one durable
//! `webhook.deliver` job per (webhook, event) — ACKing only after the job
//! is durably queued (at-least-once, retries belong to the queue). The
//! DELIVERY half performs one signed HTTP POST per claimed job
//! (`t=<ts>,v1=<hex>` HMAC-SHA256 over `"{ts}.{body}"`), records every
//! attempt through core-api's internal surface (§4.1: only core-api
//! writes), and settles the job done / retry-with-backoff / dead.
//!
//! Operational note: signing requires DECRYPTING the subscription secret,
//! so this process needs the at-rest key (`MARULA_SECRET_KEY`). Unset ⇒
//! delivery jobs dead-letter with a loud reason — a documented fail-closed
//! none mode, never plaintext anywhere in between.
//!
//! Non-purpose: no event semantics (everything not filtered by the
//! subscription is delivered verbatim), no receiver-side logic, no direct
//! writes to domain tables.

use crate::effects::{EffectApi, EffectOutcome};
use core_db::webhooks::NewDeliveryAttempt;
use core_domain::webhooks::{backoff_secs, signature_value, MAX_DELIVERY_ATTEMPTS};
use core_events::{Envelope, EVENT_STREAM};
use redis::streams::{StreamReadOptions, StreamReadReply};
use redis::AsyncCommands;
use serde_json::Value;
use sqlx::PgPool;
use tokio::sync::watch;

/// Durable group; `$` starts at the live tail on first creation so a new
/// deployment never replays history from before it existed.
const CONSUMER_GROUP: &str = "webhooks-out";
const CONSUMER_NAME: &str = "worker";
const BLOCK_MS: usize = 2_000;
const COUNT: usize = 50;

/// Job kind dispatched by the queue runner to [`process_delivery_job`].
pub const JOB_KIND: &str = "webhook.deliver";

/// One HTTP attempt's raw outcome, classified by the sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinkResult {
    /// 2xx — receiver accepted.
    Delivered(u16),
    /// Receiver answered with a non-2xx status.
    Rejected(u16),
    /// No HTTP answer at all (DNS, connect, timeout…).
    Transport(String),
}

impl SinkResult {
    /// 410 Gone is a permanent unsubscribe signal; everything else that
    /// failed may succeed on a later attempt.
    pub fn is_permanent(&self) -> bool {
        matches!(self, SinkResult::Rejected(410))
    }

    pub fn status(&self) -> Option<i32> {
        match self {
            SinkResult::Delivered(s) | SinkResult::Rejected(s) => Some(i32::from(*s)),
            SinkResult::Transport(_) => None,
        }
    }

    pub fn error(&self) -> Option<String> {
        match self {
            SinkResult::Delivered(_) => None,
            SinkResult::Rejected(s) => Some(format!("receiver returned {s}")),
            SinkResult::Transport(e) => Some(e.clone()),
        }
    }
}

/// Outbound HTTP edge. A trait so tests can record deliveries without any
/// network; production uses reqwest with rustls.
pub trait DeliverySink: Send + Sync {
    fn post(
        &self,
        url: &str,
        signature: &str,
        timestamp: i64,
        body: &[u8],
    ) -> impl std::future::Future<Output = SinkResult> + Send;
}

/// Production sink: one attempt per call, no redirects (a redirect must
/// never silently re-post a signed body somewhere else).
#[derive(Clone, Default)]
pub struct ReqwestSink {
    http: reqwest::Client,
}

impl ReqwestSink {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_default(),
        }
    }
}

impl DeliverySink for ReqwestSink {
    async fn post(&self, url: &str, signature: &str, timestamp: i64, body: &[u8]) -> SinkResult {
        let response = self
            .http
            .post(url)
            .header("content-type", "application/json")
            .header(core_domain::webhooks::SIGNATURE_HEADER, signature)
            .header(
                core_domain::webhooks::TIMESTAMP_HEADER,
                timestamp.to_string(),
            )
            .body(body.to_vec())
            .send()
            .await;
        match response {
            Ok(resp) if resp.status().is_success() => SinkResult::Delivered(resp.status().as_u16()),
            Ok(resp) => SinkResult::Rejected(resp.status().as_u16()),
            Err(e) => SinkResult::Transport(e.to_string()),
        }
    }
}

// --- stream consumer ---------------------------------------------------------

/// Runs the webhooks-out consumer until shutdown. Redis errors reconnect
/// with backoff; malformed entries are logged, ACKed and skipped.
pub async fn run_webhook_out(pool: PgPool, redis_url: String, mut shutdown: watch::Receiver<bool>) {
    let client = match redis::Client::open(redis_url.as_str()) {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(%error, "invalid redis url; webhooks-out exiting");
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
                tracing::warn!("webhooks-out stream connection ended; reconnecting");
            }
            Err(error) => {
                tracing::error!(%error, "webhooks-out redis connect failed");
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
        backoff_ms = (backoff_ms * 2).min(5_000);
    }
}

/// Drains this consumer's pending entries first (crash recovery), then
/// follows the live tail under the group.
async fn follow_stream(
    pool: &PgPool,
    conn: &mut redis::aio::MultiplexedConnection,
    shutdown: &mut watch::Receiver<bool>,
) {
    let options = StreamReadOptions::default().count(COUNT);
    if let Ok(reply) = conn
        .xread_options::<_, _, StreamReadReply>(&[EVENT_STREAM], &["0"], &options)
        .await
    {
        for entry in reply.keys.into_iter().flat_map(|k| k.ids) {
            handle_entry(pool, conn, &entry).await;
        }
    }
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
                        tracing::error!(%error, "webhooks-out xreadgroup failed");
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
    let subscriptions = match core_db::webhooks::find_enabled_for_event(
        pool,
        envelope.org_id,
        &envelope.event_type,
    )
    .await
    {
        Ok(subs) => subs,
        Err(error) => {
            // NOT acking: the group redelivers after restart, so a DB
            // hiccup retries matching instead of dropping the event.
            tracing::error!(%error, entry_id = %entry.id, "webhook match failed; leaving unacked");
            return;
        }
    };
    for sub in subscriptions {
        let job = core_db::queue::NewJob {
            org_id: envelope.org_id,
            kind: JOB_KIND.to_string(),
            payload: serde_json::json!({
                "webhook_id": sub.id,
                "event_id": envelope.id,
                // Canonical re-serialization of the whole envelope IS the
                // delivered body — receivers get the full contract.
                "envelope": envelope.to_json(),
            }),
            max_attempts: MAX_DELIVERY_ATTEMPTS,
        };
        match core_db::queue::enqueue(pool, &job).await {
            Ok(id) => {
                tracing::info!(job_id = %id, webhook_id = %sub.id, event_id = %envelope.id, "webhook delivery queued")
            }
            Err(error) => {
                tracing::error!(%error, entry_id = %entry.id, "enqueue failed; leaving unacked");
                return;
            }
        }
    }
    // Durably queued (or nothing subscribed) — safe to settle.
    ack(conn, &entry.id).await;
}

async fn ack(conn: &mut redis::aio::MultiplexedConnection, id: &str) {
    let _: Result<i64, _> = conn.xack(EVENT_STREAM, CONSUMER_GROUP, &[id]).await;
}

// --- delivery processing -------------------------------------------------------

/// The decision after one delivery attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Done,
    Retry,
    Dead,
}

/// Pure settle rule, unit-tested without any I/O: success settles done,
/// 410 Gone or an exhausted budget dead-letters, everything else retries
/// on the capped exponential schedule.
#[must_use]
pub fn decide(result: &SinkResult, attempts: i32, max_attempts: i32) -> Decision {
    match result {
        SinkResult::Delivered(_) => Decision::Done,
        _ if result.is_permanent() => Decision::Dead,
        _ if attempts >= max_attempts => Decision::Dead,
        _ => Decision::Retry,
    }
}

/// Processes one claimed `webhook.deliver` job: sign, deliver, record the
/// attempt through core-api, settle the job. Every failure path either
/// settles durably or leaves the job running so stale-lease reclaim
/// re-arms it — no silent drops.
pub async fn process_delivery_job<S: DeliverySink, E: EffectApi>(
    pool: &PgPool,
    secret_key: &crate::config::AtRestKey,
    sink: &S,
    effects: &E,
    job: &core_db::queue::ClaimedJob,
) -> anyhow::Result<()> {
    // Malformed payloads are permanent: dead-letter immediately.
    let Some(payload) = DeliveryPayload::parse(&job.payload)? else {
        core_db::queue::mark_dead(pool, job.id, "malformed webhook.deliver payload").await?;
        return Ok(());
    };

    // Load the subscription cross-org (owner pool); decrypt the secret.
    let row = match core_db::webhooks::get_webhook(pool, job.org_id, payload.webhook_id()).await {
        Ok(row) if row.enabled => row,
        Ok(_) => {
            core_db::queue::mark_done(pool, job.id).await?;
            return Ok(());
        }
        Err(_) => {
            core_db::queue::mark_dead(pool, job.id, "webhook subscription missing").await?;
            return Ok(());
        }
    };
    let secret = match secret_key.decrypt(&row.secret_encrypted) {
        Ok(secret) => secret,
        Err(error) => {
            tracing::error!(%error, webhook_id = %row.id, "secret decrypt failed");
            core_db::queue::mark_dead(pool, job.id, "subscription secret unreadable").await?;
            return Ok(());
        }
    };

    // Sign the exact bytes we send.
    let body = serde_json::to_vec(payload.envelope())?;
    let now_unix = chrono::Utc::now().timestamp();
    let signature = signature_value(&secret, &body, now_unix);

    let result = sink.post(&row.url, &signature, now_unix, &body).await;
    let decision = decide(&result, job.attempts, job.max_attempts);

    let record = NewDeliveryAttempt {
        webhook_id: row.id,
        event_id: payload.event_id(),
        event_type: payload.event_type().to_string(),
        attempt: job.attempts,
        response_status: result.status(),
        next_retry_at: match decision {
            Decision::Retry => {
                Some(chrono::Utc::now() + chrono::Duration::seconds(backoff_secs(job.attempts)))
            }
            _ => None,
        },
        delivered_at: matches!(result, SinkResult::Delivered(_)).then(chrono::Utc::now),
        last_error: result.error(),
    };
    // Attempt log precedes settling: the insert is unique-guarded, so a
    // redelivery of this very attempt can never double-log.
    let recorded = record_attempt(effects, job.org_id, &record).await;
    match (decision, recorded) {
        (_, EffectOutcome::Transient) | (_, EffectOutcome::Rejected)
            if decision == Decision::Done =>
        {
            // Delivered but the ledger write bounced — retry the job; the
            // receiver sees an idempotent duplicate, the ledger stays complete.
            core_db::queue::schedule_retry(
                pool,
                job.id,
                backoff_secs(job.attempts),
                "ledger unavailable",
            )
            .await?;
        }
        (Decision::Done, _) => core_db::queue::mark_done(pool, job.id).await?,
        (Decision::Retry, _) => {
            core_db::queue::schedule_retry(
                pool,
                job.id,
                backoff_secs(job.attempts),
                &record
                    .last_error
                    .unwrap_or_else(|| "delivery failed".into()),
            )
            .await?;
        }
        (Decision::Dead, _) => {
            core_db::queue::mark_dead(
                pool,
                job.id,
                &record
                    .last_error
                    .unwrap_or_else(|| "delivery failed".into()),
            )
            .await?
        }
    }
    Ok(())
}

async fn record_attempt<E: EffectApi>(
    effects: &E,
    org_id: uuid::Uuid,
    record: &NewDeliveryAttempt,
) -> EffectOutcome {
    let body = serde_json::json!({
        "org_id": org_id,
        "records": [{
            "webhook_id": record.webhook_id,
            "event_id": record.event_id,
            "event_type": record.event_type,
            "attempt": record.attempt,
            "response_status": record.response_status,
            "next_retry_at": record.next_retry_at.map(|t| t.to_rfc3339()),
            "delivered_at": record.delivered_at.map(|t| t.to_rfc3339()),
            "last_error": record.last_error,
        }],
    });
    effects.post("/api/internal/webhook/deliveries", body).await
}

/// Typed view over the job payload; `None` means malformed.
struct DeliveryPayload {
    webhook_id: uuid::Uuid,
    event_id: uuid::Uuid,
    event_type: String,
    envelope: Value,
}

impl DeliveryPayload {
    fn parse(value: &Value) -> anyhow::Result<Option<Self>> {
        let webhook_id = value
            .get("webhook_id")
            .and_then(Value::as_str)
            .and_then(|s| uuid::Uuid::parse_str(s).ok());
        let Some(webhook_id) = webhook_id else {
            return Ok(None);
        };
        let envelope = value.get("envelope").cloned().unwrap_or(Value::Null);
        if envelope.is_null() {
            return Ok(None);
        }
        let parsed: Envelope = serde_json::from_value(envelope.clone())
            .map_err(|e| anyhow::anyhow!("envelope malformed: {e}"))?;
        Ok(Some(Self {
            webhook_id,
            event_id: parsed.id,
            event_type: parsed.event_type.clone(),
            envelope,
        }))
    }

    fn webhook_id(&self) -> uuid::Uuid {
        self.webhook_id
    }

    fn event_id(&self) -> uuid::Uuid {
        self.event_id
    }

    fn event_type(&self) -> &str {
        &self.event_type
    }

    fn envelope(&self) -> &Value {
        &self.envelope
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settle_rule_covers_the_matrix() {
        assert_eq!(decide(&SinkResult::Delivered(200), 1, 6), Decision::Done);
        assert_eq!(decide(&SinkResult::Delivered(201), 6, 6), Decision::Done);
        // 410 Gone: permanent unsubscribe, even on attempt 1.
        assert_eq!(decide(&SinkResult::Rejected(410), 1, 6), Decision::Dead);
        assert_eq!(decide(&SinkResult::Rejected(500), 1, 6), Decision::Retry);
        assert_eq!(
            decide(
                &SinkResult::Rejected(500),
                MAX_DELIVERY_ATTEMPTS,
                MAX_DELIVERY_ATTEMPTS
            ),
            Decision::Dead
        );
        assert_eq!(
            decide(&SinkResult::Transport("timeout".into()), 5, 6),
            Decision::Retry
        );
        assert_eq!(
            decide(&SinkResult::Transport("timeout".into()), 6, 6),
            Decision::Dead
        );
    }

    #[test]
    fn malformed_payloads_are_detected_without_panicking() {
        assert!(DeliveryPayload::parse(&serde_json::json!({}))
            .expect("parse")
            .is_none());
        assert!(
            DeliveryPayload::parse(&serde_json::json!({"webhook_id": uuid::Uuid::now_v7()}))
                .expect("parse")
                .is_none()
        );
        let good = serde_json::json!({
            "webhook_id": uuid::Uuid::now_v7(),
            "event_id": uuid::Uuid::now_v7().to_string(),
            "envelope": {"id": uuid::Uuid::now_v7(), "type": "ticket.created",
                         "org_id": uuid::Uuid::now_v7(), "entity": "ticket",
                         "occurred_at": "2026-01-01T00:00:00Z",
                         "payload_schema_ref": "core-domain/v1", "payload": {}},
        });
        let parsed = DeliveryPayload::parse(&good)
            .expect("parse")
            .expect("valid");
        assert_eq!(parsed.event_type(), "ticket.created");
    }
}
