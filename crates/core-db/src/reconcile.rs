//! core-db/reconcile — the four queries behind the reconciliation passes
//! (Wave 52 lane R4, `p52-reconciliation-jobs`).
//!
//! Purpose: each pass in `crates/worker/src/reconcile.rs` asks a question only
//! SQL can answer — "which source rows have no index row", "which dead entries
//! may be replayed", "which parked runs can never wake", "which report slots
//! are due another attempt". Keeping the statements here is the crate's rule,
//! not a preference: the worker may not carry SQL.
//!
//! Runtime queries, not the `query!` macro: these are new statements with no
//! `.sqlx` entry, and the precedent for that is `workflow_runs::parked_stats`
//! (R7a) — the lane that adds a statement cannot re-prepare the cache either.
//! The compile-time check those macros would give is replaced by
//! `crates/core-db/tests/reconciliation_queries.rs`, which runs every statement
//! in this module against a migrated database.
//!
//! Non-purpose: no policy. What counts as "too old", how many attempts a slot
//! gets and when a dead entry stops being replayed are the worker's decisions
//! and arrive here as arguments, so this module cannot disagree with the
//! retry matrix.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::DbResult;

// ---------------------------------------------------------------------------
// 1. The search index's missing rows
// ---------------------------------------------------------------------------

/// One source entity that has no `search_index` row.
///
/// `entity_kind` is the `search_index.entity_kind` vocabulary
/// (`core_db::search::SearchKind::as_str`), so the caller can rebuild the
/// indexer's own target from it without a second mapping table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingIndexRow {
    pub org_id: Uuid,
    pub entity_kind: String,
    pub entity_id: Uuid,
}

/// Source rows that are indexed by the event-driven indexer but have no index
/// row: the entities whose event never arrived.
///
/// The seven sources are exactly the ones
/// `worker::search_indexer::match_event` reacts to. The anti-join is against
/// `search_index`'s unique key, so each branch is an index probe per candidate
/// row and the statement stops at `limit` hits — a repaired row stops matching,
/// which is what makes a bounded pass self-advancing rather than a rescan of
/// the same head.
///
/// Archived KB articles are excluded because the indexer drops them
/// (`search_indexer.rs` `Target::Article`: archived = retired). Everything else
/// is included unconditionally: none of the seven tables soft-deletes.
///
/// Deliberately unordered. Ordering the result would mean sorting every missing
/// row per pass; the pass repairs what it is given and the next pass sees a
/// smaller set. The cost of that choice: a permanently unresolvable source row
/// (a document whose head version is missing, say) holds one slot in each pass
/// instead of being skipped, and the backlog gauge keeps reporting it — which
/// is the outcome we want for a row that needs a person.
pub async fn missing_search_rows(pool: &PgPool, limit: i64) -> DbResult<Vec<MissingIndexRow>> {
    let rows: Vec<(Uuid, String, Uuid)> = sqlx::query_as(
        r#"WITH missing AS (
             SELECT t.org_id, 'ticket'::text AS entity_kind, t.id AS entity_id
               FROM tickets t
              WHERE NOT EXISTS (
                    SELECT 1 FROM search_index s
                     WHERE s.org_id = t.org_id
                       AND s.entity_kind = 'ticket'
                       AND s.entity_id = t.id)
             UNION ALL
             SELECT c.org_id, 'comment'::text, c.id
               FROM ticket_comments c
              WHERE NOT EXISTS (
                    SELECT 1 FROM search_index s
                     WHERE s.org_id = c.org_id
                       AND s.entity_kind = 'comment'
                       AND s.entity_id = c.id)
             UNION ALL
             SELECT cl.org_id, 'client'::text, cl.id
               FROM clients cl
              WHERE NOT EXISTS (
                    SELECT 1 FROM search_index s
                     WHERE s.org_id = cl.org_id
                       AND s.entity_kind = 'client'
                       AND s.entity_id = cl.id)
             UNION ALL
             SELECT si.org_id, 'site'::text, si.id
               FROM sites si
              WHERE NOT EXISTS (
                    SELECT 1 FROM search_index s
                     WHERE s.org_id = si.org_id
                       AND s.entity_kind = 'site'
                       AND s.entity_id = si.id)
             UNION ALL
             SELECT co.org_id, 'contact'::text, co.id
               FROM contacts co
              WHERE NOT EXISTS (
                    SELECT 1 FROM search_index s
                     WHERE s.org_id = co.org_id
                       AND s.entity_kind = 'contact'
                       AND s.entity_id = co.id)
             UNION ALL
             SELECT a.org_id, 'kb_article'::text, a.id
               FROM kb_articles a
              WHERE a.status <> 'archived'
                AND NOT EXISTS (
                    SELECT 1 FROM search_index s
                     WHERE s.org_id = a.org_id
                       AND s.entity_kind = 'kb_article'
                       AND s.entity_id = a.id)
             UNION ALL
             SELECT d.org_id, 'document'::text, d.id
               FROM documents d
              WHERE NOT EXISTS (
                    SELECT 1 FROM search_index s
                     WHERE s.org_id = d.org_id
                       AND s.entity_kind = 'document'
                       AND s.entity_id = d.id)
           )
           SELECT org_id, entity_kind, entity_id FROM missing LIMIT $1"#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(org_id, entity_kind, entity_id)| MissingIndexRow {
            org_id,
            entity_kind,
            entity_id,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// 2. Dead stream entries that may be replayed
// ---------------------------------------------------------------------------

/// One dead entry the replay pass may put back on the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayableEvent {
    /// The `dead_events` row id — the replay marker's key, not the stream entry id.
    pub id: Uuid,
    pub org_id: Option<Uuid>,
    pub consumer_group: String,
    /// How many times this row has already been replayed (`0` on the first).
    pub replay_count: i32,
    /// The raw stream body, exactly as the consumer received it.
    pub payload: String,
}

/// Dead entries that are eligible for replay, oldest abandonment first.
///
/// Eligibility is deliberately narrow, because a replay that cannot work is
/// worse than none:
///
/// * `event_id IS NOT NULL` — the body parsed as an envelope when it was
///   received. A body that never parsed will not parse now; re-publishing it
///   would dead-letter it again with the same error, so it stays for a person
///   (0144 already refuses it to the redrive verb for the same reason).
/// * `payload IS NOT NULL` — there is a body to re-publish.
/// * `replayed_at IS NULL` — this row has not been replayed yet. Without this
///   clause a replayed entry that SUCCEEDS would be replayed again on every
///   pass, because nothing about the row changes when the work finally lands.
/// * **the event has not already died more than `max_deaths` times in this
///   group.** A poison envelope fails identically every time, and a replay
///   produces a NEW stream entry, so its next death is a new row with its own
///   fresh `replayed_at`. Counting deaths per `(group, event_id)` across the
///   table is what stops that ping-pong.
/// * not cancelled by an operator: a cancellation is a decision, and 0144
///   keeps it in `dead_work_cancellations` for exactly this reason.
pub async fn replayable_dead_events(
    pool: &PgPool,
    limit: i64,
    max_deaths: i64,
) -> DbResult<Vec<ReplayableEvent>> {
    let rows: Vec<(Uuid, Option<Uuid>, String, i32, String)> = sqlx::query_as(
        r#"SELECT d.id, d.org_id, d.consumer_group, d.replay_count, d.payload
             FROM dead_events d
            WHERE d.replayed_at IS NULL
              AND d.event_id IS NOT NULL
              AND d.payload IS NOT NULL
              AND NOT EXISTS (
                    SELECT 1 FROM dead_work_cancellations c
                     WHERE c.source = 'event'
                       AND c.id = d.id
                       AND c.org_id IS NOT DISTINCT FROM d.org_id)
              AND (SELECT count(*) FROM dead_events p
                    WHERE p.consumer_group = d.consumer_group
                      AND p.event_id = d.event_id) <= $2
            ORDER BY d.dead_at
            LIMIT $1"#,
    )
    .bind(limit)
    .bind(max_deaths)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, org_id, consumer_group, replay_count, payload)| ReplayableEvent {
                id,
                org_id,
                consumer_group,
                replay_count,
                payload,
            },
        )
        .collect())
}

/// Stamps a replay. Idempotent by the `replayed_at IS NULL` guard: two
/// reconcilers racing on one row produce one replay and one no-op, and the
/// second's `rows_affected = 0` tells it to skip the publish it was about to do.
pub async fn mark_replayed(pool: &PgPool, ids: &[Uuid]) -> DbResult<u64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let result = sqlx::query(
        r#"UPDATE dead_events
              SET replayed_at = now(), replay_count = replay_count + 1
            WHERE id = ANY($1)
              AND replayed_at IS NULL"#,
    )
    .bind(ids)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// How much dead work is waiting, per consumer group — the gauge behind the
/// replay pass.
///
/// `replayable` is the number the next pass will attempt. `unrepairable` is the
/// number that will never fix itself: unparsed bodies, missing payloads, and
/// events whose death budget is spent. Splitting them is the point — one number
/// that mixes "will heal on its own" with "needs a person tonight" hides the
/// second.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadEventBacklog {
    pub consumer_group: String,
    pub replayable: i64,
    pub unrepairable: i64,
}

pub async fn dead_event_backlog(pool: &PgPool, max_deaths: i64) -> DbResult<Vec<DeadEventBacklog>> {
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        r#"SELECT d.consumer_group,
                  count(*) FILTER (
                    WHERE d.event_id IS NOT NULL
                      AND d.payload IS NOT NULL
                      AND (SELECT count(*) FROM dead_events p
                            WHERE p.consumer_group = d.consumer_group
                              AND p.event_id = d.event_id) <= $1
                  )::bigint AS replayable,
                  count(*) FILTER (
                    WHERE d.event_id IS NULL
                       OR d.payload IS NULL
                       OR (SELECT count(*) FROM dead_events p
                            WHERE p.consumer_group = d.consumer_group
                              AND p.event_id = d.event_id) > $1
                  )::bigint AS unrepairable
             FROM dead_events d
            WHERE d.replayed_at IS NULL
              AND NOT EXISTS (
                    SELECT 1 FROM dead_work_cancellations c
                     WHERE c.source = 'event'
                       AND c.id = d.id
                       AND c.org_id IS NOT DISTINCT FROM d.org_id)
            GROUP BY d.consumer_group
            ORDER BY d.consumer_group"#,
    )
    .bind(max_deaths)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(consumer_group, replayable, unrepairable)| DeadEventBacklog {
                consumer_group,
                replayable,
                unrepairable,
            },
        )
        .collect())
}

// ---------------------------------------------------------------------------
// 3. Parked runs whose wake path is gone
// ---------------------------------------------------------------------------

/// A parked run whose wake can no longer happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnwakeableRun {
    pub org_id: Uuid,
    pub run_id: Uuid,
}

/// Runs parked on a deadline that has passed, with no live wake job.
///
/// The discriminator between "parked" and "slow" is the wake job, not the
/// clock: `park_run` (`worker/src/workflow.rs`) enqueues exactly one
/// `workflow.resume` job due at the run's `wake_at`, so a run whose deadline is
/// past by `grace_secs` AND whose job is neither `pending` nor `running` has no
/// mechanism left that could wake it. A run whose job is merely late is left
/// alone, however old it is.
///
/// Event waits (`wake_on` with no `wake_at`) are never selected: the event may
/// legitimately arrive tomorrow, and abandoning a run that is doing what it was
/// told would be the worse defect. `grace_secs` is what keeps a slow queue from
/// being mistaken for a dead one.
pub async fn unwakeable_parked_runs(
    pool: &PgPool,
    grace_secs: f64,
    limit: i64,
) -> DbResult<Vec<UnwakeableRun>> {
    let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        r#"SELECT r.org_id, r.id
             FROM workflow_runs r
            WHERE r.status = 'waiting'
              AND r.wake_at IS NOT NULL
              AND r.wake_at < now() - make_interval(secs => $1)
              AND NOT EXISTS (
                    SELECT 1 FROM jobs j
                     WHERE j.org_id = r.org_id
                       AND j.kind = 'workflow.resume'
                       AND j.status IN ('pending', 'running')
                       AND j.payload->>'run_id' = r.id::text)
            ORDER BY r.wake_at
            LIMIT $2"#,
    )
    .bind(grace_secs)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(org_id, run_id)| UnwakeableRun { org_id, run_id })
        .collect())
}

/// Moves unwakeable runs to `abandoned`, which is the ledger's own terminal
/// state for a run that will not continue (`0129`'s status shape), and which
/// `retry_run` already accepts as retryable — so an operator can re-run it once
/// the cause is fixed.
///
/// The status shape constraint decides the column set: a terminal row carries
/// `finished_at`, no `wake_at`, no `wake_on` and no `resume_cursor`. Getting
/// that wrong fails the constraint rather than writing a row that looks
/// finished and still carries its continuation.
pub async fn abandon_unwakeable_runs(
    pool: &PgPool,
    runs: &[UnwakeableRun],
    error_code: &str,
    error: &str,
) -> DbResult<u64> {
    if runs.is_empty() {
        return Ok(0);
    }
    let orgs: Vec<Uuid> = runs.iter().map(|r| r.org_id).collect();
    let ids: Vec<Uuid> = runs.iter().map(|r| r.run_id).collect();
    let result = sqlx::query(
        r#"UPDATE workflow_runs AS r
              SET status = 'abandoned',
                  resume_cursor = NULL,
                  wake_at = NULL,
                  wake_on = NULL,
                  finished_at = now(),
                  updated_at = now(),
                  error_code = $3,
                  error = $4
             FROM unnest($1::uuid[], $2::uuid[]) AS t(org_id, id)
            WHERE r.org_id = t.org_id
              AND r.id = t.id
              AND r.status = 'waiting'"#,
    )
    .bind(&orgs)
    .bind(&ids)
    .bind(error_code)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

// ---------------------------------------------------------------------------
// 4. Report slots due another attempt
// ---------------------------------------------------------------------------

/// One `report_deliveries` slot whose newest outcome was a failure and which
/// still has attempts left.
#[derive(Debug, Clone)]
pub struct DueRetry {
    /// The newest ledger row for the slot — the failure being retried.
    pub row_id: Uuid,
    pub org_id: Uuid,
    pub schedule_id: Uuid,
    pub slot_at: DateTime<Utc>,
    /// How that newest attempt ended: `failed` or `partial`.
    pub status: String,
    /// The per-recipient detail, so the retry can address only the failures.
    pub detail: serde_json::Value,
    /// Every attempt recorded for this slot, including the first one.
    pub attempts: i64,
    /// When the newest attempt was recorded; the retry delay is measured from it.
    pub recorded_at: DateTime<Utc>,
}

/// Failed and partial slots, oldest attempt first.
///
/// `DISTINCT ON (schedule_id, slot_at)` picks the newest row per slot, and the
/// `WHERE` removes every slot whose newest row is `delivered` or `skipped` —
/// so a retried slot that finally landed leaves this set on its own, with no
/// status rewriting and no flag to keep in step. `min_age_secs` is the caller's
/// base delay; the per-attempt delay is applied in the worker, where the retry
/// policy lives.
///
/// The ledger is append-only by design (0038), so a retry adds a row rather
/// than editing the failure: the delivery-history surface then shows the
/// failure, the attempt that recovered it, and the instant of each.
pub async fn due_report_retries(
    pool: &PgPool,
    min_age_secs: f64,
    limit: i64,
) -> DbResult<Vec<DueRetry>> {
    let rows: Vec<(
        Uuid,
        Uuid,
        Uuid,
        DateTime<Utc>,
        String,
        serde_json::Value,
        i64,
        DateTime<Utc>,
    )> = sqlx::query_as(
        r#"SELECT d.id, d.org_id, d.schedule_id, d.slot_at, d.status, d.detail,
                  (SELECT count(*) FROM report_deliveries a
                    WHERE a.schedule_id = d.schedule_id
                      AND a.slot_at = d.slot_at)::bigint AS attempts,
                  d.created_at
             FROM report_deliveries d
            WHERE d.status IN ('failed', 'partial')
              AND d.created_at <= now() - make_interval(secs => $1)
              -- The slot's NEWEST row decides, and the newest row may be the
              -- one that recovered it. Filtering the status before ranking
              -- would keep a slot whose retry already delivered: the delivered
              -- row is filtered out, so the older failure wins the rank.
              AND NOT EXISTS (
                    SELECT 1 FROM report_deliveries n
                     WHERE n.schedule_id = d.schedule_id
                       AND n.slot_at = d.slot_at
                       AND (n.created_at, n.id) > (d.created_at, d.id))
            ORDER BY d.created_at
            LIMIT $2"#,
    )
    .bind(min_age_secs)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(row_id, org_id, schedule_id, slot_at, status, detail, attempts, recorded_at)| {
                DueRetry {
                    row_id,
                    org_id,
                    schedule_id,
                    slot_at,
                    status,
                    detail,
                    attempts,
                    recorded_at,
                }
            },
        )
        .collect())
}

/// Slots past their retry budget, for the gauge that says a person is needed.
pub async fn exhausted_report_retries(pool: &PgPool, max_attempts: i64) -> DbResult<i64> {
    let count: i64 = sqlx::query_scalar(
        r#"SELECT count(*)::bigint
             FROM report_deliveries d
            WHERE d.status IN ('failed', 'partial')
              AND NOT EXISTS (
                    SELECT 1 FROM report_deliveries n
                     WHERE n.schedule_id = d.schedule_id
                       AND n.slot_at = d.slot_at
                       AND (n.created_at, n.id) > (d.created_at, d.id))
              AND (SELECT count(*) FROM report_deliveries a
                    WHERE a.schedule_id = d.schedule_id
                      AND a.slot_at = d.slot_at) >= $1"#,
    )
    .bind(max_attempts)
    .fetch_one(pool)
    .await?;
    Ok(count)
}
