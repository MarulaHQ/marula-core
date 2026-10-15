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

/// The queue attempts a resume job gets before it dead-letters. A run that
/// cannot move is visible, not silently stuck.
const RESUME_MAX_ATTEMPTS: i32 = 5;

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
    /// The triggering envelope's payload, when an event started or woke
    /// this run. Read by conditions and by the event waits.
    event: Value,
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
    let mut backoff_ms = 100_u64;
    loop {
        if *shutdown.borrow() {
            return;
        }
        match client.get_multiplexed_async_connection().await {
            Ok(mut conn) => {
                backoff_ms = 100;
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
                        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                        backoff_ms = (backoff_ms * 2).min(5_000);
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
        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
        backoff_ms = (backoff_ms * 2).min(5_000);
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
    // The whole envelope rides the job: its `type` is what a wait matches,
    // and its `payload` is the context a reference reads.
    let event = serde_json::to_value(envelope).unwrap_or(Value::Null);
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
            "event": event,
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
            "event": event,
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
            max_attempts: RESUME_MAX_ATTEMPTS,
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
            max_attempts: RESUME_MAX_ATTEMPTS,
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
        Some(ExecutorPath::Condition)
        | Some(ExecutorPath::DomainWrapper(_))
        | Some(ExecutorPath::DomainUpdate)
        | Some(ExecutorPath::DomainQuery)
        | Some(ExecutorPath::Ai(_))
        | Some(ExecutorPath::Plugin(_)) => call_node(effects, ctx, node).await,
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
                .and_then(|o| o.get("holds"))
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
    // Resumed: was the wait satisfied?
    let event_type = ctx.event.get("type").and_then(Value::as_str);
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
async fn call_node<E: EffectApi>(effects: &E, ctx: &RunContext, node: &WorkflowNode) -> NodeExec {
    let body = json!({
        "org_id": ctx.org_id,
        "actor_user_id": ctx.actor_user_id,
        "node_id": node.node_type,
        "params": node.params,
        "context": { "event": ctx.event, "run_id": ctx.run_id },
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
}
