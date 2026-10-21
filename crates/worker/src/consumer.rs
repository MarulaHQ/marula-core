//! worker/consumer — the one stream-consumer harness.
//!
//! Purpose: every consumer of `marula:events` runs the same loop shape, and
//! this module is that shape. A consumer supplies a group name, a page size and
//! one handler; the harness supplies the parts that are easy to get subtly
//! wrong once and impossible to review eight times:
//!
//! * **A consumer name per replica.** `format!("{group}-{host}-{pid}")`. A
//!   consumer name is what Valkey keys a pending list (PEL) by, so a shared
//!   name lets one replica drain an entry a live sibling is mid-handling and
//!   hand the same event to two consumers.
//! * **Own-PEL recovery, then `XAUTOCLAIM`.** A restarted process drains
//!   entries already delivered to its own name, then takes over entries whose
//!   owner went away (idle past [`CLAIM_MIN_IDLE_MS`]). Both passes are bounded
//!   by [`MAX_DRAIN_PAGES`]: a huge backlog reconnects and continues instead of
//!   holding the loop for minutes.
//! * **Live retry.** A failed entry stays in the PEL and is retried in the
//!   same process once its delay from `core_domain::retry` has elapsed
//!   (`Subsystem::Consumer`). Before this harness the only retry was a restart.
//! * **Per-entry attempts and a dead-letter queue.** The attempt count lives in
//!   `consumer_deliveries`; an entry that exhausts [`DEFAULT_ENTRY_ATTEMPTS`]
//!   is written to `dead_events` with its event identity and raw payload and
//!   is acked, so a poison entry becomes visible instead of holding the tail.
//! * **Reconnect with the R1 policy.** `Reconnect::consumer()` owns the
//!   backoff; this module never invents a second schedule.
//!
//! Non-purpose: no event parsing beyond the envelope (the handler decides what
//! an event means), no job enqueueing, no domain writes.

use crate::reconnect::Reconnect;
use crate::stream_health::GroupSample;
use core_db::consumer::{self as deliveries, Attempt, DeadEventSpec, DeliveryState};
use core_domain::retry::{self, Subsystem};
use core_events::{Envelope, EVENT_STREAM};
use redis::streams::{
    StreamAutoClaimOptions, StreamAutoClaimReply, StreamId, StreamReadOptions, StreamReadReply,
};
use redis::AsyncCommands;
use sqlx::PgPool;
use std::collections::HashMap;
use std::future::Future;
use std::time::{Duration, Instant};
use tokio::sync::watch;

/// Blocking-`XREADGROUP` wait per poll. Bounds shutdown latency: a consumer
/// that is idle answers a shutdown within this window.
pub const BLOCK_MS: usize = 2_000;

/// How long an entry may sit unacked under one consumer name before another
/// process may take it over with `XAUTOCLAIM`.
///
/// Below: handling one entry is a database read plus, at most, a job insert —
/// milliseconds against the same database the rest of the process uses. A
/// minute is orders of magnitude past that, so a live replica is never
/// mistaken for a dead one merely because it was slow.
///
/// Above: the consumer retry policy's cap is ten seconds
/// (`core_domain::retry`), so an entry still unacked a full minute later
/// belongs to a consumer that is gone, not one that is busy.
///
/// Being wrong is bounded, not free: an over-eager claim redoes work, which is
/// why handlers that enqueue use `queue::enqueue_deduped` and why the ones
/// that write use unique keys.
pub const CLAIM_MIN_IDLE_MS: u64 = 60_000;

/// Pages one recovery pass reads before it hands control back to the reconnect
/// loop. A crash leaves at most the batch that was in flight; the cap only
/// bites on a real backlog (a replica that was down, or a peer that died
/// holding many entries).
pub const MAX_DRAIN_PAGES: usize = 20;

/// How many failures one stream entry gets before it is dead-lettered.
///
/// The queue's budget, not a second number: a unit of work handed to a stream
/// entry is usually a job enqueue, and an operator who widens the queue's
/// attempts should not have to find a second budget to widen with it. The
/// retry DELAYS come from `Subsystem::Consumer`; this is only how many tries
/// the same work is worth. After them, the entry is *visible* — `dead_events`
/// carries its envelope and its last error — rather than silently redelivered
/// forever.
fn entry_attempt_budget() -> i32 {
    retry::attempt_limit_i32(Subsystem::Queue)
}

/// The static shape of one consumer.
#[derive(Debug, Clone, Copy)]
pub struct ConsumerSpec {
    /// The consumer group. One per subsystem, never shared.
    pub group: &'static str,
    /// Entries per read.
    pub count: usize,
    /// Blocking wait per live read.
    pub block_ms: usize,
    /// Idle time before another consumer's entry may be claimed.
    pub claim_min_idle_ms: u64,
    /// Pages one recovery or claim pass may read.
    pub max_drain_pages: usize,
}

impl ConsumerSpec {
    /// A spec with the harness defaults and a subsystem's own group name and
    /// page size.
    #[must_use]
    pub const fn new(group: &'static str, count: usize) -> Self {
        Self {
            group,
            count,
            block_ms: BLOCK_MS,
            claim_min_idle_ms: CLAIM_MIN_IDLE_MS,
            max_drain_pages: MAX_DRAIN_PAGES,
        }
    }
}

/// What a handler decided about one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryOutcome {
    /// The work is durably done (or there was nothing to do): settle and ack.
    Settled,
    /// A transient failure. The entry stays in the PEL and is retried after
    /// the consumer policy's delay, up to the attempt budget.
    Retry(String),
    /// A permanent failure that retrying cannot fix. Dead-letter now.
    Dead(String),
}

/// One parsed entry handed to a handler.
#[derive(Debug, Clone)]
pub struct Delivery {
    /// The stream id. The DLQ key for this entry.
    pub entry_id: String,
    pub envelope: Envelope,
    /// The exact bytes the entry carried, kept for the dead-letter record.
    pub raw: String,
}

/// The consumer half of a subsystem: what to do with a page of events.
///
/// `handle` gets the entries that are due, in stream order, and must answer
/// with exactly one outcome each, in the same order. It is a page rather than a
/// single entry because some consumers batch (the search indexer flushes one
/// request per org) and splitting that would turn one HTTP call into two
/// hundred.
pub trait EntryHandler: Send + Sync {
    fn handle(&self, batch: &[Delivery]) -> impl Future<Output = Vec<EntryOutcome>> + Send;

    /// Runs when the live read timed out with nothing to do. Periodic work that
    /// belongs to the consumer's loop (the CSAT deferred sweep) goes here
    /// rather than in its own timer.
    fn idle(&self) -> impl Future<Output = ()> + Send {
        async {}
    }
}

/// One entry's state, as the harness reads it before handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryState {
    pub due: bool,
    pub dead: bool,
    pub attempts: i32,
}

/// Where per-entry attempts live. A trait so the loop can be exercised without
/// Postgres; production is [`PgLedger`].
pub trait DeliveryLedger: Send + Sync {
    /// States for one page's entry ids. An id with no state has never failed
    /// and is due.
    fn states(
        &self,
        group: &str,
        entry_ids: &[String],
    ) -> impl Future<Output = Result<HashMap<String, EntryState>, String>> + Send;

    /// Records one failure; answers the attempt count including it.
    fn record_attempt(
        &self,
        attempt: &Attempt<'_>,
    ) -> impl Future<Output = Result<i32, String>> + Send;

    /// Marks a previously failed entry settled. A no-op for an entry that
    /// never had a row.
    fn settle(
        &self,
        group: &str,
        entry_id: &str,
    ) -> impl Future<Output = Result<(), String>> + Send;

    /// Records the abandonment.
    fn dead_letter(
        &self,
        spec: &DeadEventSpec<'_>,
    ) -> impl Future<Output = Result<(), String>> + Send;
}

/// The Postgres ledger: `consumer_deliveries` + `dead_events`.
#[derive(Clone)]
pub struct PgLedger {
    pool: PgPool,
}

impl PgLedger {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl DeliveryLedger for PgLedger {
    async fn states(
        &self,
        group: &str,
        entry_ids: &[String],
    ) -> Result<HashMap<String, EntryState>, String> {
        let rows: Vec<DeliveryState> = deliveries::delivery_states(&self.pool, group, entry_ids)
            .await
            .map_err(|error| error.to_string())?;
        Ok(rows
            .into_iter()
            .map(|row| {
                (
                    row.entry_id,
                    EntryState {
                        due: row.due,
                        dead: row.dead,
                        attempts: row.attempts,
                    },
                )
            })
            .collect())
    }

    async fn record_attempt(&self, attempt: &Attempt<'_>) -> Result<i32, String> {
        deliveries::record_attempt(&self.pool, attempt)
            .await
            .map_err(|error| error.to_string())
    }

    async fn settle(&self, group: &str, entry_id: &str) -> Result<(), String> {
        deliveries::settle(&self.pool, group, entry_id)
            .await
            .map_err(|error| error.to_string())
    }

    async fn dead_letter(&self, spec: &DeadEventSpec<'_>) -> Result<(), String> {
        deliveries::dead_letter(&self.pool, spec)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

// --- the stream seam ---------------------------------------------------------

/// The stream operations a consumer needs, behind a trait so the loop can be
/// exercised without a Valkey server in unit tests.
///
/// There is deliberately no "read the stream from an id" operation. The reads
/// are [`pending`](EventStream::pending) (this consumer's own PEL, therefore
/// group-scoped), [`live`](EventStream::live) (`>`), and
/// [`claim`](EventStream::claim) (entries another consumer already owns). A
/// group-less read from `"0"` — the bug that replayed the stream's whole
/// history on every restart — is unrepresentable here.
pub trait EventStream: Send {
    /// Entries already delivered to *this consumer* and not yet acked —
    /// `XREADGROUP` with id `"0"`.
    fn pending(
        &mut self,
        count: usize,
    ) -> impl Future<Output = Result<Vec<StreamId>, String>> + Send;

    /// Entries never delivered to this group — `XREADGROUP` with `">"`,
    /// blocking up to `block_ms`.
    fn live(
        &mut self,
        count: usize,
        block_ms: usize,
    ) -> impl Future<Output = Result<Vec<StreamId>, String>> + Send;

    /// Takes over entries pending under *another* consumer that have been idle
    /// for at least `min_idle_ms` — `XAUTOCLAIM`.
    fn claim(
        &mut self,
        count: usize,
        min_idle_ms: u64,
    ) -> impl Future<Output = Result<Vec<StreamId>, String>> + Send;

    /// Removes one entry from the PEL. True when Valkey accepted the ack: the
    /// only progress signal the recovery pass trusts.
    fn ack(&mut self, id: &str) -> impl Future<Output = bool> + Send;

    /// This group's backlog. `None` means the seam cannot tell (a test double
    /// or an unreachable server), and the caller emits nothing rather than a
    /// zero that would read as "caught up".
    fn health(&mut self) -> impl Future<Output = Option<GroupSample>> + Send {
        async { None }
    }
}

/// Production stream: one multiplexed connection against the consumer's group,
/// under this replica's consumer name.
pub struct RedisEventStream {
    conn: redis::aio::MultiplexedConnection,
    group: &'static str,
    name: String,
}

impl RedisEventStream {
    /// Connects under `name` inside `group`.
    pub async fn connect(
        client: &redis::Client,
        group: &'static str,
        name: String,
    ) -> Result<Self, String> {
        Ok(Self {
            conn: client
                .get_multiplexed_async_connection()
                .await
                .map_err(|error| error.to_string())?,
            group,
            name,
        })
    }

    /// Creates the group at `$` on first use, so a new deployment never
    /// replays the stream's history. `BUSYGROUP` is not a failure: the group
    /// outlives every process, which is the point of it.
    async fn ensure_group(&mut self) -> Result<(), String> {
        match self
            .conn
            .xgroup_create_mkstream::<&str, &str, &str, ()>(EVENT_STREAM, self.group, "$")
            .await
        {
            Ok(()) => Ok(()),
            Err(error) if error.to_string().contains("BUSYGROUP") => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

impl EventStream for RedisEventStream {
    async fn pending(&mut self, count: usize) -> Result<Vec<StreamId>, String> {
        let options = StreamReadOptions::default()
            .count(count)
            .group(self.group, self.name.as_str());
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
            .group(self.group, self.name.as_str());
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
                self.group,
                name.as_str(),
                min_idle_ms,
                "0-0",
                options,
            )
            .await
            .map_err(|error| error.to_string())?;
        if !reply.deleted_ids.is_empty() {
            // Trimmed out of the stream while still pending: there is no body
            // left to handle, and XAUTOCLAIM has already dropped them from the
            // PEL. Logged because a live consumer losing entries to trimming is
            // a retention setting worth seeing.
            tracing::warn!(
                group = self.group,
                deleted = reply.deleted_ids.len(),
                "claim dropped entries already trimmed from the stream"
            );
        }
        Ok(reply.claimed)
    }

    async fn ack(&mut self, id: &str) -> bool {
        let result: Result<i64, _> = self.conn.xack(EVENT_STREAM, self.group, &[id]).await;
        result.is_ok()
    }

    async fn health(&mut self) -> Option<GroupSample> {
        crate::stream_health::read_group(&mut self.conn, self.group).await
    }
}

fn entries(reply: StreamReadReply) -> Vec<StreamId> {
    reply.keys.into_iter().flat_map(|key| key.ids).collect()
}

// --- consumer identity -------------------------------------------------------

/// The host half of a consumer name. Read from the environment (set by Docker,
/// systemd and Kubernetes), then `/etc/hostname`; when neither yields anything
/// a random label keeps two processes from sharing a name, trading readability
/// for the guarantee the name exists for.
pub fn host_label() -> &'static str {
    static HOST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HOST.get_or_init(|| {
        std::env::var("HOSTNAME")
            .ok()
            .into_iter()
            .chain(std::fs::read_to_string("/etc/hostname").ok())
            .map(|host| host.trim().to_string())
            .find(|host| !host.is_empty())
            .unwrap_or_else(|| format!("anon-{}", uuid::Uuid::now_v7().simple()))
    })
}

/// This process's consumer name inside one group.
///
/// Group-prefixed because the name is a PEL key inside a group: a name that
/// repeats across groups is harmless to Valkey but ambiguous in
/// `consumer_deliveries`, which stores the group and the consumer side by side
/// and should read as "this group's replica", not "some replica".
///
/// Crash recovery does not depend on the name surviving: a restarted process
/// gets a new name, finds its own PEL empty, and takes its predecessor's entries
/// over with `XAUTOCLAIM` once they go idle.
#[must_use]
pub fn consumer_name(group: &str) -> String {
    format!("{group}-{}-{}", host_label(), std::process::id())
}

// --- the harness -------------------------------------------------------------

/// What one page did: how many entries were attempted, how many left the PEL,
/// and whether any of them was left pending for another attempt.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PageResult {
    attempted: usize,
    acked: usize,
    retried: bool,
}

/// What one pass over the pending list did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrainOutcome {
    /// The pending list is empty: nothing left to recover.
    Empty,
    /// Entries remain, but none is due yet — they are inside their retry
    /// delay. This is not a fault: the caller comes back after the next read.
    Deferred,
    /// Entries were attempted and nothing left the PEL: the database or the ack
    /// is the failing part, so the caller backs off instead of spinning.
    Stuck,
}

/// The loop, bound to one consumer's spec, name, ledger and handler.
pub struct Harness<'a, L: DeliveryLedger, H: EntryHandler> {
    spec: &'a ConsumerSpec,
    name: &'a str,
    ledger: &'a L,
    handler: &'a H,
}

impl<'a, L: DeliveryLedger, H: EntryHandler> Harness<'a, L, H> {
    #[must_use]
    pub fn new(spec: &'a ConsumerSpec, name: &'a str, ledger: &'a L, handler: &'a H) -> Self {
        Self {
            spec,
            name,
            ledger,
            handler,
        }
    }

    /// Recovers what this group owes, then follows the tail until shutdown or a
    /// stream error. Returns when the caller should reconnect.
    pub async fn follow<S: EventStream>(
        &self,
        stream: &mut S,
        shutdown: &mut watch::Receiver<bool>,
    ) {
        // Order matters: the own-PEL drain is the cheap, definitely-ours pass,
        // and a database that cannot settle it is not a database to start
        // claiming strangers' work against. A `Deferred` answer (entries inside
        // their retry delay) is not a fault and does not stop the loop.
        let recovered = self.drain(stream).await;
        if recovered == DrainOutcome::Stuck {
            tracing::error!(
                group = self.spec.group,
                "pending drain made no progress; backing off"
            );
            return;
        }
        if !self.claim(stream).await {
            return;
        }

        // True while entries this process failed on are still in its PEL. It
        // stays true across an idle read: an entry recovered at startup may
        // still be inside its retry delay, and dropping the flag because
        // nothing is due yet would leave it pending until the next restart —
        // the bug this harness exists to remove.
        let mut retry_pending = recovered == DrainOutcome::Deferred;
        let mut last_claim = Instant::now();
        loop {
            if *shutdown.borrow() {
                return;
            }
            // The read owns `stream` for its lifetime. Keep it in an inner
            // block so the borrow ends before the page is handled.
            let result = {
                let mut read = std::pin::pin!(stream.live(self.spec.count, self.spec.block_ms));
                tokio::select! {
                    _ = shutdown.changed() => return,
                    result = &mut read => result,
                }
            };
            let batch = match result {
                Ok(batch) => batch,
                Err(error) => {
                    tracing::error!(group = self.spec.group, %error, "xreadgroup failed");
                    return;
                }
            };
            if batch.is_empty() {
                self.handler.idle().await;
            } else {
                let page = self.handle_page(stream, &batch).await;
                retry_pending |= page.retried || page.attempted > page.acked;
            }

            // Consumer backlog: PEL depth, lag and the age of the oldest unacked
            // entry for THIS group, read after the page. The trait default
            // answers `None`, so a test double emits nothing instead of a
            // fabricated zero.
            if let Some(sample) = stream.health().await {
                crate::stream_health::emit(self.spec.group, sample);
            }

            // An entry that failed stays in this process's PEL and is retried
            // here, after its policy delay, instead of waiting for a restart —
            // the reason this harness exists. The flag clears only when the PEL
            // is empty; `Deferred` keeps it set for the next read.
            if retry_pending {
                match self.drain(stream).await {
                    DrainOutcome::Empty => retry_pending = false,
                    DrainOutcome::Deferred => {}
                    DrainOutcome::Stuck => {
                        // Attempted and nothing settled: the entries are now
                        // deferred by the attempt write, and the next blocking
                        // read is the pause before they are due.
                        tracing::warn!(
                            group = self.spec.group,
                            "pending retry made no progress; retrying after the next read"
                        );
                    }
                }
            }

            // Take over work from consumers that are gone, at most once per
            // claim window so a healthy group does not pay a scan per batch.
            if last_claim.elapsed() >= Duration::from_millis(self.spec.claim_min_idle_ms) {
                last_claim = Instant::now();
                if !self.claim(stream).await {
                    return;
                }
            }
        }
    }

    /// Drains this consumer's own PEL, page by page, bounded by
    /// [`ConsumerSpec::max_drain_pages`].
    async fn drain<S: EventStream>(&self, stream: &mut S) -> DrainOutcome {
        for _ in 0..self.spec.max_drain_pages {
            let batch = match stream.pending(self.spec.count).await {
                Ok(batch) => batch,
                Err(error) => {
                    tracing::error!(group = self.spec.group, %error, "pending read failed");
                    return DrainOutcome::Stuck;
                }
            };
            if batch.is_empty() {
                return DrainOutcome::Empty;
            }
            let page = self.handle_page(stream, &batch).await;
            if page.acked > 0 {
                continue;
            }
            if page.attempted == 0 {
                return DrainOutcome::Deferred;
            }
            return DrainOutcome::Stuck;
        }
        tracing::warn!(
            group = self.spec.group,
            pages = self.spec.max_drain_pages,
            "pending drain hit its page cap; reconnecting to continue"
        );
        DrainOutcome::Deferred
    }

    /// Takes over entries another consumer abandoned. False means the claim
    /// call failed or the page cap was hit; the caller backs off.
    async fn claim<S: EventStream>(&self, stream: &mut S) -> bool {
        for _ in 0..self.spec.max_drain_pages {
            let batch = match stream
                .claim(self.spec.count, self.spec.claim_min_idle_ms)
                .await
            {
                Ok(batch) => batch,
                Err(error) => {
                    tracing::error!(group = self.spec.group, %error, "claim failed");
                    return false;
                }
            };
            if batch.is_empty() {
                return true;
            }
            tracing::info!(
                group = self.spec.group,
                entries = batch.len(),
                min_idle_ms = self.spec.claim_min_idle_ms,
                "claiming entries abandoned by another consumer"
            );
            let _ = self.handle_page(stream, &batch).await;
        }
        tracing::warn!(
            group = self.spec.group,
            pages = self.spec.max_drain_pages,
            "claim hit its page cap; reconnecting to continue"
        );
        false
    }

    /// Handles one page and reports what left the PEL.
    async fn handle_page<S: EventStream>(
        &self,
        stream: &mut S,
        entries: &[StreamId],
    ) -> PageResult {
        let ids: Vec<String> = entries.iter().map(|entry| entry.id.clone()).collect();
        // A ledger that cannot answer is treated as "never failed": the entry
        // is handled anyway, and a successful handle still acks it. Refusing to
        // work because bookkeeping is down would turn a database blip into a
        // stalled stream.
        let states = match self.ledger.states(self.spec.group, &ids).await {
            Ok(states) => states,
            Err(error) => {
                tracing::warn!(
                    group = self.spec.group,
                    %error,
                    "delivery state read failed; treating the page as fresh"
                );
                HashMap::new()
            }
        };

        let mut due: Vec<(String, Envelope, String)> = Vec::new();
        let mut page = PageResult::default();
        for entry in entries {
            let state = states.get(&entry.id);
            if let Some(state) = state {
                if state.dead {
                    // Dead-lettered already: the record exists, so the only
                    // remaining work is to get the entry out of the PEL.
                    if stream.ack(&entry.id).await {
                        page.acked += 1;
                    }
                    continue;
                }
                if !state.due {
                    // Inside its retry delay. Leaving it pending is the retry.
                    continue;
                }
            }
            match parse_entry(entry) {
                Ok((envelope, raw)) => {
                    page.attempted += 1;
                    due.push((entry.id.clone(), envelope, raw));
                }
                Err(reason) => {
                    page.attempted += 1;
                    // A body that will never parse cannot be retried into
                    // validity: dead-letter it now, with the raw bytes when
                    // they were readable at all.
                    let raw = entry.get::<String>("data");
                    if self
                        .dead_letter(&entry.id, None, raw.as_deref(), 1, &reason)
                        .await
                        && stream.ack(&entry.id).await
                    {
                        page.acked += 1;
                    }
                }
            }
        }

        if due.is_empty() {
            return page;
        }

        let payloads: HashMap<String, String> = due
            .iter()
            .map(|(id, _, raw)| (id.clone(), raw.clone()))
            .collect();
        let batch: Vec<Delivery> = due
            .into_iter()
            .map(|(entry_id, envelope, raw)| Delivery {
                entry_id,
                envelope,
                raw,
            })
            .collect();
        let outcomes = self.handler.handle(&batch).await;
        if outcomes.len() != batch.len() {
            // A handler that answers the wrong length is a bug, but dropping an
            // entry is worse: the missing ones are retried, which is the safe
            // direction.
            tracing::error!(
                group = self.spec.group,
                expected = batch.len(),
                got = outcomes.len(),
                "handler returned the wrong number of outcomes; retrying the rest"
            );
        }

        for (index, delivery) in batch.iter().enumerate() {
            let outcome = outcomes
                .get(index)
                .cloned()
                .unwrap_or_else(|| EntryOutcome::Retry("handler returned no outcome".to_string()));
            match outcome {
                EntryOutcome::Settled => {
                    if let Err(error) = self
                        .ledger
                        .settle(self.spec.group, &delivery.entry_id)
                        .await
                    {
                        tracing::warn!(
                            group = self.spec.group,
                            entry_id = %delivery.entry_id,
                            %error,
                            "settle write failed; acking anyway"
                        );
                    }
                    if stream.ack(&delivery.entry_id).await {
                        page.acked += 1;
                    }
                }
                EntryOutcome::Retry(reason) => {
                    page.retried = true;
                    let attempts_before = states
                        .get(&delivery.entry_id)
                        .map_or(0, |state: &EntryState| state.attempts);
                    let attempts = self
                        .record_attempt(
                            &delivery.entry_id,
                            Some(&delivery.envelope),
                            &reason,
                            attempts_before,
                        )
                        .await;
                    if attempts >= entry_attempt_budget() {
                        tracing::warn!(
                            group = self.spec.group,
                            entry_id = %delivery.entry_id,
                            attempts,
                            %reason,
                            "entry exhausted its attempts; dead-lettering"
                        );
                        if self
                            .dead_letter(
                                &delivery.entry_id,
                                Some(&delivery.envelope),
                                payloads.get(&delivery.entry_id).map(String::as_str),
                                attempts,
                                &reason,
                            )
                            .await
                            && stream.ack(&delivery.entry_id).await
                        {
                            page.acked += 1;
                        }
                    } else {
                        tracing::warn!(
                            group = self.spec.group,
                            entry_id = %delivery.entry_id,
                            attempts,
                            %reason,
                            "entry failed; left pending for its next attempt"
                        );
                    }
                }
                EntryOutcome::Dead(reason) => {
                    let attempts_before = states
                        .get(&delivery.entry_id)
                        .map_or(0, |state: &EntryState| state.attempts);
                    let attempts = self
                        .record_attempt(
                            &delivery.entry_id,
                            Some(&delivery.envelope),
                            &reason,
                            attempts_before,
                        )
                        .await;
                    if self
                        .dead_letter(
                            &delivery.entry_id,
                            Some(&delivery.envelope),
                            payloads.get(&delivery.entry_id).map(String::as_str),
                            attempts.max(1),
                            &reason,
                        )
                        .await
                        && stream.ack(&delivery.entry_id).await
                    {
                        page.acked += 1;
                    }
                }
            }
        }
        page
    }

    /// Records a failure and answers the attempt count. When the ledger write
    /// fails the answer is the previous floor (0), which can never dead-letter:
    /// an unknown count must not abandon work.
    ///
    /// The retry delay comes from the consumer policy for the attempt number
    /// this failure becomes (`attempts_before + 1`), so the schedule is R1's,
    /// not a second one grown here.
    async fn record_attempt(
        &self,
        entry_id: &str,
        envelope: Option<&Envelope>,
        error: &str,
        attempts_before: i32,
    ) -> i32 {
        let attempt_number = u32::try_from(attempts_before.max(0))
            .unwrap_or(0)
            .saturating_add(1);
        let delay = retry::policy(Subsystem::Consumer).delay(attempt_number, retry::sample());
        let attempt = Attempt {
            group: self.spec.group,
            consumer: self.name,
            entry_id,
            org_id: envelope.map(|e| e.org_id),
            event_id: envelope.map(|e| e.id),
            event_type: envelope.map(|e| e.event_type.as_str()),
            error,
            retry_in_secs: delay.as_secs_f64(),
        };
        match self.ledger.record_attempt(&attempt).await {
            Ok(attempts) => attempts,
            Err(error) => {
                tracing::warn!(
                    group = self.spec.group,
                    entry_id,
                    %error,
                    "attempt write failed; entry stays pending without a count"
                );
                0
            }
        }
    }

    /// Writes the dead-letter record. False when the write failed, which keeps
    /// the entry in the PEL: an abandonment nobody recorded is not an
    /// abandonment.
    async fn dead_letter(
        &self,
        entry_id: &str,
        envelope: Option<&Envelope>,
        payload: Option<&str>,
        attempts: i32,
        error: &str,
    ) -> bool {
        let spec = DeadEventSpec {
            group: self.spec.group,
            consumer: self.name,
            entry_id,
            org_id: envelope.map(|e| e.org_id),
            event_id: envelope.map(|e| e.id),
            event_type: envelope.map(|e| e.event_type.as_str()),
            attempts,
            last_error: error,
            payload,
        };
        match self.ledger.dead_letter(&spec).await {
            Ok(()) => {
                tracing::warn!(
                    group = self.spec.group,
                    entry_id,
                    attempts,
                    %error,
                    "stream entry dead-lettered"
                );
                true
            }
            Err(error) => {
                tracing::error!(
                    group = self.spec.group,
                    entry_id,
                    %error,
                    "dead-letter write failed; entry stays pending"
                );
                false
            }
        }
    }
}

/// Parses one entry's body. The failure is a string because the reason is
/// recorded in the dead-letter row.
fn parse_entry(entry: &StreamId) -> Result<(Envelope, String), String> {
    let Some(raw) = entry.get::<String>("data") else {
        return Err("stream entry has no readable data field".to_string());
    };
    match serde_json::from_str::<Envelope>(&raw) {
        Ok(envelope) => Ok((envelope, raw)),
        Err(error) => Err(format!("unparseable event: {error}")),
    }
}

/// Runs one consumer until `shutdown`, reconnecting with the R1 consumer
/// policy on any connection failure.
pub async fn run<L: DeliveryLedger, H: EntryHandler>(
    redis_url: &str,
    spec: &ConsumerSpec,
    name: String,
    ledger: &L,
    handler: &H,
    shutdown: &mut watch::Receiver<bool>,
) {
    let client = match redis::Client::open(redis_url) {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(group = spec.group, %error, "invalid redis url; consumer exiting");
            return;
        }
    };
    let mut backoff = Reconnect::consumer();
    loop {
        if *shutdown.borrow() {
            return;
        }
        match RedisEventStream::connect(&client, spec.group, name.clone()).await {
            Ok(mut stream) => {
                backoff.reset();
                if let Err(error) = stream.ensure_group().await {
                    tracing::error!(group = spec.group, %error, "group creation failed");
                    backoff.sleep().await;
                    continue;
                }
                Harness::new(spec, &name, ledger, handler)
                    .follow(&mut stream, shutdown)
                    .await;
                tracing::warn!(group = spec.group, %name, "stream connection ended; reconnecting");
            }
            Err(error) => {
                tracing::error!(group = spec.group, %error, "redis connect failed");
            }
        }
        backoff.sleep().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A stream stand-in that models a consumer group: `pending` returns the
    /// scripted PEL pages in order, `claim` its scripted takeover pages (empty
    /// once exhausted), `live` its scripted pages and then a failure so the
    /// follow loop returns instead of blocking.
    struct FakeStream {
        pending_pages: VecDeque<Vec<StreamId>>,
        claim_pages: VecDeque<Vec<StreamId>>,
        live_pages: VecDeque<Vec<StreamId>>,
        ack_succeeds: bool,
        calls: Vec<String>,
    }

    impl FakeStream {
        fn new(pending_pages: Vec<Vec<StreamId>>, live_pages: Vec<Vec<StreamId>>) -> Self {
            Self {
                pending_pages: pending_pages.into(),
                claim_pages: VecDeque::new(),
                live_pages: live_pages.into(),
                ack_succeeds: true,
                calls: Vec::new(),
            }
        }

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

    /// Records what the loop asked of it, and answers as if nothing has ever
    /// failed. Every entry is due with zero attempts unless it is listed as
    /// deferred, which makes one `states` call answer "not yet".
    #[derive(Default)]
    struct MemoryLedger {
        attempts: std::sync::Mutex<HashMap<String, i32>>,
        settled: std::sync::Mutex<Vec<String>>,
        dead: std::sync::Mutex<Vec<(String, i32, String)>>,
        deferred_once: std::sync::Mutex<std::collections::HashSet<String>>,
    }

    impl DeliveryLedger for MemoryLedger {
        async fn states(
            &self,
            _group: &str,
            entry_ids: &[String],
        ) -> Result<HashMap<String, EntryState>, String> {
            let mut deferred = self.deferred_once.lock().expect("mutex");
            Ok(entry_ids
                .iter()
                .filter(|id| deferred.remove(*id))
                .map(|id| {
                    (
                        id.clone(),
                        EntryState {
                            due: false,
                            dead: false,
                            attempts: 1,
                        },
                    )
                })
                .collect())
        }

        async fn record_attempt(&self, attempt: &Attempt<'_>) -> Result<i32, String> {
            let mut attempts = self.attempts.lock().expect("mutex");
            let count = attempts.entry(attempt.entry_id.to_string()).or_insert(0);
            *count += 1;
            Ok(*count)
        }

        async fn settle(&self, _group: &str, entry_id: &str) -> Result<(), String> {
            self.settled
                .lock()
                .expect("mutex")
                .push(entry_id.to_string());
            Ok(())
        }

        async fn dead_letter(&self, spec: &DeadEventSpec<'_>) -> Result<(), String> {
            self.dead.lock().expect("mutex").push((
                spec.entry_id.to_string(),
                spec.attempts,
                spec.last_error.to_string(),
            ));
            Ok(())
        }
    }

    /// A handler that answers every entry with the same verdict.
    struct ConstantHandler {
        outcome: fn() -> EntryOutcome,
        seen: std::sync::Mutex<Vec<String>>,
    }

    impl ConstantHandler {
        fn new(outcome: fn() -> EntryOutcome) -> Self {
            Self {
                outcome,
                seen: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl EntryHandler for ConstantHandler {
        async fn handle(&self, batch: &[Delivery]) -> Vec<EntryOutcome> {
            let mut seen = self.seen.lock().expect("mutex");
            for delivery in batch {
                seen.push(delivery.entry_id.clone());
            }
            batch.iter().map(|_| (self.outcome)()).collect()
        }
    }

    fn parseable(id: &str) -> StreamId {
        let envelope = serde_json::json!({
            "id": uuid::Uuid::now_v7().to_string(),
            "type": "ticket.created",
            "org_id": uuid::Uuid::now_v7().to_string(),
            "entity": "ticket",
            "entity_id": null,
            "occurred_at": chrono::Utc::now(),
            "payload_schema_ref": "core-domain/v1",
            "payload": {},
        });
        stream_entry(id, &envelope.to_string())
    }

    fn stream_entry(id: &str, body: &str) -> StreamId {
        let mut map = HashMap::new();
        map.insert(
            "data".to_string(),
            redis::Value::BulkString(body.as_bytes().to_vec()),
        );
        StreamId {
            id: id.to_string(),
            map,
        }
    }

    fn unparseable(id: &str) -> StreamId {
        stream_entry(id, "not-json")
    }

    const SPEC: ConsumerSpec = ConsumerSpec::new("harness-test", 50);

    #[tokio::test]
    async fn recovery_drains_own_pending_pages_before_the_tail() {
        let ledger = MemoryLedger::default();
        let handler = ConstantHandler::new(|| EntryOutcome::Settled);
        let mut stream = FakeStream::new(
            vec![
                vec![parseable("1-0"), parseable("2-0")],
                vec![parseable("3-0")],
            ],
            Vec::new(),
        );
        let (_tx, mut shutdown) = watch::channel(false);

        Harness::new(&SPEC, "replica-a", &ledger, &handler)
            .follow(&mut stream, &mut shutdown)
            .await;

        assert_eq!(
            stream.calls,
            vec![
                "pending", "ack:1-0", "ack:2-0", "pending", "ack:3-0",
                // A third read proves the drain stopped on an empty page, not
                // on a page count.
                "pending",
                // One claim after the own PEL is empty, answered with nothing.
                "claim", // Then the tail; the error after this page ends the loop.
                "live",
            ]
        );
        assert_eq!(
            *handler.seen.lock().expect("mutex"),
            vec!["1-0".to_string(), "2-0".to_string(), "3-0".to_string()]
        );
    }

    #[tokio::test]
    async fn a_recovery_pass_that_cannot_ack_stops_before_the_tail() {
        let ledger = MemoryLedger::default();
        let handler = ConstantHandler::new(|| EntryOutcome::Settled);
        let mut stream = FakeStream::new(vec![vec![parseable("1-0")]], Vec::new());
        stream.ack_succeeds = false;
        let (_tx, mut shutdown) = watch::channel(false);

        Harness::new(&SPEC, "replica-a", &ledger, &handler)
            .follow(&mut stream, &mut shutdown)
            .await;

        // No second `pending` (that would be the spin), no claim, no live read.
        assert_eq!(stream.calls, vec!["pending", "ack:1-0"]);
    }

    /// Fails the first sighting of each entry, settles after that: models a
    /// transient handler failure followed by the live retry.
    #[derive(Default)]
    struct FlakyHandler {
        failed: std::sync::Mutex<std::collections::HashSet<String>>,
        seen: std::sync::Mutex<Vec<String>>,
    }

    impl EntryHandler for FlakyHandler {
        async fn handle(&self, batch: &[Delivery]) -> Vec<EntryOutcome> {
            batch
                .iter()
                .map(|delivery| {
                    self.seen
                        .lock()
                        .expect("mutex")
                        .push(delivery.entry_id.clone());
                    let first = self
                        .failed
                        .lock()
                        .expect("mutex")
                        .insert(delivery.entry_id.clone());
                    if first {
                        EntryOutcome::Retry("transient failure".to_string())
                    } else {
                        EntryOutcome::Settled
                    }
                })
                .collect()
        }
    }

    #[tokio::test]
    async fn a_failing_entry_is_retried_in_the_same_process() {
        let ledger = MemoryLedger::default();
        let handler = FlakyHandler::default();
        let mut stream = FakeStream::new(vec![Vec::new()], vec![vec![parseable("7-0")]]);
        // The PEL answers with the entry that just failed, as Valkey does.
        stream.pending_pages.push_back(vec![parseable("7-0")]);
        let (_tx, mut shutdown) = watch::channel(false);

        Harness::new(&SPEC, "replica-a", &ledger, &handler)
            .follow(&mut stream, &mut shutdown)
            .await;

        assert!(
            stream
                .calls
                .iter()
                .filter(|call| *call == "pending")
                .count()
                >= 2,
            "a failed entry must trigger a live PEL retry: {:?}",
            stream.calls
        );
        assert!(
            stream.calls.contains(&"ack:7-0".to_string()),
            "the retried entry must settle once the handler stops failing: {:?}",
            stream.calls
        );
        assert_eq!(
            handler.seen.lock().expect("mutex").as_slice(),
            ["7-0", "7-0"],
            "the same entry is handled twice, not re-read as a new one"
        );
        assert_eq!(
            *ledger
                .attempts
                .lock()
                .expect("mutex")
                .get("7-0")
                .expect("attempt"),
            1
        );
    }

    /// The headline fix: an entry recovered at startup that is still inside its
    /// retry delay is retried by this process once it comes due, not by the
    /// next restart.
    #[tokio::test]
    async fn a_deferred_entry_is_retried_after_an_idle_read() {
        let ledger = MemoryLedger::default();
        ledger
            .deferred_once
            .lock()
            .expect("mutex")
            .insert("6-0".to_string());
        let handler = ConstantHandler::new(|| EntryOutcome::Settled);
        let mut stream = FakeStream::new(
            // The startup recovery finds it, but it is not due yet; the next
            // PEL read (after an idle tail read) finds it due and settles it.
            vec![vec![parseable("6-0")], vec![parseable("6-0")], Vec::new()],
            vec![Vec::new()],
        );
        let (_tx, mut shutdown) = watch::channel(false);

        Harness::new(&SPEC, "replica-a", &ledger, &handler)
            .follow(&mut stream, &mut shutdown)
            .await;

        assert!(
            stream.calls.contains(&"ack:6-0".to_string()),
            "the deferred entry must be retried and settled in this process: {:?}",
            stream.calls
        );
        assert_eq!(
            *ledger.attempts.lock().expect("mutex"),
            HashMap::new(),
            "settling a deferred entry records no extra attempt"
        );
    }

    #[tokio::test]
    async fn an_entry_claimed_from_a_dead_consumer_is_handled() {
        let ledger = MemoryLedger::default();
        let handler = ConstantHandler::new(|| EntryOutcome::Settled);
        let mut stream =
            FakeStream::new(vec![Vec::new()], Vec::new()).with_claims(vec![vec![parseable("8-0")]]);
        let (_tx, mut shutdown) = watch::channel(false);

        Harness::new(&SPEC, "replica-a", &ledger, &handler)
            .follow(&mut stream, &mut shutdown)
            .await;

        assert_eq!(
            stream.calls,
            vec!["pending", "claim", "ack:8-0", "claim", "live"],
            "the takeover pass handles and acks before the tail starts"
        );
    }

    #[tokio::test]
    async fn a_poison_body_is_dead_lettered_and_acked() {
        let ledger = MemoryLedger::default();
        let handler = ConstantHandler::new(|| EntryOutcome::Settled);
        let mut stream = FakeStream::new(vec![vec![unparseable("4-0")]], Vec::new());
        let (_tx, mut shutdown) = watch::channel(false);

        Harness::new(&SPEC, "replica-a", &ledger, &handler)
            .follow(&mut stream, &mut shutdown)
            .await;

        let dead = ledger.dead.lock().expect("mutex").clone();
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].0, "4-0");
        assert!(dead[0].2.contains("unparseable"));
        assert!(stream.calls.contains(&"ack:4-0".to_string()));
    }

    #[tokio::test]
    async fn an_entry_past_its_budget_is_dead_lettered_and_acked() {
        let ledger = MemoryLedger::default();
        let handler = ConstantHandler::new(|| EntryOutcome::Dead("never valid".to_string()));
        let mut stream = FakeStream::new(vec![vec![parseable("5-0")]], Vec::new());
        let (_tx, mut shutdown) = watch::channel(false);

        Harness::new(&SPEC, "replica-a", &ledger, &handler)
            .follow(&mut stream, &mut shutdown)
            .await;

        let dead = ledger.dead.lock().expect("mutex").clone();
        assert_eq!(dead.len(), 1, "one abandonment, recorded once");
        assert_eq!(dead[0].0, "5-0");
        assert_eq!(dead[0].1, 1, "the attempt that killed it is counted");
        assert!(stream.calls.contains(&"ack:5-0".to_string()));
    }

    #[test]
    fn consumer_names_are_group_scoped_and_replica_specific() {
        let a = consumer_name("automations");
        let b = consumer_name("automations");
        assert_eq!(a, b, "the name is stable for the process lifetime");
        assert!(a.starts_with("automations-"));
        assert_ne!(consumer_name("csat"), a, "groups get distinct names");
        assert!(a.ends_with(&std::process::id().to_string()));
    }

    #[test]
    fn an_unreadable_body_is_a_parse_failure_not_a_panic() {
        let mut map = HashMap::new();
        map.insert("other".to_string(), redis::Value::BulkString(b"x".to_vec()));
        let entry = StreamId {
            id: "1-0".to_string(),
            map,
        };
        assert!(parse_entry(&entry).is_err());
        assert!(parse_entry(&unparseable("2-0")).is_err());
        assert!(parse_entry(&parseable("3-0")).is_ok());
    }
}
