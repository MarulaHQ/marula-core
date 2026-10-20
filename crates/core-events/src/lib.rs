//! core-events — the integration contract (§4.2).
//!
//! Purpose: the versioned event envelope and the outbox→Redis relay. Every
//! consumer of Marula's change feed (websockets, webhooks, plugins, search
//! indexing) reads this one stream.
//!
//! Non-purpose: no business logic, no HTTP, no storage writes other than
//! marking outbox rows published.

use core_db::outbox::OutboxRow;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use uuid::Uuid;

pub mod contract;

/// The channel migration `0068_outbox_notify.sql` fires on every outbox
/// insert (renamed from `meridian_events` by the rebrand, `0114`). The relay
/// LISTENs on it so a committed event wakes the relay at once instead of
/// waiting out the idle poll. The payload is the event identity, but the
/// relay does not read it: the outbox row is the queue, and reading the
/// payload here would only introduce a second source of truth.
pub const NOTIFY_CHANNEL: &str = "marula_events";

/// The wire envelope. `payload_schema_ref` pins the payload contract so
/// consumers can version-gate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Envelope {
    pub id: Uuid,
    #[serde(rename = "type")]
    pub event_type: String,
    pub org_id: Uuid,
    pub entity: String,
    pub entity_id: Option<String>,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    pub payload_schema_ref: String,
    pub payload: serde_json::Value,
}

impl Envelope {
    pub fn from_outbox(row: OutboxRow) -> Self {
        Self {
            id: row.id,
            event_type: row.event_type,
            org_id: row.org_id,
            entity: row.entity,
            entity_id: row.entity_id,
            occurred_at: row.occurred_at,
            payload_schema_ref: row.payload_schema_ref,
            payload: row.payload,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_else(|_| serde_json::json!({}))
    }
}

/// The single Redis stream every event flows through.
pub const EVENT_STREAM: &str = "marula:events";

/// Entries kept in [`EVENT_STREAM`] before the oldest are trimmed.
///
/// The stream is bounded by COUNT rather than by age on purpose. The failure
/// this guards against is a hard memory ceiling, not a stale tail: the shipped
/// Valkey runs `--maxmemory 192mb --maxmemory-policy noeviction`, so an
/// unbounded stream does not degrade gracefully — once the ceiling is reached
/// `XADD` starts failing with OOM and the relay stops publishing, which takes
/// every consumer down at once. A `MINID` window cannot promise headroom for a
/// burst; a count bound can.
///
/// Sizing: an entry is an id, a type and the JSON payload, so call it 1-2 KiB
/// each. 50,000 entries is roughly 50-100 MiB at the top end, which is why the
/// default is not larger on the shipped 192 MiB instance. A deployment with a
/// bigger Valkey raises `MARULA_EVENT_STREAM_MAXLEN`; nothing replays the
/// stream from the head (consumers resume at the tail and reconcile the gap
/// from a REST snapshot), so a longer stream buys history nobody reads.
pub const DEFAULT_STREAM_MAXLEN: usize = 50_000;

/// Floor a configured bound is clamped up to.
///
/// `core-api`'s `REPLAY_BUFFER` is 256 events, and an SSE reconnect asks the
/// outbox — not this stream — for anything it missed. A bound at or below the
/// replay window would still be safe, but it would leave so little room that a
/// brief consumer stall starts costing entries, so the floor is set an order of
/// magnitude above it. A typo or a `0` therefore lands on the floor instead of
/// silently disabling the bound.
pub const MIN_STREAM_MAXLEN: usize = 4_096;

/// The default has to sit above the floor. A default below it would be clamped
/// on every read, which is a configuration that lies about itself. Checked at
/// compile time rather than in a test: neither number is an input, so a test
/// that compared them would be asserting something clippy is right to call a
/// constant.
const _: () = assert!(DEFAULT_STREAM_MAXLEN > MIN_STREAM_MAXLEN);

/// How many entries survive an `XADD`: the stream's trim policy, kept as a
/// type so the clamp is one testable rule instead of an `if` at every call
/// site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamBound {
    maxlen: usize,
}

impl StreamBound {
    /// Resolves a configured entry count, clamping to [`MIN_STREAM_MAXLEN`].
    ///
    /// Clamps rather than rejects: a self-hoster who typos `MAXLEN` should get
    /// a bounded stream and a log line, not a worker that refuses to start.
    pub fn from_entries(configured: usize) -> Self {
        Self {
            maxlen: configured.max(MIN_STREAM_MAXLEN),
        }
    }

    /// The shipped default.
    pub fn default_bound() -> Self {
        Self::from_entries(DEFAULT_STREAM_MAXLEN)
    }

    /// The resolved entry count.
    pub fn maxlen(self) -> usize {
        self.maxlen
    }

    /// Whether a configured value was raised to the floor.
    pub fn was_clamped(configured: usize) -> bool {
        configured < MIN_STREAM_MAXLEN
    }

    /// The `MAXLEN` argument for `XADD`/`XTRIM`. Approximate (`~`) so Redis
    /// trims whole macro-nodes instead of splitting enough to hit the exact
    /// count — the cost of an exact trim is paid on every single publish.
    fn maxlen_arg(self) -> redis::streams::StreamMaxlen {
        redis::streams::StreamMaxlen::Approx(self.maxlen)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EventError {
    #[error("redis: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("db: {0}")]
    Db(#[from] core_db::DbError),
    #[error("sql: {0}")]
    Sql(#[from] sqlx::Error),
}

pub struct Publisher {
    conn: redis::aio::MultiplexedConnection,
    bound: StreamBound,
}

impl Publisher {
    /// Connects with the shipped default bound ([`DEFAULT_STREAM_MAXLEN`]).
    pub async fn connect(redis_url: &str) -> Result<Self, EventError> {
        Self::connect_bounded(redis_url, StreamBound::default_bound()).await
    }

    /// Connects with an explicit bound (`MARULA_EVENT_STREAM_MAXLEN`).
    pub async fn connect_bounded(redis_url: &str, bound: StreamBound) -> Result<Self, EventError> {
        let client = redis::Client::open(redis_url)?;
        let conn = client.get_multiplexed_async_connection().await?;
        Ok(Self { conn, bound })
    }

    /// The bound this publisher trims to.
    pub fn bound(&self) -> StreamBound {
        self.bound
    }

    /// Appends an envelope to the stream, trimming the oldest entries past
    /// [`StreamBound`] in the same command. Consumers reconcile via REST
    /// snapshots, not full replay, so the trim is invisible to them.
    pub async fn publish(&mut self, envelope: &Envelope) -> Result<(), EventError> {
        let id = envelope.id.to_string();
        let json = envelope.to_json().to_string();
        let bound = self.bound;
        let _: String = self
            .conn
            .xadd_maxlen(
                EVENT_STREAM,
                bound.maxlen_arg(),
                "*",
                &[
                    ("id", id.as_str()),
                    ("type", envelope.event_type.as_str()),
                    ("data", json.as_str()),
                ],
            )
            .await?;
        Ok(())
    }

    /// Clears a backlog left behind by a build that did not bound the stream.
    ///
    /// [`publish`](Self::publish) keeps the stream bounded from now on, but a
    /// deployment that has been running unbounded is carrying every event it
    /// ever emitted. Handed to a single `MAXLEN` command that backlog would
    /// hold the Valkey event loop for the whole trim — the outage the bound
    /// exists to prevent — so this trims in bounded chunks instead, and the
    /// caller decides how many passes to spend.
    ///
    /// Returns the entries removed and whether the stream is now inside the
    /// bound. `settled` is false when the pass cap ran out first, which means
    /// the next publish is still facing a large trim and the caller should say
    /// so rather than reporting a clean startup.
    pub async fn drain_backlog(
        &mut self,
        chunk: usize,
        passes: usize,
    ) -> Result<(u64, bool), EventError> {
        use redis::streams::{StreamTrimOptions, StreamTrimmingMode};

        let mut removed = 0_u64;
        for _ in 0..passes.max(1) {
            let options =
                StreamTrimOptions::maxlen(StreamTrimmingMode::Approx, self.bound.maxlen())
                    .limit(chunk.max(1));
            let trimmed: i64 = self.conn.xtrim_options(EVENT_STREAM, &options).await?;
            let trimmed = trimmed.max(0) as u64;
            removed += trimmed;
            if trimmed == 0 {
                // Nothing left that an approximate trim will take: either the
                // stream is inside the bound, or the remainder is inside one
                // macro-node, which `~` deliberately will not split.
                return Ok((removed, true));
            }
        }
        Ok((removed, false))
    }
}

/// Relay loop: drains unpublished outbox rows and publishes them to the
/// stream inside one transaction per batch (fetch → publish → mark).
///
/// Telemetry (TASKLIST 2.18): one `relay.pass` span per batch carrying the
/// batch size, plus `marula.relay.batch_size` and the outbox-lag gauge
/// (age of the oldest unpublished row, host-clock vs outbox timestamp —
/// skew caveat documented in core-otel module docs).
pub async fn relay_once(
    pool: &sqlx::PgPool,
    publisher: &mut Publisher,
    batch: i64,
) -> Result<usize, EventError> {
    use tracing::Instrument as _;
    let span = tracing::info_span!("relay.pass", batch_size = batch);
    relay_inner(pool, publisher, batch).instrument(span).await
}

async fn relay_inner(
    pool: &sqlx::PgPool,
    publisher: &mut Publisher,
    batch: i64,
) -> Result<usize, EventError> {
    let mut tx = pool.begin().await?;
    let rows = core_db::outbox::fetch_unpublished(&mut *tx, batch).await?;
    if rows.is_empty() {
        return Ok(0);
    }
    // Outbox-lag gauge + batch histogram: computed BEFORE rows are consumed
    // by the publish loop below.
    let lag_ms = rows
        .iter()
        .map(|r| r.occurred_at)
        .min()
        .map(|oldest| (chrono::Utc::now() - oldest).num_milliseconds().max(0) as u64);
    core_otel::metrics::relay_pass(rows.len() as u64, lag_ms);
    let mut ids = Vec::with_capacity(rows.len());
    for row in rows {
        let envelope = Envelope::from_outbox(row);
        publisher.publish(&envelope).await?;
        ids.push(envelope.id);
    }
    core_db::outbox::mark_published(&mut *tx, &ids).await?;
    tx.commit().await?;
    Ok(ids.len())
}

/// How the relay waits between passes.
///
/// `safety_poll` is the backstop: it bounds how long a missed notification
/// (a reconnect window, a listener that never started) can delay delivery.
/// `wakeup` is the fast path. `None` keeps the loop polling only, which is
/// the shape the relay had before the trigger existed — kept because the
/// poll rate is a deployment parameter, and because it is the baseline the
/// wakeup is measured against.
pub struct RelayPacing {
    pub safety_poll: Duration,
    pub wakeup: Option<Arc<Notify>>,
}

impl RelayPacing {
    /// Poll only, with `safety_poll` between idle passes.
    pub fn polling(safety_poll: Duration) -> Self {
        Self {
            safety_poll,
            wakeup: None,
        }
    }

    /// Wake on `wakeup`, with `safety_poll` as the backstop.
    pub fn notified(safety_poll: Duration, wakeup: Arc<Notify>) -> Self {
        Self {
            safety_poll,
            wakeup: Some(wakeup),
        }
    }

    /// Idle wait: until a notification arrives, or the safety poll expires.
    ///
    /// The permit matters. `notify_one` stores one when no waiter is
    /// registered, so a notification that lands while a pass is running is
    /// not lost: the next wait returns at once instead of sleeping through
    /// the safety poll. A burst therefore drains at the batch rate rather
    /// than one batch per poll interval.
    async fn wait(&self) {
        match &self.wakeup {
            Some(wakeup) => {
                let _ = tokio::time::timeout(self.safety_poll, wakeup.notified()).await;
            }
            None => tokio::time::sleep(self.safety_poll).await,
        }
    }
}

/// Long-running relay task with backoff on error.
///
/// A non-empty pass loops without waiting, so a backlog drains at once; an
/// empty pass waits on `pacing`. Errors back off separately and never
/// consume a wakeup permit.
pub async fn run_relay(
    pool: sqlx::PgPool,
    mut publisher: Publisher,
    batch: i64,
    pacing: RelayPacing,
) -> Result<(), EventError> {
    let policy = core_domain::retry::policy(core_domain::retry::Subsystem::Relay);
    let mut failed_passes = 0_u32;
    loop {
        match relay_once(&pool, &mut publisher, batch).await {
            Ok(0) => {
                failed_passes = 0;
                pacing.wait().await;
            }
            Ok(_) => {
                failed_passes = 0;
            }
            Err(e) => {
                tracing::error!(error = %e, "event relay failed");
                failed_passes = failed_passes.saturating_add(1);
                tokio::time::sleep(policy.delay(failed_passes, core_domain::retry::sample())).await;
            }
        }
    }
}

/// Long-running LISTEN task for the relay, on its own dedicated connection.
///
/// Reconnects with backoff. A notification carries no work of its own — the
/// outbox row is the queue and the relay reads it — so a notification lost
/// during a reconnect costs at most one safety poll, which is why this task
/// needs no resumable offset and no cursor.
pub async fn run_wakeup_listener(database_url: String, wakeup: Arc<Notify>) {
    let policy = core_domain::retry::policy(core_domain::retry::Subsystem::Relay);
    let mut failed_passes = 0_u32;
    loop {
        match wakeup_session(&database_url, &wakeup).await {
            Ok(()) => {
                // Unreachable: wakeup_session only returns on error.
                failed_passes = 0;
            }
            Err(e) => {
                tracing::warn!(error = %e, "relay wakeup listener disconnected; reconnecting");
                failed_passes = failed_passes.saturating_add(1);
                tokio::time::sleep(policy.delay(failed_passes, core_domain::retry::sample())).await;
            }
        }
    }
}

async fn wakeup_session(database_url: &str, wakeup: &Notify) -> Result<(), sqlx::Error> {
    let mut listener = sqlx::postgres::PgListener::connect(database_url).await?;
    listener.listen(NOTIFY_CHANNEL).await?;
    tracing::info!(
        channel = NOTIFY_CHANNEL,
        "outbox relay wakeup listener connected"
    );
    loop {
        listener.recv().await?;
        wakeup.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_serializes_with_type_field() {
        let e = Envelope {
            id: Uuid::now_v7(),
            event_type: "ticket.created".into(),
            org_id: Uuid::now_v7(),
            entity: "ticket".into(),
            entity_id: Some(Uuid::now_v7().to_string()),
            occurred_at: chrono::Utc::now(),
            payload_schema_ref: "core-domain/v1".into(),
            payload: serde_json::json!({"title": "x"}),
        };
        let v = e.to_json();
        assert_eq!(v["type"], "ticket.created");
        let round: Envelope = serde_json::from_value(v).unwrap();
        assert_eq!(round, e);
    }

    /// The default has to sit above the floor, or the shipped configuration
    /// would be clamped at startup and log a warning on every clean install.
    #[test]
    fn default_bound_is_above_the_floor() {
        let bound = StreamBound::default_bound();
        assert_eq!(bound.maxlen(), DEFAULT_STREAM_MAXLEN);
        assert!(!StreamBound::was_clamped(DEFAULT_STREAM_MAXLEN));
    }

    /// A bound that is zero, or below the SSE replay window, must land on the
    /// floor rather than disabling the trim. An unbounded stream is the bug
    /// this constant exists to prevent, so "0 means unlimited" is the one
    /// reading that must not survive.
    #[test]
    fn a_bound_below_the_floor_clamps_up_instead_of_disabling_the_trim() {
        for configured in [0_usize, 1, 255, 256, MIN_STREAM_MAXLEN - 1] {
            let bound = StreamBound::from_entries(configured);
            assert_eq!(
                bound.maxlen(),
                MIN_STREAM_MAXLEN,
                "configured {configured} must clamp to the floor"
            );
            assert!(StreamBound::was_clamped(configured));
        }
        // At and above the floor the value is taken as given.
        assert_eq!(
            StreamBound::from_entries(MIN_STREAM_MAXLEN).maxlen(),
            MIN_STREAM_MAXLEN
        );
        assert!(!StreamBound::was_clamped(MIN_STREAM_MAXLEN));
        assert_eq!(StreamBound::from_entries(1_000_000).maxlen(), 1_000_000);
    }

    /// The trim argument is approximate on purpose: an exact trim splits
    /// macro-nodes on every write, which is work Redis should not do per
    /// event when `~` gets within a node boundary for free.
    #[test]
    fn the_trim_argument_is_an_approximate_maxlen() {
        // Above the floor the configured number passes through as given.
        let bound = StreamBound::from_entries(20_000);
        assert_eq!(
            bound.maxlen_arg(),
            redis::streams::StreamMaxlen::Approx(20_000)
        );
        // A value that clamps is approximate as well: the mode belongs to the
        // bound, not to the number that survived the clamp. Asserting the
        // un-clamped 1234 here is what this test got wrong the first time.
        assert_eq!(
            StreamBound::from_entries(1_234).maxlen_arg(),
            redis::streams::StreamMaxlen::Approx(MIN_STREAM_MAXLEN)
        );
    }

    /// A notification that lands while a pass is running must not be lost.
    /// `notify_one` stores a permit when nobody is waiting, so the next wait
    /// returns at once instead of sleeping through a 30-second safety poll.
    /// This is the property a burst depends on.
    #[tokio::test]
    async fn a_notification_that_arrives_before_the_wait_is_not_lost() {
        let wakeup = Arc::new(Notify::new());
        wakeup.notify_one();
        let pacing = RelayPacing::notified(Duration::from_secs(30), Arc::clone(&wakeup));
        let started = std::time::Instant::now();
        pacing.wait().await;
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "a stored permit must wake the wait immediately, took {:?}",
            started.elapsed()
        );
    }

    /// Without a wakeup the wait is exactly the safety poll, which is the
    /// shape the relay had before the trigger and the baseline the wakeup is
    /// measured against.
    #[tokio::test]
    async fn polling_waits_the_safety_poll() {
        let pacing = RelayPacing::polling(Duration::from_millis(50));
        let started = std::time::Instant::now();
        pacing.wait().await;
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(45),
            "an empty polling wait must not spin, took {elapsed:?}"
        );
        assert!(elapsed < Duration::from_secs(5), "took {elapsed:?}");
    }
}
