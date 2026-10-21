//! worker/webhook_out — signed webhook delivery for the plugin runtime.
//!
//! Purpose: two halves. The STREAM CONSUMER subscribes to the domain-event
//! stream via a durable consumer group, matches each envelope against the
//! org's enabled webhook subscriptions, and enqueues one durable
//! `webhook.deliver` job per (webhook, event) — ACKing only after the job
//! is durably queued (at-least-once, retries belong to the queue). Each
//! process consumes under its own name (the shared harness in
//! [`crate::consumer`]), so the group keeps one pending list per process; a
//! restart recovers what its predecessor left by claiming the entries that
//! went idle past the harness's claim threshold, never by re-reading the
//! stream's history. The job insert is dedupe-keyed, so the at-least-once
//! read cannot become a second signed POST. The
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

use crate::consumer::{self, ConsumerSpec, Delivery, EntryHandler, EntryOutcome, PgLedger};
use crate::effects::{EffectApi, EffectOutcome};
use core_db::webhooks::NewDeliveryAttempt;
use core_domain::webhooks::{backoff_secs, signature_value, MAX_DELIVERY_ATTEMPTS};
use core_events::Envelope;
use serde_json::Value;
use sqlx::PgPool;
use tokio::sync::watch;

/// Durable group; `$` starts at the live tail on first creation so a new
/// deployment never replays history from before it existed.
const CONSUMER_GROUP: &str = "webhooks-out";

/// The page size this consumer reads. The claim window and the recovery page
/// cap are the harness defaults ([`crate::consumer`]): a takeover can cost a
/// redundant read, never a second delivery, because the job insert is
/// dedupe-keyed on `(webhook, event)`.
const COUNT: usize = 50;
const SPEC: ConsumerSpec = ConsumerSpec::new(CONSUMER_GROUP, COUNT);

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
    /// Whether this failure is permanent *for the webhook policy*.
    ///
    /// 410 is the receiver saying "stop sending me this" — a permanent
    /// unsubscribe, never retried. Every other status goes through the
    /// webhook [`RetryPolicy`](core_domain::retry::RetryPolicy): a 429 (the
    /// receiver is throttling us) or a 5xx retries, a 404 means the endpoint
    /// the operator configured does not exist, and a 409 means the receiver
    /// already has this event — a replay adds nothing.
    #[must_use]
    pub fn is_permanent(&self) -> bool {
        match self {
            Self::Rejected(410) => true,
            Self::Rejected(status) => {
                !core_domain::retry::policy(core_domain::retry::Subsystem::Webhook)
                    .retries_status(*status)
            }
            Self::Delivered(_) | Self::Transport(_) => false,
        }
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

/// Runs the webhooks-out consumer under a name derived from this host and
/// process.
pub async fn run_webhook_out(pool: PgPool, redis_url: String, shutdown: watch::Receiver<bool>) {
    run_webhook_out_as(
        pool,
        redis_url,
        consumer::consumer_name(CONSUMER_GROUP),
        shutdown,
    )
    .await;
}

/// [`run_webhook_out`] with the consumer name supplied.
///
/// Public so a test can run two named consumers against one stream in one
/// process — the arrangement the duplicate-delivery fix exists for, and one
/// that a per-process name would otherwise make untestable without spawning
/// binaries.
pub async fn run_webhook_out_as(
    pool: PgPool,
    redis_url: String,
    consumer_name: String,
    shutdown: watch::Receiver<bool>,
) {
    let handler = WebhookHandler { pool: pool.clone() };
    let ledger = PgLedger::new(pool);
    let mut shutdown = shutdown;
    consumer::run(
        &redis_url,
        &SPEC,
        consumer_name,
        &ledger,
        &handler,
        &mut shutdown,
    )
    .await;
}

/// Matches one page of events against the enabled subscriptions and queues one
/// delivery per match.
struct WebhookHandler {
    pool: PgPool,
}

impl EntryHandler for WebhookHandler {
    async fn handle(&self, batch: &[Delivery]) -> Vec<EntryOutcome> {
        let mut outcomes = Vec::with_capacity(batch.len());
        for delivery in batch {
            outcomes.push(self.enqueue_for(&delivery.envelope).await);
        }
        outcomes
    }
}

impl WebhookHandler {
    async fn enqueue_for(&self, envelope: &Envelope) -> EntryOutcome {
        let subscriptions = match core_db::webhooks::find_enabled_for_event(
            &self.pool,
            envelope.org_id,
            &envelope.event_type,
        )
        .await
        {
            Ok(subscriptions) => subscriptions,
            Err(error) => {
                // The entry stays in the PEL: a database hiccup retries
                // matching instead of dropping the event.
                return EntryOutcome::Retry(format!("webhook match failed: {error}"));
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
                max_attempts: core_domain::retry::policy(core_domain::retry::Subsystem::Webhook)
                    .max_attempts_i32(MAX_DELIVERY_ATTEMPTS),
            };
            // One job per (subscription, event) for the life of the queue row.
            // The stream side is at-least-once — a replay after a crash, or a
            // claim from a consumer that died mid-handling, reads this same
            // entry again — and a delivery is an HTTP POST to a third party, so
            // a second job is a second signed body at the receiver. The key is
            // what makes the repeat a no-op instead. A queued, settled or
            // dead-lettered job all keep their key on purpose: from here on the
            // queue owns the retry, and re-enqueuing would fork that history.
            let dedupe_key = format!("{JOB_KIND}:{}:{}", sub.id, envelope.id);
            match core_db::queue::enqueue_deduped(&self.pool, &job, &dedupe_key).await {
                Ok(Some(id)) => {
                    tracing::info!(job_id = %id, webhook_id = %sub.id, event_id = %envelope.id, "webhook delivery queued")
                }
                Ok(None) => {
                    // Already queued by an earlier read of this entry, here or
                    // on another replica. Settling the entry is correct: the
                    // delivery exists.
                    tracing::info!(
                        webhook_id = %sub.id,
                        event_id = %envelope.id,
                        "webhook delivery already queued; not enqueuing again"
                    )
                }
                Err(error) => {
                    return EntryOutcome::Retry(format!("enqueue failed: {error}"));
                }
            }
        }
        EntryOutcome::Settled
    }
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
/// 410 Gone or an exhausted budget dead-letters, everything else retries on
/// the policy's schedule. Which 4xx are permanent is the webhook policy's
/// call (`SinkResult::is_permanent`), not a status the sender hard-codes.
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
    use crate::consumer::{DeliveryLedger, EntryState, EventStream, Harness};
    use core_db::consumer::{Attempt, DeadEventSpec};
    use redis::streams::StreamId;
    use std::collections::{HashMap, VecDeque};

    /// A stream stand-in that models a consumer group: `pending` returns the
    /// scripted PEL pages in order, `claim` returns its scripted takeover
    /// pages (empty once exhausted, so a claim pass ends when the script
    /// does), `live` returns its scripted pages and then fails so the follow
    /// loop returns instead of blocking, and `ack` records what it was asked
    /// to settle.
    struct FakeStream {
        pending_pages: VecDeque<Vec<StreamId>>,
        claim_pages: VecDeque<Vec<StreamId>>,
        claim_fails: bool,
        live_pages: VecDeque<Vec<StreamId>>,
        ack_succeeds: bool,
        calls: Vec<String>,
    }

    impl FakeStream {
        fn new(pending_pages: Vec<Vec<StreamId>>, live_pages: Vec<Vec<StreamId>>) -> Self {
            Self {
                pending_pages: pending_pages.into(),
                claim_pages: VecDeque::new(),
                claim_fails: false,
                live_pages: live_pages.into(),
                ack_succeeds: true,
                calls: Vec::new(),
            }
        }

        /// Scripts the takeover pages `claim` answers with.
        fn with_claims(mut self, claim_pages: Vec<Vec<StreamId>>) -> Self {
            self.claim_pages = claim_pages.into();
            self
        }
    }

    impl EventStream for FakeStream {
        async fn pending(&mut self, _count: usize) -> Result<Vec<StreamId>, String> {
            self.calls.push("pending".to_string());
            Ok(self.pending_pages.pop_front().unwrap_or_default())
        }

        async fn claim(
            &mut self,
            _count: usize,
            _min_idle_ms: u64,
        ) -> Result<Vec<StreamId>, String> {
            self.calls.push("claim".to_string());
            if self.claim_fails {
                return Err("fake: claim failed".to_string());
            }
            Ok(self.claim_pages.pop_front().unwrap_or_default())
        }

        async fn live(&mut self, _count: usize, _block_ms: usize) -> Result<Vec<StreamId>, String> {
            self.calls.push("live".to_string());
            self.live_pages
                .pop_front()
                .ok_or_else(|| "fake: connection closed".to_string())
        }

        async fn ack(&mut self, id: &str) -> bool {
            self.calls.push(format!("ack:{id}"));
            self.ack_succeeds
        }
    }

    /// An entry whose payload does not parse: [`handle_entry`] acks it without
    /// touching Postgres, which is what keeps these tests database-free.
    fn unparseable(id: &str) -> StreamId {
        let mut map = std::collections::HashMap::new();
        map.insert(
            "data".to_string(),
            redis::Value::BulkString(b"not-json".to_vec()),
        );
        StreamId {
            id: id.to_string(),
            map,
        }
    }

    /// Never connected: these tests must not open a socket.
    fn lazy_pool() -> PgPool {
        sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://lazy:lazy@127.0.0.1:1/lazy")
            .expect("lazy pool")
    }

    /// Stands in for `consumer_deliveries` and `dead_events` in the loop tests:
    /// no entry has ever failed and every write succeeds, which keeps these
    /// tests free of Postgres. The attempt and dead-letter behaviour itself is
    /// covered by the harness's own tests in `crate::consumer`.
    struct NoopLedger;

    impl DeliveryLedger for NoopLedger {
        async fn states(
            &self,
            _group: &str,
            _entry_ids: &[String],
        ) -> Result<HashMap<String, EntryState>, String> {
            Ok(HashMap::new())
        }

        async fn record_attempt(&self, _attempt: &Attempt<'_>) -> Result<i32, String> {
            Ok(1)
        }

        async fn settle(&self, _group: &str, _entry_id: &str) -> Result<(), String> {
            Ok(())
        }

        async fn dead_letter(&self, _spec: &DeadEventSpec<'_>) -> Result<(), String> {
            Ok(())
        }
    }

    /// The harness with this consumer's spec, a working ledger double, and a
    /// handler that is never reached: every entry these tests script is
    /// unparseable, so the harness dead-letters and acks it before any
    /// database call.
    async fn follow_stream(
        pool: &PgPool,
        stream: &mut FakeStream,
        shutdown: &mut watch::Receiver<bool>,
    ) {
        let ledger = NoopLedger;
        let handler = WebhookHandler { pool: pool.clone() };
        Harness::new(&SPEC, "test-replica", &ledger, &handler)
            .follow(stream, shutdown)
            .await;
    }

    #[tokio::test]
    async fn the_pending_list_is_drained_in_pages_before_the_tail() {
        let pool = lazy_pool();
        let mut stream = FakeStream::new(
            vec![
                vec![unparseable("1-0"), unparseable("2-0")],
                vec![unparseable("3-0")],
            ],
            Vec::new(),
        );
        let (_shutdown_tx, mut shutdown) = watch::channel(false);

        follow_stream(&pool, &mut stream, &mut shutdown).await;

        assert_eq!(
            stream.calls,
            vec![
                "pending", "ack:1-0", "ack:2-0", "pending", "ack:3-0",
                // A third read is what proves the drain stops on an empty
                // page rather than on a page count.
                "pending",
                // One claim read after the own PEL is empty; it answers with
                // nothing, so the tail starts.
                "claim", "live",
            ]
        );
    }

    #[tokio::test]
    async fn a_drain_that_cannot_ack_stops_before_the_tail() {
        let pool = lazy_pool();
        let mut stream = FakeStream::new(vec![vec![unparseable("1-0")]], Vec::new());
        stream.ack_succeeds = false;
        let (_shutdown_tx, mut shutdown) = watch::channel(false);

        follow_stream(&pool, &mut stream, &mut shutdown).await;

        // No second `pending` (that would be the spin), no claim and no `live`
        // read: the loop returned so the caller can back off.
        assert_eq!(stream.calls, vec!["pending", "ack:1-0"]);
    }

    #[tokio::test]
    async fn the_live_tail_under_the_group_is_acked_too() {
        let pool = lazy_pool();
        let mut stream = FakeStream::new(
            vec![Vec::new()],
            vec![vec![unparseable("9-0")], vec![unparseable("10-0")]],
        );
        let (_shutdown_tx, mut shutdown) = watch::channel(false);

        follow_stream(&pool, &mut stream, &mut shutdown).await;

        assert_eq!(
            stream.calls,
            vec!["pending", "claim", "live", "ack:9-0", "live", "ack:10-0", "live",]
        );
    }

    #[tokio::test]
    async fn an_entry_abandoned_by_a_dead_consumer_is_claimed_and_acked() {
        let pool = lazy_pool();
        // The own PEL is empty — a restarted process has a new name — so the
        // only way this entry is handled is the takeover pass.
        let mut stream = FakeStream::new(vec![Vec::new()], Vec::new())
            .with_claims(vec![vec![unparseable("7-0")], Vec::new()]);
        let (_shutdown_tx, mut shutdown) = watch::channel(false);

        follow_stream(&pool, &mut stream, &mut shutdown).await;

        // One claim read takes the entry, handling acks it, and the next claim
        // page is empty — the same stop rule as the own-PEL drain — so the
        // tail starts after exactly two claim reads.
        assert_eq!(
            stream.calls,
            vec!["pending", "claim", "ack:7-0", "claim", "live"]
        );
    }

    #[tokio::test]
    async fn a_failed_claim_returns_to_the_reconnect_loop() {
        let pool = lazy_pool();
        let mut stream = FakeStream::new(vec![Vec::new()], Vec::new());
        stream.claim_fails = true;
        let (_shutdown_tx, mut shutdown) = watch::channel(false);

        follow_stream(&pool, &mut stream, &mut shutdown).await;

        // Redis is the failing part; the loop stops instead of reading the
        // tail on a connection that cannot claim.
        assert_eq!(stream.calls, vec!["pending", "claim"]);
    }

    #[test]
    fn the_consumer_name_is_fixed_for_the_process_and_names_it() {
        let first = consumer::consumer_name(CONSUMER_GROUP);
        let second = consumer::consumer_name(CONSUMER_GROUP);
        assert_eq!(first, second, "the name must not change mid-process");
        assert!(
            first.ends_with(&format!("-{}", std::process::id())),
            "the name must carry the pid, got {first}"
        );
        assert_ne!(
            first, "worker",
            "a constant name is the shared PEL this consumer must not have again"
        );
    }

    #[test]
    fn settle_rule_covers_the_matrix() {
        assert_eq!(decide(&SinkResult::Delivered(200), 1, 6), Decision::Done);
        assert_eq!(decide(&SinkResult::Delivered(201), 6, 6), Decision::Done);
        // 410 Gone: permanent unsubscribe, even on attempt 1.
        assert_eq!(decide(&SinkResult::Rejected(410), 1, 6), Decision::Dead);
        assert_eq!(decide(&SinkResult::Rejected(500), 1, 6), Decision::Retry);
        // A missing endpoint and an already-delivered event are both
        // permanent; a throttled receiver is not.
        assert_eq!(decide(&SinkResult::Rejected(404), 1, 6), Decision::Dead);
        assert_eq!(decide(&SinkResult::Rejected(409), 1, 6), Decision::Dead);
        assert_eq!(decide(&SinkResult::Rejected(429), 1, 6), Decision::Retry);
        assert_eq!(decide(&SinkResult::Rejected(408), 1, 6), Decision::Retry);
        assert_eq!(decide(&SinkResult::Rejected(400), 1, 6), Decision::Dead);
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
