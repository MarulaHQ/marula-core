//! The workflow run ledger's operator surface (the private build tracker Wave 43 lane 14,
//! `p43-workflow-run-ledger`).
//!
//! Purpose: view runs, and retry failures. The reads are the webhook
//! delivery ledger's shape (keyset-cursored, `next_cursor` opaquely encoded)
//! so the two logs page and behave alike; the retry is a first-class
//! operator action on the IF1–IF3 envelope, not a queue flag.
//!
//! # Why the retry is an action and not a flag
//!
//! Re-running a failed run must not silently double-apply the steps that
//! already succeeded. The only authority on that is the node registry's
//! per-node `idempotent` flag (lane 13), so the decision is made in
//! core-domain (`workflow_runs::retry_plan`) and REFUSED here with a typed
//! reason that names every offending node. A retry that would double-apply
//! is never recorded as having happened.
//!
//! When it is allowed, the retry is recorded as a NEW run row with
//! `retry_of` pointing at the failed one — the ledger gets longer and the
//! failed run keeps saying `failed`, which is what happened.
//!
//! Non-purpose: no execution (the `workflow.resume` job is enqueued here on
//! the EXISTING queue and settled by the executor lane), no step writes
//! (the executor owns those through core-db), and no workflow authoring.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, MutationMeta, MutationOutcome, MutationParams};
use crate::org_tx::{self, OrgTx};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use core_domain::audit::AuditEvent;
use core_domain::entities::Actor;
use core_domain::policy::{Action, Module, Permission};
use core_domain::workflow_runs::{RetryMode, RunStatus, RunTrigger, StepStatus};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// Admin-surface permission helper, the same shape the SLA and automation
/// admin routes use: the run ledger is operator tooling, not a per-board
/// member read.
fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

/// A run as the operator surface reads it. The document snapshot is NOT
/// here: it is large, it is write-once, and the operator does not need it to
/// read a status. It is on the detail response, where an operator who wants
/// to know exactly what ran can find it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RunView {
    pub id: Uuid,
    pub rule_id: Option<Uuid>,
    pub rule_name: String,
    pub rule_version: i32,
    pub catalogue_version: String,
    pub status: RunStatus,
    pub trigger: RunTrigger,
    pub trigger_event_id: Option<Uuid>,
    pub trigger_job_id: Option<Uuid>,
    pub trigger_user_id: Option<Uuid>,
    /// The parked run this row continues. A row is an EXECUTION ATTEMPT;
    /// a resume is a new row, never an edit of the parked one.
    pub resumed_from: Option<Uuid>,
    /// The failed run this row re-runs.
    pub retry_of: Option<Uuid>,
    /// Derived, never stored: a later run continues this one. Storing the
    /// flag would be a mutation of the parked row wearing a different name.
    pub superseded: bool,
    /// Derived, never stored: a later run re-runs this one.
    pub retried: bool,
    pub wake_at: Option<DateTime<Utc>>,
    pub wake_on: Option<String>,
    pub error_code: Option<String>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One step attempt inside a run.
#[derive(Debug, Serialize, ToSchema)]
pub struct RunStepView {
    pub id: Uuid,
    pub step_index: i32,
    pub node_id: String,
    pub node_label: String,
    pub attempt: i32,
    pub iteration: Option<i32>,
    pub status: StepStatus,
    pub input: serde_json::Value,
    pub output: Option<serde_json::Value>,
    pub error_code: Option<String>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// A run plus the exact document it ran and the state it continues from.
/// This is the response that makes the ledger an audit record: with it, a
/// run explains itself without a second table.
#[derive(Debug, Serialize, ToSchema)]
pub struct RunDetailView {
    #[serde(flatten)]
    pub run: RunView,
    pub rule_definition: serde_json::Value,
    pub resume_cursor: Option<serde_json::Value>,
    /// The whole trace in execution order.
    pub steps: Vec<RunStepView>,
}

// --- opaque cursors ---------------------------------------------------------
//
// The same encoding as the webhook delivery ledger and the audit trail:
// base64url of `timestamp|id`, which the client treats as a blob and never
// parses. The step trace carries a third field because a step is identified
// by (step_index, attempt) rather than by id alone.

fn encode_cursor(parts: &str) -> String {
    URL_SAFE_NO_PAD.encode(parts.as_bytes())
}

fn decode_cursor(raw: Option<&str>) -> Result<Option<(String, String, String)>, ApiError> {
    let Some(raw) = raw else { return Ok(None) };
    let decoded = URL_SAFE_NO_PAD
        .decode(raw)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .ok_or_else(|| ApiError::Validation("malformed cursor".into()))?;
    let mut fields = decoded.split('|');
    let a = fields
        .next()
        .ok_or_else(|| ApiError::Validation("malformed cursor".into()))?;
    let b = fields
        .next()
        .ok_or_else(|| ApiError::Validation("malformed cursor".into()))?;
    let c = fields
        .next()
        .ok_or_else(|| ApiError::Validation("malformed cursor".into()))?;
    Ok(Some((a.into(), b.into(), c.into())))
}

/// Query for the run ledger listing.
#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct RunListQuery {
    /// Optional status filter: running | waiting | succeeded | failed | abandoned.
    #[serde(default)]
    pub status: Option<String>,
    /// Optional rule filter: only that workflow's runs.
    #[serde(default)]
    pub rule_id: Option<Uuid>,
    /// Cursor from the previous page's `next_cursor`.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size (default 50, capped at 200).
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RunListResponse {
    pub runs: Vec<RunView>,
    /// Opaque cursor for the next page; `null` on the last one.
    pub next_cursor: Option<String>,
}

/// GET /api/workflows/runs/{id} — one run with its whole trace.
#[utoipa::path(operation_id = "workflow_run_detail", get, path = "/api/workflows/runs/{id}", tag = "workflows", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = RunDetailView),
              (status = 404, description = "run not visible in scope")))]
pub async fn get(
    State(state): State<AppState>,
    extensions: axum::http::Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<RunDetailView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let run = core_db::workflow_runs::get_run(&mut tx, org_id, id).await?;
    let steps = core_db::workflow_runs::steps_for_run(&mut tx, org_id, id).await?;
    Ok(Json(RunDetailView {
        run: RunView::from_row(run.clone(), false, false),
        rule_definition: run.rule_definition,
        resume_cursor: run.resume_cursor,
        steps: steps.into_iter().map(RunStepView::from_row).collect(),
    }))
}

/// Query for one run's step trace.
#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct RunStepListQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RunStepListResponse {
    pub steps: Vec<RunStepView>,
    pub next_cursor: Option<String>,
}

/// POST /api/workflows/runs/{id}/retry — the operator action.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RetryRunRequest {
    /// How far back the re-run goes. `resume` (the default) starts at the
    /// first step that did not succeed, so the steps before it are not run
    /// again. `from_start` re-runs the whole document and is only allowed
    /// when every step that already succeeded is safe to repeat.
    #[serde(default)]
    pub mode: RetryMode,
    #[serde(flatten)]
    pub instant: MutationParams,
}

/// The retry's response: the NEW run row (the continuation of the failed
/// one) and the IF2 mutation metadata, on the same envelope every other
/// mutation in the product answers with.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RunRetryResponse {
    pub run: RunView,
    pub mutation: MutationMeta,
}

/// GET /api/workflows/runs — the run ledger, newest first, filterable by
/// status and by rule. Keyset-cursored on `(created_at, id)`, exactly like
/// the webhook delivery ledger: the two logs page and behave alike.
#[utoipa::path(operation_id = "workflow_runs_list", get, path = "/api/workflows/runs", tag = "workflows", security(("bearer" = [])),
    params(RunListQuery),
    responses((status = 200, body = RunListResponse)))]
pub async fn list(
    State(state): State<AppState>,
    extensions: axum::http::Extensions,
    Query(q): Query<RunListQuery>,
) -> ApiResult<Json<RunListResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let status = match q.status.as_deref() {
        Some(raw) => Some(
            RunStatus::parse(raw)
                .ok_or_else(|| ApiError::Validation(format!("unknown run status {raw:?}")))?,
        ),
        None => None,
    };
    let cursor_parts = decode_cursor(q.cursor.as_deref())?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let cursor = match cursor_parts {
        Some((at, id, _)) => Some((
            at.parse::<DateTime<Utc>>()
                .map_err(|_| ApiError::Validation("malformed cursor".into()))?,
            id.parse::<Uuid>()
                .map_err(|_| ApiError::Validation("malformed cursor".into()))?,
        )),
        None => None,
    };
    let page = core_db::workflow_runs::list_runs(
        &mut tx,
        org_id,
        &core_db::workflow_runs::RunFilter {
            status,
            rule_id: q.rule_id,
            cursor,
        },
        q.limit.unwrap_or(50),
    )
    .await?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    // Only a FULL page carries a next cursor: the same rule as the webhook
    // delivery ledger, so a client does not have to special-case this log.
    let next_cursor = if page.len() as i64 >= limit {
        page.last().map(|row| {
            encode_cursor(&format!(
                "{}|{}|",
                row.run.created_at.to_rfc3339(),
                row.run.id
            ))
        })
    } else {
        None
    };
    Ok(Json(RunListResponse {
        runs: page
            .into_iter()
            .map(|row| RunView::from_row(row.run, row.superseded, row.retried))
            .collect(),
        next_cursor,
    }))
}

/// GET /api/workflows/runs/{id}/steps — one run's step trace, paged.
#[utoipa::path(operation_id = "workflow_run_steps", get, path = "/api/workflows/runs/{id}/steps", tag = "workflows", security(("bearer" = [])),
    params(RunStepListQuery, ("id" = Uuid, Path)),
    responses((status = 200, body = RunStepListResponse),
              (status = 404, description = "run not visible in scope")))]
pub async fn list_steps(
    State(state): State<AppState>,
    extensions: axum::http::Extensions,
    Path(id): Path<Uuid>,
    Query(q): Query<RunStepListQuery>,
) -> ApiResult<Json<RunStepListResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let cursor_parts = decode_cursor(q.cursor.as_deref())?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    // The run must be visible in scope before its trace is read: a 404 on
    // the run and a 404 on the trace are the same fact.
    core_db::workflow_runs::get_run(&mut tx, org_id, id).await?;
    let cursor = match cursor_parts {
        Some((step_index, attempt, step_id)) => Some((
            step_index
                .parse::<i32>()
                .map_err(|_| ApiError::Validation("malformed cursor".into()))?,
            attempt
                .parse::<i32>()
                .map_err(|_| ApiError::Validation("malformed cursor".into()))?,
            step_id
                .parse::<Uuid>()
                .map_err(|_| ApiError::Validation("malformed cursor".into()))?,
        )),
        None => None,
    };
    let steps =
        core_db::workflow_runs::list_steps(&mut tx, org_id, id, cursor, q.limit.unwrap_or(100))
            .await?;
    let limit = q.limit.unwrap_or(100).clamp(1, 200);
    // Same rule as the run listing: a full page carries a next cursor, a
    // short one is the end of the trace.
    let next_cursor = if steps.len() as i64 >= limit {
        steps
            .last()
            .map(|s| encode_cursor(&format!("{}|{}|{}", s.step_index, s.attempt, s.id)))
    } else {
        None
    };
    Ok(Json(RunStepListResponse {
        steps: steps.into_iter().map(RunStepView::from_row).collect(),
        next_cursor,
    }))
}

/// POST /api/workflows/runs/{id}/retry — re-run a failed run as a NEW row.
///
/// The refusal is the point: if re-running would repeat a step that
/// declares `idempotent = false`, the answer is a typed 409 that names every
/// offending node rather than a second ticket or a second email. The
/// idempotency key (IF1) is claimed before anything is written, so a
/// double-submit returns the first retry instead of queueing two.
#[utoipa::path(operation_id = "workflow_run_retry", post, path = "/api/workflows/runs/{id}/retry", tag = "workflows", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = RetryRunRequest,
    responses((status = 200, body = RunRetryResponse),
              (status = 404, description = "run not visible in scope"),
              (status = 409, description = "not retryable, or a repeat would double-apply"),
              (status = 422, description = "unknown retry mode")))]
pub async fn retry(
    State(state): State<AppState>,
    extensions: axum::http::Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<RetryRunRequest>,
) -> ApiResult<Json<RunRetryResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let outcome = org_tx::with_org_tx(
        &state,
        org_id,
        user_id,
        move |mut ctx: OrgTx<'static>| async move {
            let key = req.instant.idempotency_key.clone();
            let fingerprint = serde_json::json!({
                "run_id": id,
                "mode": req.mode.as_str(),
            });
            // IF1: the idempotency gate is the FIRST statement sequence. A
            // replay answers with the stored response before anything changes.
            match instant::claim(
                &mut ctx,
                instant::ClaimSpec {
                    entity: "workflow_run",
                    entity_id: Some(id),
                    key: key.as_deref(),
                    fingerprint: &fingerprint,
                },
            )
            .await?
            {
                instant::InstantClaim::Replay(stored) => {
                    // The exact response is returned; `MutationOutcome`'s
                    // deserialization proves it into the same shape it was
                    // stored as, so a replay is byte-identical.
                    return Ok((ctx, MutationOutcome::Replayed(stored)));
                }
                instant::InstantClaim::Fresh => {}
            }

            // The run must be visible in scope: under RLS that also means it
            // exists, and a write that did not land because the target left the
            // visible scope reconciles by resyncing, not by re-POSTing.
            core_db::workflow_runs::get_run(&mut ctx.tx, org_id, id).await?;

            // The re-run is a job on the EXISTING queue, and the retry row
            // records which job: the two are one fact, so the job is enqueued
            // INSIDE this transaction. `workflow.resume` is settled by the
            // executor lane (that is its own to-do); until then the queue
            // dead-letters it as an unknown kind, which is loud and recoverable
            // rather than a parked run that silently never wakes.
            let job_payload = serde_json::json!({
                "run_id": id,
                "retry": true,
            });
            let job_id = core_db::queue::enqueue(
                &mut *ctx.tx,
                &core_db::queue::NewJob {
                    org_id,
                    kind: core_db::workflow_runs::WORKFLOW_RESUME_JOB.to_string(),
                    payload: job_payload,
                    max_attempts: core_domain::retry::attempt_limit_i32(
                        core_domain::retry::Subsystem::Queue,
                    ),
                },
            )
            .await?;

            // The decision (which steps may re-run, and the refusal when they
            // may not) is made by core-domain against lane 13's idempotence
            // flags; this persists its answer as a new row. The failed run is
            // not touched.
            let row = core_db::workflow_runs::retry_run(
                &mut ctx.tx,
                org_id,
                id,
                user_id,
                Some(job_id),
                req.mode,
            )
            .await
            .map_err(refusal_to_api)?;

            core_db::audit::insert(
                &mut *ctx.tx,
                &AuditEvent {
                    id: Uuid::now_v7(),
                    org_id,
                    actor: Actor::Human(user_id),
                    action: "workflow.run_retried".into(),
                    entity_type: Some("workflow_run".into()),
                    entity_id: Some(id.to_string()),
                    detail: serde_json::json!({
                        "run_id": id,
                        "retry_run_id": row.id,
                        "mode": req.mode.as_str(),
                        "resume_job_id": job_id,
                    }),
                    ip: None,
                    occurred_at: chrono::Utc::now(),
                },
            )
            .await?;

            let view = RunView::from_row(row, false, false);
            let write_seq = instant::entity_seq(
                &mut ctx,
                instant::SeqSpec {
                    entity: "workflow_run",
                    entity_id: id,
                    version: None,
                },
            )
            .await?;
            let after = serde_json::to_value(view.clone()).unwrap_or(serde_json::Value::Null);
            let now = chrono::Utc::now();
            let placeholder = RunRetryResponse {
                run: view.clone(),
                mutation: MutationMeta {
                    seq: write_seq,
                    version: write_seq,
                    updated_at: now,
                    before: None,
                    after: serde_json::Value::Null,
                    idempotency_key: key.clone(),
                },
            };
            let mutation = instant::record(
                &mut ctx,
                instant::RecordSpec {
                    entity: "workflow_run",
                    entity_id: id,
                    op: "retry",
                    seq: write_seq,
                    before: Some(serde_json::json!({ "status": "failed" })),
                    after,
                    updated_at: now,
                    key: key.as_deref(),
                    payload: &placeholder,
                    provenance: Default::default(),
                },
            )
            .await?;
            Ok((
                ctx,
                MutationOutcome::Applied(RunRetryResponse {
                    run: view,
                    mutation,
                }),
            ))
        },
    )
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// Maps a ledger refusal onto the typed 409 envelope (IF3): the reason stays
/// machine-readable and the message names every offending node, so a client
/// can render exactly what stopped the retry without parsing prose.
fn refusal_to_api(error: core_db::DbError) -> ApiError {
    match error {
        core_db::DbError::Conflict(message) => ApiError::RunRetryRefused { message },
        other => other.into(),
    }
}

impl RunView {
    fn from_row(row: core_db::workflow_runs::RunRow, superseded: bool, retried: bool) -> Self {
        RunView {
            id: row.id,
            rule_id: row.rule_id,
            rule_name: row.rule_name,
            rule_version: row.rule_version,
            catalogue_version: row.catalogue_version,
            status: row.status,
            trigger: row.trigger,
            trigger_event_id: row.trigger_event_id,
            trigger_job_id: row.trigger_job_id,
            trigger_user_id: row.trigger_user_id,
            resumed_from: row.resumed_from,
            retry_of: row.retry_of,
            superseded,
            retried,
            wake_at: row.wake_at,
            wake_on: row.wake_on,
            error_code: row.error_code,
            error: row.error,
            started_at: row.started_at,
            finished_at: row.finished_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

impl RunStepView {
    fn from_row(row: core_db::workflow_runs::RunStepRow) -> Self {
        RunStepView {
            id: row.id,
            step_index: row.step_index,
            node_id: row.node_id,
            node_label: row.node_label,
            attempt: row.attempt,
            iteration: row.iteration,
            status: row.status,
            input: row.input,
            output: row.output,
            error_code: row.error_code,
            error: row.error,
            started_at: row.started_at,
            finished_at: row.finished_at,
        }
    }
}
