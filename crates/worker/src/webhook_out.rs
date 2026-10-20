//! worker/webhook_out — signed webhook delivery for the plugin runtime.
//!
//! Purpose: two halves. The STREAM CONSUMER subscribes to the domain-event
//! stream via a durable consumer group, matches each envelope against the
//! org's enabled webhook subscriptions, and enqueues one durable
//! `webhook.deliver` job per (webhook, event) — ACKing only after the job
//! is durably queued (at-least-once, retries belong to the queue). Each
//! process consumes under its own name, so the group keeps one pending list
//! per process; a restart recovers what its predecessor left by claiming the
//! entries that went idle past [`CLAIM_MIN_IDLE_MS`], never by re-reading the
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

use crate::effects::{EffectApi, EffectOutcome};
use crate::reconnect::Reconnect;
use core_db::webhooks::NewDeliveryAttempt;
use core_domain::webhooks::{backoff_secs, signature_value, MAX_DELIVERY_ATTEMPTS};
use core_events::{Envelope, EVENT_STREAM};
use redis::streams::{
    StreamAutoClaimOptions, StreamAutoClaimReply, StreamId, StreamReadOptions, StreamReadReply,
};
use redis::AsyncCommands;
use serde_json::Value;
use sqlx::PgPool;
use tokio::sync::watch;

/// Durable group; `$` starts at the live tail on first creation so a new
/// deployment never replays history from before it existed.
const CONSUMER_GROUP: &str = "webhooks-out";

/// How long an entry may sit unacked under a consumer name before another
/// process may take it over (`XAUTOCLAIM`).
///
/// Two intervals bound this from above and below.
///
/// Below: handling one entry is a subscription lookup plus one insert per
/// match — milliseconds against the same database the rest of this process
/// uses. A threshold of a minute is three orders of magnitude past that, so a
/// live replica is never mistaken for a dead one merely because it was slow,
/// and a stalled database is a database-wide failure rather than a reason to
/// steal one replica's work.
///
/// Above: the queue's first retry lands one webhook base delay after a failed
/// delivery (`core_domain::webhooks::backoff_secs(1)`), widening to the
/// policy's cap. Waiting twice the first retry means an entry still unacked
/// after a whole retry cycle belongs to a dead consumer, not a busy one — if
/// the replica that took it were alive, its own retry would already have
/// moved. The seconds live in `core_domain::retry`, not in this comment.
///
/// The cost of being wrong is bounded by the dedupe key on the job insert, not
/// by this number: an over-eager claim enqueues into a key that already exists
/// and is refused, so a second replica can waste a read but cannot cause a
/// second delivery. This constant decides how much redundant work a takeover
/// costs, not whether it is correct.
const CLAIM_MIN_IDLE_MS: u64 = 60_000;
const BLOCK_MS: usize = 2_000;
const COUNT: usize = 50;
/// Pages one recovery pass (own pending list, then claimed strangers) will
/// read before it hands control back to the reconnect loop. A crash leaves at
/// most the batch that was in flight — far below this — so the cap only bites
/// on a large backlog: a replica that was down for a while, or a peer that
/// died holding many entries.
const MAX_DRAIN_PAGES: usize = 20;

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

/// The stream operations this consumer needs, behind a trait so the replay
/// path can be exercised without a Redis server in unit tests. Production is
/// [`RedisEventStream`].
///
/// The trait deliberately has no "read the stream from an id" operation. The
/// only reads are [`pending`](EventStream::pending), this consumer's own PEL,
/// [`live`](EventStream::live), `">"`, and [`claim`](EventStream::claim),
/// which only takes entries another consumer already owns. A group-less read
/// from `"0"` is the bug this seam exists to make unrepresentable.
pub trait EventStream: Send {
    /// Entries already delivered to *this consumer* and not yet acked —
    /// `XREADGROUP` with id `"0"`. Never reads entries this group has not
    /// delivered, and never an entry another consumer owns.
    fn pending(
        &mut self,
        count: usize,
    ) -> impl std::future::Future<Output = Result<Vec<StreamId>, String>> + Send;

    /// Entries never delivered to this group — `XREADGROUP` with id `">"`,
    /// blocking up to `block_ms`.
    fn live(
        &mut self,
        count: usize,
        block_ms: usize,
    ) -> impl std::future::Future<Output = Result<Vec<StreamId>, String>> + Send;

    /// Takes over entries pending under *another* consumer that have been idle
    /// for at least `min_idle_ms` — `XAUTOCLAIM`. Ownership moves to this
    /// consumer for every entry returned.
    ///
    /// One page per call, starting the scan at `0-0`; the caller loops. A
    /// claimed entry becomes this consumer's pending entry, so it cannot be
    /// claimed a second time by the same scan and the page cannot repeat.
    fn claim(
        &mut self,
        count: usize,
        min_idle_ms: u64,
    ) -> impl std::future::Future<Output = Result<Vec<StreamId>, String>> + Send;

    /// Removes one entry from the PEL. True when the ack was accepted, which
    /// is the only progress signal [`replay_pending`] trusts.
    fn ack(&mut self, id: &str) -> impl std::future::Future<Output = bool> + Send;
}

/// Production stream: one multiplexed connection against the durable group,
/// under this process's consumer name.
pub struct RedisEventStream {
    conn: redis::aio::MultiplexedConnection,
    name: String,
}

impl RedisEventStream {
    async fn from_client(client: &redis::Client, name: String) -> Result<Self, redis::RedisError> {
        Ok(Self {
            conn: client.get_multiplexed_async_connection().await?,
            name,
        })
    }

    /// Creates the group at `$` on first use. `BUSYGROUP` is not a failure:
    /// the group outlives every process, which is the point of it.
    async fn ensure_group(&mut self) -> Result<(), redis::RedisError> {
        self.conn
            .xgroup_create_mkstream::<&str, &str, &str, ()>(EVENT_STREAM, CONSUMER_GROUP, "$")
            .await
    }
}

impl EventStream for RedisEventStream {
    async fn pending(&mut self, count: usize) -> Result<Vec<StreamId>, String> {
        let options = StreamReadOptions::default()
            .count(count)
            .group(CONSUMER_GROUP, self.name.as_str());
        let reply: StreamReadReply = self
            .conn
            .xread_options(&[EVENT_STREAM], &["0"], &options)
            .await
            .map_err(|error| error.to_string())?;
        Ok(entries(reply))
    }

    async fn live(&mut self, count: usize, block_ms: usize) -> Result<Vec<StreamId>, String> {
        let options = StreamReadOptions::default()
            .block(block_ms)
            .count(count)
            .group(CONSUMER_GROUP, self.name.as_str());
        let reply: StreamReadReply = self
            .conn
            .xread_options(&[EVENT_STREAM], &[">"], &options)
            .await
            .map_err(|error| error.to_string())?;
        Ok(entries(reply))
    }

    async fn claim(&mut self, count: usize, min_idle_ms: u64) -> Result<Vec<StreamId>, String> {
        let options = StreamAutoClaimOptions::default().count(count);
        let name = self.name.clone();
        let reply: StreamAutoClaimReply = self
            .conn
            .xautoclaim_options(
                EVENT_STREAM,
                CONSUMER_GROUP,
                name.as_str(),
                min_idle_ms,
                "0-0",
                options,
            )
            .await
            .map_err(|error| error.to_string())?;
        if !reply.deleted_ids.is_empty() {
            // Trimmed out of the stream while still pending: there is no body
            // left to deliver, and XAUTOCLAIM has already dropped them from the
            // PEL. Logged because a live consumer losing entries to trimming is
            // a retention setting worth seeing.
            tracing::warn!(
                deleted = reply.deleted_ids.len(),
                "webhooks-out claim dropped entries already trimmed from the stream"
            );
        }
        Ok(reply.claimed)
    }

    async fn ack(&mut self, id: &str) -> bool {
        let result: Result<i64, _> = self.conn.xack(EVENT_STREAM, CONSUMER_GROUP, &[id]).await;
        result.is_ok()
    }
}

fn entries(reply: StreamReadReply) -> Vec<StreamId> {
    reply.keys.into_iter().flat_map(|key| key.ids).collect()
}

/// This process's consumer name inside the group, fixed for the process
/// lifetime.
///
/// Why a name per process and not a constant: a consumer name is what the
/// group keys its pending list (PEL) by. With one shared name every replica
/// read the same PEL, so a recovery drain could pick up an entry a live
/// sibling had just taken and hand the same event to two consumers at once.
/// Distinct names give each process its own list, and one entry can only be
/// owned by one consumer at a time.
///
/// Hostname plus pid, because a pid alone collides across replicas: a
/// container usually runs as pid 1, so `worker-1` would name every replica on
/// every host. Hostname is read from the environment (set by Docker, systemd
/// and Kubernetes), then `/etc/hostname`; when neither yields anything the
/// name falls back to a UUID, which trades readability for the guarantee the
/// name exists for — two live processes must never share one.
///
/// Crash recovery does not depend on the name surviving: a restarted process
/// gets a new name, finds its own PEL empty, and takes its predecessor's
/// entries over with [`claim_strangers`] once they go idle. A name that does
/// survive (a recycled pid on the same host) is harmless — it simply finds
/// its own entries in the own-PEL drain instead.
pub fn consumer_name() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| format!("worker-{}-{}", host_label(), std::process::id()))
}

fn host_label() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .into_iter()
        .chain(std::fs::read_to_string("/etc/hostname").ok())
        .map(|host| host.trim().to_string())
        .find(|host| !host.is_empty())
        .unwrap_or_else(|| format!("anon-{}", uuid::Uuid::now_v7().simple()))
}

/// Runs the webhooks-out consumer under a name derived from this host and
/// process. See [`consumer_name`] for why the name is per process.
pub async fn run_webhook_out(pool: PgPool, redis_url: String, shutdown: watch::Receiver<bool>) {
    run_webhook_out_as(pool, redis_url, consumer_name().to_string(), shutdown).await
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
    consumer: String,
    mut shutdown: watch::Receiver<bool>,
) {
    let client = match redis::Client::open(redis_url.as_str()) {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(%error, "invalid redis url; webhooks-out exiting");
            return;
        }
    };
    let mut backoff = Reconnect::consumer();
    loop {
        if *shutdown.borrow() {
            return;
        }
        match RedisEventStream::from_client(&client, consumer.clone()).await {
            Ok(mut stream) => {
                backoff.reset();
                if let Err(error) = stream.ensure_group().await {
                    let msg = error.to_string();
                    if !msg.contains("BUSYGROUP") {
                        tracing::error!(%error, "xgroup create failed");
                        backoff.sleep().await;
                        continue;
                    }
                }
                follow_stream(&pool, &mut stream, &mut shutdown).await;
                tracing::warn!(%consumer, "webhooks-out stream connection ended; reconnecting");
            }
            Err(error) => {
                tracing::error!(%error, "webhooks-out redis connect failed");
            }
        }
        backoff.sleep().await;
    }
}

/// Recovers entries this consumer already owns, then takes over the ones a
/// dead consumer left behind, then follows the live tail under the group.
///
/// The order matters: the own-PEL drain is the cheap, definitely-ours pass,
/// and a database that cannot settle it is not a database to start claiming
/// strangers' work against.
async fn follow_stream<S: EventStream>(
    pool: &PgPool,
    stream: &mut S,
    shutdown: &mut watch::Receiver<bool>,
) {
    if !replay_pending(pool, stream).await {
        return;
    }
    if !claim_strangers(pool, stream).await {
        return;
    }
    loop {
        if *shutdown.borrow() {
            return;
        }
        // The read owns `stream` for its lifetime. Keep it in an inner block
        // so the borrow ends before the batch is handled.
        let result = {
            let mut read = std::pin::pin!(stream.live(COUNT, BLOCK_MS));
            tokio::select! {
                _ = shutdown.changed() => return,
                result = &mut read => result,
            }
        };
        match result {
            Ok(batch) => {
                for entry in &batch {
                    handle_entry(pool, stream, entry).await;
                }
            }
            Err(error) => {
                tracing::error!(%error, "webhooks-out xreadgroup failed");
                return;
            }
        }
    }
}

/// Crash recovery, first pass: re-delivers entries this consumer took delivery
/// of and never acked.
///
/// `XREADGROUP ... "0"` answers with this consumer's own pending entries (its
/// PEL), which is exactly the set a crash, a dropped connection or a failed DB
/// write can leave behind for a process whose name survived. It does not
/// return entries the group has not delivered, and it cannot return another
/// consumer's entries — with one name per process ([`consumer_name`]), a
/// sibling's in-flight work is not visible here at all. That is the change
/// from the shared name this consumer used to run under: a drain could take an
/// entry a live sibling had just read, so the same event reached two
/// consumers, and each enqueued its own `webhook.deliver` job.
///
/// It also cannot return the stream's history. The group-less read from `"0"`
/// that this replaced meant "every entry the stream still holds", so every
/// restart re-enqueued a delivery for every resident event.
///
/// Entries a *dead* consumer left are [`claim_strangers`]' job — a restarted
/// process gets a new name, so its own PEL starts empty and the predecessor's
/// entries would otherwise sit there forever.
///
/// Returns false when no entry could leave the PEL, which tells
/// [`follow_stream`] to return so [`run_webhook_out`] backs off instead of
/// spinning against a database that is down.
async fn replay_pending<S: EventStream>(pool: &PgPool, stream: &mut S) -> bool {
    for _ in 0..MAX_DRAIN_PAGES {
        let batch = match stream.pending(COUNT).await {
            Ok(batch) => batch,
            Err(error) => {
                tracing::error!(%error, "webhooks-out pending read failed; reconnecting");
                return false;
            }
        };
        if batch.is_empty() {
            return true;
        }
        let mut acked = 0_usize;
        for entry in &batch {
            if handle_entry(pool, stream, entry).await {
                acked += 1;
            }
        }
        if acked == 0 {
            // Nothing left the PEL: Postgres is down, or every entry in the
            // page fails the same way. Reading it again would spin.
            tracing::error!(
                pending = batch.len(),
                "webhooks-out pending drain made no progress; backing off"
            );
            return false;
        }
    }
    // The page cap trips on a large backlog — a replica that was down for a
    // while, or one that died holding many entries. Reconnect to continue from
    // where this pass stopped.
    tracing::warn!(
        pages = MAX_DRAIN_PAGES,
        "webhooks-out pending drain hit its page cap; reconnecting to continue"
    );
    false
}

/// Crash recovery, second pass: takes over entries whose owner stopped
/// existing.
///
/// A restarted process gets a new consumer name, so its own PEL is empty and
/// whatever its predecessor had read but not acked would stay pending for
/// ever. Those entries are visibly abandoned — idle past
/// [`CLAIM_MIN_IDLE_MS`] — so `XAUTOCLAIM` hands them to this consumer, and
/// handling them here is the same code path as the own-PEL drain.
///
/// No "make no progress" rule is needed, unlike [`replay_pending`]: a claim
/// moves ownership, so the entry cannot come back in the next page, and the
/// page loop terminates on the first page that claims nothing. A failed handle
/// leaves the entry in *this* consumer's PEL, where the next reconnect's drain
/// picks it up — the failure does not have to be retried inside this pass.
///
/// One bound worth knowing: Redis scans a window of the pending list per call
/// (ten times `COUNT`), so a page can come back empty while an eligible entry
/// sits further along. The pass then ends and the next reconnect scans from the
/// start again. That is a delay rather than starvation, because an entry ahead
/// of the abandoned one is either fresh — and gets acked by the live consumer
/// that owns it — or it is abandoned too and ages into eligibility. Either way
/// the window moves within [`CLAIM_MIN_IDLE_MS`].
///
/// Returns false when the claim call itself fails (Redis is the problem, so
/// stop and reconnect) or when the page cap is reached.
async fn claim_strangers<S: EventStream>(pool: &PgPool, stream: &mut S) -> bool {
    for _ in 0..MAX_DRAIN_PAGES {
        let batch = match stream.claim(COUNT, CLAIM_MIN_IDLE_MS).await {
            Ok(batch) => batch,
            Err(error) => {
                tracing::error!(%error, "webhooks-out claim failed; reconnecting");
                return false;
            }
        };
        if batch.is_empty() {
            return true;
        }
        tracing::info!(
            entries = batch.len(),
            min_idle_ms = CLAIM_MIN_IDLE_MS,
            "webhooks-out claiming entries abandoned by another consumer"
        );
        for entry in &batch {
            handle_entry(pool, stream, entry).await;
        }
    }
    tracing::warn!(
        pages = MAX_DRAIN_PAGES,
        "webhooks-out claim hit its page cap; reconnecting to continue"
    );
    false
}

/// Handles one stream entry. Returns true when the entry left the PEL, which
/// is what [`replay_pending`] counts as progress.
async fn handle_entry<S: EventStream>(pool: &PgPool, stream: &mut S, entry: &StreamId) -> bool {
    let Some(raw) = entry.get::<String>("data") else {
        // No `data` field can never become valid. Leaving it unacked would
        // replay it on every reconnect, so skip it like an unparseable event.
        tracing::warn!(entry_id = %entry.id, "stream entry missing data field; acking to skip");
        return stream.ack(&entry.id).await;
    };
    let Ok(envelope) = serde_json::from_str::<Envelope>(&raw) else {
        tracing::warn!(entry_id = %entry.id, "unparseable event; acking to skip");
        return stream.ack(&entry.id).await;
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
            return false;
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
        // claim from a consumer that died mid-handling, reads this same entry
        // again — and a delivery is an HTTP POST to a third party, so a second
        // job is a second signed body at the receiver. The key is what makes
        // the repeat a no-op instead. A queued, settled or dead-lettered job
        // all keep their key on purpose: from here on the queue owns the
        // retry, and re-enqueuing would fork that history.
        let dedupe_key = format!("{JOB_KIND}:{}:{}", sub.id, envelope.id);
        match core_db::queue::enqueue_deduped(pool, &job, &dedupe_key).await {
            Ok(Some(id)) => {
                tracing::info!(job_id = %id, webhook_id = %sub.id, event_id = %envelope.id, "webhook delivery queued")
            }
            Ok(None) => {
                // Already queued by an earlier read of this entry, here or on
                // another replica. Settling the entry is correct: the delivery
                // exists.
                tracing::info!(
                    webhook_id = %sub.id,
                    event_id = %envelope.id,
                    "webhook delivery already queued; not enqueuing again"
                )
            }
            Err(error) => {
                tracing::error!(%error, entry_id = %entry.id, "enqueue failed; leaving unacked");
                return false;
            }
        }
    }
    // Durably queued (or nothing subscribed) — safe to settle.
    stream.ack(&entry.id).await
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
    use std::collections::VecDeque;

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
        let first = consumer_name();
        let second = consumer_name();
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
