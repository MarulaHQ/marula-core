//! Stream-consumer delivery bookkeeping over `consumer_deliveries` and
//! `dead_events` (migration 0143).
//!
//! Purpose: give one stream entry a durable history. A consumer that fails on
//! an entry records an attempt with the error and the moment the next try is
//! due; when the attempts run out, the entry is dead-lettered with its event
//! identity and its raw payload. Both tables hold only entries that FAILED —
//! the healthy path settles in memory and acks, so nothing is written for the
//! overwhelming majority of events.
//!
//! Non-purpose: no policy (attempt budgets and delays come from
//! `core_domain::retry`), no stream access, no requeue. This module is SQL.

use crate::{DbError, DbResult};
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// The stream every consumer reads. Stored on each row so a second stream
/// cannot silently share the entry-id namespace.
pub const EVENT_STREAM_NAME: &str = "marula:events";

/// What the harness needs to know about one entry before handling it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryState {
    /// The Valkey stream id, `<ms>-<seq>`.
    pub entry_id: String,
    /// Attempts recorded so far. Zero for an entry that has never failed.
    pub attempts: i32,
    /// Whether the retry delay has elapsed (`next_attempt_at` is null or past).
    pub due: bool,
    /// Whether the entry already exhausted its budget. A redelivered entry in
    /// this state is acked on sight: the work is recorded, not pending.
    pub dead: bool,
}

/// States for the entries of one page. Entries with no row are absent from the
/// answer, which the caller reads as "never failed, handle now".
///
/// Runtime query rather than `sqlx::query!` on purpose: the tables are new, so
/// there is no entry in the committed `.sqlx` cache, and re-preparing that
/// cache belongs to a lane that can run `cargo sqlx prepare`. The integration
/// test in `crates/worker/tests/` executes these statements against a migrated
/// database, which is what proves they parse and bind.
pub async fn delivery_states(
    pool: &PgPool,
    group: &str,
    entry_ids: &[String],
) -> DbResult<Vec<DeliveryState>> {
    let rows = sqlx::query(
        r#"SELECT entry_id, attempts, dead_at IS NOT NULL AS dead,
                  (next_attempt_at IS NULL OR next_attempt_at <= now()) AS due
             FROM consumer_deliveries
            WHERE consumer_group = $1 AND entry_id = ANY($2)"#,
    )
    .bind(group)
    .bind(entry_ids.to_vec())
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(DeliveryState {
                entry_id: row.try_get("entry_id")?,
                attempts: row.try_get("attempts")?,
                due: row.try_get("due")?,
                dead: row.try_get("dead")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(DbError::from)
}

/// One failed attempt. Everything needed to write the row, so the caller does
/// not grow a ten-argument function signature.
#[derive(Debug, Clone)]
pub struct Attempt<'a> {
    pub group: &'a str,
    pub consumer: &'a str,
    pub entry_id: &'a str,
    /// Envelope identity, when the entry parsed. Null for a body that never
    /// parsed — the attempt is still worth counting.
    pub org_id: Option<Uuid>,
    pub event_id: Option<Uuid>,
    pub event_type: Option<&'a str>,
    pub error: &'a str,
    /// When the next live retry becomes due. Zero means "immediately".
    pub retry_in_secs: f64,
}

/// Records one failure and returns the attempt count INCLUDING it.
///
/// The insert is an upsert because the first failure for an entry has no row
/// yet. `next_attempt_at` is computed by Postgres (`now() + make_interval`),
/// so a skewed worker clock cannot retry early or park the entry for an hour.
pub async fn record_attempt(pool: &PgPool, attempt: &Attempt<'_>) -> DbResult<i32> {
    let row = sqlx::query(
        r#"INSERT INTO consumer_deliveries
             (consumer_group, entry_id, stream, consumer, org_id, event_id,
              event_type, attempts, last_error, next_attempt_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, 1, $8,
                   CASE WHEN $9 > 0
                        THEN now() + make_interval(secs => $9)
                        ELSE now() END)
           ON CONFLICT (consumer_group, entry_id) DO UPDATE SET
             attempts = consumer_deliveries.attempts + 1,
             consumer = EXCLUDED.consumer,
             org_id = COALESCE(EXCLUDED.org_id, consumer_deliveries.org_id),
             event_id = COALESCE(EXCLUDED.event_id, consumer_deliveries.event_id),
             event_type = COALESCE(EXCLUDED.event_type, consumer_deliveries.event_type),
             last_attempt_at = now(),
             last_error = EXCLUDED.last_error,
             next_attempt_at = EXCLUDED.next_attempt_at,
             settled_at = NULL,
             dead_at = NULL
           RETURNING attempts"#,
    )
    .bind(attempt.group)
    .bind(attempt.entry_id)
    .bind(EVENT_STREAM_NAME)
    .bind(attempt.consumer)
    .bind(attempt.org_id)
    .bind(attempt.event_id)
    .bind(attempt.event_type)
    .bind(attempt.error)
    .bind(attempt.retry_in_secs)
    .fetch_one(pool)
    .await?;
    let attempts: i32 = row.try_get("attempts")?;
    Ok(attempts)
}

/// Marks a previously failed entry settled. A no-op when there is no row,
/// which is the healthy path: an entry that succeeded first time was never
/// written here at all.
pub async fn settle(pool: &PgPool, group: &str, entry_id: &str) -> DbResult<()> {
    sqlx::query(
        r#"UPDATE consumer_deliveries
              SET settled_at = now(), next_attempt_at = NULL, last_error = NULL
            WHERE consumer_group = $1 AND entry_id = $2
              AND settled_at IS NULL"#,
    )
    .bind(group)
    .bind(entry_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// One abandonment, ready to insert.
#[derive(Debug, Clone)]
pub struct DeadEventSpec<'a> {
    pub group: &'a str,
    pub consumer: &'a str,
    pub entry_id: &'a str,
    pub org_id: Option<Uuid>,
    pub event_id: Option<Uuid>,
    pub event_type: Option<&'a str>,
    pub attempts: i32,
    pub last_error: &'a str,
    /// The raw stream body. Text, not jsonb: an entry that did not parse is
    /// the one most worth keeping, and it cannot be stored as jsonb.
    pub payload: Option<&'a str>,
}

/// Dead-letters one entry and marks its delivery row dead, in one transaction.
///
/// The insert is conflict-tolerant: a reclaimed entry whose ack was lost can
/// be presented twice, and the second presentation must not fail the consumer
/// or produce a second record of the same abandonment.
pub async fn dead_letter(pool: &PgPool, spec: &DeadEventSpec<'_>) -> DbResult<Uuid> {
    let id = Uuid::now_v7();
    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"INSERT INTO dead_events
             (id, stream, consumer_group, entry_id, consumer, org_id, event_id,
              event_type, attempts, last_error, payload)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
           ON CONFLICT (consumer_group, entry_id) DO NOTHING"#,
    )
    .bind(id)
    .bind(EVENT_STREAM_NAME)
    .bind(spec.group)
    .bind(spec.entry_id)
    .bind(spec.consumer)
    .bind(spec.org_id)
    .bind(spec.event_id)
    .bind(spec.event_type)
    .bind(spec.attempts)
    .bind(spec.last_error)
    .bind(spec.payload)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"UPDATE consumer_deliveries
              SET dead_at = COALESCE(dead_at, now()), next_attempt_at = NULL
            WHERE consumer_group = $1 AND entry_id = $2"#,
    )
    .bind(spec.group)
    .bind(spec.entry_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// Rows in the dead-letter queue for one group, newest first. Exposed for the
/// operational read surfaces (a listing verb belongs to a later lane) and used
/// by this lane's integration test to read what the harness wrote.
#[derive(Debug, Clone)]
pub struct DeadEventRow {
    pub id: Uuid,
    pub entry_id: String,
    pub consumer: String,
    pub event_id: Option<Uuid>,
    pub event_type: Option<String>,
    pub org_id: Option<Uuid>,
    pub attempts: i32,
    pub last_error: String,
    pub payload: Option<String>,
}

pub async fn dead_events(pool: &PgPool, group: &str, limit: i64) -> DbResult<Vec<DeadEventRow>> {
    let rows = sqlx::query(
        r#"SELECT id, entry_id, consumer, event_id, event_type, org_id,
                  attempts, last_error, payload
             FROM dead_events
            WHERE consumer_group = $1
            ORDER BY dead_at DESC
            LIMIT $2"#,
    )
    .bind(group)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(DeadEventRow {
                id: row.try_get("id")?,
                entry_id: row.try_get("entry_id")?,
                consumer: row.try_get("consumer")?,
                event_id: row.try_get("event_id")?,
                event_type: row.try_get("event_type")?,
                org_id: row.try_get("org_id")?,
                attempts: row.try_get("attempts")?,
                last_error: row.try_get("last_error")?,
                payload: row.try_get("payload")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(DbError::from)
}

/// Count of live (unsettled, undead) delivery rows for one group — the size of
/// the "this entry keeps failing" set an operator can page on.
pub async fn live_failure_count(pool: &PgPool, group: &str) -> DbResult<i64> {
    let count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM consumer_deliveries
            WHERE consumer_group = $1 AND settled_at IS NULL AND dead_at IS NULL"#,
    )
    .bind(group)
    .fetch_one(pool)
    .await?;
    Ok(count)
}
