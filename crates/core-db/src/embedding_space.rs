//! The instance embedding space and the re-index runs that move the population
//! from one space to the next (Wave 52 lane R14, migration 0152).
//!
//! Purpose:
//! - [`read_state`] / [`read_spaces`] — which space this instance writes, and
//!   which spaces a retrieval may rank in. During a re-index that is two: the
//!   target, and the previous one when its population is safe to read beside
//!   the target (same model id, unrecorded build).
//! - [`record_switch`] / [`enqueue_missing_runs`] — the operator's model change
//!   turned into a queue: one run per org, grouped by a switch id. The switch is
//!   recorded by core-api at boot (it resolves the model); the runs are created
//!   by the worker's sweep, because the insert spans orgs and only the worker's
//!   owner connection may do that.
//! - [`next_run`] / [`lease`] / [`record_progress`] — the sweep's half. A run is
//!   claimed for a bounded window, and progress is written from the counts the
//!   embedding pass reported, not from a position the pass guessed.
//!
//! Resumability is a property of the selection predicate (`search::rows_needing_embedding`
//! selects rows without a fresh vector in the target space), not of a cursor
//! here. A crash mid-batch repeats at most one batch. `total_rows` and
//! `done_rows` exist for reporting: the gauge an operator watches, and the row a
//! support conversation can read.
//!
//! Non-purpose: no embedding calls (core-api embeds), no scheduling decisions
//! (the worker's loop owns the window and the pacing), and no deletion of the
//! previous space's rows here — the backfill pass deletes them when an org's run
//! completes, in the same transaction that marks the run.

use crate::DbResult;
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool, Postgres};
use uuid::Uuid;

/// The singleton row's id. The embedding model is an instance choice, so its
/// state has no org.
pub const INSTANCE_ID: &str = "instance";

/// How long one claim protects a run. Long enough for a batch at the slowest
/// measured local rate (128 rows at 600 rows/min is 12.8 s) plus the request,
/// short enough that a killed worker's claim expires inside a sweep or two.
pub const LEASE_SECS: i64 = 120;

/// Which space an instance writes, and which one it still reads.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpaceState {
    /// NULL only before the first process with the catalogue recorded a space.
    pub target_space: Option<String>,
    /// Set while a re-index is unfinished and the old population is readable
    /// beside the new one. NULL for a model change, which is not readable.
    pub previous_space: Option<String>,
    /// Groups the runs this switch created. `None` before the first switch.
    pub switch_id: Option<Uuid>,
    pub switched_at: Option<DateTime<Utc>>,
}

impl SpaceState {
    /// The spaces a retrieval may rank in, target first. Empty when no space
    /// has been recorded yet: the vector leg is dark rather than mixing the
    /// untagged population with a query vector from whatever model is running.
    #[must_use]
    pub fn readable(&self) -> Vec<String> {
        let mut spaces = Vec::with_capacity(2);
        if let Some(target) = &self.target_space {
            spaces.push(target.clone());
        }
        if let Some(previous) = &self.previous_space {
            if Some(previous) != self.target_space.as_ref() {
                spaces.push(previous.clone());
            }
        }
        spaces
    }
}

/// One re-index run: one org, one switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReindexRun {
    pub id: Uuid,
    pub switch_id: Uuid,
    pub org_id: Uuid,
    pub from_space: Option<String>,
    pub to_space: String,
    pub status: String,
    pub total_rows: i64,
    pub done_rows: i64,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// What one lease + progress step needs to know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunProgress {
    pub total_rows: i64,
    pub done_rows: i64,
    /// Rows still lacking a fresh vector in the target space.
    pub remaining: i64,
}

/// The state row. A missing row reads as "nothing recorded", which is what the
/// first boot after migration 0152 sees.
pub async fn read_state<'e, E>(executor: E) -> DbResult<SpaceState>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT target_space, previous_space, switch_id, switched_at
           FROM embedding_space_state WHERE id = $1"#,
        INSTANCE_ID
    )
    .fetch_optional(executor)
    .await?;
    Ok(match row {
        Some(row) => SpaceState {
            target_space: row.target_space,
            previous_space: row.previous_space,
            switch_id: row.switch_id,
            switched_at: row.switched_at,
        },
        None => SpaceState::default(),
    })
}

/// The spaces this instance may rank in right now, target first.
pub async fn read_spaces<'e, E>(executor: E) -> DbResult<Vec<String>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(read_state(executor).await?.readable())
}

/// What one recorded switch changed, for the boot log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedSwitch {
    pub switch_id: Uuid,
    /// The space the instance wrote before this call. `None` on the first boot
    /// after migration 0152.
    pub from_space: Option<String>,
    pub to_space: String,
    /// Whether the old population stayed readable beside the new one.
    pub previous_readable: bool,
}

/// Records the instance's space, returning the switch when it changed.
///
/// A process that resolves the same target as the recorded one writes nothing,
/// so a restart is not a re-index. A different target keeps the old one as
/// `previous_space` only when `previous_is_readable` says the two populations
/// are comparable — same model id — and clears it otherwise, which is what
/// makes a real model change go dark rather than mixed.
pub async fn record_switch(
    conn: &mut PgConnection,
    target: &str,
    previous_is_readable: bool,
) -> DbResult<Option<RecordedSwitch>> {
    let current = read_state(&mut *conn).await?;
    if current.target_space.as_deref() == Some(target) {
        return Ok(None);
    }
    let switch_id = Uuid::now_v7();
    let from_space = current.target_space.clone();
    let previous = match &from_space {
        Some(old) if previous_is_readable => Some(old.as_str()),
        _ => None,
    };
    sqlx::query!(
        r#"UPDATE embedding_space_state
              SET target_space = $2,
                  previous_space = $3,
                  switch_id = $4,
                  switched_at = now(),
                  updated_at = now()
            WHERE id = $1"#,
        INSTANCE_ID,
        target,
        previous,
        switch_id
    )
    .execute(conn)
    .await?;
    Ok(Some(RecordedSwitch {
        switch_id,
        from_space: from_space.clone(),
        to_space: target.to_string(),
        previous_readable: previous.is_some(),
    }))
}

/// Queues one run per org that still holds rows outside `to_space`.
///
/// Called by the worker's sweep, not by core-api: the insert spans orgs, and
/// the only role that may do that is the worker's owner connection. The rows
/// themselves are core-api's (it owns every embedding write); this table is
/// the sweep's own bookkeeping, which is why it carries the policy above.
///
/// An org is queued when it has at least one index row with no fresh vector in
/// `to_space` — the same predicate [`crate::search::rows_needing_embedding`]
/// works from. An org that is already caught up gets no run, which also means a
/// brand-new tenant waits for its first row rather than for a switch.
///
/// Idempotent on `(switch_id, org_id)`: a retried boot, a restarted worker or
/// two sweeps racing all converge on one run per org.
///
/// Returns how many runs this call created.
pub async fn enqueue_missing_runs(pool: &PgPool, switch_id: Uuid, to_space: &str) -> DbResult<u64> {
    let result = sqlx::query(
        r#"INSERT INTO embedding_reindex_runs
             (id, switch_id, org_id, from_space, to_space, status, total_rows)
           SELECT gen_random_uuid(), $1, i.org_id,
                  (SELECT CASE WHEN COUNT(DISTINCT e.space) = 1
                               THEN MIN(e.space) END
                     FROM search_embeddings e
                    WHERE e.org_id = i.org_id AND e.space <> $2),
                  $2, 'queued', COUNT(*)
             FROM search_index i
            WHERE EXISTS (
                    SELECT 1
                      FROM search_index s
                      LEFT JOIN search_embeddings e
                        ON e.org_id = s.org_id
                       AND e.entity_kind = s.entity_kind
                       AND e.entity_id = s.entity_id
                       AND e.space = $2
                     WHERE s.org_id = i.org_id
                       AND (
                             e.id IS NULL
                          OR e.content_hash <> md5(s.title || E'\x1f' || s.body)
                         )
                  )
            GROUP BY i.org_id
           ON CONFLICT (switch_id, org_id) DO NOTHING"#,
    )
    .bind(switch_id)
    .bind(to_space)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Every distinct space stored for one org. Used to decide whether the old
/// population is readable beside the new one: a population under one model id
/// that matches the target is a re-recorded build, and a population under any
/// other id is not comparable.
pub async fn org_spaces<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<String>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let spaces = sqlx::query_scalar!(
        r#"SELECT DISTINCT space FROM search_embeddings WHERE org_id = $1 ORDER BY space"#,
        org_id
    )
    .fetch_all(executor)
    .await?;
    Ok(spaces)
}

/// The next run the sweep may claim: unfinished, and either unclaimed or past
/// its lease. Reads on the worker's owner pool, across orgs.
pub async fn next_run(pool: &PgPool) -> DbResult<Option<ReindexRun>> {
    let row = sqlx::query!(
        r#"SELECT id, switch_id, org_id, from_space, to_space, status,
                  total_rows, done_rows, last_error, created_at
           FROM embedding_reindex_runs
          WHERE status IN ('queued', 'running')
            AND (leased_until IS NULL OR leased_until < now())
          ORDER BY created_at, org_id
          LIMIT 1"#
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| ReindexRun {
        id: row.id,
        switch_id: row.switch_id,
        org_id: row.org_id,
        from_space: row.from_space,
        to_space: row.to_space,
        status: row.status,
        total_rows: row.total_rows,
        done_rows: row.done_rows,
        last_error: row.last_error,
        created_at: row.created_at,
    }))
}

/// Claims a run for [`LEASE_SECS`]. `false` means another sweep won the claim,
/// and the caller moves on rather than working the same org twice.
pub async fn lease(conn: &mut PgConnection, run_id: Uuid) -> DbResult<bool> {
    let result = sqlx::query!(
        r#"UPDATE embedding_reindex_runs
              SET status = 'running',
                  leased_until = now() + ($2::bigint * interval '1 second'),
                  started_at = COALESCE(started_at, now()),
                  updated_at = now()
            WHERE id = $1
              AND status IN ('queued', 'running')
              AND (leased_until IS NULL OR leased_until < now())"#,
        run_id,
        LEASE_SECS
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Writes one pass's progress. `remaining == 0` completes the run.
///
/// `total_rows` is snapshotted on the first pass and left alone afterwards: it
/// is the denominator an operator reads, and a population that grows mid-run
/// (new tickets arriving) should not make progress look like a crawl.
pub async fn record_progress(
    conn: &mut PgConnection,
    run_id: Uuid,
    total_rows: i64,
    remaining: i64,
    error: Option<&str>,
) -> DbResult<RunProgress> {
    let done_rows = (total_rows - remaining).max(0);
    let status = if error.is_some() {
        "failed"
    } else if remaining <= 0 {
        "complete"
    } else {
        "running"
    };
    sqlx::query!(
        r#"UPDATE embedding_reindex_runs
              SET total_rows = CASE WHEN total_rows = 0 THEN $2 ELSE total_rows END,
                  done_rows = $3,
                  status = $4,
                  last_error = $5,
                  leased_until = NULL,
                  finished_at = CASE WHEN $4 IN ('complete', 'failed') THEN now() ELSE NULL END,
                  updated_at = now()
            WHERE id = $1"#,
        run_id,
        total_rows,
        done_rows,
        status,
        error
    )
    .execute(conn)
    .await?;
    Ok(RunProgress {
        total_rows,
        done_rows,
        remaining,
    })
}

/// Releases a claim without recording anything: the pass that held it finished
/// its batch and wants the next one to be claimable immediately.
pub async fn release(conn: &mut PgConnection, run_id: Uuid) -> DbResult<()> {
    sqlx::query!(
        r#"UPDATE embedding_reindex_runs
              SET leased_until = NULL,
                  updated_at = now()
            WHERE id = $1
              AND status IN ('queued', 'running')"#,
        run_id
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Keeps a run alive after a failure it can recover from: the message is
/// stored, the lease is released, and the next sweep retries it.
///
/// This is the dark-leg case (the provider needs a credential an operator has
/// not set yet) and the outage case (the provider answered badly once). Both
/// are fixed by waiting; marking the run `failed` would mean the install never
/// finishes its re-index after the operator does set the key.
pub async fn note_error(conn: &mut PgConnection, run_id: Uuid, error: &str) -> DbResult<()> {
    sqlx::query!(
        r#"UPDATE embedding_reindex_runs
              SET last_error = $2,
                  leased_until = NULL,
                  updated_at = now()
            WHERE id = $1
              AND status IN ('queued', 'running')"#,
        run_id,
        error
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Ends a run that will never be worked: its target space is not the instance's
/// any more, because the model changed again before the sweep reached it.
/// Retrying would rewrite rows into a space nothing reads.
pub async fn fail_run(conn: &mut PgConnection, run_id: Uuid, error: &str) -> DbResult<()> {
    sqlx::query!(
        r#"UPDATE embedding_reindex_runs
              SET status = 'failed',
                  last_error = $2,
                  leased_until = NULL,
                  finished_at = now(),
                  updated_at = now()
            WHERE id = $1
              AND status IN ('queued', 'running')"#,
        run_id,
        error
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Unfinished runs on the whole install: the sweep's backlog gauge.
pub async fn runs_pending<'e, E>(executor: E) -> DbResult<i64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let count = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "count!"
           FROM embedding_reindex_runs
          WHERE status IN ('queued', 'running')"#
    )
    .fetch_one(executor)
    .await?;
    Ok(count)
}

/// One org's runs, newest first — the operator's view of "is this org's
/// re-index moving".
pub async fn list_runs<'e, E>(executor: E, org_id: Uuid, limit: i64) -> DbResult<Vec<ReindexRun>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, switch_id, org_id, from_space, to_space, status,
                  total_rows, done_rows, last_error, created_at
           FROM embedding_reindex_runs
          WHERE org_id = $1
          ORDER BY created_at DESC
          LIMIT $2"#,
        org_id,
        limit
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ReindexRun {
            id: row.id,
            switch_id: row.switch_id,
            org_id: row.org_id,
            from_space: row.from_space,
            to_space: row.to_space,
            status: row.status,
            total_rows: row.total_rows,
            done_rows: row.done_rows,
            last_error: row.last_error,
            created_at: row.created_at,
        })
        .collect())
}

/// Clears `previous_space` once no run is unfinished. The last org's completion
/// is what ends the readable window, not a timer: a slow tenant keeps the
/// previous space readable for the whole install until its own rows land.
pub async fn clear_previous_when_done<'e, E>(executor: E) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"UPDATE embedding_space_state
              SET previous_space = NULL, updated_at = now()
            WHERE id = $1
              AND previous_space IS NOT NULL
              AND NOT EXISTS (
                SELECT 1 FROM embedding_reindex_runs
                 WHERE status IN ('queued', 'running')
              )"#,
        INSTANCE_ID
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// The target space this instance writes, or `None` when nothing recorded one.
pub async fn target_space<'e, E>(executor: E) -> DbResult<Option<String>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(read_state(executor).await?.target_space)
}
