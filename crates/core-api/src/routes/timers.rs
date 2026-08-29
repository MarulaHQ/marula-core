//! Timer + time-entry endpoints implementing the single-active-timer
//! workflow (§1): one running timer per user; starting another auto-pauses
//! the current one into the docked working set. All clock math is
//! server-authoritative.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Query, State},
    http::Extensions,
    Json,
};
use core_domain::entities::{TimeEntry, TimerSession};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

fn time_perm(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Time,
        action,
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct StartTimerRequest {
    pub ticket_id: Uuid,
    /// Instant-first autosave parameters (IF1): idempotency key. Absent in
    /// legacy clients — purely additive. (Creates carry no version.)
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PauseTimerRequest {
    /// Optional reason, preserved on the segment forever.
    pub reason: Option<String>,
    /// Omit to act on the caller's active timer.
    pub session_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct StopTimerRequest {
    pub reason: Option<String>,
    pub billable: Option<bool>,
    pub notes: Option<String>,
    /// Omit to stop the caller's active timer.
    pub session_id: Option<Uuid>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive. A retried stop
    /// replays the original time entry instead of failing "no active timer".
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Instant-first mutation response (IF2): session fields at the top level
/// (unchanged shape) plus the additive `mutation` metadata.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TimerMutationResponse {
    #[serde(flatten)]
    pub session: TimerSession,
    pub mutation: instant::MutationMeta,
}

/// Same additive envelope for the stop flow (time-entry body).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TimeEntryMutationResponse {
    #[serde(flatten)]
    pub entry: TimeEntry,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTimeEntryRequest {
    pub ticket_id: Uuid,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub ended_at: chrono::DateTime<chrono::Utc>,
    pub billable: bool,
    pub notes: Option<String>,
    pub work_type: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct ListEntriesQuery {
    pub ticket_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    100
}

/// GET /api/timers/active — the caller's running session (at most one).
#[utoipa::path(operation_id = "timers_active", get, path = "/api/timers/active", tag = "timers", security(("bearer" = [])),
    responses((status = 200, body = Object)))]
pub async fn active(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Option<TimerSession>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(Action::View))?;
    let session = core_db::timers::active_session(&state.pool, authed.principal.user.id).await?;
    Ok(Json(session))
}

/// GET /api/timers/working-set — recent sessions (running first): the
/// docked working set the UI renders.
#[utoipa::path(operation_id = "timers_working_set", get, path = "/api/timers/working-set", tag = "timers", security(("bearer" = [])),
    responses((status = 200, body = [core_domain::entities::TimerSession])))]
pub async fn working_set(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<TimerSession>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(Action::View))?;
    let sessions =
        core_db::timers::recent_sessions(&state.pool, authed.principal.user.id, 10).await?;
    Ok(Json(sessions))
}

/// POST /api/timers/start — starts (or switches to) a timer. If another
/// session is running it is auto-paused with reason `switched` and stays in
/// the working set; the DB's partial unique index is the backstop.
/// Instant-first (IF1/IF2): idempotency-keyed and read-your-write.
#[utoipa::path(operation_id = "timers_start", post, path = "/api/timers/start", tag = "timers", security(("bearer" = [])),
    request_body = StartTimerRequest,
    responses((status = 200, body = TimerMutationResponse),
              (status = 404, description = "ticket not visible in scope (rls_sparse)"),
              (status = 409, description = "idempotency key reused")))]
pub async fn start(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<StartTimerRequest>,
) -> ApiResult<Json<TimerMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let now = chrono::Utc::now();
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "timer_session",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // Ticket existence check under RLS (also validates org membership).
        let _ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, req.ticket_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;

        let mut switched_from = None;
        if let Some(mut current) = core_db::timers::active_session(&mut *ctx.tx, user_id).await? {
            switched_from = Some(current.id);
            // Auto-pause with Switched semantics; elapsed is preserved.
            let _ = core_domain::entities::TimerSession::start_switch(
                &mut current,
                org_id,
                user_id,
                req.ticket_id,
                now,
            );
            core_db::timers::update_session(&mut *ctx.tx, &current).await?;
            ctx.emit(
                types::TIMER_SWITCHED,
                "timer",
                Some(current.id.to_string().as_str()),
                &serde_json::json!({ "session_id": current.id, "user_id": user_id }),
            )
            .await?;
        }

        let session = TimerSession::start(org_id, user_id, req.ticket_id, now);
        core_db::timers::insert_session(&mut *ctx.tx, &session).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "timer_session",
                entity_id: session.id,
                version: None,
            },
        )
        .await?;
        ctx.emit(
            types::TIMER_STARTED,
            "timer",
            Some(session.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TimerStarted {
                session_id: session.id,
                user_id,
                ticket_id: req.ticket_id,
                started_at: session.started_at,
                switched_from,
            })
            .unwrap_or_default(),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "timer_session",
                entity_id: session.id,
                op: "timer_start",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&session).unwrap_or(Value::Null),
                updated_at: session.started_at,
                key: key.as_deref(),
                payload: &session,
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TimerMutationResponse { session, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/timers/pause — pauses the active (or given) session, recording
/// the reason on the segment.
#[utoipa::path(operation_id = "timers_pause", post, path = "/api/timers/pause", tag = "timers", security(("bearer" = [])),
    request_body = PauseTimerRequest,
    responses((status = 200, body = core_domain::entities::TimerSession)))]
pub async fn pause(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PauseTimerRequest>,
) -> ApiResult<Json<TimerSession>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(Action::Create))?;
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let now = chrono::Utc::now();

    let session = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let mut session = match req.session_id {
            Some(id) => {
                let s = core_db::timers::get_session(&mut *ctx.tx, id).await?;
                if s.user_id != user_id {
                    return Err(ApiError::Forbidden("not your timer".into()));
                }
                s
            }
            None => core_db::timers::active_session(&mut *ctx.tx, user_id)
                .await?
                .ok_or_else(|| ApiError::Validation("no active timer".into()))?,
        };
        let elapsed = session.total_elapsed_secs(now);
        session.pause(now, req.reason.clone())?;
        core_db::timers::update_session(&mut *ctx.tx, &session).await?;
        ctx.emit(
            types::TIMER_PAUSED,
            "timer",
            Some(session.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TimerPaused {
                session_id: session.id,
                user_id,
                ticket_id: session.ticket_id,
                reason: req.reason.clone(),
                elapsed_secs: elapsed,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, session))
    })
    .await?;

    Ok(Json(session))
}

/// POST /api/timers/resume — resumes a paused session with a new segment.
#[utoipa::path(operation_id = "timers_resume", post, path = "/api/timers/resume", tag = "timers", security(("bearer" = [])),
    request_body = PauseTimerRequest,
    responses((status = 200, body = core_domain::entities::TimerSession)))]
pub async fn resume(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PauseTimerRequest>,
) -> ApiResult<Json<TimerSession>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(Action::Create))?;
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let now = chrono::Utc::now();

    let session = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        // Single-active invariant: resuming requires no other running timer.
        if let Some(active) = core_db::timers::active_session(&mut *ctx.tx, user_id).await? {
            if Some(active.id) != req.session_id {
                return Err(ApiError::Validation(
                    "another timer is running — switch or stop it first".into(),
                ));
            }
        }
        let session_id = req
            .session_id
            .ok_or_else(|| ApiError::Validation("session_id is required to resume".into()))?;
        let mut session = core_db::timers::get_session(&mut *ctx.tx, session_id).await?;
        if session.user_id != user_id {
            return Err(ApiError::Forbidden("not your timer".into()));
        }
        session.resume(now)?;
        core_db::timers::update_session(&mut *ctx.tx, &session).await?;
        ctx.emit(
            types::TIMER_RESUMED,
            "timer",
            Some(session.id.to_string().as_str()),
            &serde_json::json!({ "session_id": session.id, "user_id": user_id }),
        )
        .await?;
        Ok((ctx, session))
    })
    .await?;

    Ok(Json(session))
}

/// POST /api/timers/stop — stops the session and materializes a time entry
/// (rounded up to the minute) in the same transaction.
#[utoipa::path(operation_id = "timers_stop", post, path = "/api/timers/stop", tag = "timers", security(("bearer" = [])),
    request_body = StopTimerRequest,
    responses((status = 200, body = TimeEntryMutationResponse),
              (status = 404, description = "session not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused")))]
pub async fn stop(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<StopTimerRequest>,
) -> ApiResult<Json<TimeEntryMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(Action::Create))?;
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let now = chrono::Utc::now();
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "timer_session",
                entity_id: req.session_id,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before_session = match req.session_id {
            Some(id) => {
                let s = core_db::timers::get_session(&mut *ctx.tx, id)
                    .await
                    .map_err(|e| instant::mutation_scope_err(e.into()))?;
                if s.user_id != user_id {
                    return Err(ApiError::Forbidden("not your timer".into()));
                }
                s
            }
            None => core_db::timers::active_session(&mut *ctx.tx, user_id)
                .await?
                .ok_or_else(|| ApiError::Validation("no active timer".into()))?,
        };
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "timer_session",
                entity_id: before_session.id,
                version: req.instant.version,
            },
        )
        .await?;
        let mut session = before_session.clone();
        let total = session.total_elapsed_secs(now);
        session.stop(now, req.reason.clone())?;
        core_db::timers::update_session(&mut *ctx.tx, &session).await?;

        let entry = TimeEntry::from_session(
            &session,
            now,
            req.notes.clone(),
            req.billable.unwrap_or(true),
        )?;
        core_db::timers::insert_time_entry(&mut *ctx.tx, &entry).await?;
        ctx.emit(
            types::TIMER_STOPPED,
            "timer",
            Some(session.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TimerStopped {
                session_id: session.id,
                user_id,
                ticket_id: session.ticket_id,
                total_secs: total,
                time_entry_id: Some(entry.id),
            })
            .unwrap_or_default(),
        )
        .await?;
        ctx.emit(
            types::TIME_ENTRY_CREATED,
            "time_entry",
            Some(entry.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TimeEntryCreated {
                entry: entry.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "timer_session",
                entity_id: session.id,
                op: "timer_stop",
                seq: write_seq,
                before: Some(serde_json::to_value(&before_session).unwrap_or(Value::Null)),
                after: serde_json::to_value(&session).unwrap_or(Value::Null),
                updated_at: entry.ended_at,
                key: key.as_deref(),
                payload: &entry,
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TimeEntryMutationResponse { entry, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// GET /api/time-entries
#[utoipa::path(operation_id = "timers_list_entries", get, path = "/api/time-entries", tag = "timers", security(("bearer" = [])),
    params(ListEntriesQuery),
    responses((status = 200, body = [core_domain::entities::TimeEntry])))]
pub async fn list_entries(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ListEntriesQuery>,
) -> ApiResult<Json<Vec<TimeEntry>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(Action::View))?;
    // Non-admins only see their own entries.
    let user_id = if authed.principal.is_owner_or_admin() {
        q.user_id
    } else {
        Some(authed.principal.user.id)
    };
    let entries = core_db::timers::list_time_entries(
        &state.pool,
        authed.principal.user.org_id,
        q.ticket_id,
        user_id,
        q.since,
        q.limit.clamp(1, 500),
    )
    .await?;
    Ok(Json(entries))
}

/// POST /api/time-entries — manual entry (no timer).
#[utoipa::path(operation_id = "timers_create_entry", post, path = "/api/time-entries", tag = "timers", security(("bearer" = [])),
    request_body = CreateTimeEntryRequest,
    responses((status = 200, body = core_domain::entities::TimeEntry)))]
pub async fn create_entry(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateTimeEntryRequest>,
) -> ApiResult<Json<TimeEntry>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(Action::Create))?;
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;

    if req.ended_at <= req.started_at {
        return Err(ApiError::Validation(
            "ended_at must be after started_at".into(),
        ));
    }
    let minutes = (req.ended_at - req.started_at).num_minutes();
    if minutes < 0 {
        return Err(ApiError::Validation("duration is negative".into()));
    }

    let entry = TimeEntry {
        id: Uuid::now_v7(),
        org_id,
        ticket_id: req.ticket_id,
        user_id,
        started_at: req.started_at,
        ended_at: req.ended_at,
        minutes,
        billable: req.billable,
        notes: req.notes,
        work_type: req.work_type,
        timer_session_id: None,
        created_at: chrono::Utc::now(),
    };

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| {
        let entry = entry.clone();
        async move {
            // Validate the ticket exists in this org.
            let _ = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, req.ticket_id).await?;
            core_db::timers::insert_time_entry(&mut *ctx.tx, &entry).await?;
            ctx.emit(
                types::TIME_ENTRY_CREATED,
                "time_entry",
                Some(entry.id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::TimeEntryCreated {
                    entry: entry.clone(),
                })
                .unwrap_or_default(),
            )
            .await?;
            Ok((ctx, ()))
        }
    })
    .await?;

    Ok(Json(entry))
}
