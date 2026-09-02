//! Staff-side appointment-request decisions (Portal v2, TASKLIST Phase
//! 3): list, approve, decline.
//!
//! Purpose: the OTHER half of `routes::portal_v2`'s request flow. Portal
//! requests are intents against published availability; the staff
//! approval is the SINGLE point that books a real calendar row:
//! availability containment + booking/on-call conflicts are re-validated
//! at decision time against the live calendar, then the request row is
//! moved to `approved` and linked to the created appointment — all in
//! ONE transaction, so a slot can never be booked twice by two racing
//! approvals (the appointments EXCLUDE constraint is the concurrency
//! backstop; the `WHERE status = 'pending'` decision predicate is the
//! single-winner gate).
//!
//! Instant-first contract (IF1–IF3): approve/decline are idempotency-
//! keyed with read-your-write `mutation` metadata and land in the write
//! trail under the acting staff user.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use chrono::Utc;
use core_domain::entities::{
    Appointment, AppointmentRequest, AppointmentRequestStatus, AppointmentStatus,
};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

fn sched(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Scheduling,
        action,
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct AppointmentRequestListQuery {
    /// Exact status: `pending | approved | declined | cancelled`.
    pub status: Option<core_domain::entities::AppointmentRequestStatus>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

/// GET /api/appointment-requests — the staff queue (optionally narrowed
/// by status), newest first.
#[utoipa::path(operation_id = "appointment_requests_list", get, path = "/api/appointment-requests", tag = "scheduling", security(("bearer" = [])),
    params(AppointmentRequestListQuery),
    responses((status = 200, body = [core_domain::entities::AppointmentRequest])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<AppointmentRequestListQuery>,
) -> ApiResult<Json<Vec<AppointmentRequest>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let rows = core_db::portal::list_appointment_requests(
        &state.pool,
        authed.principal.user.org_id,
        q.status.map(|s| s.as_str()),
        q.limit.clamp(1, 200),
        q.offset.max(0),
    )
    .await?;
    Ok(Json(rows))
}

/// A staff decision body. `decision_note` is the (optional) reason shown
/// to the client on decline and a confirmation note on approve.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct DecideAppointmentRequest {
    #[serde(default)]
    pub decision_note: Option<String>,
    /// Instant-first autosave parameters (IF1): idempotency key + If-Match
    /// `version` on the request.
    #[serde(flatten, default)]
    pub instant: crate::instant::MutationParams,
}

/// Instant-first mutation response (IF2) for a decision. `appointment`
/// is present ONLY on approve: the calendar booking this decision made.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DecideResponse {
    pub request: AppointmentRequest,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appointment: Option<Appointment>,
    pub mutation: instant::MutationMeta,
}

/// POST /api/appointment-requests/{id}/approve — book the requested slot
/// on the staff member's calendar. Re-validates availability + conflicts
/// at decision time (the client's listing may be stale); a slot that
/// stopped being free is a typed 409, never a wedged booking. Emits BOTH
/// `appointment.created` (the booking enters the world through the
/// normal creation event) and `appointment_request.approved`.
#[utoipa::path(operation_id = "appointment_requests_approve", post, path = "/api/appointment-requests/{id}/approve", tag = "scheduling", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = DecideAppointmentRequest,
    responses((status = 200, body = DecideResponse),
              (status = 404, description = "request not found in scope"),
              (status = 409, description = "not pending, slot no longer free, double-booking backstop, version conflict, or idempotency key reused")))]
pub async fn approve(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<DecideAppointmentRequest>,
) -> ApiResult<Json<DecideResponse>> {
    decide(
        state,
        extensions,
        id,
        req,
        AppointmentRequestStatus::Approved,
        "approve",
    )
    .await
}

/// POST /api/appointment-requests/{id}/decline — refuse a pending
/// request. No calendar effect; the note reaches the client.
#[utoipa::path(operation_id = "appointment_requests_decline", post, path = "/api/appointment-requests/{id}/decline", tag = "scheduling", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = DecideAppointmentRequest,
    responses((status = 200, body = DecideResponse),
              (status = 404, description = "request not found in scope"),
              (status = 409, description = "not pending, version conflict, or idempotency key reused")))]
pub async fn decline(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<DecideAppointmentRequest>,
) -> ApiResult<Json<DecideResponse>> {
    decide(
        state,
        extensions,
        id,
        req,
        AppointmentRequestStatus::Declined,
        "decline",
    )
    .await
}

/// Shared decision path: claim → load → pending gate → (approve only)
/// re-validate slot + create the appointment → decide → emit → audit →
/// write trail, all in ONE transaction.
async fn decide(
    state: AppState,
    extensions: axum::http::Extensions,
    id: Uuid,
    req: DecideAppointmentRequest,
    to_status: AppointmentRequestStatus,
    op: &'static str,
) -> ApiResult<Json<DecideResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "appointment_request",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let request = core_db::portal::get_appointment_request(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if request.status != AppointmentRequestStatus::Pending {
            return Err(ApiError::Conflict(format!(
                "request is {} and can no longer be decided",
                request.status.as_str()
            )));
        }

        let mut appointment: Option<Appointment> = None;
        if to_status == AppointmentRequestStatus::Approved {
            // Re-validate the slot at decision time against the LIVE
            // calendar (windows minus time-off minus bookings). A stale
            // listing can never wedge a booking in.
            crate::routes::portal_v2::ensure_slot_available(
                &mut ctx,
                org_id,
                request.user_id,
                request.starts_at,
                request.ends_at,
            )
            .await
            .map_err(|e| match e {
                // A stale listing is a STATE conflict for the staff
                // caller, not their input error.
                ApiError::Validation(m) => ApiError::Conflict(m),
                other => other,
            })?;
            let booking = Appointment {
                id: Uuid::now_v7(),
                org_id,
                user_id: request.user_id,
                ticket_id: None,
                client_id: Some(request.client_id),
                site_id: None,
                title: request.title.clone(),
                starts_at: request.starts_at,
                ends_at: request.ends_at,
                kind: request.kind,
                status: AppointmentStatus::Scheduled,
                travel_minutes: 0,
                notes: request.notes.clone(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            core_db::scheduling::create_appointment(&mut *ctx.tx, &booking).await?;
            ctx.emit(
                types::APPOINTMENT_CREATED,
                "appointment",
                Some(booking.id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::AppointmentCreated {
                    appointment: booking.clone(),
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;
            appointment = Some(booking);
        }

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "appointment_request",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let decided = core_db::portal::decide_appointment_request(
            &mut *ctx.tx,
            org_id,
            id,
            core_db::portal::AppointmentRequestDecision {
                status: to_status.as_str(),
                decision_note: req.decision_note.as_deref(),
                decided_by: user_id,
                appointment_id: appointment.as_ref().map(|a| a.id),
            },
        )
        .await?
        .ok_or_else(|| ApiError::Conflict("request left its pending state; retry".into()))?;

        match to_status {
            AppointmentRequestStatus::Approved => {
                ctx.emit(
                    types::APPOINTMENT_REQUEST_APPROVED,
                    "appointment_request",
                    Some(id.to_string().as_str()),
                    &serde_json::to_value(core_domain::events::AppointmentRequestApproved {
                        request_id: id,
                        appointment_id: appointment.as_ref().map(|a| a.id).unwrap_or_default(),
                        client_id: request.client_id,
                        decided_by: user_id,
                        decided_at: Utc::now(),
                    })
                    .unwrap_or_else(|_| serde_json::json!({})),
                )
                .await?
            }
            AppointmentRequestStatus::Declined => {
                ctx.emit(
                    types::APPOINTMENT_REQUEST_DECLINED,
                    "appointment_request",
                    Some(id.to_string().as_str()),
                    &serde_json::to_value(core_domain::events::AppointmentRequestDeclined {
                        request_id: id,
                        client_id: request.client_id,
                        decided_by: user_id,
                        note: req.decision_note.clone(),
                        decided_at: Utc::now(),
                    })
                    .unwrap_or_else(|_| serde_json::json!({})),
                )
                .await?
            }
            // decide() is only ever called with these two; the statuses
            // below would write nonsense history.
            _ => unreachable!("decide handles approve/decline only"),
        }
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: format!("appointment_request.{op}"),
                entity_type: Some("appointment_request".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "client_id": request.client_id,
                    "staff_user_id": request.user_id,
                    "starts_at": request.starts_at,
                    "ends_at": request.ends_at,
                    "note": req.decision_note,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let meta = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "appointment_request",
                entity_id: id,
                op,
                seq: write_seq,
                before: Some(serde_json::to_value(&request).unwrap_or(serde_json::Value::Null)),
                after: serde_json::to_value(&decided).unwrap_or(serde_json::Value::Null),
                updated_at: decided.updated_at,
                key: key.as_deref(),
                payload: &DecideStoredPayload {
                    request: decided.clone(),
                    appointment: appointment.clone(),
                },
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(DecideResponse {
                request: decided,
                appointment,
                mutation: meta,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// The stored decision payload (mutation metadata merges in on record).
#[derive(Debug, Serialize, Deserialize)]
struct DecideStoredPayload {
    request: AppointmentRequest,
    #[serde(skip_serializing_if = "Option::is_none")]
    appointment: Option<Appointment>,
}
