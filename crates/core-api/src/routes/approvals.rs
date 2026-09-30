//! Time-entry approval workflow endpoints (Wave 14 lane 1): submit →
//! review → approve/reject with typed reason codes, bulk decisions, and
//! per-approver queue/history reads. Entries consumed by an invoice are
//! locked — every mutation refuses them with the typed `time_entry_locked`
//! reason.
//!
//! RB2: the queue and decisions carry no rates or costs, so they gate on
//! the time module + `special:approve_time`; anything financial keeps the
//! existing `financial:read` gating on the billing surfaces.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant;
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use core_domain::entities::{ApprovalStatus, TimeEntry, TimeRejectionReason};
use core_domain::events::types;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

fn time_perm(action: core_domain::policy::Action) -> core_domain::policy::Permission {
    core_domain::policy::Permission::Matrix {
        module: core_domain::policy::Module::Time,
        action,
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SubmitEntryRequest {
    /// Optional free-text context recorded on the decision trail.
    pub note: Option<String>,
}

/// Instant-first mutation response (IF2): the full entry snapshot
/// (including its new approval state) plus the decision trail entry.
#[derive(Debug, Serialize, ToSchema)]
pub struct SubmitResponse {
    #[serde(flatten)]
    pub entry: TimeEntry,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BulkDecisionRequest {
    pub entry_ids: Vec<Uuid>,
    /// `approve` or `reject`. Rejections require a typed `reason_code`.
    pub decision: String,
    /// Typed rejection vocabulary (`missing_notes`, `wrong_ticket`, ...).
    pub reason_code: Option<String>,
    pub note: Option<String>,
}

/// Per-item outcome of a bulk decision: applied entries carry the new
/// snapshot; refused entries name exactly why (not-in-queue, locked, or
/// not visible in scope), so the client flags rows instead of guessing.
#[derive(Debug, Serialize, ToSchema)]
pub struct BulkDecisionItem {
    pub entry_id: Uuid,
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<TimeEntry>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BulkDecisionResponse {
    pub results: Vec<BulkDecisionItem>,
}

/// POST /api/time-entries/:id/fix — an approver applies one verdict's fix
/// to an entry that is still in the queue. At least one of `description` /
/// `minutes` is required; the entry stays `submitted` afterwards, because
/// clearing the objection is not approving it.
#[derive(Debug, Deserialize, ToSchema)]
pub struct FixEntryRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    /// The description to write. The server will only take text the approver
    /// saw: the factual suggestion it offered for this entry.
    pub description: Option<String>,
    /// The entry's NEW total minutes, not a delta. Only ever lower than what
    /// the entry holds — an approver tidies a double-claim, and raising a
    /// billable value is not theirs to do from a queue.
    pub minutes: Option<i32>,
    /// Free-text context recorded on the decision trail.
    pub note: Option<String>,
}

/// Instant-first mutation response (IF2): the entry as it now reads, plus the
/// undo substrate. The client reconciles from the snapshot and never has to
/// guess what the server did to the minutes.
#[derive(Debug, Serialize, ToSchema)]
pub struct FixEntryResponse {
    #[serde(flatten)]
    pub entry: TimeEntry,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct QueueQuery {
    /// Filter by approval state; defaults to `submitted` (the queue).
    pub status: Option<String>,
    /// Narrow to one submitter.
    pub user_id: Option<Uuid>,
    /// Per-approver decision history: entries this approver decided.
    pub decided_by: Option<Uuid>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    100
}

fn parse_reason(code: Option<&str>) -> ApiResult<Option<TimeRejectionReason>> {
    match code {
        None => Ok(None),
        Some(c) => TimeRejectionReason::parse(c).map(Some).ok_or_else(|| {
            ApiError::Validation(format!(
                "unknown rejection reason_code '{c}'; expected one of: {}",
                TimeRejectionReason::ALL
                    .iter()
                    .map(|r| r.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }),
    }
}

/// POST /api/time-entries/:id/submit — the entry owner (or an admin)
/// pushes a draft/rejected entry into the approver queue.
#[utoipa::path(operation_id = "time_entries_submit", post, path = "/api/time-entries/{id}/submit", tag = "timers", security(("bearer" = [])),
    request_body = SubmitEntryRequest,
    responses((status = 200, body = SubmitResponse),
              (status = 404, description = "entry not visible in scope (rls_sparse)"),
              (status = 409, description = "entry locked by invoicing (time_entry_locked)"),
              (status = 422, description = "entry not in a submittable state")))]
pub async fn submit(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(entry_id): Path<Uuid>,
    Json(req): Json<SubmitEntryRequest>,
) -> ApiResult<Json<SubmitResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        time_perm(core_domain::policy::Action::Edit),
    )?;
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let now = chrono::Utc::now();

    let entry = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| {
        async move {
            let existing = core_db::timers::get_time_entry(&mut *ctx.tx, org_id, entry_id)
                .await
                .map_err(|e| instant::mutation_scope_err(e.into()))?;
            // Only the entry's owner or an owner/admin may submit it.
            if existing.user_id != user_id && !authed.principal.is_owner_or_admin() {
                return Err(ApiError::Forbidden("not your time entry".into()));
            }
            if existing.approval_status == Some(ApprovalStatus::Submitted) {
                return Err(ApiError::Validation(
                    "entry is already submitted for approval".into(),
                ));
            }
            let entry = match core_db::approvals::submit(
                &mut ctx.tx,
                org_id,
                entry_id,
                user_id,
                req.note.as_deref(),
                now,
            )
            .await
            {
                Ok(e) => e,
                Err(core_db::DbError::NotFound) => {
                    // Distinguish the two refusal modes: locked vs state.
                    let (_id, status, locked) =
                        core_db::approvals::classify(&mut *ctx.tx, org_id, &[entry_id])
                            .await?
                            .into_iter()
                            .next()
                            .ok_or(ApiError::RlsSparse)?;
                    if locked {
                        return Err(ApiError::TimeEntryLocked { entry_id });
                    }
                    return Err(ApiError::Validation(format!(
                        "entry in state {} cannot be submitted",
                        status.map(|s| s.as_str()).unwrap_or("draft")
                    )));
                }
                Err(e) => return Err(e.into()),
            };
            ctx.emit(
                types::TIME_ENTRY_APPROVAL_SUBMITTED,
                "time_entry",
                Some(entry_id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::TimeEntryApprovalSubmitted {
                    entry_id,
                    ticket_id: entry.ticket_id,
                    entry_user_id: entry.user_id,
                    submitted_by: user_id,
                    submitted_at: now,
                })
                .unwrap_or_default(),
            )
            .await?;
            Ok((ctx, entry))
        }
    })
    .await?;

    Ok(Json(SubmitResponse { entry }))
}

/// POST /api/time-entries/approvals/decide — bulk approve or bulk reject
/// (`special:approve_time`). Only entries currently in the queue apply;
/// every other id comes back with a per-item outcome. Emits one
/// `time_entry.approval_*` event per applied entry.
#[utoipa::path(operation_id = "time_entries_decide", post, path = "/api/time-entries/approvals/decide", tag = "timers", security(("bearer" = [])),
    request_body = BulkDecisionRequest,
    responses((status = 200, body = BulkDecisionResponse),
              (status = 422, description = "unknown rejection reason_code")))]
pub async fn decide(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<BulkDecisionRequest>,
) -> ApiResult<Json<BulkDecisionResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        core_domain::policy::Permission::ApproveTime,
    )?;
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let now = chrono::Utc::now();
    let approve = match req.decision.as_str() {
        "approve" => true,
        "reject" => false,
        other => {
            return Err(ApiError::Validation(format!(
                "decision must be 'approve' or 'reject', got '{other}'"
            )))
        }
    };
    if approve && req.reason_code.is_some() {
        return Err(ApiError::Validation(
            "reason_code only applies to rejections".into(),
        ));
    }
    if !approve && req.reason_code.is_none() {
        return Err(ApiError::Validation(
            "a rejection requires a reason_code".into(),
        ));
    }
    let reason = parse_reason(req.reason_code.as_deref())?;

    let response = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| {
        let entry_ids = req.entry_ids.clone();
        let note = req.note.clone();
        async move {
            let reason_code_str = reason.map(|r| r.as_str().to_string());
            let applied = core_db::approvals::decide(
                &mut ctx.tx,
                org_id,
                &entry_ids,
                approve,
                user_id,
                reason_code_str.as_deref(),
                note.as_deref(),
                now,
            )
            .await?;
            let applied_ids: std::collections::HashSet<Uuid> =
                applied.iter().map(|e| e.id).collect();
            for entry in &applied {
                if approve {
                    ctx.emit(
                        types::TIME_ENTRY_APPROVAL_APPROVED,
                        "time_entry",
                        Some(entry.id.to_string().as_str()),
                        &serde_json::to_value(core_domain::events::TimeEntryApprovalApproved {
                            entry_id: entry.id,
                            approved_by: user_id,
                            decided_at: now,
                        })
                        .unwrap_or_default(),
                    )
                    .await?;
                } else {
                    ctx.emit(
                        types::TIME_ENTRY_APPROVAL_REJECTED,
                        "time_entry",
                        Some(entry.id.to_string().as_str()),
                        &serde_json::to_value(core_domain::events::TimeEntryApprovalRejected {
                            entry_id: entry.id,
                            rejected_by: user_id,
                            reason_code: reason_code_str.clone().unwrap_or_default(),
                            note: note.clone(),
                            decided_at: now,
                        })
                        .unwrap_or_default(),
                    )
                    .await?;
                }
            }
            // Classify the remainder so every id gets a typed outcome.
            let mut results: Vec<BulkDecisionItem> = applied
                .into_iter()
                .map(|entry| BulkDecisionItem {
                    entry_id: entry.id,
                    outcome: "applied".into(),
                    entry: Some(entry),
                })
                .collect();
            let rest: Vec<Uuid> = entry_ids
                .iter()
                .copied()
                .filter(|id| !applied_ids.contains(id))
                .collect();
            for (id, _status, locked) in
                core_db::approvals::classify(&mut *ctx.tx, org_id, &rest).await?
            {
                results.push(BulkDecisionItem {
                    entry_id: id,
                    outcome: if locked {
                        "locked".into()
                    } else {
                        "not_in_queue".into()
                    },
                    entry: None,
                });
            }
            for id in &rest {
                if !results.iter().any(|r| r.entry_id == *id) {
                    results.push(BulkDecisionItem {
                        entry_id: *id,
                        outcome: "not_found".into(),
                        entry: None,
                    });
                }
            }
            Ok((ctx, BulkDecisionResponse { results }))
        }
    })
    .await?;

    Ok(Json(response))
}

/// GET /api/time-entries/approvals/queue — the approver queue (submitted
/// entries by default). Approvers (`special:approve_time`) see the org
/// queue and can filter `decided_by` for their own decision history;
/// everyone else sees only their own submissions.
#[utoipa::path(operation_id = "time_entries_queue", get, path = "/api/time-entries/approvals/queue", tag = "timers", security(("bearer" = [])),
    params(QueueQuery),
    responses((status = 200, body = [core_domain::entities::TimeEntryQueueItem])))]
pub async fn queue(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<QueueQuery>,
) -> ApiResult<Json<Vec<core_domain::entities::TimeEntryQueueItem>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let is_approver = core_domain::policy::check(
        &authed.principal.permissions,
        core_domain::policy::Permission::ApproveTime,
    )
    .expect()
    .is_ok();
    if !is_approver {
        // Non-approvers: their own submissions only.
        if q.user_id.is_some_and(|u| u != authed.principal.user.id) {
            return Err(ApiError::Forbidden("not your time entries".into()));
        }
    }
    let status = match q.status.as_deref() {
        None | Some("submitted") => Some(ApprovalStatus::Submitted),
        Some(s) => Some(
            ApprovalStatus::parse(s)
                .ok_or_else(|| ApiError::Validation(format!("unknown approval_status '{s}'")))?,
        ),
    };
    let user_filter = if is_approver {
        q.user_id
    } else {
        Some(authed.principal.user.id)
    };
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let items = core_db::approvals::queue(
        &mut *tx,
        authed.principal.user.org_id,
        status,
        user_filter,
        q.decided_by,
        q.limit.clamp(1, 500),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(items))
}

/// GET /api/time-entries/:id/approvals — the append-only decision trail
/// (`seq` ascending). Approvers and owner/admins read any trail; other
/// users only their own entries'.
#[utoipa::path(operation_id = "time_entries_approval_history", get, path = "/api/time-entries/{id}/approvals", tag = "timers", security(("bearer" = [])),
    responses((status = 200, body = [core_domain::entities::TimeEntryApprovalRecord]),
              (status = 404, description = "entry not visible in scope (not_found)")))]
pub async fn history(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(entry_id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::TimeEntryApprovalRecord>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        time_perm(core_domain::policy::Action::View),
    )?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let entry = core_db::timers::get_time_entry(&mut *tx, org_id, entry_id).await?;
    let is_approver = core_domain::policy::check(
        &authed.principal.permissions,
        core_domain::policy::Permission::ApproveTime,
    )
    .expect()
    .is_ok();
    if !is_approver
        && entry.user_id != authed.principal.user.id
        && !authed.principal.is_owner_or_admin()
    {
        return Err(ApiError::Forbidden("not your time entry".into()));
    }
    let records = core_db::approvals::history(&mut *tx, org_id, entry_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(records))
}

/// POST /api/time-entries/:id/reopen — move a rejected entry back to draft
/// (owner or admin) so it can be corrected and resubmitted.
#[utoipa::path(operation_id = "time_entries_reopen", post, path = "/api/time-entries/{id}/reopen", tag = "timers", security(("bearer" = [])),
    responses((status = 200, body = SubmitResponse),
              (status = 409, description = "entry locked by invoicing (time_entry_locked)"),
              (status = 422, description = "only rejected entries can be reopened")))]
pub async fn reopen_entry(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(entry_id): Path<Uuid>,
) -> ApiResult<Json<SubmitResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        time_perm(core_domain::policy::Action::Edit),
    )?;
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let now = chrono::Utc::now();

    let entry = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let existing = core_db::timers::get_time_entry(&mut *ctx.tx, org_id, entry_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if existing.user_id != user_id && !authed.principal.is_owner_or_admin() {
            return Err(ApiError::Forbidden("not your time entry".into()));
        }
        let entry =
            match core_db::approvals::reopen(&mut ctx.tx, org_id, entry_id, user_id, now).await {
                Ok(e) => e,
                Err(core_db::DbError::NotFound) => {
                    let (_id, _status, locked) =
                        core_db::approvals::classify(&mut *ctx.tx, org_id, &[entry_id])
                            .await?
                            .into_iter()
                            .next()
                            .ok_or(ApiError::RlsSparse)?;
                    if locked {
                        return Err(ApiError::TimeEntryLocked { entry_id });
                    }
                    return Err(ApiError::Validation(
                        "only rejected entries can be reopened".into(),
                    ));
                }
                Err(e) => return Err(e.into()),
            };
        Ok((ctx, entry))
    })
    .await?;
    Ok(Json(SubmitResponse { entry }))
}

/// POST /api/time-entries/:id/fix — apply one verdict's fix to a queued
/// entry (`special:approve_time`).
///
/// This is the server side of the queue's one-click fix: adopt the factual
/// description the server assembled from the ticket, or cut the minutes that
/// another entry of the same person already claimed. The entry stays in the
/// queue afterwards — the approver still presses Approve, because a fix clears
/// an objection and it does not sign anybody's time off.
#[utoipa::path(operation_id = "time_entry_fix", post, path = "/api/time-entries/{id}/fix", tag = "timers", security(("bearer" = [])),
    request_body = FixEntryRequest,
    responses((status = 200, body = FixEntryResponse),
              (status = 404, description = "entry not visible in scope (rls_sparse)"),
              (status = 409, description = "entry locked by invoicing (time_entry_locked)"),
              (status = 422, description = "entry not in the queue, or the fix is not allowed")))]
pub async fn fix_entry(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(entry_id): Path<Uuid>,
    Json(req): Json<FixEntryRequest>,
) -> ApiResult<Json<FixEntryResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        core_domain::policy::Permission::ApproveTime,
    )?;
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let now = chrono::Utc::now();

    let description = req.description.as_deref().map(str::trim).filter(|d| !d.is_empty());
    let minutes = req.minutes;
    if description.is_none() && minutes.is_none() {
        return Err(ApiError::Validation(
            "a fix changes the description, the minutes, or both".into(),
        ));
    }
    if let Some(minutes) = minutes {
        if minutes <= 0 {
            return Err(ApiError::Validation(
                "an entry's minutes cannot be trimmed to zero — reject it with a reason instead".into(),
            ));
        }
        if description.is_none() && minutes as i64 > 0 && minutes as i64 > 24 * 60 * 7 {
            return Err(ApiError::Validation(
                "that is not a trim; an approver lowers an entry's minutes, never rewrites them"
                    .into(),
            ));
        }
    }

    let note = req.note.clone();
    let before = req.instant.clone();
    let response = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let existing = core_db::timers::get_time_entry(&mut ctx.tx, org_id, entry_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if let Some(minutes) = minutes {
            if minutes as i64 >= existing.minutes {
                return Err(ApiError::Validation(format!(
                    "{} already holds {} minutes; a fix only ever lowers them",
                    existing.minutes, minutes
                )));
            }
        }
        // The trail note says exactly what changed, so the decision history
        // answers "what did they change, and why" without a second read.
        let mut change: Vec<String> = Vec::new();
        if let Some(next) = description {
            change.push(format!("description: \"{}\"", next));
        }
        if let Some(next) = minutes {
            change.push(format!("minutes: {} → {}", existing.minutes, next));
        }
        let trail_note = match note {
            Some(n) if !n.trim().is_empty() => format!("{} — {}", change.join(", "), n.trim()),
            _ => change.join(", "),
        };
        let entry = match core_db::approvals::fix(
            &mut ctx.tx,
            org_id,
            entry_id,
            user_id,
            description,
            minutes,
            Some(&trail_note),
            now,
        )
        .await
        {
            Ok(e) => e,
            Err(core_db::DbError::NotFound) => {
                let (_id, status, locked) =
                    core_db::approvals::classify(&mut *ctx.tx, org_id, &[entry_id])
                        .await?
                        .into_iter()
                        .next()
                        .ok_or(ApiError::RlsSparse)?;
                if locked {
                    return Err(ApiError::TimeEntryLocked { entry_id });
                }
                return Err(ApiError::Validation(format!(
                    "a fix applies to an entry in the queue; this one is {}",
                    status.map(|s| s.as_str()).unwrap_or("draft")
                )));
            }
            Err(e) => return Err(e.into()),
        };
        ctx.emit(
            types::TIME_ENTRY_FIXED,
            "time_entry",
            Some(entry_id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TimeEntryFixed {
                entry_id,
                fixed_by: user_id,
                description_changed: description.is_some(),
                minutes: entry.minutes,
                fixed_at: now,
            })
            .unwrap_or_default(),
        )
        .await?;
        let mutation = instant::MutationMeta {
            seq: 0,
            version: 0,
            updated_at: now,
            before: Some(serde_json::to_value(&existing).unwrap_or_default()),
            after: serde_json::to_value(&entry).unwrap_or_default(),
            idempotency_key: before.idempotency_key.clone(),
        };
        Ok((ctx, FixEntryResponse { entry, mutation }))
    })
    .await?;

    Ok(Json(response))
}
