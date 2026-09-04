//! Appointment endpoints (TASKLIST 2.12): create/get/patch/cancel/move
//! plus the conflict dry-run used by booking UIs.
//!
//! Purpose: every mutation validates through core_domain::scheduling,
//! persists via core_db::scheduling inside an org-bound transaction, and
//! emits its domain event in the same commit. Overlap rejection is
//! belt-and-braces: a pre-check names conflicts for humans while the DB
//! EXCLUDE constraint remains the hard guarantee.
//!
//! Non-purpose: no drag-and-drop dispatch board, no travel routing
//! (`travel_minutes` is stored verbatim), no reminders/recurrence
//! expansion (worker lane, deferred). Calendar views live in
//! routes/calendar.rs; availability/on-call/time-off CRUD in
//! routes/availability.rs.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use chrono::{DateTime, Duration, Utc};
use core_domain::entities::{Appointment, AppointmentKind, AppointmentStatus};
use core_domain::events::{types, AppointmentCancelled, AppointmentCreated, AppointmentUpdated};
use core_domain::policy::{Action, Module, Permission};
use core_domain::scheduling::conflict_indices;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Scheduling-module permission shorthand shared by the sibling route files.
pub(crate) fn sched(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Scheduling,
        action,
    }
}

/// Resolve whose calendar a request touches: default self; naming someone
/// else requires owner/admin (manager override).
pub(crate) fn target_user(
    principal: &core_domain::entities::Principal,
    requested: Option<Uuid>,
    what: &str,
) -> ApiResult<Uuid> {
    match requested {
        None => Ok(principal.user.id),
        Some(id) if id == principal.user.id => Ok(id),
        Some(_) if principal.is_owner_or_admin() => Ok(requested.unwrap_or(principal.user.id)),
        Some(_) => Err(ApiError::Forbidden(format!(
            "{what} for another user requires owner or admin"
        ))),
    }
}

pub(crate) fn validate_span(starts_at: DateTime<Utc>, ends_at: DateTime<Utc>) -> ApiResult<()> {
    if ends_at <= starts_at {
        return Err(ApiError::Validation(
            "ends_at must be after starts_at".into(),
        ));
    }
    if ends_at - starts_at > Duration::days(31) {
        return Err(ApiError::Validation("span exceeds 31 days".into()));
    }
    Ok(())
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAppointmentRequest {
    /// Assignee whose calendar holds the slot. Defaults to the caller;
    /// booking for someone else requires owner/admin.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub user_id: Option<Uuid>,
    pub title: String,
    #[schema(value_type = String, format = "date-time")]
    pub starts_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub ends_at: DateTime<Utc>,
    /// Defaults to on_site when omitted.
    pub kind: Option<AppointmentKind>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub client_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub site_id: Option<Uuid>,
    /// Dispatch placeholder, stored verbatim (RoutingProvider comes later).
    #[serde(default)]
    pub travel_minutes: i32,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchAppointmentRequest {
    pub title: Option<String>,
    pub kind: Option<AppointmentKind>,
    pub status: Option<AppointmentStatus>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    pub travel_minutes: Option<i32>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MoveAppointmentRequest {
    #[schema(value_type = String, format = "date-time")]
    pub starts_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub ends_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ConflictCheckRequest {
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    #[schema(value_type = String, format = "date-time")]
    pub starts_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub ends_at: DateTime<Utc>,
    /// Appointment excluded from the check (its own id when rescheduling).
    #[schema(value_type = Option<uuid::Uuid>)]
    pub exclude_appointment_id: Option<Uuid>,
}

/// One blocking booking named by the dry-run.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConflictView {
    #[schema(value_type = uuid::Uuid)]
    pub appointment_id: Uuid,
    pub title: String,
    #[schema(value_type = String, format = "date-time")]
    pub starts_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub ends_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ConflictCheckResponse {
    pub has_conflict: bool,
    pub conflicts: Vec<ConflictView>,
}

/// POST /api/appointments — create a booking (409 on overlap).
#[utoipa::path(operation_id = "appointment_create", post, path = "/api/appointments", tag = "scheduling", security(("bearer" = [])),
    request_body = CreateAppointmentRequest,
    responses((status = 201, body = Appointment), (status = 409, description = "slot already booked")))]
pub async fn create_appointment(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateAppointmentRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<Appointment>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::Create))?;
    let user_id = target_user(&authed.principal, req.user_id, "booking")?;
    validate_span(req.starts_at, req.ends_at)?;
    if req.travel_minutes < 0 {
        return Err(ApiError::Validation("travel_minutes must be >= 0".into()));
    }
    let org_id = authed.principal.user.org_id;
    let appointment = Appointment {
        id: Uuid::now_v7(),
        org_id,
        user_id,
        ticket_id: req.ticket_id,
        client_id: req.client_id,
        site_id: req.site_id,
        title: req.title.clone(),
        starts_at: req.starts_at,
        ends_at: req.ends_at,
        kind: req.kind.unwrap_or(AppointmentKind::OnSite),
        status: AppointmentStatus::Scheduled,
        travel_minutes: req.travel_minutes,
        notes: req.notes.clone(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut tx| async move {
            // Friendly pre-check; the EXCLUDE constraint remains the guarantee.
            let existing = core_db::scheduling::list_appointments_between(
                &mut *tx.tx,
                org_id,
                &[user_id],
                req.starts_at,
                req.ends_at,
            )
            .await?;
            if !conflict_indices(
                req.starts_at,
                req.ends_at,
                existing.iter().map(|a| (a.starts_at, a.ends_at)),
            )
            .is_empty()
            {
                return Err(ApiError::Conflict("slot already booked".into()));
            }
            core_db::scheduling::create_appointment(&mut *tx.tx, &appointment).await?;
            tx.emit(
                types::APPOINTMENT_CREATED,
                "appointment",
                Some(&appointment.id.to_string()),
                &serde_json::to_value(AppointmentCreated {
                    appointment: appointment.clone(),
                })
                .map_err(|e| ApiError::Internal(e.into()))?,
            )
            .await?;
            Ok((tx, appointment))
        },
    )
    .await
    .map(|appt| (axum::http::StatusCode::CREATED, Json(appt)))
}

/// GET /api/appointments/{id} — fetch one booking in the caller's org.
#[utoipa::path(operation_id = "appointment_get", get, path = "/api/appointments/{id}", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = Appointment), (status = 404)))]
pub async fn get_appointment(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Appointment>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let appt =
        core_db::scheduling::get_appointment(&mut *tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(appt))
}

/// PATCH /api/appointments/{id} — edit fields/status (times unchanged;
/// moves use the dedicated endpoint so clients get rescheduled events).
#[utoipa::path(operation_id = "appointment_patch", patch, path = "/api/appointments/{id}", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    request_body = PatchAppointmentRequest,
    responses((status = 200, body = Appointment), (status = 404)))]
pub async fn patch_appointment(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchAppointmentRequest>,
) -> ApiResult<Json<Appointment>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::Edit))?;
    if let Some(m) = req.travel_minutes {
        if m < 0 {
            return Err(ApiError::Validation("travel_minutes must be >= 0".into()));
        }
    }
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let mut appt = core_db::scheduling::get_appointment(&mut *tx.tx, org_id, id).await?;
        if let Some(t) = req.title {
            appt.title = t;
        }
        if let Some(k) = req.kind {
            appt.kind = k;
        }
        if let Some(s) = req.status {
            appt.status = s;
        }
        if let Some(t) = req.ticket_id {
            appt.ticket_id = Some(t);
        }
        if let Some(m) = req.travel_minutes {
            appt.travel_minutes = m;
        }
        if let Some(n) = req.notes {
            appt.notes = Some(n);
        }
        core_db::scheduling::update_appointment(&mut *tx.tx, &appt).await?;
        tx.emit(
            types::APPOINTMENT_UPDATED,
            "appointment",
            Some(&id.to_string()),
            &serde_json::to_value(AppointmentUpdated {
                appointment: appt.clone(),
                rescheduled: false,
            })
            .map_err(|e| ApiError::Internal(e.into()))?,
        )
        .await?;
        Ok((tx, appt))
    })
    .await
    .map(Json)
}

/// POST /api/appointments/{id}/move — reschedule with re-validation
/// (dry-run semantics inline: a conflicting move is rejected atomically
/// and emits a `rescheduled` event only when it lands).
#[utoipa::path(operation_id = "appointment_move", post, path = "/api/appointments/{id}/move", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    request_body = MoveAppointmentRequest,
    responses((status = 200, body = Appointment), (status = 409, description = "target slot already booked"), (status = 404)))]
pub async fn move_appointment(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<MoveAppointmentRequest>,
) -> ApiResult<Json<Appointment>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::Edit))?;
    validate_span(req.starts_at, req.ends_at)?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let mut appt = core_db::scheduling::get_appointment(&mut *tx.tx, org_id, id).await?;
        let moved = appt.starts_at != req.starts_at || appt.ends_at != req.ends_at;
        let existing = core_db::scheduling::list_appointments_between(
            &mut *tx.tx,
            org_id,
            &[appt.user_id],
            req.starts_at,
            req.ends_at,
        )
        .await?;
        let blocked = existing
            .iter()
            .any(|x| x.id != id && x.status != AppointmentStatus::Cancelled);
        if blocked
            || !conflict_indices(
                req.starts_at,
                req.ends_at,
                existing
                    .iter()
                    .filter(|x| x.id != id && x.status != AppointmentStatus::Cancelled)
                    .map(|x| (x.starts_at, x.ends_at)),
            )
            .is_empty()
        {
            return Err(ApiError::Conflict("target slot already booked".into()));
        }
        appt.starts_at = req.starts_at;
        appt.ends_at = req.ends_at;
        core_db::scheduling::update_appointment(&mut *tx.tx, &appt).await?;
        tx.emit(
            types::APPOINTMENT_UPDATED,
            "appointment",
            Some(&id.to_string()),
            &serde_json::to_value(AppointmentUpdated {
                appointment: appt.clone(),
                rescheduled: moved,
            })
            .map_err(|e| ApiError::Internal(e.into()))?,
        )
        .await?;
        Ok((tx, appt))
    })
    .await
    .map(Json)
}

/// POST /api/appointments/{id}/cancel — frees the slot (status flip, never
/// deletion, so history survives).
#[utoipa::path(operation_id = "appointment_cancel", post, path = "/api/appointments/{id}/cancel", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = Appointment), (status = 404)))]
pub async fn cancel_appointment(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Appointment>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let mut appt = core_db::scheduling::get_appointment(&mut *tx.tx, org_id, id).await?;
        appt.status = AppointmentStatus::Cancelled;
        core_db::scheduling::update_appointment(&mut *tx.tx, &appt).await?;
        tx.emit(
            types::APPOINTMENT_CANCELLED,
            "appointment",
            Some(&id.to_string()),
            &serde_json::to_value(AppointmentCancelled {
                appointment_id: id,
                user_id: appt.user_id,
                cancelled_by: Some(actor),
            })
            .map_err(|e| ApiError::Internal(e.into()))?,
        )
        .await?;
        Ok((tx, appt))
    })
    .await
    .map(Json)
}

/// POST /api/scheduling/conflict-check — dry-run overlap probe that writes
/// nothing: returns every blocking booking for the candidate slot.
#[utoipa::path(operation_id = "scheduling_conflict_check", post, path = "/api/scheduling/conflict-check", tag = "scheduling", security(("bearer" = [])),
    request_body = ConflictCheckRequest,
    responses((status = 200, body = ConflictCheckResponse)))]
pub async fn conflict_check(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<ConflictCheckRequest>,
) -> ApiResult<Json<ConflictCheckResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    target_user(&authed.principal, Some(req.user_id), "conflict checks")?;
    validate_span(req.starts_at, req.ends_at)?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let existing = core_db::scheduling::list_appointments_between(
        &mut *tx,
        authed.principal.user.org_id,
        &[req.user_id],
        req.starts_at,
        req.ends_at,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    let hits = conflict_indices(
        req.starts_at,
        req.ends_at,
        existing
            .iter()
            .filter(|x| Some(x.id) != req.exclude_appointment_id)
            .filter(|x| x.status != AppointmentStatus::Cancelled)
            .map(|x| (x.starts_at, x.ends_at)),
    );
    let conflicts: Vec<ConflictView> = hits
        .into_iter()
        .filter_map(|i| existing.get(i))
        .map(|a| ConflictView {
            appointment_id: a.id,
            title: a.title.clone(),
            starts_at: a.starts_at,
            ends_at: a.ends_at,
        })
        .collect();
    Ok(Json(ConflictCheckResponse {
        has_conflict: !conflicts.is_empty(),
        conflicts,
    }))
}
