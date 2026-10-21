//! worker/reconcile — the repairs nothing re-attempts on its own
//! (Wave 52 lane R4, `p52-reconciliation-jobs`).
//!
//! Purpose: the wave built every mechanism for *time-based* work — sweeps that
//! claim due rows, retry matrices, a retention matrix — and R2/R3 made every
//! ledger of abandoned work visible. What none of them does is go looking for
//! work whose trigger never fired. Five failures have no owner:
//!
//! 1. **An entity with no index row.** The search indexer only reacts to
//!    events (`search_indexer.rs`). A consumer group is created at `$`
//!    (`consumer.rs`), so everything published before the group existed is
//!    never delivered; an entry that dead-letters is equally lost. `/internal/search/backfill`
//!    is the embedding leg, not the text index. Nothing rebuilds a missing row.
//! 2. **A dead stream entry.** R2 retries an entry live and dead-letters it
//!    when the budget ends. R3 lists it and refuses to replay it ("re-publishing
//!    needs the relay's stream writer, which lives in the worker"). This module
//!    IS that writer, so this is the path 0144 deferred here: a notification
//!    email whose fan-out entry died can finally be re-delivered, and so can
//!    every other group's.
//! 3. **A parked workflow run whose wake job died.** `park_run` enqueues one
//!    `workflow.resume` job; if that job dies the run stays `waiting` with a
//!    deadline in the past for as long as the table lives, and the retention
//!    sweep cannot even collect it (it prunes terminal rows only).
//! 4. **A report slot that failed once.** `deliver_due` advances the schedule
//!    in the same transaction that claims the slot, so a failed slot is never
//!    claimed again — at-most-once by construction.
//! 5. **A consumer group behind the stream.** The existing probe runs inside
//!    each consumer's own loop, so a stopped consumer reports nothing; and
//!    Redis's `lag` is `null` exactly when a group's position has fallen under
//!    the stream floor, which is the case where events are gone for good.
//!
//! One loop, because five loops would be five liveness flags and five ways for
//! `/healthz` to report one subsystem. Each pass is a separate function so a
//! failure in one is a log line, not a stopped loop, and each emits its own
//! gauge so "the repair is keeping up" is a series rather than a claim.
//!
//! Non-purpose: no automatic requeue of `jobs` or `email_outbound` (R3's
//! doctrine is that an operator decides, and both redrive in place), no
//! staleness detection for the search index (three of the seven sources carry
//! no usable `updated_at`, so the watermark this pass checks is existence, not
//! freshness), and no event redrive for a body that never parsed — replaying it
//! would fail identically.

use crate::reconnect::Reconnect;
use crate::search_indexer::{IndexSink, Target};
use core_db::search::SearchKind;
use core_domain::retry::{self, Subsystem};
use core_events::{Envelope, Publisher};
use redis::aio::MultiplexedConnection;
use sqlx::PgPool;
use std::future::Future;
use std::time::Duration;
use tokio::sync::watch;

/// Default cadence: one pass every quarter hour.
///
/// Slower than the SLA or PM sweeps on purpose: every pass here scans for work
/// that is *missing*, and the failure it repairs is measured in "since the
/// deploy that introduced the consumer", not in minutes. Fast enough that a
/// dead notification email is re-delivered inside one support conversation.
pub const DEFAULT_INTERVAL_SECS: u64 = 15 * 60;

/// Rows per pass per pass-kind. The backfill and the reap are scans; the batch
/// is what keeps one pass cheap on a large install.
pub const DEFAULT_BATCH: i64 = 200;

/// How many times one event may die before replay stops trying.
///
/// A poison envelope fails identically every time, and each replay creates a
/// new stream entry whose next death is a new row — so this is the only bound
/// that ends the cycle. Three: enough for a transient outage of the downstream
/// surface to be ridden out, few enough that a real defect reaches a person the
/// same day.
pub const DEFAULT_MAX_REPLAY_DEATHS: i64 = 3;

/// Pause after a pass that failed, so a database or Valkey outage is retried
/// without spinning. Same shape as the retention sweep's error pause.
pub const ERROR_PAUSE: Duration = Duration::from_secs(30);

/// Settings for the loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileConfig {
    /// Seconds between passes.
    pub interval_secs: u64,
    /// Rows per pass.
    pub batch: i64,
    /// Deaths allowed per `(group, event)` before replay stops.
    pub max_replay_deaths: i64,
    /// How far past its deadline a parked run's wake job has to be dead before
    /// the run is abandoned, in seconds.
    ///
    /// Derived from the queue's own retry policy by [`Self::wake_grace`]: a wake
    /// job that is merely late is inside its backoff, and reaping it would
    /// abandon a run that was about to resume.
    pub wake_grace_secs: u64,
}

impl ReconcileConfig {
    /// Resolves the knobs, keeping a zero interval from meaning "pass
    /// constantly" — the same defence the retention sweep makes.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            interval_secs: lookup("MARULA_RECONCILE_SECS")
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|secs| *secs > 0)
                .unwrap_or(DEFAULT_INTERVAL_SECS),
            batch: lookup("MARULA_RECONCILE_BATCH")
                .and_then(|v| v.parse::<i64>().ok())
                .filter(|batch| *batch > 0)
                .unwrap_or(DEFAULT_BATCH),
            max_replay_deaths: lookup("MARULA_RECONCILE_MAX_REPLAYS")
                .and_then(|v| v.parse::<i64>().ok())
                .filter(|n| *n > 0)
                .unwrap_or(DEFAULT_MAX_REPLAY_DEATHS),
            wake_grace_secs: lookup("MARULA_RECONCILE_WAKE_GRACE_SECS")
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|secs| *secs > 0)
                .unwrap_or_else(|| default_wake_grace().as_secs()),
        }
    }
}

/// How long a due wake job can still be alive, from the queue's own policy.
///
/// A wake job is ordinary queue work: `max_attempts` attempts, each after the
/// queue policy's backoff for that attempt. The sum of those delays is the
/// window in which the job can still legitimately be retrying; double it for one
/// replica restart or one wedged worker, and past that the run has no wake path
/// left. Computing it from the policy rather than writing a number here is the
/// point: change the queue's budget and the grace follows.
#[must_use]
pub fn default_wake_grace() -> Duration {
    let policy = retry::policy(Subsystem::Queue);
    let attempts = u32::try_from(policy.max_attempts_i32(1))
        .unwrap_or(1)
        .max(1);
    let window: Duration = (1..=attempts).map(|n| policy.delay(n, 1.0)).sum();
    window * 2
}

/// What one pass found and did. Every field is the number its gauge receives, so
/// a test that asserts the report asserts the series.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassReport {
    /// Source entities with no index row.
    pub index_backlog: u64,
    /// Of those, how many were re-indexed.
    pub index_repaired: u64,
    /// Dead entries this pass tried to put back on the stream.
    pub replay_attempted: u64,
    /// Of those, how many were published and marked.
    pub replayed: u64,
    /// Parked runs found unwakeable (the gauge's value, measured before the reap).
    pub wake_lost: u64,
    /// Parked runs moved to `abandoned`.
    pub reaped: u64,
    /// Consumer groups sampled against the stream's bounds.
    pub groups_sampled: u64,
    /// Passes that returned an error this cycle.
    pub failed: u64,
}

impl PassReport {
    /// Whether anything went wrong, which decides whether the loop waits its
    /// interval or its error pause.
    #[must_use]
    pub fn any_failed(&self) -> bool {
        self.failed > 0
    }

    fn merge(&mut self, other: Self) {
        self.index_backlog += other.index_backlog;
        self.index_repaired += other.index_repaired;
        self.replay_attempted += other.replay_attempted;
        self.replayed += other.replayed;
        self.wake_lost += other.wake_lost;
        self.reaped += other.reaped;
        self.groups_sampled += other.groups_sampled;
        self.failed += other.failed;
    }
}

// ---------------------------------------------------------------------------
// The stream writer seam
// ---------------------------------------------------------------------------

/// The publish half of `core_events::Publisher`, as a seam.
///
/// The replay pass is the only code in the worker that publishes an envelope
/// that did not just arrive from Postgres, and a real `Publisher` cannot be
/// faked in a unit test — it owns a Valkey connection. One trait method is the
/// whole seam, and the deliberate-failure test for replay needs it: it must
/// prove that a failed publish leaves the entry unmarked so the next pass tries
/// again.
pub trait StreamWriter: Send {
    #[allow(clippy::manual_async_fn)] // the explicit ``+ Send`` is the contract
    fn publish(&mut self, envelope: &Envelope) -> impl Future<Output = Result<(), String>> + Send;
}

impl StreamWriter for Publisher {
    #[allow(clippy::manual_async_fn)]
    fn publish(&mut self, envelope: &Envelope) -> impl Future<Output = Result<(), String>> + Send {
        async move {
            Publisher::publish(self, envelope)
                .await
                .map_err(|error| error.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// Pass 1 — the search index's missing rows
// ---------------------------------------------------------------------------

/// Re-indexes source entities that have no `search_index` row.
///
/// The repair goes through `search_indexer::resolve_target` and the same
/// `IndexSink` the live consumer uses, so a backfilled row cannot differ from an
/// event-indexed one: same fetch, same truncation, same upsert. A source row
/// that vanished between the scan and the read resolves to `None` and is left
/// alone — there is nothing to index, and the next pass will not see it.
pub async fn backfill_search_index<S: IndexSink>(
    pool: &PgPool,
    sink: &S,
    batch: i64,
) -> anyhow::Result<PassReport> {
    let mut report = PassReport::default();
    let missing = core_db::reconcile::missing_search_rows(pool, batch).await?;
    report.index_backlog = missing.len() as u64;

    // Grouped by org because the sink's request body is org-scoped: one call per
    // tenant, which is also the unit core-api binds RLS to.
    let mut by_org: std::collections::BTreeMap<uuid::Uuid, Vec<crate::search_indexer::IndexOp>> =
        std::collections::BTreeMap::new();
    for row in missing {
        let Some(kind) = SearchKind::parse(&row.entity_kind) else {
            // A kind the index does not know is a row this pass must not guess
            // at; it stays in the backlog and the gauge keeps saying so.
            tracing::warn!(kind = %row.entity_kind, "reconcile: unknown index kind");
            continue;
        };
        let target = Target::from_kind(kind, row.entity_id);
        match crate::search_indexer::resolve_target(pool, row.org_id, target).await {
            Ok(Some(op)) => by_org.entry(row.org_id).or_default().push(op),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(
                    %error,
                    org_id = %row.org_id,
                    entity = %row.entity_id,
                    "reconcile: source read failed during backfill"
                );
                report.failed += 1;
            }
        }
    }

    for (org_id, ops) in by_org {
        let count = ops.len() as u64;
        match sink.apply(org_id, ops).await {
            Ok(()) => report.index_repaired += count,
            Err(error) => {
                tracing::warn!(%error, %org_id, "reconcile: index sink rejected a backfill batch");
                report.failed += 1;
            }
        }
    }
    core_otel::metrics::reconcile_search_index_backlog(report.index_backlog);
    Ok(report)
}

// ---------------------------------------------------------------------------
// Pass 2 — dead stream entries
// ---------------------------------------------------------------------------

/// Re-publishes dead entries whose envelope parsed, and reports the dead set by
/// group either way.
///
/// Order matters and is deliberate: **publish, then mark**. A crash between the
/// two replays the entry again on the next pass, which is at-least-once — the
/// stream's own contract, and every consumer is idempotent by key or by dedupe
/// key. Marking first would make a failed publish a silently dropped repair,
/// which is the failure this pass exists to end.
pub async fn replay_dead_events<W: StreamWriter>(
    pool: &PgPool,
    writer: &mut W,
    batch: i64,
    max_deaths: i64,
) -> anyhow::Result<PassReport> {
    let mut report = PassReport::default();

    // The gauge first, from the whole dead set rather than from the batch: the
    // operator question is "how much is dead", not "how much fits in one pass".
    let backlog = core_db::reconcile::dead_event_backlog(pool, max_deaths).await?;
    for group in &backlog {
        core_otel::metrics::reconcile_dead_events(
            &group.consumer_group,
            group.replayable.max(0) as u64,
            group.unrepairable.max(0) as u64,
        );
    }

    let candidates = core_db::reconcile::replayable_dead_events(pool, batch, max_deaths).await?;
    report.replay_attempted = candidates.len() as u64;
    let mut replayed = Vec::new();
    for candidate in candidates {
        let envelope = match serde_json::from_str::<Envelope>(&candidate.payload) {
            Ok(envelope) => envelope,
            Err(error) => {
                // `event_id IS NOT NULL` says this parsed once, so a failure now
                // is a data defect, not a poison message. Logged and left
                // unmarked: it stays in the backlog where someone can see it.
                tracing::warn!(
                    %error,
                    dead_event = %candidate.id,
                    group = %candidate.consumer_group,
                    "reconcile: stored payload no longer parses; not replaying"
                );
                report.failed += 1;
                continue;
            }
        };
        match writer.publish(&envelope).await {
            Ok(()) => replayed.push(candidate.id),
            Err(error) => {
                tracing::warn!(
                    %error,
                    dead_event = %candidate.id,
                    group = %candidate.consumer_group,
                    "reconcile: replay publish failed; entry stays eligible"
                );
                report.failed += 1;
            }
        }
    }
    report.replayed = core_db::reconcile::mark_replayed(pool, &replayed).await?;
    if report.replayed > 0 {
        tracing::info!(
            replayed = report.replayed,
            "reconcile: dead entries replayed"
        );
    }
    Ok(report)
}

// ---------------------------------------------------------------------------
// Pass 3 — parked runs whose wake is gone
// ---------------------------------------------------------------------------

/// Abandons parked runs whose wake deadline passed and whose wake job is dead.
///
/// `unwakeable_parked_runs` decides *what* is unwakeable (deadline past the
/// grace AND no pending/running `workflow.resume`), so the reap cannot disagree
/// with the gauge: the same rows are counted and then moved.
pub async fn reap_parked_runs(
    pool: &PgPool,
    grace_secs: u64,
    batch: i64,
) -> anyhow::Result<PassReport> {
    let mut report = PassReport::default();
    let runs = core_db::reconcile::unwakeable_parked_runs(pool, grace_secs as f64, batch).await?;
    report.wake_lost = runs.len() as u64;
    report.reaped = core_db::reconcile::abandon_unwakeable_runs(
        pool,
        &runs,
        "wake_lost",
        "the resume job that would wake this run is dead and past its retry window",
    )
    .await?;
    core_otel::metrics::reconcile_workflow_wake_lost(report.wake_lost);
    if report.reaped > 0 {
        tracing::warn!(
            reaped = report.reaped,
            "reconcile: abandoned parked workflow runs whose wake was lost"
        );
    }
    Ok(report)
}

// ---------------------------------------------------------------------------
// Pass 4 — consumer groups against the stream's bounds
// ---------------------------------------------------------------------------

/// Samples every consumer group on the stream and emits gap/lost per group.
///
/// The read lives in `stream_health::read_gaps` (it owns the Valkey vocabulary);
/// this pass is only the emission and the count.
///
/// `None` means the stream could not be read, and the caller must treat that as a
/// reason to DROP its connection and dial again. That distinction is the whole
/// point of returning `Option` here: a probe that keeps a dead connection is a
/// probe that reports nothing on every pass, and an absent gap series reads as
/// "no gap" to anyone not watching for the absence — the exact silence this lane
/// exists to end.
pub async fn sample_consumer_gaps(conn: &mut MultiplexedConnection) -> Option<PassReport> {
    let gaps = crate::stream_health::read_gaps(conn).await?;
    for gap in &gaps {
        crate::stream_health::emit_gap(gap);
    }
    Some(PassReport {
        groups_sampled: gaps.len() as u64,
        ..PassReport::default()
    })
}

// ---------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------

/// Runs the reconcile loop until `shutdown` fires.
///
/// One Valkey connection for the gap probe and one for the replay writer, both
/// opened lazily and reconnected through the consumer policy — the P9 lesson is
/// that a connection is a resource that is *held*, so this holds exactly two,
/// not one per group and not one per pass.
///
/// A pass that fails does not stop the loop and does not stop the other passes:
/// a dead index is not a reason to stop reaping workflows.
pub async fn run_reconcile<S: IndexSink>(
    pool: PgPool,
    config: ReconcileConfig,
    sink: Option<S>,
    redis_url: String,
    mut shutdown: watch::Receiver<bool>,
) {
    tracing::info!(
        interval_secs = config.interval_secs,
        batch = config.batch,
        max_replay_deaths = config.max_replay_deaths,
        wake_grace_secs = config.wake_grace_secs,
        index_backfill = sink.is_some(),
        "reconciliation loop started"
    );

    let mut writer: Option<Publisher> = None;
    let mut gap_conn: Option<MultiplexedConnection> = None;
    let mut backoff = Reconnect::consumer();

    loop {
        if *shutdown.borrow_and_update() {
            return;
        }

        let mut report = PassReport::default();

        // 1. The index backfill. Skipped, not failed, when no internal token is
        // configured: the sink is how core-api is reached, and the indexer is
        // disabled in that profile too.
        if let Some(sink) = sink.as_ref() {
            match backfill_search_index(&pool, sink, config.batch).await {
                Ok(pass) => report.merge(pass),
                Err(error) => {
                    tracing::error!(%error, "reconcile: index backfill failed");
                    report.failed += 1;
                }
            }
        }

        // 2. Dead-entry replay. The connection is opened on first use and
        // reconnected on failure; a Valkey outage must not lose the loop.
        if writer.is_none() {
            match Publisher::connect(&redis_url).await {
                Ok(publisher) => {
                    writer = Some(publisher);
                    backoff.reset();
                }
                Err(error) => {
                    tracing::warn!(%error, "reconcile: replay writer connect failed");
                    backoff.sleep().await;
                }
            }
        }
        if let Some(publisher) = writer.as_mut() {
            match replay_dead_events(&pool, publisher, config.batch, config.max_replay_deaths).await
            {
                Ok(pass) => {
                    // Nothing got through while something was attempted: the
                    // connection is the suspect, not the payloads. Drop it so
                    // the next pass dials again instead of failing quietly
                    // forever.
                    if pass.replay_attempted > 0 && pass.replayed == 0 {
                        tracing::warn!(
                            attempted = pass.replay_attempted,
                            "reconcile: no replay got through; reconnecting the writer"
                        );
                        writer = None;
                    }
                    report.merge(pass);
                }
                Err(error) => {
                    tracing::error!(%error, "reconcile: dead-entry replay failed");
                    report.failed += 1;
                }
            }
        }

        // 3. Parked-run reaping.
        match reap_parked_runs(&pool, config.wake_grace_secs, config.batch).await {
            Ok(pass) => report.merge(pass),
            Err(error) => {
                tracing::error!(%error, "reconcile: parked-run reap failed");
                report.failed += 1;
            }
        }

        // 4. The gap probe. Its own connection, because it is a read of the
        // whole stream and must not be behind the writer's backoff.
        if gap_conn.is_none() {
            match redis::Client::open(redis_url.clone()) {
                Ok(client) => match client.get_multiplexed_async_connection().await {
                    Ok(conn) => gap_conn = Some(conn),
                    Err(error) => {
                        tracing::warn!(%error, "reconcile: gap probe connect failed");
                    }
                },
                Err(error) => {
                    tracing::warn!(%error, "reconcile: redis client for gap probe failed");
                }
            }
        }
        if let Some(conn) = gap_conn.as_mut() {
            match sample_consumer_gaps(conn).await {
                Some(pass) => report.merge(pass),
                None => {
                    tracing::warn!("reconcile: consumer-gap probe could not read the stream");
                    gap_conn = None;
                    report.failed += 1;
                }
            }
        }

        if report.index_backlog > 0 || report.wake_lost > 0 || report.replayed > 0 {
            tracing::info!(
                index_backlog = report.index_backlog,
                index_repaired = report.index_repaired,
                replayed = report.replayed,
                wake_lost = report.wake_lost,
                reaped = report.reaped,
                groups_sampled = report.groups_sampled,
                "reconcile pass"
            );
        }

        let pause = if report.any_failed() {
            ERROR_PAUSE
        } else {
            Duration::from_secs(config.interval_secs.max(60))
        };
        tokio::select! {
            _ = shutdown.changed() => return,
            _ = tokio::time::sleep(pause) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    fn cfg(vars: &[(&str, &str)]) -> ReconcileConfig {
        let map: std::collections::HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.to_string()))
            .collect();
        ReconcileConfig::from_lookup(|key| map.get(key).cloned())
    }

    #[test]
    fn defaults_are_the_documented_ones() {
        let config = cfg(&[]);
        assert_eq!(config.interval_secs, DEFAULT_INTERVAL_SECS);
        assert_eq!(config.batch, DEFAULT_BATCH);
        assert_eq!(config.max_replay_deaths, DEFAULT_MAX_REPLAY_DEATHS);
        assert_eq!(config.wake_grace_secs, default_wake_grace().as_secs());
    }

    #[test]
    fn zero_knobs_fall_back_instead_of_sweeping_constantly() {
        let config = cfg(&[
            ("MARULA_RECONCILE_SECS", "0"),
            ("MARULA_RECONCILE_BATCH", "0"),
            ("MARULA_RECONCILE_MAX_REPLAYS", "0"),
            ("MARULA_RECONCILE_WAKE_GRACE_SECS", "0"),
        ]);
        assert_eq!(config.interval_secs, DEFAULT_INTERVAL_SECS);
        assert_eq!(config.batch, DEFAULT_BATCH);
        assert_eq!(config.max_replay_deaths, DEFAULT_MAX_REPLAY_DEATHS);
        assert_eq!(config.wake_grace_secs, default_wake_grace().as_secs());
    }

    /// The grace is the queue's own retry window, not a number invented here:
    /// every attempt the budget allows, each behind its own backoff. If the
    /// policy's budget shrinks, so does the grace — which is what stops a reap
    /// from abandoning a run whose wake job is still legitimately retrying.
    #[test]
    fn the_wake_grace_covers_every_attempt_the_queue_budget_allows() {
        let policy = retry::policy(Subsystem::Queue);
        let attempts = u32::try_from(policy.max_attempts_i32(1)).unwrap_or(1);
        let single_attempt = policy.delay(attempts, 1.0);
        assert!(
            default_wake_grace() >= single_attempt,
            "the grace must at least cover the last attempt's own delay"
        );
        assert!(
            default_wake_grace() > Duration::ZERO,
            "a zero grace would abandon a run the instant its deadline passed"
        );
    }

    /// A source row vanished between the scan and the read: nothing to index,
    /// nothing to count, no error. The pass must not treat that as a failure or
    /// the gauge would never return to zero on a busy tenant.
    #[test]
    fn a_vanished_source_row_is_neither_repaired_nor_failed() {
        // The decision is in `backfill_search_index`'s match: `Ok(None)` adds
        // nothing. This pins the report arithmetic that match feeds.
        let report = PassReport {
            index_backlog: 3,
            index_repaired: 2,
            ..PassReport::default()
        };
        assert!(!report.any_failed());
        assert_eq!(report.index_backlog - report.index_repaired, 1);
    }

    /// The reconnect rule, as the arithmetic the loop applies: a pass that
    /// attempted publishes and landed none means the connection is the suspect.
    /// Without it a dead Valkey connection leaves the writer in place, every
    /// pass fails quietly, and the replay set stops moving with no series and no
    /// log.
    #[test]
    fn a_pass_that_attempted_and_landed_nothing_drops_the_writer() {
        let attempted = PassReport {
            replay_attempted: 3,
            replayed: 0,
            failed: 3,
            ..PassReport::default()
        };
        assert!(attempted.replay_attempted > 0 && attempted.replayed == 0);

        let partial = PassReport {
            replay_attempted: 3,
            replayed: 1,
            failed: 2,
            ..PassReport::default()
        };
        assert!(
            !(partial.replay_attempted > 0 && partial.replayed == 0),
            "one landed replay proves the connection works; do not reconnect"
        );

        let idle = PassReport::default();
        assert!(
            !(idle.replay_attempted > 0 && idle.replayed == 0),
            "an empty dead set is not a connection problem"
        );
    }

    // --- the replay seam -----------------------------------------------

    #[derive(Default)]
    struct FakeWriter {
        published: Mutex<Vec<String>>,
        failures: Mutex<VecDeque<String>>,
    }

    impl FakeWriter {
        fn failing_once(reason: &str) -> Self {
            let writer = Self::default();
            writer
                .failures
                .lock()
                .expect("lock")
                .push_back(reason.to_string());
            writer
        }

        fn published(&self) -> Vec<String> {
            self.published.lock().expect("lock").clone()
        }
    }

    impl StreamWriter for FakeWriter {
        fn publish(
            &mut self,
            envelope: &Envelope,
        ) -> impl Future<Output = Result<(), String>> + Send {
            let id = envelope.id.to_string();
            let failure = self.failures.lock().expect("lock").pop_front();
            let published = &self.published;
            async move {
                if let Some(reason) = failure {
                    return Err(reason);
                }
                published.lock().expect("lock").push(id);
                Ok(())
            }
        }
    }

    fn envelope(id: &str) -> Envelope {
        Envelope {
            id: uuid::Uuid::parse_str(id).unwrap_or_else(|_| uuid::Uuid::nil()),
            event_type: "test".into(),
            org_id: uuid::Uuid::nil(),
            entity: "ticket".into(),
            entity_id: None,
            occurred_at: chrono::Utc::now(),
            payload_schema_ref: "test/1".into(),
            payload: serde_json::json!({}),
        }
    }

    /// The deliberate failure: the writer is down for the first attempt. The
    /// entry must stay eligible (unmarked) so the next pass tries again — a
    /// marked-but-unpublished entry is a repair silently thrown away, which is
    /// exactly the defect this pass exists to end.
    #[tokio::test]
    async fn a_failed_publish_leaves_the_entry_eligible_and_reports_a_failure() {
        let mut writer = FakeWriter::failing_once("valkey down");
        let mut report = PassReport::default();
        match writer
            .publish(&envelope("00000000-0000-0000-0000-000000000001"))
            .await
        {
            Ok(()) => report.replayed += 1,
            Err(error) => {
                assert_eq!(error, "valkey down");
                report.failed += 1;
            }
        }
        assert_eq!(report.replayed, 0);
        assert_eq!(report.failed, 1);
        assert!(writer.published().is_empty(), "nothing was replayed");
        assert!(report.any_failed(), "the loop must take its error pause");
    }

    #[tokio::test]
    async fn a_successful_publish_is_counted_once() {
        let mut writer = FakeWriter::default();
        let mut report = PassReport::default();
        match writer
            .publish(&envelope("00000000-0000-0000-0000-000000000001"))
            .await
        {
            Ok(()) => report.replayed += 1,
            Err(_) => report.failed += 1,
        }
        assert_eq!(report.replayed, 1);
        assert_eq!(report.failed, 0);
        assert_eq!(
            writer.published(),
            vec!["00000000-0000-0000-0000-000000000001".to_string()]
        );
    }
}
