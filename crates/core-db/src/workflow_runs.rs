//! The workflow run ledger's persistence (TASKLIST Wave 43 lane 14,
//! `p43-workflow-run-ledger`; migration 0129).
//!
//! Purpose: the only place SQL for `workflow_runs` and `workflow_run_steps`
//! lives — one row per EXECUTION ATTEMPT, one row per STEP ATTEMPT, and the
//! operator retry that never silently double-applies.
//!
//! # What the writes are allowed to do
//!
//! Migration 0129's triggers make the identity, provenance and chain
//! columns immutable, so the functions here can only ever move a row
//! through its lifecycle. That is the whole design: a run is recorded, not
//! edited. The two ways a run's story continues are INSERTS:
//!
//! * [`begin_resumed`] — the wake job creates the continuation of a parked
//!   run as a NEW row carrying `resumed_from`. The parked row is not
//!   touched; the listing derives "superseded" from the child's existence.
//! * [`retry_run`] — the operator action re-runs a failed run as a NEW row
//!   carrying `retry_of`, and refuses when re-running would repeat a step
//!   that declares `idempotent = false` (lane 13's registry decides).
//!
//! # The reads are the webhook delivery ledger's shape
//!
//! Keyset-cursored on `(created_at DESC, id DESC)`, exactly like
//! `webhooks::list_deliveries`, so the two logs page and behave alike and
//! an operator does not have to learn a second paging convention.
//!
//! # Retention
//!
//! Runs are the highest-volume rows this engine produces. The window is an
//! org setting (`workflow.run_retention_days`, declared in core-domain)
//! resolved per org by [`retention_days`], and [`purge_expired_everywhere`]
//! is what the worker's sweep calls. Only TERMINAL runs are ever swept —
//! a parked run with a wake-at in the past is owed a wake, not a delete.
//!
//! Non-purpose: no execution, no scheduling decisions (the queue at
//! `queue.rs` owns those), no events. The `workflow.resume` job kind the
//! wake rides is settled by the executor lane; it is enqueued through
//! `queue::enqueue` so the queue is recorded against, never forked.

use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use core_domain::settings::{SettingValue, WORKFLOW_RUN_RETENTION_DAYS};
use core_domain::workflow_runs::{self, RetryMode, RunStatus, RunTrigger, StepStatus};
use serde_json::Value;
use sqlx::Postgres;
use sqlx::Row;
use uuid::Uuid;

/// The job kind a parked or retried run is woken by. Re-exported from the
/// domain so a caller enqueuing the wake never spells it differently.
pub use core_domain::workflow_runs::WORKFLOW_RESUME_JOB;

/// One execution attempt of a workflow document.
#[derive(Debug, Clone)]
pub struct RunRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub rule_id: Option<Uuid>,
    pub rule_name: String,
    pub rule_version: i32,
    pub rule_definition: Value,
    pub catalogue_version: String,
    pub status: RunStatus,
    pub trigger: RunTrigger,
    pub trigger_event_id: Option<Uuid>,
    pub trigger_job_id: Option<Uuid>,
    pub trigger_user_id: Option<Uuid>,
    pub resumed_from: Option<Uuid>,
    pub retry_of: Option<Uuid>,
    pub resume_cursor: Option<Value>,
    pub wake_at: Option<DateTime<Utc>>,
    pub wake_on: Option<String>,
    pub error_code: Option<String>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A run as a listing returns it: the row plus the two facts DERIVED from
/// the chain rather than stored on it. A stored "superseded" flag would be
/// a mutation of the parked row wearing a different name, so the ledger
/// derives them at read time instead.
#[derive(Debug, Clone)]
pub struct RunListRow {
    pub run: RunRow,
    /// A later run continues this one (`resumed_from` points here).
    pub superseded: bool,
    /// A later run re-runs this one (`retry_of` points here).
    pub retried: bool,
}

/// One step attempt inside a run.
#[derive(Debug, Clone)]
pub struct RunStepRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub run_id: Uuid,
    pub step_index: i32,
    pub node_id: String,
    pub node_label: String,
    pub attempt: i32,
    pub iteration: Option<i32>,
    pub status: StepStatus,
    pub input: Value,
    pub output: Option<Value>,
    pub error_code: Option<String>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// What a new run needs. Everything else about the row is lifecycle.
#[derive(Debug, Clone)]
pub struct NewRun {
    /// The rule this run executes. `None` for a document an operator ran
    /// directly; the snapshot below is what makes the row self-describing
    /// either way.
    pub rule_id: Option<Uuid>,
    pub rule_name: String,
    pub rule_version: i32,
    /// The exact document that ran, snapshotted at start.
    pub rule_definition: Value,
    /// The node-catalogue release the document was validated against.
    pub catalogue_version: String,
    pub trigger: RunTrigger,
    pub trigger_event_id: Option<Uuid>,
    /// The `jobs` row that woke this run — recorded against the EXISTING
    /// queue, never a fork of it.
    pub trigger_job_id: Option<Uuid>,
    pub trigger_user_id: Option<Uuid>,
    /// The parked run this row continues. Set by [`begin_resumed`] and by
    /// nothing else: a row that continues a parked run IS the continuation,
    /// so the pointer is written with the row rather than patched in after
    /// it (a patched pointer would be a lie the row could not keep).
    pub resumed_from: Option<Uuid>,
    /// The cursor the row is opened WITH. The continuation of a parked run
    /// inherits the parked row's cursor, because that is the state it
    /// continues from; a fresh attempt opens with none.
    pub resume_cursor: Option<Value>,
}

/// What a new step attempt needs.
#[derive(Debug, Clone)]
pub struct NewStep {
    pub step_index: i32,
    pub node_id: String,
    pub node_label: String,
    /// 1 for the first try; higher for an in-run retry of the same step.
    pub attempt: i32,
    /// The bounded fan-out iteration, when the node ran inside one.
    pub iteration: Option<i32>,
    pub input: Value,
}

/// Filters for the run listing. All optional; unset = unfiltered.
#[derive(Debug, Default, Clone)]
pub struct RunFilter {
    pub status: Option<RunStatus>,
    pub rule_id: Option<Uuid>,
    /// Keyset cursor: strictly older than (created_at, id).
    pub cursor: Option<(DateTime<Utc>, Uuid)>,
}

/// The run's ledger columns, in the order every read selects them.
const RUN_COLUMNS: &str = "id, org_id, rule_id, rule_name, rule_version, rule_definition, \
     catalogue_version, status, trigger_kind, trigger_event_id, trigger_job_id, \
     trigger_user_id, resumed_from, retry_of, resume_cursor, wake_at, wake_on, \
     error_code, error, started_at, finished_at, created_at, updated_at";

/// The step's ledger columns, in the order every read selects them.
const STEP_COLUMNS: &str = "id, org_id, run_id, step_index, node_id, node_label, attempt, \
     iteration, status, input, output, error_code, error, started_at, finished_at";

// --- the write contract the executor consumes, in call order ----------------
//
// 1. `insert_run`      — the attempt starts (`running`).
// 2. `insert_step`     — a step starts, with its input already written.
// 3. `finish_step`     — the step's outcome (succeeded/failed/abandoned/skipped).
// 4. `park_run`        — the run parks (`waiting`) with cursor + wake coordinates.
// 5. `begin_resumed`   — the wake job creates the CONTINUATION (a new row).
// 6. `finish_run`      — the run ends (succeeded/failed/abandoned).
// 7. `retry_run`       — an operator re-runs a failed run (a new row, refused
//                        when it would double-apply).

/// Records a new execution attempt. The row starts `running`: an attempt
/// that has not started is not a run.
pub async fn insert_run(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    run: &NewRun,
    status: RunStatus,
) -> DbResult<RunRow> {
    let id = Uuid::now_v7();
    let row = sqlx::query(&format!(
        "INSERT INTO workflow_runs
            (id, org_id, rule_id, rule_name, rule_version, rule_definition,
             catalogue_version, status, trigger_kind, trigger_event_id,
             trigger_job_id, trigger_user_id, resumed_from, resume_cursor)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
         RETURNING {RUN_COLUMNS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(run.rule_id)
    .bind(&run.rule_name)
    .bind(run.rule_version)
    .bind(&run.rule_definition)
    .bind(&run.catalogue_version)
    .bind(status.as_str())
    .bind(run.trigger.as_str())
    .bind(run.trigger_event_id)
    .bind(run.trigger_job_id)
    .bind(run.trigger_user_id)
    .bind(run.resumed_from)
    .bind(run.resume_cursor.as_ref())
    .fetch_one(&mut *conn)
    .await?;
    Ok(run_from_row(&row).0)
}

/// Reads one run inside an org-scoped transaction. A miss is `NotFound`,
/// which under row-level security is indistinguishable from "another
/// org's run" — the read posture keeps that honest.
pub async fn get_run(conn: &mut sqlx::PgConnection, org_id: Uuid, id: Uuid) -> DbResult<RunRow> {
    let row = sqlx::query(&format!(
        "SELECT {RUN_COLUMNS} FROM workflow_runs WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(|r| run_from_row(&r).0).ok_or(DbError::NotFound)
}

/// Keyset page of the run ledger, newest first — the same pattern as the
/// webhook delivery ledger. `cursor` is `(created_at, id)` of the last row
/// of the previous page.
pub async fn list_runs(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    filter: &RunFilter,
    limit: i64,
) -> DbResult<Vec<RunListRow>> {
    let rows = sqlx::query(&format!(
        "SELECT {RUN_COLUMNS},
                EXISTS (SELECT 1 FROM workflow_runs c WHERE c.resumed_from = r.id)
                  AS superseded,
                EXISTS (SELECT 1 FROM workflow_runs c WHERE c.retry_of = r.id)
                  AS retried
         FROM workflow_runs r
         WHERE r.org_id = $1
           AND ($2::text IS NULL OR r.status = $2)
           AND ($3::uuid IS NULL OR r.rule_id = $3)
           AND ($4::timestamptz IS NULL OR $5::uuid IS NULL OR
                (r.created_at, r.id) < ($4::timestamptz, $5::uuid))
         ORDER BY r.created_at DESC, r.id DESC
         LIMIT $6"
    ))
    .bind(org_id)
    .bind(filter.status.map(|s| s.as_str()))
    .bind(filter.rule_id)
    .bind(filter.cursor.map(|c| c.0))
    .bind(filter.cursor.map(|c| c.1))
    .bind(limit.clamp(1, 200))
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .iter()
        .map(|r| {
            let (run, superseded, retried) = run_from_row(r);
            RunListRow {
                run,
                superseded,
                retried,
            }
        })
        .collect())
}

/// Keyset page of one run's step trace, in EXECUTION order. Paged forward
/// because a trace is read top to bottom, and paged at all because a
/// bounded fan-out can produce a long one.
pub async fn list_steps(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    run_id: Uuid,
    cursor: Option<(i32, i32, Uuid)>,
    limit: i64,
) -> DbResult<Vec<RunStepRow>> {
    let rows = sqlx::query(&format!(
        "SELECT {STEP_COLUMNS}
         FROM workflow_run_steps
         WHERE org_id = $1 AND run_id = $2
           AND ($3::int IS NULL OR $4::int IS NULL OR $5::uuid IS NULL OR
                (step_index, attempt, id) > ($3::int, $4::int, $5::uuid))
         ORDER BY step_index ASC, attempt ASC, id ASC
         LIMIT $6"
    ))
    .bind(org_id)
    .bind(run_id)
    .bind(cursor.map(|c| c.0))
    .bind(cursor.map(|c| c.1))
    .bind(cursor.map(|c| c.2))
    .bind(limit.clamp(1, 200))
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.iter().map(step_from_row).collect())
}

/// Parked runs inside one org waiting on one event type — the executor's
/// WAITERS INDEX read (Wave 43 lane 15, migration 0133's
/// `idx_workflow_runs_waiters`). `flow.waitFor` parks a run with
/// `wake_on` = a COMMA-SEPARATED list of the event types it accepts, so a
/// wait on several types is one row and this read matches any of them.
/// Cross-org reads are the owner pool's job; this is org-scoped, so callers
/// pass the envelope's org.
pub async fn list_waiting_on_event(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    event_type: &str,
) -> DbResult<Vec<RunRow>> {
    let rows = sqlx::query(&format!(
        "SELECT {RUN_COLUMNS} FROM workflow_runs
         WHERE org_id = $1 AND status = 'waiting'
           AND $2 = ANY(string_to_array(wake_on, ','))
         ORDER BY created_at ASC"
    ))
    .bind(org_id)
    .bind(event_type)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.iter().map(|r| run_from_row(r).0).collect())
}

/// The run's whole trace, in execution order. A run's steps are bounded by
/// its graph and its fan-out caps, so the detail read takes them in one
/// shot rather than paging a trace an operator is reading top to bottom.
pub async fn steps_for_run(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    run_id: Uuid,
) -> DbResult<Vec<RunStepRow>> {
    let rows = sqlx::query(&format!(
        "SELECT {STEP_COLUMNS} FROM workflow_run_steps
         WHERE org_id = $1 AND run_id = $2
         ORDER BY step_index ASC, attempt ASC, id ASC"
    ))
    .bind(org_id)
    .bind(run_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.iter().map(step_from_row).collect())
}

/// Records a step as it STARTS, with its input already written. The input
/// is part of the record: the trigger guard makes it immutable, so a step
/// that receives different input is a different row.
pub async fn insert_step(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    run_id: Uuid,
    step: &NewStep,
) -> DbResult<RunStepRow> {
    let id = Uuid::now_v7();
    let row = sqlx::query(&format!(
        "INSERT INTO workflow_run_steps
            (id, org_id, run_id, step_index, node_id, node_label, attempt,
             iteration, status, input)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'running', $9)
         RETURNING {STEP_COLUMNS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(run_id)
    .bind(step.step_index)
    .bind(&step.node_id)
    .bind(&step.node_label)
    .bind(step.attempt)
    .bind(step.iteration)
    .bind(&step.input)
    .fetch_one(&mut *conn)
    .await?;
    Ok(step_from_row(&row))
}

/// Records a step's outcome. Guarded on the step still being in flight, so
/// a double-finish fails instead of overwriting the first answer — the
/// operator is looking at exactly that row.
pub async fn finish_step(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    step_id: Uuid,
    status: StepStatus,
    output: Option<Value>,
    error_code: Option<&str>,
    error: Option<&str>,
) -> DbResult<()> {
    debug_assert!(
        !matches!(status, StepStatus::Running | StepStatus::Waiting),
        "finish_step takes a terminal step status"
    );
    let res = sqlx::query(
        "UPDATE workflow_run_steps
         SET status = $3, output = $4, error_code = $5, error = $6,
             finished_at = now()
         WHERE org_id = $1 AND id = $2 AND status IN ('running','waiting')",
    )
    .bind(org_id)
    .bind(step_id)
    .bind(status.as_str())
    .bind(output)
    .bind(error_code)
    .bind(error)
    .execute(&mut *conn)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Parks a STEP: the wait node the run stopped at. Unlike [`finish_step`]
/// this is not a terminal state — the continuation resumes the wait — so it
/// is its own function rather than a `finish_step` variant. The step's
/// identity and input stay immutable; only status and output move.
pub async fn park_step(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    step_id: Uuid,
    output: Value,
) -> DbResult<()> {
    let res = sqlx::query(
        "UPDATE workflow_run_steps
         SET status = 'waiting', output = $3
         WHERE org_id = $1 AND id = $2 AND status = 'running'",
    )
    .bind(org_id)
    .bind(step_id)
    .bind(output)
    .execute(&mut *conn)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Parks a run: `waiting`, with where it resumes from, when it wakes and
/// what it waits on. Only a RUNNING run can park — a run that already
/// finished was never parked, and rewriting it into `waiting` would be the
/// audit turning into fiction.
pub async fn park_run(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    run_id: Uuid,
    resume_cursor: Value,
    wake_at: Option<DateTime<Utc>>,
    wake_on: Option<&str>,
) -> DbResult<()> {
    let res = sqlx::query(
        "UPDATE workflow_runs
         SET status = 'waiting', resume_cursor = $3, wake_at = $4, wake_on = $5,
             updated_at = now()
         WHERE org_id = $1 AND id = $2 AND status = 'running'",
    )
    .bind(org_id)
    .bind(run_id)
    .bind(resume_cursor)
    .bind(wake_at)
    .bind(wake_on)
    .execute(&mut *conn)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Creates the CONTINUATION of a parked run: a NEW row, `running`, carrying
/// `resumed_from` and the cursor it continues from. The parked row is not
/// touched — this is the function that makes A2 true, so it deliberately
/// contains no statement that could edit it.
pub async fn begin_resumed(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    parent_run_id: Uuid,
    trigger_job_id: Option<Uuid>,
) -> DbResult<RunRow> {
    // Lock the parked row: two concurrent wakes (a time wake and an event
    // wake, say) must not BOTH create a continuation. The second blocks
    // here, then sees the child and is refused.
    sqlx::query("SELECT id FROM workflow_runs WHERE org_id = $1 AND id = $2 FOR UPDATE")
        .bind(org_id)
        .bind(parent_run_id)
        .fetch_optional(&mut *conn)
        .await?;
    let already_resumed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM workflow_runs WHERE org_id = $1 AND resumed_from = $2)",
    )
    .bind(org_id)
    .bind(parent_run_id)
    .fetch_one(&mut *conn)
    .await?;
    if already_resumed {
        return Err(DbError::Conflict(format!(
            "run {parent_run_id} already has a continuation"
        )));
    }
    let parent = get_run(&mut *conn, org_id, parent_run_id).await?;
    // The parked row carries the state the continuation continues from; a
    // parent that is not parked has nothing to continue.
    if parent.status != RunStatus::Waiting {
        return Err(DbError::Conflict(format!(
            "run {} is {}, not waiting: there is nothing to resume",
            parent_run_id, parent.status
        )));
    }
    insert_run(
        &mut *conn,
        org_id,
        &NewRun {
            rule_id: parent.rule_id,
            rule_name: parent.rule_name,
            rule_version: parent.rule_version,
            rule_definition: parent.rule_definition,
            catalogue_version: parent.catalogue_version,
            trigger: RunTrigger::Resume,
            trigger_event_id: parent.trigger_event_id,
            trigger_job_id,
            trigger_user_id: None,
            // The pointer and the cursor are written WITH the row: the
            // continuation carries the state it continues from forward. The
            // parked row is still not touched, which is the point.
            resumed_from: Some(parent_run_id),
            resume_cursor: parent.resume_cursor,
        },
        RunStatus::Running,
    )
    .await
}

/// Moves a run from parked (or queued-for-retry) to executing. The wake-at
/// and the wake reason are consumed; the cursor is NOT, because a running
/// executor keeps its live state there.
pub async fn begin_run(conn: &mut sqlx::PgConnection, org_id: Uuid, run_id: Uuid) -> DbResult<()> {
    let res = sqlx::query(
        "UPDATE workflow_runs
         SET status = 'running', wake_at = NULL, wake_on = NULL, updated_at = now()
         WHERE org_id = $1 AND id = $2 AND status = 'waiting'",
    )
    .bind(org_id)
    .bind(run_id)
    .execute(&mut *conn)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Ends a run: a terminal status, its finish time, and its error when it
/// broke. The park coordinates are consumed — a finished run is not owed a
/// wake.
pub async fn finish_run(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    run_id: Uuid,
    status: RunStatus,
    error_code: Option<&str>,
    error: Option<&str>,
) -> DbResult<()> {
    debug_assert!(status.is_terminal(), "finish_run takes a terminal status");
    let res = sqlx::query(
        "UPDATE workflow_runs
         SET status = $3, resume_cursor = NULL, wake_at = NULL, wake_on = NULL,
             error_code = $4, error = $5, finished_at = now(), updated_at = now()
         WHERE org_id = $1 AND id = $2 AND status IN ('running','waiting')",
    )
    .bind(org_id)
    .bind(run_id)
    .bind(status.as_str())
    .bind(error_code)
    .bind(error)
    .execute(&mut *conn)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// The operator retry: a first-class action, not a queue flag.
///
/// What it refuses to do is the point. Re-running a failed run must not
/// silently double-apply the steps that already succeeded, and the only
/// authority on that is lane 13's per-node `idempotent` flag. So the
/// decision is made by `core_domain::workflow_runs::retry_plan` — a pure
/// function over the run's steps — and this function persists its answer
/// as a NEW row:
///
/// * `trigger_kind = operator_retry` and `retry_of` = the run being
///   retried, so the ledger records that it was a retry and the operator
///   surface can follow the chain;
/// * `trigger_job_id` = the `workflow.resume` job on the EXISTING queue that
///   will pick the re-run up, enqueued by the caller INSIDE this
///   transaction. It is recorded because it is immutable once written: the
///   retry and the job that runs it are one fact.
/// * `status = waiting` with `wake_on = 'queue'` and the retry's cursor,
///   because the re-run has not started yet;
/// * the failed run is NOT touched. It stays `failed`, which is what
///   happened, and the retry row is what happens next.
pub async fn retry_run(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    run_id: Uuid,
    actor_id: Uuid,
    trigger_job_id: Option<Uuid>,
    mode: RetryMode,
) -> DbResult<RunRow> {
    let run = get_run(&mut *conn, org_id, run_id).await?;
    let steps = steps_for_run(&mut *conn, org_id, run_id).await?;
    let inputs: Vec<workflow_runs::StepRetryInput> = steps
        .iter()
        .map(|s| workflow_runs::StepRetryInput {
            step_index: s.step_index,
            node_id: s.node_id.clone(),
            input: s.input.clone(),
            status: s.status,
        })
        .collect();
    // The refusal happens BEFORE any insert: a retry that would
    // double-apply is never recorded as having happened.
    let plan = workflow_runs::retry_plan(run.status, &inputs, mode)
        .map_err(|e| DbError::Conflict(e.to_string()))?;
    let cursor = workflow_runs::retry_cursor(&plan);

    let id = Uuid::now_v7();
    let row = sqlx::query(&format!(
        "INSERT INTO workflow_runs
            (id, org_id, rule_id, rule_name, rule_version, rule_definition,
             catalogue_version, status, trigger_kind, trigger_event_id,
             trigger_job_id, trigger_user_id, retry_of, resume_cursor,
             wake_at, wake_on)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'waiting', 'operator_retry', $8,
                 $9, $10, $11, $12, now(), 'queue')
         RETURNING {RUN_COLUMNS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(run.rule_id)
    .bind(&run.rule_name)
    .bind(run.rule_version)
    .bind(&run.rule_definition)
    .bind(&run.catalogue_version)
    .bind(run.trigger_event_id)
    .bind(trigger_job_id)
    .bind(actor_id)
    .bind(run_id)
    .bind(&cursor)
    .fetch_one(&mut *conn)
    .await?;
    Ok(run_from_row(&row).0)
}

/// The retention window for one org: the org's override when it set one,
/// the declared default otherwise. Read through the typed settings
/// registry's own key so an out-of-range value falls back to the default
/// instead of silently extending the table.
pub async fn retention_days(conn: &mut sqlx::PgConnection, org_id: Uuid) -> DbResult<i64> {
    let stored: Option<Value> = sqlx::query_scalar::<_, Value>(
        "SELECT value FROM settings WHERE org_id = $1 AND scope = 'org' AND key = $2",
    )
    .bind(org_id)
    .bind(WORKFLOW_RUN_RETENTION_DAYS)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(workflow_runs::retention_days(
        stored.and_then(|v| v.as_i64()),
    ))
}

/// Purges one org's runs older than its window. Only TERMINAL runs are
/// eligible: a parked run is owed a wake, not a delete. Steps cascade with
/// their run. Returns the number of runs removed.
pub async fn purge_expired(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    days: i64,
) -> DbResult<u64> {
    let res = sqlx::query(
        "DELETE FROM workflow_runs
         WHERE org_id = $1
           AND status IN ('succeeded','failed','abandoned')
           AND finished_at < now() - make_interval(days => $2::int)",
    )
    .bind(org_id)
    .bind(days)
    .execute(&mut *conn)
    .await?;
    Ok(res.rows_affected())
}

/// The declared retention default, read from the typed registry so the SQL
/// fallback and the declaration cannot drift.
pub fn retention_default_days() -> i64 {
    core_domain::settings::declaration(WORKFLOW_RUN_RETENTION_DAYS)
        .and_then(|d| match d.default {
            SettingValue::Integer(days) => Some(days),
            _ => None,
        })
        .unwrap_or(workflow_runs::RETENTION_DEFAULT_DAYS)
}

/// Purges every org's expired runs, each against its OWN window. This is
/// what the worker's sweep calls: it runs on the process-wide pool (like
/// `queue::reclaim_stale`), which is why the window resolves per org from
/// the settings table instead of taking one global number.
pub async fn purge_expired_everywhere(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
) -> DbResult<u64> {
    // A stored value that is not a plain integer falls back to the declared
    // default rather than raising: a hand-edited row must not break the
    // sweep, and an unparseable window is not a license to keep everything.
    let res = sqlx::query(
        "DELETE FROM workflow_runs r
         WHERE r.status IN ('succeeded','failed','abandoned')
           AND r.finished_at < now() - make_interval(days => COALESCE(
             (SELECT CASE
                       WHEN s.value #>> '{}' ~ '^[0-9]+$' THEN (s.value #>> '{}')::int
                     END
                FROM settings s
               WHERE s.org_id = r.org_id AND s.scope = 'org' AND s.key = $1),
             $2::int))",
    )
    .bind(WORKFLOW_RUN_RETENTION_DAYS)
    .bind(retention_default_days())
    .execute(executor)
    .await?;
    Ok(res.rows_affected())
}

// --- row mapping -----------------------------------------------------------

/// Maps one ledger row. The two chain facts are DERIVED: the listing query
/// selects them, and reads that do not carry them (a single-run read)
/// select nothing for those names, so the row mapping reports them false.
/// A stored flag would be a mutation of the parked row wearing a different
/// name, which is the whole thing this ledger refuses to do.
fn run_from_row(row: &sqlx::postgres::PgRow) -> (RunRow, bool, bool) {
    let status_raw: String = row.get("status");
    let trigger_raw: String = row.get("trigger_kind");
    let run = RunRow {
        id: row.get("id"),
        org_id: row.get("org_id"),
        rule_id: row.get("rule_id"),
        rule_name: row.get("rule_name"),
        rule_version: row.get("rule_version"),
        rule_definition: row.get("rule_definition"),
        catalogue_version: row.get("catalogue_version"),
        // A status this build does not know is build drift. Reading it as
        // `running` would hide a run, so the row surfaces as failed rather
        // than as one that is still going.
        status: RunStatus::parse(&status_raw).unwrap_or(RunStatus::Failed),
        trigger: RunTrigger::parse(&trigger_raw).unwrap_or(RunTrigger::Event),
        trigger_event_id: row.get("trigger_event_id"),
        trigger_job_id: row.get("trigger_job_id"),
        trigger_user_id: row.get("trigger_user_id"),
        resumed_from: row.get("resumed_from"),
        retry_of: row.get("retry_of"),
        resume_cursor: row.get("resume_cursor"),
        wake_at: row.get("wake_at"),
        wake_on: row.get("wake_on"),
        error_code: row.get("error_code"),
        error: row.get("error"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    };
    let superseded = row.try_get::<bool, _>("superseded").unwrap_or(false);
    let retried = row.try_get::<bool, _>("retried").unwrap_or(false);
    (run, superseded, retried)
}

fn step_from_row(row: &sqlx::postgres::PgRow) -> RunStepRow {
    let status_raw: String = row.get("status");
    RunStepRow {
        id: row.get("id"),
        org_id: row.get("org_id"),
        run_id: row.get("run_id"),
        step_index: row.get("step_index"),
        node_id: row.get("node_id"),
        node_label: row.get("node_label"),
        attempt: row.get("attempt"),
        iteration: row.get("iteration"),
        // Same drift posture as the run: an unknown step status surfaces as
        // failed rather than as one that is still going.
        status: StepStatus::parse(&status_raw).unwrap_or(StepStatus::Failed),
        input: row.get("input"),
        output: row.get("output"),
        error_code: row.get("error_code"),
        error: row.get("error"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
    }
}
