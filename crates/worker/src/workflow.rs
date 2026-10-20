//! worker/workflow — the multi-step workflow executor (TASKLIST Wave 43
//! lane 15, `p43-workflow-executor`).
//!
//! Purpose: run a declarative workflow document step by step, and NEVER
//! block a worker. The document model, its author-time validation and the
//! DAG walk live in `core_domain::workflow_document` (pure); THIS module is
//! the executor around it: it consumes the event stream for triggers and
//! for parked-run wakes, drives the walk, records every step on lane 14's
//! ledger, executes registry nodes through core-api's ORG-SCOPED seam, and
//! parks a run as a queued job rather than holding a connection.
//!
//! # The envelope-cursor model
//!
//! Like `automations.rs` and `webhook_out.rs`, this is a durable stream
//! consumer: it reads the domain-event stream under its own consumer group,
//! matches each envelope, and only ACKs after the work is durably queued.
//! Unlike them, the unit of work is a RUN: an event either STARTS a run (a
//! rule matched) or WAKES a parked one (a `flow.waitFor` matched). Both
//! become one `workflow.resume` job so the queue owns retry and dead-letter.
//!
//! # Parking is the engine change (spec item C2)
//!
//! `flow.delay`, `flow.waitUntil`, `flow.deadline`, `flow.waitFor`,
//! `flow.waitForAll` and `flow.awaitApproval` all PARK the run. The run's
//! cursor is written to `workflow_runs.resume_cursor` and the wake is a
//! `workflow.resume` job enqueued with the right `run_after` (the THREE
//! traps the spec names: `queue_runner::settle_job` learns this kind; the
//! wake is a QUEUE job, not `schedule_retry`; and the event router gains a
//! waiters index — `idx_workflow_runs_waiters`, read by
//! [`wake_matches`]).
//!
//! # The run is the author's
//!
//! Every node effect crosses to core-api as the run's AUTHOR
//! (`trigger_user_id`); the executor refuses a document with no author
//! rather than run it as a system user with more power (spec item B).
//!
//! Non-purpose: no document validation (core-domain refuses it), no ledger
//! SQL (lane 14), no domain writes here (effects flow through core-api).

use crate::effects::{EffectApi, EffectOutcome};
use crate::reconnect::Reconnect;
use core_db::queue::{ClaimedJob, NewJob};
use core_db::workflow_runs as runs_db;
use core_db::workflow_runs::NewStep;
use core_domain::node_registry::{self, ExecutorPath};
use core_domain::workflow_document::{
    self, EdgeOutcome, WorkflowCursor, WorkflowDocument, WorkflowNode, DEFAULT_FALSE_LABEL,
    DEFAULT_TRUE_LABEL, ERROR_LABEL, FLOW_ABANDON, FLOW_BRANCH, FLOW_DEADLINE, FLOW_DELAY,
    FLOW_FAIL, FLOW_FOREACH, FLOW_JOIN, FLOW_PARALLEL, FLOW_STOP, FLOW_SWITCH, FLOW_WAIT_FOR,
    FLOW_WAIT_FOR_ALL, FLOW_WAIT_UNTIL,
};
use core_domain::workflow_runs::{RunStatus, RunTrigger, StepStatus, WORKFLOW_RESUME_JOB};
use core_events::{Envelope, EVENT_STREAM};
use redis::streams::{StreamReadOptions, StreamReadReply};
use redis::AsyncCommands;
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::sync::watch;

/// Durable group; `$` starts at the live tail on first creation.
const CONSUMER_GROUP: &str = "workflows";
const CONSUMER_NAME: &str = "worker";
const BLOCK_MS: usize = 2_000;
const COUNT: usize = 50;

/// The core-api internal endpoint that executes ONE registry node
/// org-scoped, as the run's author.
pub const NODE_ENDPOINT: &str = "/api/internal/workflow/node";

/// A resume job's attempts come from the queue's retry policy
/// (`crate::queue_runner::queue_max_attempts`): a run that cannot move is
/// visible, not silently stuck, and the number is the same one every other
/// queued job gets.

/// How a `workflow.resume` job settled. The queue runner maps these 1:1
/// onto done / retry / dead so the executor owns its own classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobOutcome {
    Done,
    Retry,
    Dead,
}

/// The executor's answer to one `workflow.resume` job.
#[derive(Debug, Clone)]
pub struct JobResult {
    pub outcome: JobOutcome,
    /// The reason recorded when the outcome is Retry or Dead.
    pub error: String,
}

impl JobResult {
    fn done() -> Self {
        Self {
            outcome: JobOutcome::Done,
            error: String::new(),
        }
    }
    fn retry(error: impl Into<String>) -> Self {
        Self {
            outcome: JobOutcome::Retry,
            error: error.into(),
        }
    }
    fn dead(error: impl Into<String>) -> Self {
        Self {
            outcome: JobOutcome::Dead,
            error: error.into(),
        }
    }
}

/// The immutable facts one run carries through its walk.
struct RunContext {
    org_id: uuid::Uuid,
    run_id: uuid::Uuid,
    /// The author the run executes as (spec item B).
    actor_user_id: uuid::Uuid,
    /// The triggering envelope's PAYLOAD (Wave 43 lane 20, item A) — the
    /// same value core-api resolves a ticket from (`context.event.ticket_id`).
    /// It used to carry the whole envelope, which is why a ticket-writing
    /// node with no explicit `ticketId` refused: the payload was one level
    /// down. `event_type` and `event_entity` ride separately so the parking
    /// nodes and the condition scope still see them.
    event: Value,
    /// The triggering event's type (`ticket.created`, …), when an event
    /// started or woke this run.
    event_type: Option<String>,
    /// The triggering event's entity name (`ticket`, `client`, …), so a
    /// condition may name a subject field (`ticket.priority`).
    event_entity: Option<String>,
    /// True when this attempt CONTINUES a parked run: a park node the run
    /// stops at re-evaluates rather than parking again blindly.
    resumed: bool,
}

// ---------------------------------------------------------------------------
// The stream consumer: triggers and parked-run wakes
// ---------------------------------------------------------------------------

/// Runs the workflow stream consumer until shutdown. Matches each envelope
/// against enabled workflow documents (to START runs) and against parked
/// runs waiting on the event type (to WAKE them); both enqueue a
/// `workflow.resume` job and ACK only once the job is durable.
pub async fn run_workflow_stream(
    pool: PgPool,
    redis_url: String,
    mut shutdown: watch::Receiver<bool>,
) {
    let client = match redis::Client::open(redis_url.as_str()) {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(%error, "invalid redis url; workflows exiting");
            return;
        }
    };
    let mut backoff = Reconnect::consumer();
    loop {
        if *shutdown.borrow() {
            return;
        }
        match client.get_multiplexed_async_connection().await {
            Ok(mut conn) => {
                backoff.reset();
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
                        tracing::error!(%error, "workflows xgroup create failed");
                        backoff.sleep().await;
                        continue;
                    }
                }
                follow_stream(&pool, &mut conn, &mut shutdown).await;
                tracing::warn!("workflows stream connection ended; reconnecting");
            }
            Err(error) => {
                tracing::error!(%error, "workflows redis connect failed");
            }
        }
        backoff.sleep().await;
    }
}

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
                        tracing::error!(%error, "workflows xreadgroup failed");
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
    if let Err(error) = match_envelope(pool, &envelope).await {
        // NOT acking: the group redelivers after restart, so a DB hiccup
        // retries matching instead of dropping the event.
        tracing::error!(%error, entry_id = %entry.id, "workflow match failed; leaving unacked");
        return;
    }
    ack(conn, &entry.id).await;
}

/// Starts runs for every enabled workflow document listening for this
/// event type, and wakes every parked run waiting on it.
async fn match_envelope(pool: &PgPool, envelope: &Envelope) -> anyhow::Result<()> {
    // The PAYLOAD rides the job as `event`: it is the value a node reads a
    // ticket from (`context.event.ticket_id`) and the subject a condition
    // reads a field from. The type and entity ride beside it — the whole
    // envelope used to be `event`, which is the disagreement item A fixed.
    let event = envelope.payload.clone();
    let mut tx = pool.begin().await?;

    // (1) START: enabled workflow documents whose trigger is this event.
    let rules = core_db::automations::list_enabled_workflows_by_trigger(
        &mut *tx,
        envelope.org_id,
        &envelope.event_type,
    )
    .await?;
    for rule in rules {
        let payload = json!({
            "op": "start",
            "rule_id": rule.id,
            "trigger_event_id": envelope.id,
            "event": event.clone(),
            "event_type": envelope.event_type,
            "event_entity": envelope.entity,
        });
        if let Err(error) = enqueue_resume(&mut *tx, envelope.org_id, payload).await {
            tracing::error!(%error, rule_id = %rule.id, "workflow start enqueue failed; leaving unacked");
            return Err(error.into());
        }
    }

    // (2) WAKE: parked runs waiting on this event type (the waiters index).
    let waiting =
        runs_db::list_waiting_on_event(&mut tx, envelope.org_id, &envelope.event_type).await?;
    for run in waiting {
        let payload = json!({
            "op": "resume",
            "run_id": run.id,
            "event": event.clone(),
            "event_type": envelope.event_type,
            "event_entity": envelope.entity,
        });
        if let Err(error) = enqueue_resume(&mut *tx, envelope.org_id, payload).await {
            tracing::error!(%error, run_id = %run.id, "workflow resume enqueue failed; leaving unacked");
            return Err(error.into());
        }
    }

    tx.commit().await?;
    Ok(())
}

/// Enqueues one `workflow.resume` job inside the caller's transaction, so
/// the trigger/wake and the job that runs it are one durable fact.
async fn enqueue_resume<'e, E>(
    executor: E,
    org_id: uuid::Uuid,
    payload: Value,
) -> core_db::DbResult<uuid::Uuid>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    core_db::queue::enqueue(
        executor,
        &NewJob {
            org_id,
            kind: WORKFLOW_RESUME_JOB.to_string(),
            payload,
            max_attempts: crate::queue_runner::queue_max_attempts(),
        },
    )
    .await
}

async fn ack(conn: &mut redis::aio::MultiplexedConnection, id: &str) {
    let _: Result<i64, _> = conn.xack(EVENT_STREAM, CONSUMER_GROUP, &[id]).await;
}

// ---------------------------------------------------------------------------
// The resume job: start a run, or continue a parked one
// ---------------------------------------------------------------------------

/// Executes one `workflow.resume` job. Called in-process by the queue
/// runner (like `webhook.deliver`), so the executor never leaves the worker
/// and node effects still cross the org-scoped seam.
pub async fn process_resume_job<E: EffectApi>(
    pool: &PgPool,
    effects: &E,
    job: &ClaimedJob,
) -> JobResult {
    let op = job.payload.get("op").and_then(Value::as_str).unwrap_or("");
    let event = job.payload.get("event").cloned().unwrap_or(Value::Null);
    match op {
        "start" => process_start(pool, effects, job, event).await,
        "resume" => process_resume(pool, effects, job, event).await,
        other => JobResult::dead(format!(
            "malformed workflow.resume payload: unknown op {other:?}"
        )),
    }
}

async fn process_start<E: EffectApi>(
    pool: &PgPool,
    effects: &E,
    job: &ClaimedJob,
    event: Value,
) -> JobResult {
    let Some(rule_id) = job
        .payload
        .get("rule_id")
        .and_then(Value::as_str)
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
    else {
        return JobResult::dead("malformed start payload: rule_id missing");
    };
    let trigger_event_id = job
        .payload
        .get("trigger_event_id")
        .and_then(Value::as_str)
        .and_then(|s| uuid::Uuid::parse_str(s).ok());

    let mut conn = match pool.acquire().await {
        Ok(conn) => conn,
        Err(error) => return JobResult::retry(format!("db acquire failed: {error}")),
    };
    let rule = match core_db::automations::get_workflow(&mut *conn, job.org_id, rule_id).await {
        Ok(rule) => rule,
        Err(_) => return JobResult::dead("workflow rule missing or not a workflow document"),
    };
    // A workflow runs as its author; no author means no safe principal.
    let Some(author) = rule.created_by else {
        return JobResult::dead("workflow has no author; refusing to run as a system user");
    };
    // Validate the document before a single step runs.
    let doc = match WorkflowDocument::parse(&rule.definition) {
        Ok(doc) => doc,
        Err(error) => return JobResult::dead(format!("invalid workflow document: {error}")),
    };
    let run = match runs_db::insert_run(
        &mut conn,
        job.org_id,
        &runs_db::NewRun {
            rule_id: Some(rule.id),
            rule_name: rule.name.clone(),
            rule_version: rule.revision,
            rule_definition: rule.definition.clone(),
            catalogue_version: node_registry::CATALOGUE_VERSION.to_string(),
            trigger: RunTrigger::Event,
            trigger_event_id,
            trigger_job_id: Some(job.id),
            trigger_user_id: Some(author),
            resumed_from: None,
            resume_cursor: None,
        },
        RunStatus::Running,
    )
    .await
    {
        Ok(run) => run,
        Err(error) => return JobResult::retry(format!("insert_run failed: {error}")),
    };
    let ctx = RunContext {
        org_id: job.org_id,
        run_id: run.id,
        actor_user_id: author,
        event,
        event_type: job
            .payload
            .get("event_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        event_entity: job
            .payload
            .get("event_entity")
            .and_then(Value::as_str)
            .map(str::to_string),
        resumed: false,
    };
    execute_run(pool, effects, &ctx, &doc, WorkflowCursor::start()).await
}

async fn process_resume<E: EffectApi>(
    pool: &PgPool,
    effects: &E,
    job: &ClaimedJob,
    event: Value,
) -> JobResult {
    let Some(parent_id) = job
        .payload
        .get("run_id")
        .and_then(Value::as_str)
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
    else {
        return JobResult::dead("malformed resume payload: run_id missing");
    };
    // The lock + child check + continuation insert must be one transaction,
    // so two concurrent wakes cannot both create a continuation.
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(error) => return JobResult::retry(format!("db begin failed: {error}")),
    };
    let parent = match runs_db::get_run(&mut tx, job.org_id, parent_id).await {
        Ok(run) => run,
        Err(_) => return JobResult::dead("parked run missing"),
    };
    if parent.status != RunStatus::Waiting {
        // Already resumed by another wake: nothing to do.
        return JobResult::done();
    }
    let Some(author) = parent.trigger_user_id else {
        return JobResult::dead("parked run has no author; refusing to resume as a system user");
    };
    // The continuation is a NEW row; the parked row is never rewritten.
    let run = match runs_db::begin_resumed(&mut tx, job.org_id, parent_id, Some(job.id)).await {
        Ok(run) => run,
        // A second wake found the child: the first already continued it.
        Err(core_db::DbError::Conflict(_)) => return JobResult::done(),
        Err(error) => return JobResult::retry(format!("begin_resumed failed: {error}")),
    };
    if let Err(error) = tx.commit().await {
        return JobResult::retry(format!("begin_resumed commit failed: {error}"));
    }
    let doc = match WorkflowDocument::parse(&run.rule_definition) {
        Ok(doc) => doc,
        Err(error) => return JobResult::dead(format!("invalid workflow document: {error}")),
    };
    // Loop protection: a resumed run may not re-enter past the document's
    // bound (spec item D).
    let mut cursor: WorkflowCursor = run
        .resume_cursor
        .clone()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    if cursor.depth >= doc.loop_max_depth {
        if let Ok(mut conn) = pool.acquire().await {
            let _ = runs_db::finish_run(
                &mut conn,
                job.org_id,
                run.id,
                RunStatus::Failed,
                Some("loop_max_depth"),
                Some("resumed past the document's loop.maxDepth"),
            )
            .await;
        }
        return JobResult::done();
    }
    cursor.depth += 1;
    let ctx = RunContext {
        org_id: job.org_id,
        run_id: run.id,
        actor_user_id: author,
        event,
        event_type: job
            .payload
            .get("event_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        event_entity: job
            .payload
            .get("event_entity")
            .and_then(Value::as_str)
            .map(str::to_string),
        resumed: true,
    };
    execute_run(pool, effects, &ctx, &doc, cursor).await
}

// ---------------------------------------------------------------------------
// The walk
// ---------------------------------------------------------------------------

/// Drives the document from `cursor` to park or terminal. One step per
/// loop; the cursor carries the whole continuation.
async fn execute_run<E: EffectApi>(
    pool: &PgPool,
    effects: &E,
    ctx: &RunContext,
    doc: &WorkflowDocument,
    mut cursor: WorkflowCursor,
) -> JobResult {
    let mut conn = match pool.acquire().await {
        Ok(conn) => conn,
        Err(error) => return JobResult::retry(format!("db acquire failed: {error}")),
    };
    loop {
        let Some(node) = cursor.next_ready(doc).cloned() else {
            // No node ready: the document ran out of work — a clean finish.
            return match runs_db::finish_run(
                &mut conn,
                ctx.org_id,
                ctx.run_id,
                RunStatus::Succeeded,
                None,
                None,
            )
            .await
            {
                Ok(()) => JobResult::done(),
                Err(error) => JobResult::retry(format!("finish_run failed: {error}")),
            };
        };
        match run_one_node(&mut conn, effects, ctx, doc, &mut cursor, &node).await {
            NodeFlow::Continue => {}
            NodeFlow::Parked => return park_run(&mut conn, ctx, cursor).await,
            NodeFlow::Stop(status, code, error) => {
                return match runs_db::finish_run(
                    &mut conn,
                    ctx.org_id,
                    ctx.run_id,
                    status,
                    code.as_deref(),
                    error.as_deref(),
                )
                .await
                {
                    Ok(()) => JobResult::done(),
                    Err(e) => JobResult::retry(format!("finish_run failed: {e}")),
                };
            }
        }
    }
}

/// What running one node decided the loop should do next.
enum NodeFlow {
    Continue,
    Parked,
    Stop(RunStatus, Option<String>, Option<String>),
}

/// Runs ONE node: record the step, execute it, finish the step, apply the
/// outcome to the cursor. Shared by the main walk and a foreach body.
async fn run_one_node<E: EffectApi>(
    conn: &mut sqlx::PgConnection,
    effects: &E,
    ctx: &RunContext,
    doc: &WorkflowDocument,
    cursor: &mut WorkflowCursor,
    node: &WorkflowNode,
) -> NodeFlow {
    let step_index = cursor.index;
    cursor.index += 1;
    let label = node_registry::get(&node.node_type)
        .map(|n| n.label)
        .unwrap_or(node.node_type.as_str());
    let step = match runs_db::insert_step(
        conn,
        ctx.org_id,
        ctx.run_id,
        &NewStep {
            step_index,
            node_id: node.node_type.clone(),
            node_label: label.to_string(),
            attempt: 1,
            iteration: None,
            input: node.params.clone(),
        },
    )
    .await
    {
        Ok(step) => step,
        Err(error) => {
            tracing::error!(%error, node = %node.id, "insert_step failed");
            return NodeFlow::Stop(
                RunStatus::Failed,
                Some("ledger_write_failed".into()),
                Some(error.to_string()),
            );
        }
    };

    // `flow.foreach` runs its body as a sub-walk; every other node runs
    // through the executor-path dispatch below.
    if node.node_type == FLOW_FOREACH {
        return run_foreach(conn, effects, ctx, doc, cursor, node, step.id).await;
    }

    let exec = execute_node(effects, ctx, cursor, node).await;
    match exec {
        NodeExec::Parked {
            wake_at,
            wake_on,
            received,
        } => {
            // The step is parked, not finished: the continuation resumes it.
            let out = json!({ "parked": true, "wake_at": wake_at.map(|t| t.to_rfc3339()), "wake_on": wake_on });
            let _ = runs_db::park_step(conn, ctx.org_id, step.id, out).await;
            cursor.parked_node = Some(node.id.clone());
            cursor.wake_at = wake_at.map(|t| t.to_rfc3339());
            cursor.wake_on = wake_on;
            if !received.is_empty() {
                cursor
                    .received
                    .insert(node.id.clone(), received.into_iter().collect());
            }
            NodeFlow::Parked
        }
        NodeExec::Terminal {
            status,
            code,
            error,
        } => {
            let _ = runs_db::finish_step(
                conn,
                ctx.org_id,
                step.id,
                terminal_step_status(status),
                None,
                code.as_deref(),
                error.as_deref(),
            )
            .await;
            cursor.parked_node = None;
            NodeFlow::Stop(status, code, error)
        }
        NodeExec::Failed { code, error } => match node.on_error() {
            workflow_document::OnError::Halt => {
                let _ = runs_db::finish_step(
                    conn,
                    ctx.org_id,
                    step.id,
                    StepStatus::Failed,
                    None,
                    Some(&code),
                    Some(&error),
                )
                .await;
                NodeFlow::Stop(RunStatus::Failed, Some(code), Some(error))
            }
            workflow_document::OnError::Continue => {
                let _ = runs_db::finish_step(
                    conn,
                    ctx.org_id,
                    step.id,
                    StepStatus::Failed,
                    None,
                    Some(&code),
                    Some(&error),
                )
                .await;
                cursor.record_node(&node.id, json!({ "error": error }));
                cursor.fire_all_outgoing(doc, &node.id);
                NodeFlow::Continue
            }
            workflow_document::OnError::Route => {
                let _ = runs_db::finish_step(
                    conn,
                    ctx.org_id,
                    step.id,
                    StepStatus::Failed,
                    None,
                    Some(&code),
                    Some(&error),
                )
                .await;
                cursor.record_node(&node.id, json!({ "error": error }));
                cursor.fire_labelled(doc, &node.id, &[ERROR_LABEL]);
                NodeFlow::Continue
            }
        },
        NodeExec::Succeeded { output, edges } => {
            let _ = runs_db::finish_step(
                conn,
                ctx.org_id,
                step.id,
                StepStatus::Succeeded,
                Some(output.clone()),
                None,
                None,
            )
            .await;
            cursor.record_node(&node.id, output);
            cursor.parked_node = None;
            match edges {
                EdgeOutcome::All => cursor.fire_all_outgoing(doc, &node.id),
                EdgeOutcome::Labels(labels) => {
                    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                    cursor.fire_labelled(doc, &node.id, &refs);
                }
                EdgeOutcome::None => {}
            }
            NodeFlow::Continue
        }
    }
}

/// Runs a `flow.foreach`'s body once per item, bounded by the node's cap.
/// The body is a sub-document (see `WorkflowDocument::foreach_body`), so its
/// nodes never enter the main walk; each body node records its own step on
/// the SAME run, and the step index continues monotonically across items.
async fn run_foreach<E: EffectApi>(
    conn: &mut sqlx::PgConnection,
    effects: &E,
    ctx: &RunContext,
    doc: &WorkflowDocument,
    cursor: &mut WorkflowCursor,
    node: &WorkflowNode,
    step_id: uuid::Uuid,
) -> NodeFlow {
    let collection = node
        .params
        .get("collection")
        .and_then(Value::as_str)
        .unwrap_or("");
    let items = cursor
        .outputs
        .get(collection)
        .map(collection_items)
        .unwrap_or_default();
    let cap = node
        .params
        .get("maxItems")
        .and_then(Value::as_i64)
        .unwrap_or(workflow_document::DEFAULT_FOREACH_CAP);
    if items.len() as i64 > cap {
        // Refused outright above the cap rather than silently truncated.
        let error = format!("collection has {} items, above the cap {cap}", items.len());
        let _ = runs_db::finish_step(
            conn,
            ctx.org_id,
            step_id,
            StepStatus::Failed,
            None,
            Some("foreach_over_cap"),
            Some(&error),
        )
        .await;
        return NodeFlow::Stop(
            RunStatus::Failed,
            Some("foreach_over_cap".into()),
            Some(error),
        );
    }

    let body = doc.foreach_body(&node.id);
    let body_doc = body_document(doc, &body);
    let mut processed = 0usize;
    for item in &items {
        let mut sub = WorkflowCursor::start();
        sub.index = cursor.index;
        // The item is bound for the body's `value.reference` nodes.
        sub.outputs.insert("item".to_string(), item.clone());
        sub.outputs.insert(node.id.clone(), json!({ "item": item }));
        while let Some(body_node) = sub.next_ready(&body_doc).cloned() {
            // Boxed to break the async recursion: a foreach body re-enters
            // the node runner, so the future is a cycle the compiler cannot
            // size without an indirection.
            match Box::pin(run_one_node(
                conn, effects, ctx, &body_doc, &mut sub, &body_node,
            ))
            .await
            {
                NodeFlow::Continue => {}
                NodeFlow::Parked => {
                    cursor.index = sub.index;
                    let _ = runs_db::finish_step(
                        conn,
                        ctx.org_id,
                        step_id,
                        StepStatus::Failed,
                        None,
                        Some("foreach_body_parked"),
                        Some("a foreach body may not park"),
                    )
                    .await;
                    return NodeFlow::Stop(
                        RunStatus::Failed,
                        Some("foreach_body_parked".into()),
                        Some("a foreach body may not park".into()),
                    );
                }
                NodeFlow::Stop(status, code, error) => {
                    cursor.index = sub.index;
                    let _ = runs_db::finish_step(
                        conn,
                        ctx.org_id,
                        step_id,
                        terminal_step_status(status),
                        None,
                        code.as_deref(),
                        error.as_deref(),
                    )
                    .await;
                    return NodeFlow::Stop(status, code, error);
                }
            }
        }
        cursor.index = sub.index;
        processed += 1;
    }

    let output = json!({
        "processed": processed,
        "batches": 1,
        "item": items.last().cloned().unwrap_or(Value::Null),
    });
    let _ = runs_db::finish_step(
        conn,
        ctx.org_id,
        step_id,
        StepStatus::Succeeded,
        Some(output.clone()),
        None,
        None,
    )
    .await;
    cursor.record_node(&node.id, output);
    cursor.fire_all_outgoing(doc, &node.id);
    NodeFlow::Continue
}

/// The sub-document a foreach body runs: the body's nodes and the edges
/// between them. An edge that leaves the body is dropped — a body node
/// continues to the foreach's non-body edge through the MAIN walk, not
/// here.
fn body_document(
    doc: &WorkflowDocument,
    body: &std::collections::BTreeSet<String>,
) -> WorkflowDocument {
    WorkflowDocument {
        version: doc.version.clone(),
        nodes: doc
            .nodes
            .iter()
            .filter(|n| body.contains(&n.id))
            .cloned()
            .collect(),
        edges: doc
            .edges
            .iter()
            .filter(|e| body.contains(&e.from) && body.contains(&e.to))
            .cloned()
            .collect(),
        loop_max_depth: 1,
    }
}

/// Parks the run: writes the cursor and wake coordinates to the ledger and
/// enqueues the `workflow.resume` job that will wake it. The wake is a
/// QUEUE job with the right `run_after` — never `schedule_retry`, which
/// carries a bounded attempt budget and would die overnight (spec trap ii).
async fn park_run(
    conn: &mut sqlx::PgConnection,
    ctx: &RunContext,
    cursor: WorkflowCursor,
) -> JobResult {
    let cursor_value = match serde_json::to_value(&cursor) {
        Ok(value) => value,
        Err(error) => return JobResult::retry(format!("cursor serialize failed: {error}")),
    };
    let wake_on = cursor.wake_on.clone();
    // The cursor stores the wake-at as an RFC3339 string (so the cursor is
    // pure JSON); the ledger and the queue want a real instant.
    let wake_at = cursor
        .wake_at
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&chrono::Utc));
    if let Err(error) = runs_db::park_run(
        conn,
        ctx.org_id,
        ctx.run_id,
        cursor_value.clone(),
        wake_at,
        wake_on.as_deref(),
    )
    .await
    {
        return JobResult::retry(format!("park_run failed: {error}"));
    }
    // Enqueue the wake. A time wake is due at wake_at; an event wake with no
    // time bound is due only when the event arrives, so it is woken by the
    // waiters path instead — but a run with BOTH a time bound and an event
    // bound must still wake at the time. So: enqueue due at wake_at when
    // there is one, else leave the wait to the waiters index.
    if let Some(wake_at) = wake_at {
        let job = NewJob {
            org_id: ctx.org_id,
            kind: WORKFLOW_RESUME_JOB.to_string(),
            payload: json!({ "op": "resume", "run_id": ctx.run_id }),
            max_attempts: crate::queue_runner::queue_max_attempts(),
        };
        if let Err(error) = core_db::queue::enqueue_after(conn, &job, wake_at).await {
            tracing::error!(%error, run_id = %ctx.run_id, "workflow wake enqueue failed");
            return JobResult::retry("wake enqueue failed");
        }
    }
    JobResult::done()
}

/// Maps a terminal run status onto the step status its terminating node
/// records.
fn terminal_step_status(status: RunStatus) -> StepStatus {
    match status {
        RunStatus::Succeeded => StepStatus::Succeeded,
        RunStatus::Failed => StepStatus::Failed,
        RunStatus::Abandoned => StepStatus::Abandoned,
        RunStatus::Running | RunStatus::Waiting => StepStatus::Succeeded,
    }
}

// ---------------------------------------------------------------------------
// Node execution
// ---------------------------------------------------------------------------

/// What executing one node produced.
enum NodeExec {
    Succeeded {
        output: Value,
        edges: EdgeOutcome,
    },
    Failed {
        code: String,
        error: String,
    },
    Parked {
        wake_at: Option<chrono::DateTime<chrono::Utc>>,
        wake_on: Option<String>,
        /// For `flow.waitForAll`: the event types that have arrived so far,
        /// carried back into the cursor so a re-park remembers them.
        received: Vec<String>,
    },
    Terminal {
        status: RunStatus,
        code: Option<String>,
        error: Option<String>,
    },
}

/// Executes one node by its registry executor path. Value and Control nodes
/// are pure and run here; every effect-bearing node (actions, AI edges,
/// plugin nodes, conditions) crosses to core-api as the run's author.
async fn execute_node<E: EffectApi>(
    effects: &E,
    ctx: &RunContext,
    cursor: &WorkflowCursor,
    node: &WorkflowNode,
) -> NodeExec {
    // Terminal nodes are decided before the registry so their parameters
    // read cleanly.
    match node.node_type.as_str() {
        FLOW_STOP => {
            return NodeExec::Terminal {
                status: RunStatus::Succeeded,
                code: None,
                error: None,
            }
        }
        FLOW_FAIL => {
            let reason = node
                .params
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("flow.fail");
            return NodeExec::Terminal {
                status: RunStatus::Failed,
                code: Some("flow_fail".into()),
                error: Some(reason.to_string()),
            };
        }
        FLOW_ABANDON => {
            let reason = node
                .params
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("flow.abandon");
            return NodeExec::Terminal {
                status: RunStatus::Abandoned,
                code: Some("flow_abandon".into()),
                error: Some(reason.to_string()),
            };
        }
        _ => {}
    }

    match workflow_document::executor_path(&node.node_type) {
        Some(ExecutorPath::Value) => value_node(cursor, node),
        Some(ExecutorPath::Control) => control_node(ctx, cursor, node),
        // Conditions are pure predicates over the run context, evaluated
        // HERE in the worker (Wave 43 lane 20, item B/C): the cursor holds
        // every upstream output, and the alternative — round-tripping to
        // core-api — would need the whole output set on the wire. The one
        // exception, `decision.matches`, reads recorded answers from the
        // database and is marked unimplemented in the catalogue, so it can
        // never be authored.
        Some(ExecutorPath::Condition) => condition_node(ctx, cursor, node),
        Some(ExecutorPath::DomainWrapper(_))
        | Some(ExecutorPath::DomainUpdate)
        | Some(ExecutorPath::DomainQuery)
        | Some(ExecutorPath::Ai(_))
        | Some(ExecutorPath::Plugin(_)) => call_node(effects, ctx, cursor, node).await,
        Some(ExecutorPath::Trigger) => NodeExec::Failed {
            code: "trigger_in_body".into(),
            error: "a trigger node is a document entry, never a step".into(),
        },
        None => NodeExec::Failed {
            code: "unknown_node".into(),
            error: format!("unknown node type {:?}", node.node_type),
        },
    }
}

/// The pure value nodes: literal, reference, math, template.
fn value_node(cursor: &WorkflowCursor, node: &WorkflowNode) -> NodeExec {
    match node.node_type.as_str() {
        "value.literal" => NodeExec::Succeeded {
            output: json!({ "value": node.params.get("value").cloned().unwrap_or(Value::Null) }),
            edges: EdgeOutcome::All,
        },
        "value.reference" => {
            let target = node
                .params
                .get("node")
                .and_then(Value::as_str)
                .unwrap_or("");
            let path = node
                .params
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("");
            let value = cursor
                .outputs
                .get(target)
                .map(|out| lookup_path(out, path))
                .unwrap_or(Value::Null);
            NodeExec::Succeeded {
                output: json!({ "value": value, "path": path }),
                edges: EdgeOutcome::All,
            }
        }
        "value.math" => {
            let left = node.params.get("left").and_then(Value::as_f64);
            let right = node.params.get("right").and_then(Value::as_f64);
            let op = node.params.get("operator").and_then(Value::as_str);
            match (left, right, op) {
                (Some(l), Some(r), Some(op)) => match math(l, op, r) {
                    Some(v) => NodeExec::Succeeded {
                        output: json!({ "value": v }),
                        edges: EdgeOutcome::All,
                    },
                    None => NodeExec::Failed {
                        code: "math_error".into(),
                        error: format!("cannot apply {op}"),
                    },
                },
                _ => NodeExec::Failed {
                    code: "math_error".into(),
                    error: "left/operator/right must be numbers and a known operator".into(),
                },
            }
        }
        "value.template" => {
            let template = node
                .params
                .get("template")
                .and_then(Value::as_str)
                .unwrap_or("");
            let fields = node.params.get("fields").cloned().unwrap_or(json!({}));
            NodeExec::Succeeded {
                output: json!({ "text": render_template(template, &fields) }),
                edges: EdgeOutcome::All,
            }
        }
        other => NodeExec::Failed {
            code: "unknown_value_node".into(),
            error: format!("unknown value node {other:?}"),
        },
    }
}

/// The pure condition nodes (Wave 43 lane 20, items B and C). Each answers
/// ONE question about the run context and returns `{"matched": bool, …}` —
/// the key `flow.branch` reads. Nothing here writes and nothing crosses to
/// core-api: the cursor already holds every upstream output, so a predicate
/// is a pure function of the run context.
///
/// `decision.matches` is the one condition that needs recorded answers from
/// the database. It is marked unimplemented in the catalogue, so
/// `WorkflowDocument::validate` refuses a document that places it before a
/// run ever starts, and it falls to the default arm here.
fn condition_node(ctx: &RunContext, cursor: &WorkflowCursor, node: &WorkflowNode) -> NodeExec {
    let field = node
        .params
        .get("field")
        .and_then(Value::as_str)
        .unwrap_or("");
    let lhs = condition_value(ctx, cursor, field);
    let rhs = node.params.get("value").cloned().unwrap_or(Value::Null);
    let param_num = |key: &str| node.params.get(key).and_then(numeric);

    let matched = match node.node_type.as_str() {
        "condition.else" => true,
        "field.equals" => lhs == rhs,
        "field.notEquals" => lhs != rhs,
        "field.contains" => {
            let needle = node
                .params
                .get("needle")
                .and_then(Value::as_str)
                .unwrap_or("");
            value_to_text(&lhs)
                .to_lowercase()
                .contains(&needle.to_lowercase())
        }
        "field.isEmpty" => is_empty_value(&lhs),
        "number.gt" => numeric(&lhs)
            .zip(numeric(&rhs))
            .map(|(l, r)| l > r)
            .unwrap_or(false),
        "number.gte" => numeric(&lhs)
            .zip(numeric(&rhs))
            .map(|(l, r)| l >= r)
            .unwrap_or(false),
        "number.lt" => numeric(&lhs)
            .zip(numeric(&rhs))
            .map(|(l, r)| l < r)
            .unwrap_or(false),
        "number.lte" => numeric(&lhs)
            .zip(numeric(&rhs))
            .map(|(l, r)| l <= r)
            .unwrap_or(false),
        "number.between" => match (numeric(&lhs), param_num("min"), param_num("max")) {
            (Some(v), Some(min), Some(max)) => v >= min && v <= max,
            _ => false,
        },
        "date.isBefore" => match (as_instant(&lhs), as_instant(&rhs)) {
            (Some(l), Some(r)) => l < r,
            _ => false,
        },
        "date.isAfter" => match (as_instant(&lhs), as_instant(&rhs)) {
            (Some(l), Some(r)) => l > r,
            _ => false,
        },
        "date.withinLast" => {
            let window = node
                .params
                .get("window")
                .and_then(Value::as_str)
                .and_then(parse_duration)
                .unwrap_or(0);
            match as_instant(&lhs) {
                Some(l) => {
                    let now = chrono::Utc::now();
                    l <= now && now - l <= chrono::Duration::seconds(window)
                }
                None => false,
            }
        }
        "date.isOverdue" => match as_instant(&lhs) {
            Some(l) => l < chrono::Utc::now(),
            None => false,
        },
        "list.contains" => lhs
            .as_array()
            .map(|items| items.contains(&rhs))
            .unwrap_or(false),
        "list.length.gte" => {
            let want = node
                .params
                .get("length")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            lhs.as_array()
                .map(|items| items.len() as i64 >= want)
                .unwrap_or(false)
        }
        "list.isEmpty" => lhs
            .as_array()
            .map(|items| items.is_empty())
            .unwrap_or(false),
        other => {
            return NodeExec::Failed {
                code: "condition_unimplemented".into(),
                error: format!("condition {other:?} has no worker evaluator"),
            }
        }
    };
    NodeExec::Succeeded {
        output: json!({ "matched": matched, "field": field }),
        edges: EdgeOutcome::All,
    }
}

/// Resolves a condition's `field` against the run context. The scope is
/// explicit so an author can always tell where a value came from:
///
/// * `event.<path>` — the triggering payload;
/// * `nodes.<id>.<path>` — an upstream node's recorded output;
/// * `<entity>.<path>` — a field of the triggering SUBJECT, where `<entity>`
///   is the event's entity (`ticket.priority` on a `ticket.created` run);
/// * anything else — a bare dotted path, read from the payload.
fn condition_value(ctx: &RunContext, cursor: &WorkflowCursor, field: &str) -> Value {
    if field.is_empty() {
        return Value::Null;
    }
    if let Some(rest) = field.strip_prefix("event.") {
        return lookup_path(&ctx.event, rest);
    }
    if let Some(rest) = field.strip_prefix("nodes.") {
        let (id, path) = rest.split_once('.').unwrap_or((rest, ""));
        return cursor
            .outputs
            .get(id)
            .map(|out| lookup_path(out, path))
            .unwrap_or(Value::Null);
    }
    if let Some(entity) = ctx.event_entity.as_deref() {
        if let Some(rest) = field
            .strip_prefix(entity)
            .and_then(|rest| rest.strip_prefix('.'))
        {
            return lookup_path(&ctx.event, rest);
        }
    }
    lookup_path(&ctx.event, field)
}

/// A JSON value as a float: numbers directly, numeric strings parsed.
fn numeric(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// A JSON value as an RFC 3339 instant.
fn as_instant(value: &Value) -> Option<chrono::DateTime<chrono::Utc>> {
    let raw = value.as_str()?;
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// "Empty" for a field: absent/null, an empty string, an empty array or an
/// empty object. Mirrors what `field.isEmpty` and `list.isEmpty` describe.
fn is_empty_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.is_empty(),
        _ => false,
    }
}

/// The control nodes that do not park: branch, switch, join, parallel,
/// errorHandler, retry. `flow.foreach` is handled in the async walk.
fn control_node(ctx: &RunContext, cursor: &WorkflowCursor, node: &WorkflowNode) -> NodeExec {
    match node.node_type.as_str() {
        FLOW_JOIN | FLOW_PARALLEL | "flow.errorHandler" | "flow.retry" => NodeExec::Succeeded {
            output: json!({}),
            edges: EdgeOutcome::All,
        },
        FLOW_BRANCH => {
            let condition = node
                .params
                .get("condition")
                .and_then(Value::as_str)
                .unwrap_or("");
            let holds = cursor
                .outputs
                .get(condition)
                .and_then(|o| o.get("matched"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let label = if holds {
                node.params
                    .get("trueLabel")
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_TRUE_LABEL)
            } else {
                node.params
                    .get("falseLabel")
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_FALSE_LABEL)
            };
            NodeExec::Succeeded {
                output: json!({ "branch": label }),
                edges: EdgeOutcome::Labels(vec![label.to_string()]),
            }
        }
        FLOW_SWITCH => {
            let value = node
                .params
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or("");
            let matched = cursor
                .outputs
                .get(value)
                .and_then(|o| o.get("value"))
                .cloned()
                .unwrap_or(Value::Null);
            let cases: Vec<String> = node
                .params
                .get("cases")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let case = matched
                .as_str()
                .map(str::to_string)
                .filter(|c| cases.contains(c));
            let default = node
                .params
                .get("defaultLabel")
                .and_then(Value::as_str)
                .map(str::to_string);
            match case.or(default) {
                Some(label) => NodeExec::Succeeded {
                    output: json!({ "case": label, "matched": true }),
                    edges: EdgeOutcome::Labels(vec![label]),
                },
                None => NodeExec::Failed {
                    code: "switch_unmatched".into(),
                    error: "no case matched and no defaultLabel edge".into(),
                },
            }
        }
        FLOW_FOREACH => NodeExec::Failed {
            code: "foreach_misrouted".into(),
            error: "flow.foreach runs in the async walk, not here".into(),
        },
        other if workflow_document::is_parking_node(other) => {
            parking_node(ctx, cursor, node, other)
        }
        other => NodeExec::Failed {
            code: "unknown_control_node".into(),
            error: format!("unknown control node {other:?}"),
        },
    }
}

/// One parking node. On FIRST entry the run parks; on resume the wait is
/// re-evaluated against the wake context.
fn parking_node(
    ctx: &RunContext,
    cursor: &WorkflowCursor,
    node: &WorkflowNode,
    node_type: &str,
) -> NodeExec {
    // A resume carries which run this is; a first entry parks immediately.
    if !ctx.resumed {
        return park_decision(node, node_type);
    }
    // Resumed: was the wait satisfied? The event TYPE rides beside the
    // payload now (item A), so a wait still matches on the type.
    let event_type = ctx.event_type.as_deref();
    match node_type {
        FLOW_DELAY | FLOW_WAIT_UNTIL => {
            // A time wake always satisfies a pure time wait.
            NodeExec::Succeeded {
                output: json!({ "resumesAt": chrono::Utc::now().to_rfc3339() }),
                edges: EdgeOutcome::All,
            }
        }
        FLOW_WAIT_FOR => {
            let accepted = string_list(node, "eventTypes");
            match event_type {
                Some(t) if accepted.iter().any(|a| a == t) => NodeExec::Succeeded {
                    output: json!({ "matchedEventType": t, "expired": false, "event": ctx.event }),
                    edges: EdgeOutcome::All,
                },
                // A time wake with no matching event: the timeout expired.
                _ if ctx.event.is_null() => NodeExec::Succeeded {
                    output: json!({ "expired": true }),
                    edges: EdgeOutcome::All,
                },
                // A spurious wake: keep waiting.
                _ => park_decision(node, node_type),
            }
        }
        FLOW_WAIT_FOR_ALL => {
            let required = string_list(node, "eventTypes");
            let mut received = cursor.received.get(&node.id).cloned().unwrap_or_default();
            if let Some(t) = event_type {
                if required.iter().any(|r| r == t) {
                    received.insert(t.to_string());
                }
            }
            let complete = required.iter().all(|r| received.contains(r));
            if complete || ctx.event.is_null() {
                NodeExec::Succeeded {
                    output: json!({
                        "complete": complete,
                        "received": received.iter().cloned().collect::<Vec<_>>(),
                    }),
                    edges: EdgeOutcome::All,
                }
            } else {
                // Partial: re-park, carrying what has arrived.
                NodeExec::Parked {
                    wake_at: None,
                    wake_on: Some(required.join(",")),
                    received: received.iter().cloned().collect(),
                }
            }
        }
        FLOW_DEADLINE => {
            let accepted = string_list(node, "eventTypes");
            let matched = event_type
                .map(|t| accepted.iter().any(|a| a == t))
                .unwrap_or(false);
            if matched {
                NodeExec::Succeeded {
                    output: json!({ "outcome": "event", "matchedEventType": event_type, "expired": false }),
                    edges: EdgeOutcome::All,
                }
            } else {
                // The deadline expired. Honour onExpiry.
                let on_expiry = node
                    .params
                    .get("onExpiry")
                    .and_then(Value::as_str)
                    .unwrap_or("continue");
                match on_expiry {
                    "fail" => NodeExec::Terminal {
                        status: RunStatus::Failed,
                        code: Some("deadline_expired".into()),
                        error: Some("flow.deadline expired".into()),
                    },
                    "route" => NodeExec::Succeeded {
                        output: json!({ "outcome": "expired", "expired": true }),
                        edges: EdgeOutcome::Labels(vec![ERROR_LABEL.to_string()]),
                    },
                    _ => NodeExec::Succeeded {
                        output: json!({ "outcome": "expired", "expired": true }),
                        edges: EdgeOutcome::All,
                    },
                }
            }
        }
        "flow.awaitApproval" => {
            // The approval lifecycle rides the agent-proposal path; when a
            // decision event arrives the gate resolves.
            if ctx.event.is_null() {
                NodeExec::Succeeded {
                    output: json!({ "outcome": "expired" }),
                    edges: EdgeOutcome::All,
                }
            } else {
                NodeExec::Succeeded {
                    output: json!({ "outcome": "approved", "event": ctx.event }),
                    edges: EdgeOutcome::All,
                }
            }
        }
        _ => NodeExec::Failed {
            code: "unknown_park".into(),
            error: format!("unknown parking node {node_type:?}"),
        },
    }
}

/// The wake coordinates a FIRST entry to a parking node records.
fn park_decision(node: &WorkflowNode, node_type: &str) -> NodeExec {
    let now = chrono::Utc::now();
    match node_type {
        FLOW_DELAY => {
            let duration = node
                .params
                .get("duration")
                .and_then(Value::as_str)
                .unwrap_or("");
            match parse_duration(duration) {
                Some(secs) => NodeExec::Parked {
                    wake_at: Some(now + chrono::Duration::seconds(secs)),
                    wake_on: None,
                    received: vec![],
                },
                None => NodeExec::Failed {
                    code: "bad_duration".into(),
                    error: format!("cannot read duration {duration:?}"),
                },
            }
        }
        FLOW_WAIT_UNTIL => {
            let at = node.params.get("at").and_then(Value::as_str).unwrap_or("");
            match chrono::DateTime::parse_from_rfc3339(at) {
                Ok(when) => NodeExec::Parked {
                    wake_at: Some(when.with_timezone(&chrono::Utc)),
                    wake_on: None,
                    received: vec![],
                },
                Err(_) => NodeExec::Failed {
                    code: "bad_instant".into(),
                    error: format!("`at` is not an RFC3339 instant: {at:?}"),
                },
            }
        }
        FLOW_WAIT_FOR => {
            let accepted = string_list(node, "eventTypes");
            let wake_at =
                string_duration(node, "timeout").map(|s| now + chrono::Duration::seconds(s));
            NodeExec::Parked {
                wake_at,
                wake_on: Some(accepted.join(",")),
                received: vec![],
            }
        }
        FLOW_WAIT_FOR_ALL => {
            let accepted = string_list(node, "eventTypes");
            let wake_at =
                string_duration(node, "timeout").map(|s| now + chrono::Duration::seconds(s));
            NodeExec::Parked {
                wake_at,
                wake_on: Some(accepted.join(",")),
                received: vec![],
            }
        }
        FLOW_DEADLINE => {
            let accepted = string_list(node, "eventTypes");
            let wake_at = node
                .params
                .get("until")
                .and_then(Value::as_str)
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.with_timezone(&chrono::Utc));
            NodeExec::Parked {
                wake_at,
                wake_on: Some(accepted.join(",")),
                received: vec![],
            }
        }
        "flow.awaitApproval" => {
            let wake_at =
                string_duration(node, "timeout").map(|s| now + chrono::Duration::seconds(s));
            NodeExec::Parked {
                wake_at,
                wake_on: Some("agent_proposal.decided".into()),
                received: vec![],
            }
        }
        _ => NodeExec::Failed {
            code: "unknown_park".into(),
            error: format!("unknown parking node {node_type:?}"),
        },
    }
}

/// Calls core-api's org-scoped node endpoint for one effect-bearing node.
async fn call_node<E: EffectApi>(
    effects: &E,
    ctx: &RunContext,
    cursor: &WorkflowCursor,
    node: &WorkflowNode,
) -> NodeExec {
    // Any `$ref` in the params is resolved against the run's recorded
    // outputs BEFORE the call (item F): this is how one step's value
    // reaches the next step's input. A ref to a node that has not run is a
    // typed step failure, never a silent null.
    let params = match resolve_params(cursor, &node.params) {
        Ok(params) => params,
        Err(error) => {
            return NodeExec::Failed {
                code: "unresolved_reference".into(),
                error,
            }
        }
    };
    let body = json!({
        "org_id": ctx.org_id,
        "actor_user_id": ctx.actor_user_id,
        "node_id": node.node_type,
        "params": params,
        "context": {
            "event": ctx.event,
            "event_type": ctx.event_type,
            "run_id": ctx.run_id,
        },
    });
    let (outcome, response) = effects.post_json(NODE_ENDPOINT, body).await;
    match outcome {
        EffectOutcome::Applied => {
            let output = response.get("output").cloned().unwrap_or(Value::Null);
            NodeExec::Succeeded {
                output,
                edges: EdgeOutcome::All,
            }
        }
        EffectOutcome::Rejected => {
            // A 4xx is a typed, permanent node refusal (an unsupported
            // operation, a missing entity, a schema violation). It is a
            // step failure the ledger shows, never a transient retry.
            let detail = response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("node refused by core-api")
                .to_string();
            NodeExec::Failed {
                code: "node_refused".into(),
                error: detail,
            }
        }
        EffectOutcome::Transient => NodeExec::Failed {
            code: "node_unreachable".into(),
            error: "core-api unreachable while executing a node".into(),
        },
    }
}

// ---------------------------------------------------------------------------
// small pure helpers
// ---------------------------------------------------------------------------

/// Resolves every value reference (`$ref`) in one node's params against the
/// run's recorded outputs (item F). Recurses through objects and arrays so a
/// ref may sit anywhere a value can. A malformed ref, or one naming a node
/// with no recorded output, is a typed string error the caller turns into a
/// step failure — the document was validated at save time, so a missing
/// target here means the upstream step did not produce an output.
fn resolve_params(cursor: &WorkflowCursor, params: &Value) -> Result<Value, String> {
    if node_registry::is_param_ref(params) {
        let (target, path) = node_registry::param_ref_parts(params)
            .ok_or_else(|| "malformed value reference: expected {node, path}".to_string())?;
        let output = cursor
            .outputs
            .get(target)
            .ok_or_else(|| format!("value reference to {target:?}, which has not run yet"))?;
        return Ok(lookup_path(output, path));
    }
    match params {
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (key, value) in map {
                out.insert(key.clone(), resolve_params(cursor, value)?);
            }
            Ok(Value::Object(out))
        }
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(resolve_params(cursor, item)?);
            }
            Ok(Value::Array(out))
        }
        other => Ok(other.clone()),
    }
}

/// Reads a dotted path out of a JSON value. An absent path is `Null`.
fn lookup_path(value: &Value, path: &str) -> Value {
    let mut current = value;
    for segment in path.split('.').filter(|s| !s.is_empty()) {
        match current.get(segment) {
            Some(next) => current = next,
            None => return Value::Null,
        }
    }
    current.clone()
}

/// Closed arithmetic: `+ - * /`. Division by zero is `None`.
fn math(left: f64, op: &str, right: f64) -> Option<f64> {
    match op {
        "+" => Some(left + right),
        "-" => Some(left - right),
        "*" => Some(left * right),
        "/" if right != 0.0 => Some(left / right),
        _ => None,
    }
}

/// Closed substitution: `{name}` is replaced by `fields[name]`. Unmatched
/// braces are left as literal text, so a template can never execute.
fn render_template(template: &str, fields: &Value) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < template.len() {
        if bytes[i] == b'{' {
            if let Some(close) = template[i + 1..].find('}') {
                let name = &template[i + 1..i + 1 + close];
                if !name.is_empty() && !name.contains('{') {
                    let replacement = fields
                        .get(name)
                        .map(value_to_text)
                        .unwrap_or_else(|| format!("{{{name}}}"));
                    out.push_str(&replacement);
                    i = i + 1 + close + 1;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn value_to_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Reads a string-array param (empty when absent).
fn string_list(node: &WorkflowNode, key: &str) -> Vec<String> {
    node.params
        .get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Reads a duration-string param (e.g. `timeout: "4h"`) as seconds.
fn string_duration(node: &WorkflowNode, key: &str) -> Option<i64> {
    node.params
        .get(key)
        .and_then(Value::as_str)
        .and_then(parse_duration)
}

/// Parses `90s`, `10m`, `4h`, `2d`. A bare number is seconds.
fn parse_duration(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (digits, unit) = raw.split_at(raw.len() - 1);
    if let Ok(number) = raw.parse::<i64>() {
        // No unit: seconds.
        return Some(number);
    }
    let number: i64 = digits.parse().ok()?;
    match unit {
        "s" => Some(number),
        "m" => Some(number * 60),
        "h" => Some(number * 3_600),
        "d" => Some(number * 86_400),
        _ => None,
    }
}

/// The items a `flow.foreach` iterates: the collection node's output under
/// `items`, else `value`, else the output itself when it is an array.
fn collection_items(output: &Value) -> Vec<Value> {
    if let Some(items) = output.get("items").and_then(Value::as_array) {
        return items.clone();
    }
    if let Some(items) = output.get("value").and_then(Value::as_array) {
        return items.clone();
    }
    output.as_array().cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn duration_parsing_covers_the_units() {
        assert_eq!(parse_duration("90"), Some(90));
        assert_eq!(parse_duration("90s"), Some(90));
        assert_eq!(parse_duration("10m"), Some(600));
        assert_eq!(parse_duration("4h"), Some(14_400));
        assert_eq!(parse_duration("2d"), Some(172_800));
        assert_eq!(parse_duration("nope"), None);
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("4w"), None);
    }

    #[test]
    fn reference_lookup_walks_dotted_paths() {
        let value = json!({ "a": { "b": { "c": 7 } } });
        assert_eq!(lookup_path(&value, "a.b.c"), json!(7));
        assert_eq!(lookup_path(&value, "a.missing"), Value::Null);
        assert_eq!(lookup_path(&value, ""), value);
    }

    #[test]
    fn math_is_a_closed_operator_set() {
        assert_eq!(math(4.0, "+", 2.0), Some(6.0));
        assert_eq!(math(4.0, "-", 2.0), Some(2.0));
        assert_eq!(math(4.0, "*", 2.0), Some(8.0));
        assert_eq!(math(4.0, "/", 2.0), Some(2.0));
        assert_eq!(math(4.0, "/", 0.0), None);
        assert_eq!(math(4.0, "^", 2.0), None);
    }

    #[test]
    fn template_substitutes_named_fields_only() {
        let fields = json!({ "name": "Ada", "count": 3 });
        assert_eq!(render_template("Hi {name} x{count}", &fields), "Hi Ada x3");
        // A field with no entry is left literal — no evaluation.
        assert_eq!(render_template("Hi {missing}", &fields), "Hi {missing}");
        // A nested brace never opens an expression.
        assert_eq!(render_template("{{name}}", &fields), "{Ada}");
    }

    #[test]
    fn collection_items_reads_the_common_shapes() {
        assert_eq!(
            collection_items(&json!({"items": [1, 2]})),
            vec![json!(1), json!(2)]
        );
        assert_eq!(collection_items(&json!({"value": [3]})), vec![json!(3)]);
        assert_eq!(collection_items(&json!([4, 5])), vec![json!(4), json!(5)]);
        assert!(collection_items(&json!({"other": true})).is_empty());
    }

    #[test]
    fn park_decisions_read_the_right_wake_coordinates() {
        // flow.delay → a wake-at, no event.
        let delay = WorkflowNode {
            id: "d".into(),
            node_type: FLOW_DELAY.into(),
            params: json!({ "duration": "4h" }),
        };
        match park_decision(&delay, FLOW_DELAY) {
            NodeExec::Parked {
                wake_at, wake_on, ..
            } => {
                assert!(wake_at.is_some());
                assert!(wake_on.is_none());
            }
            _ => panic!("delay parks"),
        }
        // flow.waitFor → an event wake (comma list), no time when no timeout.
        let wait = WorkflowNode {
            id: "w".into(),
            node_type: FLOW_WAIT_FOR.into(),
            params: json!({ "eventTypes": ["a.b", "c.d"] }),
        };
        match park_decision(&wait, FLOW_WAIT_FOR) {
            NodeExec::Parked {
                wake_at, wake_on, ..
            } => {
                assert!(wake_at.is_none());
                assert_eq!(wake_on.as_deref(), Some("a.b,c.d"));
            }
            _ => panic!("waitFor parks on an event"),
        }
    }

    #[test]
    fn a_bad_delay_duration_is_a_typed_failure() {
        let delay = WorkflowNode {
            id: "d".into(),
            node_type: FLOW_DELAY.into(),
            params: json!({ "duration": "soon" }),
        };
        match park_decision(&delay, FLOW_DELAY) {
            NodeExec::Failed { code, .. } => assert_eq!(code, "bad_duration"),
            _ => panic!("a bad duration fails"),
        }
    }

    fn test_ctx(event: Value, entity: Option<&str>) -> RunContext {
        RunContext {
            org_id: uuid::Uuid::nil(),
            run_id: uuid::Uuid::nil(),
            actor_user_id: uuid::Uuid::nil(),
            event,
            event_type: Some("ticket.created".into()),
            event_entity: entity.map(str::to_string),
            resumed: false,
        }
    }

    fn succeeded_matched(exec: NodeExec) -> bool {
        match exec {
            NodeExec::Succeeded { output, .. } => output
                .get("matched")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            _ => panic!("a condition succeeds"),
        }
    }

    #[test]
    fn conditions_read_the_subject_the_payload_and_upstream_outputs() {
        // `ticket.priority` resolves against the triggering SUBJECT (the
        // event's entity), which is the shape item A made reachable.
        let ctx = test_ctx(
            json!({ "ticket_id": "t", "priority": "p1" }),
            Some("ticket"),
        );
        let cursor = WorkflowCursor::start();
        let node = WorkflowNode {
            id: "c".into(),
            node_type: "field.equals".into(),
            params: json!({ "field": "ticket.priority", "value": "p1" }),
        };
        assert!(succeeded_matched(condition_node(&ctx, &cursor, &node)));

        // A bare path reads the payload, and `event.` is an explicit root.
        let node = WorkflowNode {
            id: "c".into(),
            node_type: "field.isEmpty".into(),
            params: json!({ "field": "event.missing" }),
        };
        assert!(succeeded_matched(condition_node(&ctx, &cursor, &node)));

        // `nodes.<id>.<path>` reads an upstream node's recorded output.
        let mut cursor = WorkflowCursor::start();
        cursor.record_node("check", json!({ "matched": false }));
        let node = WorkflowNode {
            id: "c".into(),
            node_type: "field.equals".into(),
            params: json!({ "field": "nodes.check.matched", "value": false }),
        };
        assert!(succeeded_matched(condition_node(&ctx, &cursor, &node)));
    }

    #[test]
    fn numeric_and_list_conditions_compare_as_declared() {
        let ctx = test_ctx(json!({ "hours": 5, "tags": ["a", "b"], "empty": [] }), None);
        let cursor = WorkflowCursor::start();
        let gt = WorkflowNode {
            id: "n".into(),
            node_type: "number.gt".into(),
            params: json!({ "field": "hours", "value": 4 }),
        };
        assert!(succeeded_matched(condition_node(&ctx, &cursor, &gt)));
        let between = WorkflowNode {
            id: "n".into(),
            node_type: "number.between".into(),
            params: json!({ "field": "hours", "min": 6, "max": 9 }),
        };
        assert!(!succeeded_matched(condition_node(&ctx, &cursor, &between)));
        let contains = WorkflowNode {
            id: "l".into(),
            node_type: "list.contains".into(),
            params: json!({ "field": "tags", "value": "b" }),
        };
        assert!(succeeded_matched(condition_node(&ctx, &cursor, &contains)));
        let empty = WorkflowNode {
            id: "l".into(),
            node_type: "list.isEmpty".into(),
            params: json!({ "field": "empty" }),
        };
        assert!(succeeded_matched(condition_node(&ctx, &cursor, &empty)));
    }

    #[test]
    fn flow_branch_reads_the_matched_key_conditions_write() {
        // Item B: the branch arm read `outputs[cond]["holds"]` while every
        // condition declares `matched`, so every branch took the false arm.
        let ctx = test_ctx(json!({}), None);
        let mut cursor = WorkflowCursor::start();
        cursor.record_node("c1", json!({ "matched": true }));
        let branch = WorkflowNode {
            id: "b".into(),
            node_type: FLOW_BRANCH.into(),
            params: json!({ "condition": "c1" }),
        };
        match control_node(&ctx, &cursor, &branch) {
            NodeExec::Succeeded { edges, .. } => match edges {
                EdgeOutcome::Labels(labels) => {
                    assert_eq!(labels, vec![DEFAULT_TRUE_LABEL.to_string()])
                }
                _ => panic!("a branch fires one labelled edge"),
            },
            _ => panic!("a branch succeeds"),
        }
    }

    #[test]
    fn value_references_resolve_against_recorded_outputs() {
        // Item F: capture's output feeds the next step's uuid slot.
        let mut cursor = WorkflowCursor::start();
        cursor.record_node(
            "capture",
            json!({ "signOffId": "abc", "nested": { "x": 2 } }),
        );
        let params = json!({
            "attachmentId": { "$ref": { "node": "capture", "path": "signOffId" } },
            "nested": { "$ref": { "node": "capture", "path": "nested.x" } },
            "keep": "literal"
        });
        let resolved = resolve_params(&cursor, &params).expect("refs resolve");
        assert_eq!(resolved["attachmentId"], json!("abc"));
        assert_eq!(resolved["nested"], json!(2));
        assert_eq!(resolved["keep"], json!("literal"));

        // A ref to a node that has not run is a typed error, not a null.
        let missing = resolve_params(&cursor, &json!({ "$ref": { "node": "nope", "path": "" } }));
        assert!(missing.is_err());
    }
}
