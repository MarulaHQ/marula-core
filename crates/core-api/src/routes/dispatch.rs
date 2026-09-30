//! Dispatch board endpoints (TASKLIST 2.19): the day view a dispatcher
//! plans against, the atomic bulk-move that re-validates every target
//! slot, and queue→technician assignment.
//!
//! Purpose: assemble one page-load payload for /dispatch — active
//! technicians with availability-derived capacity utilization, their live
//! bookings (with ticket number/client name for the block face), on-call
//! and time-off shading, and the unassigned queue in dispatch order —
//! then accept batch reschedules that re-validate conflicts (the same
//! `core_domain::scheduling::conflict_indices` rule the booking route
//! uses) and reject the WHOLE batch on the first violation so the board
//! never lands in a half-applied state. Assignment creates the booking
//! AND flips the ticket assignee in one transaction, so the board, the
//! my-work view, and ticket routing always agree.
//!
//! Non-purpose: no auto-assign heuristics (worker lane, deferred), no
//! travel RoutingProvider math (`travel_minutes` annotated only), no
//! board-level permission shaping for scheduling (a Scheduling module
//! grant with the matching action governs these endpoints; board RBAC is
//! applied where ticket assignment is touched via `authorize_board_op`).
//! The day window is a UTC day — per-technician local-day capacity
//! windows are personal-view territory (routes/calendar.rs).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome, MutationParams};
use crate::org_tx;
use crate::routes::board_access::authorize_board_op;
use crate::routes::scheduling::{sched, validate_span};
use crate::routes::tickets::MyWorkItem;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use core_domain::entities::{Appointment, AppointmentKind, AppointmentStatus};
use core_domain::events::{
    types, AppointmentCreated, AppointmentUpdated, DispatchQueueUpdated, TimeEntryCreated,
    TravelDraftSubmitted,
};
use core_domain::policy::{Action, BoardOp, Module, Permission};
use core_domain::scheduling::{
    booked_minutes, capacity_percent, conflict_indices, covered_minutes, free_intervals,
    occupied_minutes,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

/// Tickets-module permission shorthand for the assignment leg.
fn tickets_op(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Tickets,
        action,
    }
}

/// Time-module permission shorthand for the travel-draft surfaces (they
/// convert through the time-entry path, so the grant matches it).
fn time_perm(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Time,
        action,
    }
}

/// Max moves per bulk call: bounded so one request cannot pin a scan of
/// the whole org schedule, and large enough for a real drag session.
const MAX_MOVES: usize = 64;
const QUEUE_LIMIT: i64 = 100;

/// One dispatch-path query: `date` is a LOCAL date interpreted as the
/// UTC day window (documented UTC scope).
#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct DispatchDayQuery {
    /// The day to assemble; defaults to today (UTC).
    #[schema(value_type = Option<String>, format = "date")]
    pub date: Option<NaiveDate>,
}

/// One booking block on the dispatch board: the appointment plus the
/// ticket number and client name the block face renders.
#[derive(Debug, Serialize, ToSchema)]
pub struct DispatchAppointment {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    pub ticket_number: Option<String>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub client_id: Option<Uuid>,
    pub client_name: Option<String>,
    pub title: String,
    #[schema(value_type = String, format = "date-time")]
    pub starts_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub ends_at: DateTime<Utc>,
    pub kind: AppointmentKind,
    pub status: AppointmentStatus,
    pub travel_minutes: i32,
    /// Travel provenance (Wave 32 lane 7): where the estimate ran from,
    /// which provider produced it, and at what quality. All absent for
    /// manual or unestimated bookings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub travel_from: Option<core_domain::entities::TravelOrigin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub travel_distance_meters: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub travel_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub travel_quality: Option<String>,
    /// Latest field-service status push against this booking (dispatch v2,
    /// additive): `None` when nothing was pushed yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatch_status: Option<DispatchStatusShade>,
}

/// The latest connector/staff status push for one booking, shaded onto
/// the board block face.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DispatchStatusShade {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// `connector` | `staff`.
    pub source: String,
    #[schema(value_type = String, format = "date-time")]
    pub occurred_at: DateTime<Utc>,
}

/// One technician column: capacity stats plus everything the board draws
/// in the column (bookings, on-call, exclusions, availability windows).
#[derive(Debug, Serialize, ToSchema)]
pub struct TechnicianView {
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    pub display_name: String,
    pub timezone: String,
    /// Working minutes covered by availability windows minus exclusions
    /// over the day window (`core_domain::scheduling::covered_minutes`).
    pub available_minutes: i64,
    /// Union of live booking minutes clipped to the day window.
    pub booked_minutes: i64,
    /// 0–100+, where >100 is over-capacity (the Ember warning state).
    pub utilization_percent: i64,
    pub appointments: Vec<DispatchAppointment>,
    pub on_call: Vec<core_domain::entities::OnCallBlock>,
    pub exclusions: Vec<core_domain::entities::CalendarExclusion>,
    pub availability_windows: Vec<core_domain::entities::AvailabilityWindow>,
}

/// The assembled day-view payload: one request, one render.
#[derive(Debug, Serialize, ToSchema)]
pub struct DispatchDayView {
    pub date: NaiveDate,
    #[schema(value_type = String, format = "date-time")]
    pub from: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub to: DateTime<Utc>,
    pub technicians: Vec<TechnicianView>,
    /// Unassigned, non-closed tickets in dispatch order (priority rank,
    /// then effective SLA/due deadline).
    pub unassigned: Vec<MyWorkItem>,
}

/// GET /api/dispatch/day — the whole board for one day.
#[utoipa::path(operation_id = "dispatch_day", get, path = "/api/dispatch/day", tag = "scheduling", security(("bearer" = [])),
    params(DispatchDayQuery),
    responses((status = 200, body = DispatchDayView)))]
pub async fn dispatch_day(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<DispatchDayQuery>,
) -> ApiResult<Json<DispatchDayView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let org_id = authed.principal.user.org_id;

    let date = q.date.unwrap_or_else(|| Utc::now().date_naive());
    let from = date
        .and_hms_opt(0, 0, 0)
        .expect("midnight is a valid local time")
        .and_utc();
    let to = from + Duration::days(1);

    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let members = core_db::users::list_users(&mut *tx, org_id).await?;
    let technicians: Vec<Uuid> = members
        .iter()
        .filter(|u| u.is_active)
        .map(|u| u.id)
        .collect();
    let appointments = core_db::scheduling::list_dispatch_appointments_between(
        &mut *tx,
        org_id,
        &technicians,
        from,
        to,
    )
    .await?;
    let on_call =
        core_db::scheduling::list_on_call_blocks_between(&mut *tx, org_id, &technicians, from, to)
            .await?;

    let mut views = Vec::with_capacity(technicians.len());
    for user in members.iter().filter(|u| u.is_active) {
        let windows =
            core_db::scheduling::list_availability_windows(&mut *tx, org_id, user.id).await?;
        let exclusions = core_db::scheduling::list_calendar_exclusions_between(
            &mut *tx, org_id, user.id, from, to,
        )
        .await?;
        let available = covered_minutes(&windows, &exclusions, from, to);
        let mut user_appointments: Vec<DispatchAppointment> = Vec::new();
        for a in appointments.iter().filter(|a| a.user_id == user.id) {
            let kind = AppointmentKind::parse(&a.kind)
                .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("bad kind on row")))?;
            let status = AppointmentStatus::parse(&a.status)
                .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("bad status on row")))?;
            user_appointments.push(DispatchAppointment {
                id: a.id,
                user_id: a.user_id,
                ticket_id: a.ticket_id,
                ticket_number: a.ticket_number.clone(),
                client_id: a.client_id,
                client_name: a.client_name.clone(),
                title: a.title.clone(),
                starts_at: a.starts_at,
                ends_at: a.ends_at,
                kind,
                status,
                travel_minutes: a.travel_minutes,
                travel_from: a
                    .travel_from
                    .clone()
                    .and_then(|value| serde_json::from_value(value).ok()),
                travel_distance_meters: a.travel_distance_meters,
                travel_provider: a.travel_provider.clone(),
                travel_quality: a.travel_quality.clone(),
                dispatch_status: None,
            });
        }
        let booked = occupied_minutes(
            user_appointments.iter().map(|a: &DispatchAppointment| {
                (a.starts_at, a.ends_at, i64::from(a.travel_minutes))
            }),
            from,
            to,
        );
        views.push(TechnicianView {
            user_id: user.id,
            display_name: user.display_name.clone(),
            timezone: user.timezone.clone(),
            available_minutes: available,
            booked_minutes: booked,
            utilization_percent: capacity_percent(booked, available),
            appointments: user_appointments,
            on_call: on_call
                .iter()
                .filter(|b| b.user_id == user.id)
                .cloned()
                .collect(),
            exclusions,
            availability_windows: windows,
        });
    }

    // Dispatch v2: the day view's queue honors dispatcher rank overrides
    // with the same merge rule as GET /dispatch/queue — overridden rows
    // first by explicit rank, then the computed order.
    let unassigned = core_db::tickets::unassigned_queue_annotated(&mut *tx, org_id, QUEUE_LIMIT)
        .await?
        .into_iter()
        .map(MyWorkItem::from_row)
        .collect::<Vec<_>>();
    let overrides = core_db::dispatch::list_queue_entries(&mut *tx, org_id)
        .await?
        .into_iter()
        .map(|row| (row.ticket_id, row.rank))
        .collect::<std::collections::HashMap<Uuid, i32>>();
    let mut indexed: Vec<(usize, Option<i32>, MyWorkItem)> = unassigned
        .into_iter()
        .enumerate()
        .map(|(i, t)| (i, overrides.get(&t.id).copied(), t))
        .collect();
    indexed.sort_by_key(|(i, rank, _)| (rank.is_none(), *rank, *i));
    let unassigned: Vec<MyWorkItem> = indexed.into_iter().map(|(_, _, t)| t).collect();

    // Dispatch v2: shade the day's blocks with the LATEST field-service
    // status push (connector or staff) when one exists. One query for the
    // whole day; appointments without pushes carry no shade.
    let day_ids: Vec<Uuid> = views
        .iter()
        .flat_map(|t| t.appointments.iter().map(|a| a.id))
        .collect();
    let latest = core_db::dispatch::latest_status_by_appointment(&mut *tx, org_id, &day_ids)
        .await?
        .into_iter()
        .map(|row| (row.appointment_id, row))
        .collect::<std::collections::HashMap<Uuid, _>>();
    for view in views.iter_mut() {
        for appt in view.appointments.iter_mut() {
            appt.dispatch_status = latest.get(&appt.id).map(|row| DispatchStatusShade {
                status: row.status.clone(),
                note: row.note.clone(),
                source: row.source.clone(),
                occurred_at: row.occurred_at,
            });
        }
    }
    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(DispatchDayView {
        date,
        from,
        to,
        technicians: views,
        unassigned,
    }))
}

/// One move in a batch: the booking's new user and slot.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MoveSpec {
    #[schema(value_type = uuid::Uuid)]
    pub appointment_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    #[schema(value_type = String, format = "date-time")]
    pub starts_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub ends_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BulkMoveRequest {
    pub moves: Vec<MoveSpec>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BulkMoveResponse {
    pub moved: Vec<DispatchAppointment>,
}

/// POST /api/dispatch/bulk-move — atomic batch reschedule.
///
/// Every move is re-validated against the TARGET user's live bookings
/// in the union window (each moved booking vacates its old slot, so
/// swaps are legal); the first violating move rejects the WHOLE batch
/// with a typed 409 naming the move index and blocking booking.
#[utoipa::path(operation_id = "dispatch_bulk_move", post, path = "/api/dispatch/bulk-move", tag = "scheduling", security(("bearer" = [])),
    request_body = BulkMoveRequest,
    responses((status = 200, body = BulkMoveResponse),
              (status = 409, description = "target slot already booked (typed conflict)")))]
pub async fn bulk_move(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<BulkMoveRequest>,
) -> ApiResult<Json<BulkMoveResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::Edit))?;
    if req.moves.is_empty() {
        return Err(ApiError::Validation("moves must not be empty".into()));
    }
    if req.moves.len() > MAX_MOVES {
        return Err(ApiError::Validation(format!(
            "moves exceeds batch limit of {MAX_MOVES}"
        )));
    }
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        for spec in &req.moves {
            validate_span(spec.starts_at, spec.ends_at)?;
        }

        // Load every booking once; org-scoped `get_appointment` turns a
        // foreign id into NotFound, so a cross-org id is rejected.
        let mut moved_appointments = Vec::with_capacity(req.moves.len());
        for spec in &req.moves {
            let appt =
                core_db::scheduling::get_appointment(&mut *tx.tx, org_id, spec.appointment_id)
                    .await?;
            moved_appointments.push(appt);
        }

        // Union window over every target user; blockers are the target
        // users' live bookings PLUS nothing else — bookings being moved
        // vacate their slots, so they are excluded wholesale from the
        // blocker set (atomic all-or-nothing makes swaps safe).
        let union_from = req
            .moves
            .iter()
            .map(|m| m.starts_at)
            .min()
            .expect("moves non-empty");
        let union_to = req
            .moves
            .iter()
            .map(|m| m.ends_at)
            .max()
            .expect("moves non-empty");
        let target_ids: Vec<Uuid> = req.moves.iter().map(|m| m.user_id).collect();
        let existing = core_db::scheduling::list_appointments_between(
            &mut *tx.tx,
            org_id,
            &target_ids,
            union_from,
            union_to,
        )
        .await?;
        let moving_ids: Vec<Uuid> = req.moves.iter().map(|m| m.appointment_id).collect();

        for (index, (spec, appt)) in req.moves.iter().zip(moved_appointments.iter()).enumerate() {
            let blockers: Vec<&core_domain::entities::Appointment> = existing
                .iter()
                .filter(|x| x.user_id == spec.user_id)
                .filter(|x| !moving_ids.contains(&x.id))
                .filter(|x| x.status != AppointmentStatus::Cancelled)
                .collect();
            let blocking_spans = blockers
                .iter()
                .map(|x| (x.starts_at, x.ends_at))
                .collect::<Vec<_>>();
            if let Some(idx) = conflict_indices(spec.starts_at, spec.ends_at, blocking_spans)
                .into_iter()
                .next()
            {
                let blocker_title = blockers
                    .get(idx)
                    .map(|b| b.title.clone())
                    .unwrap_or_default();
                return Err(ApiError::DispatchMoveConflict {
                    index,
                    appointment_id: appt.id,
                    blocker_title,
                });
            }
        }

        // Apply, two-phase so swaps are constraint-safe mid-batch: first
        // vacate EVERY moved slot (a forward move would otherwise collide
        // with a booking vacated later in the same batch), then reinsert
        // at targets. Identity stays; events carry the new user_id so the
        // realtime router fans out to the right channel.
        let mut updated = Vec::with_capacity(req.moves.len());
        for (spec, mut appt) in req.moves.iter().zip(moved_appointments) {
            appt.user_id = spec.user_id;
            appt.starts_at = spec.starts_at;
            appt.ends_at = spec.ends_at;
            core_db::scheduling::delete_appointment(&mut tx.tx, org_id, appt.id).await?;
            updated.push(appt);
        }
        let mut moved_views = Vec::with_capacity(updated.len());
        for appt in updated {
            core_db::scheduling::create_appointment(&mut *tx.tx, &appt).await?;
            tx.emit(
                types::APPOINTMENT_UPDATED,
                "appointment",
                Some(&appt.id.to_string()),
                &serde_json::to_value(AppointmentUpdated {
                    rescheduled: true,
                    appointment: appt.clone(),
                })
                .map_err(|e| ApiError::Internal(e.into()))?,
            )
            .await?;
            moved_views.push(DispatchAppointment {
                id: appt.id,
                user_id: appt.user_id,
                ticket_id: appt.ticket_id,
                ticket_number: None,
                client_id: appt.client_id,
                client_name: None,
                title: appt.title.clone(),
                starts_at: appt.starts_at,
                ends_at: appt.ends_at,
                kind: appt.kind,
                status: appt.status,
                travel_minutes: appt.travel_minutes,
                travel_from: appt.travel_from.clone(),
                travel_distance_meters: appt.travel_distance_meters,
                travel_provider: appt.travel_provider.clone(),
                travel_quality: appt.travel_quality.clone(),
                dispatch_status: None,
            });
        }
        Ok((tx, BulkMoveResponse { moved: moved_views }))
    })
    .await
    .map(Json)
}

/// POST /api/dispatch/assign — place an unassigned ticket on a
/// technician's board: creates the booking AND flips the ticket
/// assignee in one transaction.
#[utoipa::path(operation_id = "dispatch_assign", post, path = "/api/dispatch/assign", tag = "scheduling", security(("bearer" = [])),
    request_body = AssignRequest,
    responses((status = 201, body = DispatchAppointment),
              (status = 409, description = "slot already booked")))]
pub async fn assign(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<AssignRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<DispatchAppointment>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::Create))?;
    require_permission(&authed.principal, tickets_op(Action::Edit))?;
    validate_span(req.starts_at, req.ends_at)?;
    if req.travel_minutes < 0 {
        return Err(ApiError::Validation("travel_minutes must be >= 0".into()));
    }
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let ticket = core_db::tickets::get_ticket(&mut *tx.tx, org_id, req.ticket_id).await?;
        authorize_board_op(
            &mut tx.tx,
            &authed.principal,
            ticket.board_id,
            BoardOp::WorkTicket,
        )
        .await?;
        let technician = core_db::users::get_user(&mut *tx.tx, org_id, req.user_id)
            .await
            .map_err(|_| ApiError::NotFound)?;
        if !technician.is_active {
            return Err(ApiError::Validation(format!(
                "technician {} is inactive",
                technician.display_name
            )));
        }

        let existing = core_db::scheduling::list_appointments_between(
            &mut *tx.tx,
            org_id,
            &[req.user_id],
            req.starts_at,
            req.ends_at,
        )
        .await?;
        if let Some(idx) = conflict_indices(
            req.starts_at,
            req.ends_at,
            existing.iter().map(|a| (a.starts_at, a.ends_at)),
        )
        .into_iter()
        .next()
        {
            let blocker = existing
                .get(idx)
                .map(|b| b.title.clone())
                .unwrap_or_default();
            return Err(ApiError::Conflict(format!(
                "slot already booked: {blocker}"
            )));
        }

        let appointment = Appointment {
            id: Uuid::now_v7(),
            org_id,
            user_id: req.user_id,
            ticket_id: Some(req.ticket_id),
            client_id: ticket.client_id,
            site_id: ticket.site_id,
            title: format!("{} {}", ticket.number.0, ticket.title),
            starts_at: req.starts_at,
            ends_at: req.ends_at,
            kind: AppointmentKind::OnSite,
            status: AppointmentStatus::Scheduled,
            travel_minutes: req.travel_minutes,
            notes: None,
            scheduling_link_id: None,
            travel_from: None,
            travel_distance_meters: None,
            travel_provider: None,
            travel_quality: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
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

        // Flip the ticket assignee so queue/my-work/board agree.
        core_db::tickets::update_ticket(
            &mut *tx.tx,
            org_id,
            ticket.id,
            core_db::tickets::TicketPatch {
                title: None,
                body: None,
                priority: None,
                assignee_id: Some(Some(req.user_id)),
                client_id: None,
                site_id: None,
                contact_id: None,
                due_at: None,

                type_id: None,
                category_id: None,
            },
        )
        .await?;
        let after = core_db::tickets::get_ticket(&mut *tx.tx, org_id, ticket.id).await?;
        if ticket.assignee_id != after.assignee_id {
            tx.emit(
                types::TICKET_ASSIGNED,
                "ticket",
                Some(after.id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::TicketAssigned {
                    ticket_id: after.id,
                    board_id: after.board_id,
                    assignee_id: after.assignee_id,
                    assigned_by: Some(actor),
                })
                .map_err(|e| ApiError::Internal(e.into()))?,
            )
            .await?;
        }

        let view = DispatchAppointment {
            id: appointment.id,
            user_id: appointment.user_id,
            ticket_id: appointment.ticket_id,
            ticket_number: Some(ticket.number.0.clone()),
            client_id: appointment.client_id,
            client_name: None,
            title: appointment.title,
            starts_at: appointment.starts_at,
            ends_at: appointment.ends_at,
            kind: appointment.kind,
            status: appointment.status,
            travel_minutes: appointment.travel_minutes,
            travel_from: appointment.travel_from.clone(),
            travel_distance_meters: appointment.travel_distance_meters,
            travel_provider: appointment.travel_provider.clone(),
            travel_quality: appointment.travel_quality.clone(),
            dispatch_status: None,
        };
        Ok((tx, view))
    })
    .await
    .map(|view| (axum::http::StatusCode::CREATED, Json(view)))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AssignRequest {
    #[schema(value_type = uuid::Uuid)]
    pub ticket_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    #[schema(value_type = String, format = "date-time")]
    pub starts_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub ends_at: DateTime<Utc>,
    #[serde(default)]
    pub travel_minutes: i32,
}

// ===========================================================================
// Dispatch v2 — resource planning (TASKLIST Phase 3, Wave 9 lane 9)
//
// Three planning surfaces over the same Scheduling-module grant the day
// view uses, plus the travel/mileage drafts that convert through the
// existing time-entry path. Every mutation rides the IF1–IF3 envelope
// (idempotency claim → version gate → write-trail record) so autosave
// clients and the undo substrate work here exactly like everywhere else.
// ===========================================================================

/// Default day count for the capacity horizon (one planning week).
const CAPACITY_MAX_DAYS: u32 = 31;
const SKILL_MAX_LEN: usize = 80;
const REQUIRED_SKILLS_MAX: usize = 20;
const TRAVEL_NOTE_MAX: usize = 2000;
const QUEUE_NOTE_MAX: usize = 500;
const TRAVEL_DRAFT_LIMIT: i64 = 200;

// --- capacity horizon ------------------------------------------------------

/// GET /api/dispatch/capacity query: `from` date (defaults to today UTC)
/// and the horizon length in days (1..=31, default 7).
#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct CapacityQuery {
    #[schema(value_type = Option<String>, format = "date")]
    pub from: Option<NaiveDate>,
    /// Horizon length in days (1..=31).
    pub days: Option<u32>,
}

/// One technician's capacity for one day of the horizon.
#[derive(Debug, Serialize, ToSchema)]
pub struct DayCapacity {
    pub date: NaiveDate,
    /// Availability coverage minus exclusions for the UTC day.
    pub available_minutes: i64,
    /// Live booking minutes clipped to the day.
    pub booked_minutes: i64,
    /// What a NEW booking can still consume: availability minus
    /// exclusions minus existing bookings.
    pub free_minutes: i64,
    /// 0–100+, where >100 is over-capacity.
    pub utilization_percent: i64,
}

/// One technician across the whole horizon.
#[derive(Debug, Serialize, ToSchema)]
pub struct TechnicianCapacity {
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    pub display_name: String,
    pub days: Vec<DayCapacity>,
}

/// The assembled horizon payload. `unassigned_count` is the current open
/// unassigned queue depth so planners see demand next to capacity.
#[derive(Debug, Serialize, ToSchema)]
pub struct CapacityHorizonView {
    pub from: NaiveDate,
    pub days: u32,
    pub technicians: Vec<TechnicianCapacity>,
    pub unassigned_count: i64,
}

/// GET /api/dispatch/capacity — multi-day capacity horizon: the day view's
/// capacity math repeated per UTC day over the window, one org-wide
/// availability read + one exclusion read + one appointment read.
#[utoipa::path(operation_id = "dispatch_capacity", get, path = "/api/dispatch/capacity", tag = "scheduling", security(("bearer" = [])),
    params(CapacityQuery),
    responses((status = 200, body = CapacityHorizonView)))]
pub async fn capacity(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<CapacityQuery>,
) -> ApiResult<Json<CapacityHorizonView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let org_id = authed.principal.user.org_id;

    let days = q.days.unwrap_or(7).clamp(1, CAPACITY_MAX_DAYS);
    let first = q.from.unwrap_or_else(|| Utc::now().date_naive());
    let window_from = first
        .and_hms_opt(0, 0, 0)
        .expect("midnight is a valid local time")
        .and_utc();
    let window_to = window_from + Duration::days(i64::from(days));

    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let members = core_db::users::list_users(&mut *tx, org_id).await?;
    let technicians: Vec<Uuid> = members
        .iter()
        .filter(|u| u.is_active)
        .map(|u| u.id)
        .collect();

    let appointments = core_db::scheduling::list_appointments_between(
        &mut *tx,
        org_id,
        &technicians,
        window_from,
        window_to,
    )
    .await?;
    let windows = core_db::scheduling::list_all_availability_windows(&mut *tx, org_id).await?;
    let exclusions = core_db::scheduling::list_calendar_exclusions_for_users_between(
        &mut *tx,
        org_id,
        &technicians,
        window_from,
        window_to,
    )
    .await?;
    let unassigned =
        core_db::tickets::unassigned_queue_annotated(&mut *tx, org_id, i64::MAX).await?;

    let mut views = Vec::with_capacity(technicians.len());
    for user in members.iter().filter(|u| u.is_active) {
        let user_windows: Vec<_> = windows
            .iter()
            .filter(|w| w.user_id == user.id)
            .cloned()
            .collect();
        let mut day_views = Vec::with_capacity(days as usize);
        for day_offset in 0..days {
            let date = first + Duration::days(i64::from(day_offset));
            let day_from = date
                .and_hms_opt(0, 0, 0)
                .expect("midnight is a valid local time")
                .and_utc();
            let day_to = day_from + Duration::days(1);
            let user_exclusions: Vec<_> = exclusions
                .iter()
                .filter(|x| x.user_id == user.id)
                .cloned()
                .collect();
            let user_bookings: Vec<_> = appointments
                .iter()
                .filter(|a| a.user_id == user.id && a.status != AppointmentStatus::Cancelled)
                .map(|a| (a.starts_at, a.ends_at))
                .collect();
            let available = covered_minutes(&user_windows, &user_exclusions, day_from, day_to);
            let booked = booked_minutes(user_bookings.iter().copied(), day_from, day_to);
            let free = free_intervals(
                &user_windows,
                &user_exclusions,
                &user_bookings,
                day_from,
                day_to,
            )
            .iter()
            .map(|(s, e)| (*e - *s).num_seconds() / 60)
            .sum::<i64>();
            day_views.push(DayCapacity {
                date,
                available_minutes: available,
                booked_minutes: booked,
                free_minutes: free,
                utilization_percent: capacity_percent(booked, available),
            });
        }
        views.push(TechnicianCapacity {
            user_id: user.id,
            display_name: user.display_name.clone(),
            days: day_views,
        });
    }
    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(CapacityHorizonView {
        from: first,
        days,
        technicians: views,
        unassigned_count: unassigned.len() as i64,
    }))
}

// --- unassigned-queue prioritization ----------------------------------------

/// The queue endpoint's page size clamp.
const QUEUE_PAGE_LIMIT: i64 = 500;

/// One queued ticket: the day-view's queue row shape plus the explicit
/// override when one exists.
#[derive(Debug, Serialize, ToSchema)]
pub struct QueuedTicket {
    #[serde(flatten)]
    pub ticket: MyWorkItem,
    /// The dispatcher-set rank pinning this ticket's position (overrides
    /// sort ahead of the computed order); absent = computed order only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank_override: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub override_note: Option<String>,
}

/// GET /api/dispatch/queue envelope.
#[derive(Debug, Serialize, ToSchema)]
pub struct DispatchQueueView {
    pub queue: Vec<QueuedTicket>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct DispatchQueueQuery {
    /// Page size, clamped to [1, 500].
    pub limit: Option<i64>,
}

/// GET /api/dispatch/queue — the full unassigned queue with dispatcher
/// overrides applied: overridden tickets sort by explicit rank first
/// (ties fall back to the computed order), then everything else in the
/// computed order (priority rank, then effective SLA/due deadline).
#[utoipa::path(operation_id = "dispatch_queue_list", get, path = "/api/dispatch/queue", tag = "scheduling", security(("bearer" = [])),
    params(DispatchQueueQuery),
    responses((status = 200, body = DispatchQueueView)))]
pub async fn queue_list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<DispatchQueueQuery>,
) -> ApiResult<Json<DispatchQueueView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let limit = q.limit.unwrap_or(QUEUE_LIMIT).clamp(1, QUEUE_PAGE_LIMIT);
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let base = core_db::tickets::unassigned_queue_annotated(&mut *tx, org_id, limit)
        .await?
        .into_iter()
        .map(MyWorkItem::from_row)
        .collect::<Vec<_>>();
    let overrides = core_db::dispatch::list_queue_entries(&mut *tx, org_id)
        .await?
        .into_iter()
        .map(|row| (row.ticket_id, (row.rank, row.note)))
        .collect::<std::collections::HashMap<Uuid, (i32, Option<String>)>>();
    tx.commit().await.map_err(ApiError::from)?;

    // Stable partition: overridden rows first by (rank, base position),
    // then the remaining rows in base order. `sort_by` is stable, so the
    // base position carried in the tuple is the tiebreaker.
    let mut indexed: Vec<(usize, QueuedTicket)> = base
        .into_iter()
        .enumerate()
        .map(|(i, t)| {
            let (rank_override, override_note) = overrides
                .get(&t.id)
                .map(|(r, n)| (Some(*r), n.clone()))
                .unwrap_or((None, None));
            (
                i,
                QueuedTicket {
                    ticket: t,
                    rank_override,
                    override_note,
                },
            )
        })
        .collect();
    indexed.sort_by_key(|(i, qt)| (qt.rank_override.is_none(), qt.rank_override, *i));

    Ok(Json(DispatchQueueView {
        queue: indexed.into_iter().map(|(_, qt)| qt).collect(),
    }))
}

/// PUT /api/dispatch/queue/{ticket_id} body.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SetQueueRankRequest {
    /// Explicit queue position: 0 sorts first among overrides.
    pub rank: i32,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(flatten)]
    pub instant: MutationParams,
}

/// Queue mutation envelope (set + clear share the shape; clears carry
/// `cleared: true` and null note).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct QueueMutationResponse {
    #[schema(value_type = uuid::Uuid)]
    pub ticket_id: Uuid,
    pub rank: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub cleared: bool,
    pub mutation: instant::MutationMeta,
}

/// PUT /api/dispatch/queue/{ticket_id} — pin a ticket's queue position.
/// The ticket must exist and still be open+unassigned (an override for a
/// placed ticket is meaningless); idempotent via the instant envelope.
#[utoipa::path(operation_id = "dispatch_queue_set", put, path = "/api/dispatch/queue/{ticket_id}", tag = "scheduling", security(("bearer" = [])),
    params(("ticket_id" = Uuid, Path)),
    request_body = SetQueueRankRequest,
    responses((status = 200, body = QueueMutationResponse),
              (status = 404, description = "ticket not visible or not open/unassigned"),
              (status = 409, description = "version conflict or idempotency key reused")))]
pub async fn queue_set(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(ticket_id): Path<Uuid>,
    Json(req): Json<SetQueueRankRequest>,
) -> ApiResult<Json<QueueMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::Edit))?;
    if req.rank < 0 {
        return Err(ApiError::Validation("rank must be >= 0".into()));
    }
    let note = validate_queue_note(req.note.as_deref())?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "dispatch_queue_entry",
                entity_id: Some(ticket_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // Open + unassigned gate: the queue is a planning surface for
        // work that is not placed yet.
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, ctx.org_id, ticket_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if ticket.assignee_id.is_some()
            || ticket.category == core_domain::status::TicketStatusCategory::Closed
        {
            return Err(ApiError::Validation(
                "only unassigned, open tickets can hold a queue override".into(),
            ));
        }
        let before = core_db::dispatch::get_queue_entry(&mut *ctx.tx, ctx.org_id, ticket_id)
            .await
            .ok();
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "dispatch_queue_entry",
                entity_id: ticket_id,
                version: req.instant.version,
            },
        )
        .await?;
        let row = core_db::dispatch::upsert_queue_entry(
            &mut ctx.tx,
            Uuid::now_v7(),
            ctx.org_id,
            ticket_id,
            req.rank,
            note.as_deref(),
            actor,
        )
        .await?;
        ctx.emit(
            types::DISPATCH_QUEUE_UPDATED,
            "ticket",
            Some(ticket_id.to_string().as_str()),
            &serde_json::to_value(DispatchQueueUpdated {
                ticket_id,
                rank: row.rank,
                cleared: false,
            })
            .map_err(|e| ApiError::Internal(e.into()))?,
        )
        .await?;
        let response = QueueMutationResponse {
            ticket_id,
            rank: row.rank,
            note: row.note.clone(),
            cleared: false,
            mutation: instant::MutationMeta {
                seq,
                version: seq,
                updated_at: row.updated_at,
                before: before
                    .as_ref()
                    .map(|b| serde_json::to_value(b).unwrap_or(Value::Null)),
                after: serde_json::to_value(&row).unwrap_or(Value::Null),
                idempotency_key: key.clone(),
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "dispatch_queue_entry",
                entity_id: ticket_id,
                op: "queue_rank_set",
                seq,
                before: response.mutation.before.clone(),
                after: serde_json::to_value(&row).unwrap_or(Value::Null),
                updated_at: row.updated_at,
                key: key.as_deref(),
                payload: &serde_json::to_value(&response).unwrap_or(Value::Null),
                provenance: instant::ProvenanceContext::default(),
            },
        )
        .await?;
        let response = QueueMutationResponse {
            mutation,
            ..response
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/dispatch/queue/{ticket_id} — clear a rank override.
/// Idempotent: clearing an unset override still returns 200 with
/// `cleared: false` (the dispatcher's intent is already satisfied).
#[utoipa::path(operation_id = "dispatch_queue_clear", delete, path = "/api/dispatch/queue/{ticket_id}", tag = "scheduling", security(("bearer" = [])),
    params(("ticket_id" = Uuid, Path)),
    responses((status = 200, body = QueueMutationResponse),
              (status = 404, description = "ticket not visible")))]
pub async fn queue_clear(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(ticket_id): Path<Uuid>,
) -> ApiResult<Json<QueueMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;

    let outcome = org_tx::with_org_tx(&state, org_id, actor, |mut ctx| async move {
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, ctx.org_id, ticket_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let before = core_db::dispatch::get_queue_entry(&mut *ctx.tx, ctx.org_id, ticket_id)
            .await
            .ok();
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "dispatch_queue_entry",
                entity_id: ticket_id,
                version: None,
            },
        )
        .await?;
        core_db::dispatch::clear_queue_entry(&mut ctx.tx, ctx.org_id, ticket_id).await?;
        ctx.emit(
            types::DISPATCH_QUEUE_UPDATED,
            "ticket",
            Some(ticket_id.to_string().as_str()),
            &serde_json::to_value(DispatchQueueUpdated {
                ticket_id,
                rank: 0,
                cleared: true,
            })
            .map_err(|e| ApiError::Internal(e.into()))?,
        )
        .await?;
        let (rank, note, updated_at) = before
            .as_ref()
            .map(|b| (b.rank, b.note.clone(), b.updated_at))
            .unwrap_or((0, None, Utc::now()));
        let response = QueueMutationResponse {
            ticket_id: ticket.id,
            rank,
            note,
            cleared: true,
            mutation: instant::MutationMeta {
                seq,
                version: seq,
                updated_at,
                before: before
                    .as_ref()
                    .map(|b| serde_json::to_value(b).unwrap_or(Value::Null)),
                after: Value::Null,
                idempotency_key: None,
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "dispatch_queue_entry",
                entity_id: ticket_id,
                op: "queue_rank_clear",
                seq,
                before: response.mutation.before.clone(),
                // The provenance projection diffs top-level keys of
                // OBJECT snapshots only — a jsonb scalar/null cannot be
                // key-diffed, so deletes record a tombstone object.
                after: serde_json::json!({ "deleted": true }),
                updated_at: response.mutation.updated_at,
                key: None,
                payload: &serde_json::to_value(&response).unwrap_or(Value::Null),
                provenance: instant::ProvenanceContext::default(),
            },
        )
        .await?;
        let response = QueueMutationResponse {
            mutation,
            ..response
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- skills / certifications ---------------------------------------------------

/// One skill on one technician.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SkillView {
    pub skill: String,
    /// Certification expiry; absent = does not expire.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "date-time")]
    pub expires_at: Option<DateTime<Utc>>,
}

/// GET /api/dispatch/skills envelope entry: the org skill catalog with
/// holder counts (`active` = non-expired as of now).
#[derive(Debug, Serialize, ToSchema)]
pub struct SkillCatalogEntry {
    pub skill: String,
    pub holders: i64,
    pub active_holders: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SkillsCatalogView {
    pub skills: Vec<SkillCatalogEntry>,
}

/// GET /api/dispatch/skills — the org's skill/certification catalog.
#[utoipa::path(operation_id = "dispatch_skills_catalog", get, path = "/api/dispatch/skills", tag = "scheduling", security(("bearer" = [])),
    responses((status = 200, body = SkillsCatalogView)))]
pub async fn skills_catalog(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<SkillsCatalogView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::dispatch::list_org_skills(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    let now = Utc::now();
    let mut map: std::collections::BTreeMap<String, (i64, i64)> = Default::default();
    for row in rows {
        let e = map.entry(row.skill).or_insert((0, 0));
        e.0 += 1;
        if row.expires_at.is_none_or(|t| t > now) {
            e.1 += 1;
        }
    }
    Ok(Json(SkillsCatalogView {
        skills: map
            .into_iter()
            .map(|(skill, (holders, active_holders))| SkillCatalogEntry {
                skill,
                holders,
                active_holders,
            })
            .collect(),
    }))
}

/// One entry of a skill replacement set.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SkillInput {
    /// Org skill vocabulary, 1..=80 chars after trimming.
    pub skill: String,
    /// Certification expiry; must be in the future when present.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "date-time")]
    pub expires_at: Option<DateTime<Utc>>,
}

/// PUT /api/dispatch/users/{user_id}/skills body.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReplaceUserSkillsRequest {
    /// Full replacement set (an empty set clears every skill).
    pub skills: Vec<SkillInput>,
    #[serde(flatten)]
    pub instant: MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UserSkillsView {
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    pub skills: Vec<SkillView>,
}

/// The full-replace mutation envelope (IF2).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UserSkillsMutationResponse {
    #[serde(flatten)]
    pub skills: UserSkillsView,
    pub mutation: instant::MutationMeta,
}

/// PUT /api/dispatch/users/{user_id}/skills — full-replace the
/// technician's skill set. The technician must be an active member of
/// this org.
#[utoipa::path(operation_id = "dispatch_user_skills_replace", put, path = "/api/dispatch/users/{user_id}/skills", tag = "scheduling", security(("bearer" = [])),
    params(("user_id" = Uuid, Path)),
    request_body = ReplaceUserSkillsRequest,
    responses((status = 200, body = UserSkillsMutationResponse),
              (status = 404, description = "user not visible in scope"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "validation failed (labels, expiry, duplicates)")))]
pub async fn user_skills_replace(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(user_id): Path<Uuid>,
    Json(req): Json<ReplaceUserSkillsRequest>,
) -> ApiResult<Json<UserSkillsMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::Edit))?;
    let skills = validate_skill_set(&req.skills)?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "user_skills",
                entity_id: Some(user_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let technician = core_db::users::get_user(&mut *ctx.tx, ctx.org_id, user_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if !technician.is_active {
            return Err(ApiError::Validation(
                "skills belong on active members".into(),
            ));
        }
        let before = core_db::dispatch::list_user_skills(&mut *ctx.tx, ctx.org_id, user_id).await?;
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "user_skills",
                entity_id: user_id,
                version: req.instant.version,
            },
        )
        .await?;
        let rows =
            core_db::dispatch::replace_user_skills(&mut ctx.tx, ctx.org_id, user_id, &skills)
                .await?;
        let view = UserSkillsView {
            user_id,
            skills: rows
                .iter()
                .map(|r| SkillView {
                    skill: r.skill.clone(),
                    expires_at: r.expires_at,
                })
                .collect(),
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "user_skills",
                entity_id: user_id,
                op: "skills_replace",
                seq,
                // Object snapshots (the provenance projection diffs
                // top-level keys; arrays cannot be diffed).
                before: Some(serde_json::json!({
                    "user_id": user_id,
                    "skills": before.iter().map(|r| serde_json::json!({
                        "skill": r.skill, "expires_at": r.expires_at,
                    })).collect::<Vec<_>>()
                })),
                after: serde_json::json!({
                    "user_id": user_id,
                    "skills": rows.iter().map(|r| serde_json::json!({
                        "skill": r.skill, "expires_at": r.expires_at,
                    })).collect::<Vec<_>>()
                }),
                updated_at: Utc::now(),
                key: key.as_deref(),
                payload: &serde_json::to_value(&view).unwrap_or(Value::Null),
                provenance: instant::ProvenanceContext::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(UserSkillsMutationResponse {
                skills: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- matching --------------------------------------------------------------------

/// GET /api/dispatch/match query: how many unassigned tickets to match.
#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct MatchQuery {
    pub limit: Option<i64>,
}

/// One technician candidate for one ticket: the skills this candidate is
/// missing (an empty list = fully qualified right now).
#[derive(Debug, Serialize, ToSchema)]
pub struct MatchCandidate {
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    pub display_name: String,
    pub missing: Vec<String>,
}

/// Skills-matched staffing for one unassigned ticket. Tickets with no
/// required skills are not listed (the plain queue ordering covers them).
#[derive(Debug, Serialize, ToSchema)]
pub struct TicketMatch {
    #[serde(flatten)]
    pub ticket: MyWorkItem,
    pub required_skills: Vec<String>,
    /// Active technicians sorted best-first (fewest missing, then name).
    pub candidates: Vec<MatchCandidate>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MatchView {
    pub matches: Vec<TicketMatch>,
}

/// GET /api/dispatch/match — for every unassigned open ticket carrying
/// required skills, the active technicians sorted best-first by how many
/// required skills they are missing (0 = fully qualified). Expired
/// certifications never count as held.
#[utoipa::path(operation_id = "dispatch_match", get, path = "/api/dispatch/match", tag = "scheduling", security(("bearer" = [])),
    params(MatchQuery),
    responses((status = 200, body = MatchView)))]
pub async fn match_queue(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<MatchQuery>,
) -> ApiResult<Json<MatchView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);

    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let queue = core_db::tickets::unassigned_queue_annotated(&mut *tx, org_id, limit).await?;
    let ticket_ids: Vec<Uuid> = queue.iter().map(|r| r.ticket.id).collect();
    let required =
        core_db::dispatch::list_ticket_required_skills(&mut *tx, org_id, &ticket_ids).await?;
    let all_skills = core_db::dispatch::list_org_skills(&mut *tx, org_id).await?;
    let members = core_db::users::list_users(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let now = Utc::now();
    let mut held_by_user: std::collections::HashMap<Uuid, Vec<&str>> = Default::default();
    for row in &all_skills {
        if row.expires_at.is_none_or(|t| t > now) {
            held_by_user
                .entry(row.user_id)
                .or_default()
                .push(row.skill.as_str());
        }
    }
    let mut required_by_ticket: std::collections::HashMap<Uuid, Vec<&str>> = Default::default();
    for (ticket_id, skill) in &required {
        required_by_ticket
            .entry(*ticket_id)
            .or_default()
            .push(skill.as_str());
    }

    let mut matches = Vec::new();
    for row in queue {
        let Some(req_skills) = required_by_ticket.get(&row.ticket.id) else {
            continue;
        };
        let required: Vec<String> = req_skills.iter().map(|s| (*s).to_string()).collect();
        let mut candidates: Vec<MatchCandidate> = Vec::new();
        for user in members.iter().filter(|u| u.is_active) {
            let held = held_by_user
                .get(&user.id)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let missing: Vec<String> = required
                .iter()
                .filter(|r| !held.iter().any(|h| h == *r))
                .cloned()
                .collect();
            candidates.push(MatchCandidate {
                user_id: user.id,
                display_name: user.display_name.clone(),
                missing,
            });
        }
        candidates.sort_by(|a, b| {
            a.missing
                .len()
                .cmp(&b.missing.len())
                .then_with(|| a.display_name.cmp(&b.display_name))
        });
        matches.push(TicketMatch {
            ticket: MyWorkItem::from_row(row),
            required_skills: required,
            candidates,
        });
    }

    Ok(Json(MatchView { matches }))
}

/// PUT /api/dispatch/tickets/{ticket_id}/required-skills body.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReplaceTicketSkillsRequest {
    /// Full replacement set (an empty set clears every requirement).
    pub skills: Vec<String>,
    #[serde(flatten)]
    pub instant: MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TicketSkillsView {
    #[schema(value_type = uuid::Uuid)]
    pub ticket_id: Uuid,
    pub skills: Vec<String>,
}

/// The full-replace mutation envelope (IF2).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TicketSkillsMutationResponse {
    #[serde(flatten)]
    pub required: TicketSkillsView,
    pub mutation: instant::MutationMeta,
}

/// PUT /api/dispatch/tickets/{ticket_id}/required-skills — full-replace
/// the job's required-skill set. The ticket must exist and be open +
/// unassigned (requirements on placed work are history, not planning).
#[utoipa::path(operation_id = "dispatch_ticket_skills_replace", put, path = "/api/dispatch/tickets/{ticket_id}/required-skills", tag = "scheduling", security(("bearer" = [])),
    params(("ticket_id" = Uuid, Path)),
    request_body = ReplaceTicketSkillsRequest,
    responses((status = 200, body = TicketSkillsMutationResponse),
              (status = 404, description = "ticket not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "validation failed (labels, duplicates, count)")))]
pub async fn ticket_skills_replace(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(ticket_id): Path<Uuid>,
    Json(req): Json<ReplaceTicketSkillsRequest>,
) -> ApiResult<Json<TicketSkillsMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::Edit))?;
    let mut seen = std::collections::HashSet::new();
    let mut skills = Vec::with_capacity(req.skills.len());
    for raw in &req.skills {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.len() > SKILL_MAX_LEN {
            return Err(ApiError::Validation(format!(
                "skill labels must be 1..={SKILL_MAX_LEN} characters"
            )));
        }
        if !seen.insert(trimmed.to_string()) {
            return Err(ApiError::Validation(format!("duplicate skill {trimmed:?}")));
        }
        skills.push(trimmed.to_string());
    }
    if skills.len() > REQUIRED_SKILLS_MAX {
        return Err(ApiError::Validation(format!(
            "at most {REQUIRED_SKILLS_MAX} required skills per ticket"
        )));
    }
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "ticket_required_skills",
                entity_id: Some(ticket_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, ctx.org_id, ticket_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if ticket.assignee_id.is_some()
            || ticket.category == core_domain::status::TicketStatusCategory::Closed
        {
            return Err(ApiError::Validation(
                "required skills are planning data: only unassigned, open tickets carry them"
                    .into(),
            ));
        }
        let before =
            core_db::dispatch::list_ticket_required_skills(&mut *ctx.tx, ctx.org_id, &[ticket_id])
                .await?
                .into_iter()
                .map(|(_, skill)| skill)
                .collect::<Vec<_>>();
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "ticket_required_skills",
                entity_id: ticket_id,
                version: req.instant.version,
            },
        )
        .await?;
        let rows = core_db::dispatch::replace_ticket_required_skills(
            &mut ctx.tx,
            ctx.org_id,
            ticket_id,
            &skills,
        )
        .await?;
        let view = TicketSkillsView {
            ticket_id,
            skills: rows.into_iter().map(|(_, skill)| skill).collect(),
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "ticket_required_skills",
                entity_id: ticket_id,
                op: "required_skills_replace",
                seq,
                before: Some(serde_json::json!({ "skills": before })),
                after: serde_json::json!({ "skills": view.skills }),
                updated_at: Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: instant::ProvenanceContext::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TicketSkillsMutationResponse {
                required: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- travel / mileage drafts ------------------------------------------------------

const TRAVEL_DISTANCE_MAX: f64 = 10_000.0;

fn validate_travel_distance(raw: f64) -> ApiResult<f64> {
    if !raw.is_finite() || raw <= 0.0 {
        return Err(ApiError::Validation("distance must be > 0".into()));
    }
    if raw > TRAVEL_DISTANCE_MAX {
        return Err(ApiError::Validation(
            "distance exceeds the 10,000-unit sanity cap".into(),
        ));
    }
    Ok(raw)
}

fn validate_travel_unit(raw: &str) -> ApiResult<String> {
    match raw {
        "km" | "mi" => Ok(raw.to_string()),
        other => Err(ApiError::Validation(format!(
            "distance_unit must be \"km\" or \"mi\", got {other:?}"
        ))),
    }
}

fn validate_travel_vehicle(raw: &str) -> ApiResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 40 {
        return Err(ApiError::Validation(
            "vehicle must be 1..=40 characters".into(),
        ));
    }
    Ok(trimmed.to_string())
}

fn validate_queue_note(raw: Option<&str>) -> ApiResult<Option<String>> {
    match raw {
        None => Ok(None),
        Some(n) => {
            let trimmed = n.trim();
            if trimmed.len() > QUEUE_NOTE_MAX {
                return Err(ApiError::Validation(format!(
                    "note must be at most {QUEUE_NOTE_MAX} characters"
                )));
            }
            Ok((!trimmed.is_empty()).then(|| trimmed.to_string()))
        }
    }
}

fn validate_skill_set(raw: &[SkillInput]) -> ApiResult<Vec<(String, Option<DateTime<Utc>>)>> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(raw.len());
    for input in raw {
        let trimmed = input.skill.trim();
        if trimmed.is_empty() || trimmed.len() > SKILL_MAX_LEN {
            return Err(ApiError::Validation(format!(
                "skill must be 1..={SKILL_MAX_LEN} characters"
            )));
        }
        if !seen.insert(trimmed.to_lowercase()) {
            return Err(ApiError::Validation(format!("duplicate skill {trimmed:?}")));
        }
        if let Some(t) = input.expires_at {
            if t <= Utc::now() {
                return Err(ApiError::Validation(
                    "expires_at must be in the future (delete the skill instead)".into(),
                ));
            }
        }
        out.push((trimmed.to_string(), input.expires_at));
    }
    Ok(out)
}

/// One travel/mileage draft.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TravelDraftView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub ticket_id: Uuid,
    #[schema(value_type = String, format = "date-time")]
    pub started_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub ended_at: DateTime<Utc>,
    pub distance: f64,
    /// `km` | `mi`.
    pub distance_unit: String,
    pub vehicle: String,
    pub billable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// `draft` | `submitted`.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub submitted_time_entry_id: Option<Uuid>,
    /// Wave 32 lane 7: the planned appointment this trip reconciles
    /// against, when linked.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub appointment_id: Option<Uuid>,
    /// Wave 32 lane 7: the expense this draft's mileage produced, once
    /// reimbursed.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub expense_id: Option<Uuid>,
    #[schema(value_type = String, format = "date-time")]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub updated_at: DateTime<Utc>,
}

fn draft_view(row: core_db::dispatch::TravelDraftRow) -> TravelDraftView {
    TravelDraftView {
        id: row.id,
        user_id: row.user_id,
        ticket_id: row.ticket_id,
        started_at: row.started_at,
        ended_at: row.ended_at,
        distance: row.distance,
        distance_unit: row.distance_unit,
        vehicle: row.vehicle,
        billable: row.billable,
        notes: row.notes,
        status: row.status,
        submitted_time_entry_id: row.submitted_time_entry_id,
        appointment_id: row.appointment_id,
        expense_id: row.expense_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// Mutation envelope: draft fields at the top level plus the additive
/// `mutation` metadata (IF2).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TravelDraftMutationResponse {
    #[serde(flatten)]
    pub draft: TravelDraftView,
    pub mutation: instant::MutationMeta,
}

/// POST /api/dispatch/travel-drafts body.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateTravelDraftRequest {
    #[schema(value_type = uuid::Uuid)]
    pub ticket_id: Uuid,
    /// Whose work this is; defaults to the caller. Only owner/admin may
    /// file on someone else's behalf.
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub user_id: Option<Uuid>,
    #[schema(value_type = String, format = "date-time")]
    pub started_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub ended_at: DateTime<Utc>,
    pub distance: f64,
    /// `km` | `mi`.
    pub distance_unit: String,
    pub vehicle: String,
    #[serde(default)]
    pub billable: bool,
    #[serde(default)]
    pub notes: Option<String>,
    /// Wave 32 lane 7: the planned appointment this trip reconciles
    /// against (optional — ad-hoc travel against a ticket stays legal).
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub appointment_id: Option<Uuid>,
    #[serde(flatten)]
    pub instant: MutationParams,
}

/// PATCH /api/dispatch/travel-drafts/{id} body — every member optional;
/// absent members are left untouched.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PatchTravelDraftRequest {
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub user_id: Option<Uuid>,
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "date-time")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "date-time")]
    pub ended_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub distance: Option<f64>,
    #[serde(default)]
    pub distance_unit: Option<String>,
    #[serde(default)]
    pub vehicle: Option<String>,
    #[serde(default)]
    pub billable: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub notes: Option<Option<String>>,
    #[serde(flatten)]
    pub instant: MutationParams,
}

/// Distinguishes `"notes": null` (clear) from an absent member.
fn deserialize_optional_string<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<String>::deserialize(deserializer)?))
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct ListTravelDraftsQuery {
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub user_id: Option<Uuid>,
    /// `draft` | `submitted`.
    pub status: Option<String>,
    /// Page size, clamped to [1, 200].
    pub limit: Option<i64>,
}

/// POST /api/dispatch/travel-drafts — a draft mileage row against a
/// ticket. Instant-first (IF1/IF2): idempotency-keyed, version-gated,
/// write-trailed.
#[utoipa::path(operation_id = "dispatch_travel_draft_create", post, path = "/api/dispatch/travel-drafts", tag = "scheduling", security(("bearer" = [])),
    request_body = CreateTravelDraftRequest,
    responses((status = 200, body = TravelDraftMutationResponse),
              (status = 404, description = "ticket not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "validation failed")))]
pub async fn travel_draft_create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateTravelDraftRequest>,
) -> ApiResult<Json<TravelDraftMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(Action::Create))?;
    let distance = validate_travel_distance(req.distance)?;
    let distance_unit = validate_travel_unit(&req.distance_unit)?;
    let vehicle = validate_travel_vehicle(&req.vehicle)?;
    let notes = validate_travel_notes(req.notes.as_deref())?;
    if req.ended_at <= req.started_at {
        return Err(ApiError::Validation(
            "ended_at must be after started_at".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let owner = match req.user_id {
        None => actor,
        Some(uid) if uid == actor => actor,
        Some(uid) => {
            if !authed.principal.is_owner_or_admin() {
                return Err(ApiError::Forbidden(
                    "only owner/admin may file a draft for another user".into(),
                ));
            }
            uid
        }
    };
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "travel_draft",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // The ticket anchors the eventual time entry; a foreign id is a
        // typed rls_sparse, not a silent orphan draft.
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, ctx.org_id, req.ticket_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        core_db::users::get_user(&mut *ctx.tx, ctx.org_id, owner)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        // A linked planned appointment must be visible in scope and belong
        // to the same technician — reconciliation pairs like with like.
        if let Some(appointment_id) = req.appointment_id {
            let appointment =
                core_db::scheduling::get_appointment(&mut *ctx.tx, ctx.org_id, appointment_id)
                    .await
                    .map_err(|e| instant::mutation_scope_err(e.into()))?;
            if appointment.user_id != owner {
                return Err(ApiError::Validation(
                    "the linked appointment belongs to another technician".into(),
                ));
            }
        }
        let id = Uuid::now_v7();
        let now = Utc::now();
        let row = core_db::dispatch::insert_travel_draft(
            &mut ctx.tx,
            &core_db::dispatch::TravelDraftRow {
                id,
                org_id: ctx.org_id,
                user_id: owner,
                ticket_id: ticket.id,
                started_at: req.started_at,
                ended_at: req.ended_at,
                distance,
                distance_unit: distance_unit.clone(),
                vehicle: vehicle.clone(),
                billable: req.billable,
                notes: notes.clone(),
                status: "draft".into(),
                submitted_time_entry_id: None,
                appointment_id: req.appointment_id,
                expense_id: None,
                created_by: actor,
                created_at: now,
                updated_at: now,
            },
        )
        .await?;
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "travel_draft",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let view = draft_view(row.clone());
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "travel_draft",
                entity_id: id,
                op: "create",
                seq,
                before: None,
                after: serde_json::to_value(&row).unwrap_or(Value::Null),
                updated_at: view.updated_at,
                key: key.as_deref(),
                payload: &view,
                provenance: instant::ProvenanceContext::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TravelDraftMutationResponse {
                draft: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// GET /api/dispatch/travel-drafts — the caller's drafts (all users for
/// owner/admin via `user_id`), newest first.
#[utoipa::path(operation_id = "dispatch_travel_draft_list", get, path = "/api/dispatch/travel-drafts", tag = "scheduling", security(("bearer" = [])),
    params(ListTravelDraftsQuery),
    responses((status = 200, body = [TravelDraftView])))]
pub async fn travel_draft_list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ListTravelDraftsQuery>,
) -> ApiResult<Json<Vec<TravelDraftView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(Action::View))?;
    // Non-admins only see their own drafts (same posture as time entries).
    let user_id = if authed.principal.is_owner_or_admin() {
        q.user_id
    } else {
        Some(authed.principal.user.id)
    };
    let status = q.status.as_deref().filter(|s| !s.is_empty());
    if let Some(s) = status {
        if s != "draft" && s != "submitted" {
            return Err(ApiError::Validation(
                "status must be draft or submitted".into(),
            ));
        }
    }
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let rows = core_db::dispatch::list_travel_drafts(
        &mut *tx,
        org_id,
        user_id,
        status,
        q.limit.unwrap_or(100).clamp(1, TRAVEL_DRAFT_LIMIT),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows.into_iter().map(draft_view).collect()))
}

/// PATCH /api/dispatch/travel-drafts/{id} — edit a draft. Submitted
/// drafts are frozen (the time entry already exists).
#[utoipa::path(operation_id = "dispatch_travel_draft_patch", patch, path = "/api/dispatch/travel-drafts/{id}", tag = "scheduling", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = PatchTravelDraftRequest,
    responses((status = 200, body = TravelDraftMutationResponse),
              (status = 404, description = "not visible, or already submitted"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "validation failed")))]
pub async fn travel_draft_patch(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchTravelDraftRequest>,
) -> ApiResult<Json<TravelDraftMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(Action::Create))?;
    let distance = match req.distance {
        None => None,
        Some(d) => Some(validate_travel_distance(d)?),
    };
    let distance_unit = match &req.distance_unit {
        None => None,
        Some(u) => Some(validate_travel_unit(u)?),
    };
    let vehicle = match &req.vehicle {
        None => None,
        Some(v) => Some(validate_travel_vehicle(v)?),
    };
    let notes = match &req.notes {
        None => None,
        Some(n) => Some(validate_travel_notes(n.as_deref())?),
    };
    if let (Some(s), Some(e)) = (req.started_at, req.ended_at) {
        if e <= s {
            return Err(ApiError::Validation(
                "ended_at must be after started_at".into(),
            ));
        }
    }
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "travel_draft",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::dispatch::get_travel_draft(&mut *ctx.tx, ctx.org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "travel_draft",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let row = core_db::dispatch::patch_travel_draft(
            &mut ctx.tx,
            ctx.org_id,
            id,
            &core_db::dispatch::TravelDraftPatch {
                user_id: req.user_id,
                ticket_id: req.ticket_id,
                started_at: req.started_at,
                ended_at: req.ended_at,
                distance,
                distance_unit,
                vehicle,
                billable: req.billable,
                notes,
            },
        )
        .await
        .map_err(|e| match e {
            core_db::DbError::NotFound => ApiError::RlsSparse,
            other => other.into(),
        })?;
        let view = draft_view(row.clone());
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "travel_draft",
                entity_id: id,
                op: "update",
                seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&row).unwrap_or(Value::Null),
                updated_at: view.updated_at,
                key: key.as_deref(),
                payload: &view,
                provenance: instant::ProvenanceContext::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TravelDraftMutationResponse {
                draft: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/dispatch/travel-drafts/{id} — discard a draft. Submitted
/// drafts are immutable history (delete the TIME ENTRY through invoicing
/// corrections, never the draft's backlink).
#[utoipa::path(operation_id = "dispatch_travel_draft_delete", delete, path = "/api/dispatch/travel-drafts/{id}", tag = "scheduling", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = TravelDraftMutationResponse),
              (status = 404, description = "not visible, or already submitted")))]
pub async fn travel_draft_delete(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<TravelDraftMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;

    let outcome = org_tx::with_org_tx(&state, org_id, actor, |mut ctx| async move {
        let before = core_db::dispatch::get_travel_draft(&mut *ctx.tx, ctx.org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "travel_draft",
                entity_id: id,
                version: None,
            },
        )
        .await?;
        core_db::dispatch::delete_travel_draft(&mut ctx.tx, ctx.org_id, id)
            .await
            .map_err(|e| match e {
                core_db::DbError::NotFound => ApiError::RlsSparse,
                other => other.into(),
            })?;
        let view = draft_view(before.clone());
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "travel_draft",
                entity_id: id,
                op: "delete",
                seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::json!({ "deleted": true }),
                updated_at: view.updated_at,
                key: None,
                payload: &view,
                provenance: instant::ProvenanceContext::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TravelDraftMutationResponse {
                draft: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/dispatch/travel-drafts/{id}/submit body.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SubmitTravelDraftRequest {
    #[serde(flatten)]
    pub instant: MutationParams,
}

/// POST /api/dispatch/travel-drafts/{id}/submit — convert a draft into a
/// real time entry through the EXISTING time-entry path
/// (`insert_time_entry` + the `time_entry.created` event), then freeze
/// the draft with the entry backlink. The entry's notes carry the
/// machine-readable provenance marker (`travel_draft:<id>`); the write
/// trail records the full before/after so "who did this" and undo stay
/// queryable. Idempotent: the same submit replays the SAME entry.
#[utoipa::path(operation_id = "dispatch_travel_draft_submit", post, path = "/api/dispatch/travel-drafts/{id}/submit", tag = "scheduling", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = SubmitTravelDraftRequest,
    responses((status = 200, body = crate::routes::timers::TimeEntryMutationResponse),
              (status = 404, description = "draft not visible in scope (rls_sparse)"),
              (status = 409, description = "already submitted, version conflict, or idempotency key reused")))]
pub async fn travel_draft_submit(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<SubmitTravelDraftRequest>,
) -> ApiResult<Json<crate::routes::timers::TimeEntryMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "travel_draft",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let draft = core_db::dispatch::get_travel_draft(&mut *ctx.tx, ctx.org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if draft.status != "draft" {
            return Err(ApiError::Conflict(
                "draft has already been submitted".into(),
            ));
        }
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "travel_draft",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;

        // The conversion: a REAL time entry through the existing path.
        // work_type "travel" keeps mileage entries filterable; notes
        // carry the draft backlink so the entry explains itself without
        // joining anything.
        let minutes = (draft.ended_at - draft.started_at).num_minutes().max(0);
        let travel_note = format!(
            "Travel: {:.1} {} — {} (travel_draft:{id})",
            draft.distance, draft.distance_unit, draft.vehicle
        );
        let entry = core_domain::entities::TimeEntry {
            approval_status: Some(core_domain::entities::ApprovalStatus::Draft),
            approval_reason_code: None,
            unbillable_reason: None,
            id: Uuid::now_v7(),
            org_id: ctx.org_id,
            ticket_id: Some(draft.ticket_id),
            phase_id: None,
            user_id: draft.user_id,
            started_at: draft.started_at,
            ended_at: draft.ended_at,
            minutes,
            billable: draft.billable,
            notes: Some(match &draft.notes {
                Some(n) => format!("{n} · {travel_note}"),
                None => travel_note,
            }),
            work_type: Some("travel".into()),
            timer_session_id: None,
            created_at: Utc::now(),
        };
        core_db::timers::insert_time_entry(&mut *ctx.tx, &entry).await?;
        let submitted =
            core_db::dispatch::mark_travel_draft_submitted(&mut ctx.tx, ctx.org_id, id, entry.id)
                .await
                .map_err(|e| match e {
                    core_db::DbError::NotFound => ApiError::RlsSparse,
                    other => other.into(),
                })?;

        ctx.emit(
            types::TIME_ENTRY_CREATED,
            "time_entry",
            Some(entry.id.to_string().as_str()),
            &serde_json::to_value(TimeEntryCreated {
                entry: entry.clone(),
            })
            .map_err(|e| ApiError::Internal(e.into()))?,
        )
        .await?;
        ctx.emit(
            types::TRAVEL_DRAFT_SUBMITTED,
            "travel_draft",
            Some(id.to_string().as_str()),
            &serde_json::to_value(TravelDraftSubmitted {
                draft_id: id,
                ticket_id: draft.ticket_id,
                user_id: draft.user_id,
                time_entry_id: entry.id,
                distance: draft.distance,
                distance_unit: draft.distance_unit.clone(),
                vehicle: draft.vehicle.clone(),
            })
            .map_err(|e| ApiError::Internal(e.into()))?,
        )
        .await?;

        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "travel_draft",
                entity_id: id,
                op: "travel_draft_submit",
                seq,
                before: Some(serde_json::to_value(&draft).unwrap_or(Value::Null)),
                after: serde_json::to_value(&submitted).unwrap_or(Value::Null),
                updated_at: entry.ended_at,
                key: key.as_deref(),
                payload: &entry,
                provenance: instant::ProvenanceContext::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(crate::routes::timers::TimeEntryMutationResponse {
                entry,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- shared helpers ----------------------------------------------------------------

fn validate_travel_notes(raw: Option<&str>) -> ApiResult<Option<String>> {
    match raw {
        None => Ok(None),
        Some(n) if n.len() > TRAVEL_NOTE_MAX => Err(ApiError::Validation(format!(
            "notes must be at most {TRAVEL_NOTE_MAX} characters"
        ))),
        Some(n) => Ok(Some(n.to_string())),
    }
}

// --- curated per-technician work-next queue (Wave 34 claims close-out) -----

/// One row of a technician's curated queue, as the board renders it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkNextEntryView {
    pub id: Uuid,
    pub ticket_id: Uuid,
    pub position: i32,
    pub ticket_number: String,
    pub title: String,
    pub priority: String,
    pub status: String,
    pub client_id: Option<Uuid>,
    pub client_name: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// The whole queue for one technician, in curated order.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkNextQueueView {
    pub user_id: Uuid,
    /// The technician whose queue this is, resolved server-side from the
    /// authenticated principal unless an admin asks for someone else's.
    pub display_name: String,
    pub entries: Vec<WorkNextEntryView>,
}

#[derive(Debug, Clone, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct WorkNextQueueQuery {
    /// Whose queue to read. Admins may read another technician's (a
    /// dispatcher setting up a shift); anyone may read their own, and a
    /// non-admin asking for someone else is refused rather than silently
    /// served their own.
    pub user_id: Option<Uuid>,
}

/// GET /api/dispatch/work-next — the caller's curated queue, or another
/// technician's when the caller may see theirs.
#[utoipa::path(operation_id = "dispatch_work_next_list", get,
    path = "/api/dispatch/work-next", tag = "scheduling", security(("bearer" = [])),
    params(WorkNextQueueQuery),
    responses((status = 200, body = WorkNextQueueView)))]
pub async fn work_next_list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<WorkNextQueueQuery>,
) -> ApiResult<Json<WorkNextQueueView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let me = authed.principal.user.id;

    // Reading ANOTHER technician's curated queue is a dispatch action, not a
    // personal one: the order is a dispatcher's instruction to someone
    // else's day. So it needs the dispatch permission, while reading your
    // own only needs the scheduling view you already have.
    let target = match q.user_id {
        Some(id) if id != me => {
            require_permission(&authed.principal, sched(Action::Edit))?;
            id
        }
        _ => me,
    };

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, me)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::dispatch::list_work_next_queue(&mut *tx, org_id, target).await?;
    let display_name = core_db::users::list_users(&mut *tx, org_id)
        .await?
        .into_iter()
        .find(|u| u.id == target)
        .map(|u| u.display_name)
        .unwrap_or_default();
    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(WorkNextQueueView {
        user_id: target,
        display_name,
        entries: rows
            .into_iter()
            .map(|r| WorkNextEntryView {
                id: r.id,
                ticket_id: r.ticket_id,
                position: r.position,
                ticket_number: r.ticket_number,
                title: r.ticket_title,
                priority: r.ticket_priority,
                status: r.ticket_status,
                client_id: r.client_id,
                client_name: r.client_name,
                created_at: r.created_at.to_rfc3339(),
                updated_at: r.updated_at.to_rfc3339(),
            })
            .collect(),
    }))
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct WorkNextMutation {
    pub user_id: Uuid,
    pub ticket_id: Uuid,
    /// Target rank. Clamped server-side to the queue's bounds: dragging past
    /// the end is a gesture, not a client bug.
    pub position: i32,
}

/// PUT /api/dispatch/work-next — add a ticket to a technician's queue at a
/// position, or move it there if the pair already exists.
#[utoipa::path(operation_id = "dispatch_work_next_add", put,
    path = "/api/dispatch/work-next", tag = "scheduling", security(("bearer" = [])),
    request_body = WorkNextMutation,
    responses((status = 200, body = WorkNextEntryView)))]
pub async fn work_next_add(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<WorkNextMutation>,
) -> ApiResult<Json<WorkNextEntryView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let me = authed.principal.user.id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, me)
        .await
        .map_err(ApiError::from)?;
    let row = core_db::dispatch::add_work_next_entry(
        &mut tx,
        Uuid::now_v7(),
        org_id,
        req.user_id,
        req.ticket_id,
        req.position,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(WorkNextEntryView {
        id: row.id,
        ticket_id: row.ticket_id,
        position: row.position,
        ticket_number: String::new(),
        title: String::new(),
        priority: String::new(),
        status: String::new(),
        client_id: None,
        client_name: None,
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
    }))
}

/// POST /api/dispatch/work-next/reorder — move an entry and return the
/// densified queue.
///
/// Returns the WHOLE queue rather than just the moved row because the move
/// shifts everything between the old and new rank. A response with only the
/// moved entry would leave the client guessing the positions of everything
/// it did not drag, which is exactly the state a drag-and-drop list must not
/// be in.
#[utoipa::path(operation_id = "dispatch_work_next_reorder", post,
    path = "/api/dispatch/work-next/reorder", tag = "scheduling",
    security(("bearer" = [])), request_body = WorkNextMutation,
    responses((status = 200, body = Vec<WorkNextEntryView>)))]
pub async fn work_next_reorder(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<WorkNextMutation>,
) -> ApiResult<Json<Vec<WorkNextEntryView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let me = authed.principal.user.id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, me)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::dispatch::reorder_work_next_entry(
        &mut tx,
        org_id,
        req.user_id,
        req.ticket_id,
        req.position,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        rows.into_iter()
            .map(|r| WorkNextEntryView {
                id: r.id,
                ticket_id: r.ticket_id,
                position: r.position,
                ticket_number: String::new(),
                title: String::new(),
                priority: String::new(),
                status: String::new(),
                client_id: None,
                client_name: None,
                created_at: r.created_at.to_rfc3339(),
                updated_at: r.updated_at.to_rfc3339(),
            })
            .collect(),
    ))
}

/// DELETE /api/dispatch/work-next?ticket_id=… — drop a ticket from one
/// technician's queue, or from every queue when `user_id` is absent.
#[utoipa::path(operation_id = "dispatch_work_next_remove", delete,
    path = "/api/dispatch/work-next", tag = "scheduling", security(("bearer" = [])),
    params(("ticket_id" = Uuid, Query), ("user_id" = Option<Uuid>, Query)),
    responses((status = 200, body = serde_json::Value)))]
pub async fn work_next_remove(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<WorkNextRemovalQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let me = authed.principal.user.id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, me)
        .await
        .map_err(ApiError::from)?;
    let removed =
        core_db::dispatch::remove_work_next_entry(&mut tx, org_id, q.user_id, q.ticket_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({ "removed": removed })))
}

#[derive(Debug, Clone, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct WorkNextRemovalQuery {
    pub ticket_id: Uuid,
    /// Absent means "every technician's queue", which is what a placed or
    /// closed ticket needs: its queue entries are stale everywhere at once.
    pub user_id: Option<Uuid>,
}

// --- resource planning: phase allocations + capacity forecast -------------

/// One stated allocation of a technician to a project phase.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PhaseAllocationView {
    pub id: Uuid,
    pub phase_id: Uuid,
    pub user_id: Uuid,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub share_percent: i32,
    pub note: Option<String>,
    pub phase_name: String,
    pub phase_status: String,
    pub project_id: Uuid,
    pub project_name: String,
    pub user_display_name: String,
}

/// Every allocation in the org, or in one project.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PhaseAllocationListView {
    pub allocations: Vec<PhaseAllocationView>,
}

#[derive(Debug, Clone, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct PhaseAllocationQuery {
    pub project_id: Option<Uuid>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PhaseAllocationMutation {
    pub phase_id: Uuid,
    pub user_id: Uuid,
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    /// 1..=100, CHECK-enforced in the database. A 0 or a 150 entering the
    /// forecast would corrupt every capacity number on the page, so the
    /// bound is enforced in the schema rather than only in this handler.
    pub share_percent: i32,
    pub note: Option<String>,
}

/// GET /api/dispatch/planning/allocations — who is committed to which phase.
#[utoipa::path(operation_id = "dispatch_phase_allocations_list", get,
    path = "/api/dispatch/planning/allocations", tag = "scheduling",
    security(("bearer" = [])), params(PhaseAllocationQuery),
    responses((status = 200, body = PhaseAllocationListView)))]
pub async fn phase_allocations_list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<PhaseAllocationQuery>,
) -> ApiResult<Json<PhaseAllocationListView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let org_id = authed.principal.user.org_id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::dispatch::list_phase_allocations(&mut *tx, org_id, q.project_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(PhaseAllocationListView {
        allocations: rows
            .into_iter()
            .map(|r| PhaseAllocationView {
                id: r.id,
                phase_id: r.phase_id,
                user_id: r.user_id,
                start_date: r.start_date.map(|d| d.to_string()),
                end_date: r.end_date.map(|d| d.to_string()),
                share_percent: r.share_percent,
                note: r.note,
                phase_name: r.phase_name,
                phase_status: r.phase_status,
                project_id: r.project_id,
                project_name: r.project_name,
                user_display_name: r.user_display_name,
            })
            .collect(),
    }))
}

/// PUT /api/dispatch/planning/allocations — state (or restate) a
/// technician's commitment to a phase.
///
/// Upsert on `(phase_id, user_id)`, so restating a share updates the row.
/// Appending instead would leave two contradictory shares that SUM in the
/// forecast and report a technician as over-committed by construction.
#[utoipa::path(operation_id = "dispatch_phase_allocation_upsert", put,
    path = "/api/dispatch/planning/allocations", tag = "scheduling",
    security(("bearer" = [])), request_body = PhaseAllocationMutation,
    responses((status = 200, body = PhaseAllocationView)))]
pub async fn phase_allocation_upsert(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PhaseAllocationMutation>,
) -> ApiResult<Json<PhaseAllocationView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let me = authed.principal.user.id;

    // Validate before the write so the operator gets a 422 naming the bound
    // rather than a constraint-violation 500 from the database.
    if !(1..=100).contains(&req.share_percent) {
        return Err(ApiError::Validation(format!(
            "share_percent must be 1..=100, got {}",
            req.share_percent
        )));
    }
    if let (Some(s), Some(e)) = (req.start_date, req.end_date) {
        if e < s {
            return Err(ApiError::Validation(
                "end_date must not precede start_date".into(),
            ));
        }
    }

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, me)
        .await
        .map_err(ApiError::from)?;
    let row = core_db::dispatch::upsert_phase_allocation(
        &mut tx,
        core_db::dispatch::NewPhaseAllocation {
            id: Uuid::now_v7(),
            org_id,
            phase_id: req.phase_id,
            user_id: req.user_id,
            start_date: req.start_date,
            end_date: req.end_date,
            share_percent: req.share_percent,
            note: req.note.as_deref(),
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    // Read the joined view back so the response carries phase/project/user
    // names rather than making the client re-fetch to render one row.
    let mut t2 = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut t2, org_id, me)
        .await
        .map_err(ApiError::from)?;
    let joined = core_db::dispatch::list_phase_allocations(&mut *t2, org_id, None).await?;
    t2.commit().await.map_err(ApiError::from)?;
    let view = joined
        .into_iter()
        .find(|a| a.id == row.id)
        .map(|a| PhaseAllocationView {
            id: a.id,
            phase_id: a.phase_id,
            user_id: a.user_id,
            start_date: a.start_date.map(|d| d.to_string()),
            end_date: a.end_date.map(|d| d.to_string()),
            share_percent: a.share_percent,
            note: a.note,
            phase_name: a.phase_name,
            phase_status: a.phase_status,
            project_id: a.project_id,
            project_name: a.project_name,
            user_display_name: a.user_display_name,
        })
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("allocation vanished after write")))?;
    Ok(Json(view))
}

/// DELETE /api/dispatch/planning/allocations/{id} — release a commitment.
#[utoipa::path(operation_id = "dispatch_phase_allocation_delete", delete,
    path = "/api/dispatch/planning/allocations/{allocation_id}", tag = "scheduling",
    security(("bearer" = [])),
    responses((status = 200, body = serde_json::Value),
              (status = 404, description = "no such allocation")))]
pub async fn phase_allocation_delete(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(allocation_id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let me = authed.principal.user.id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, me)
        .await
        .map_err(ApiError::from)?;
    let removed =
        core_db::dispatch::delete_phase_allocation(&mut tx, org_id, allocation_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    if !removed {
        return Err(ApiError::NotFound);
    }
    Ok(Json(serde_json::json!({ "removed": true })))
}

/// One technician's line in the forecast: committed project load beside
/// booked appointments.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CapacityForecastEntryView {
    pub user_id: Uuid,
    pub display_name: String,
    /// Committed project load in PERCENT-DAYS. 200.0 = "100% of one
    /// technician's time for two days", or two phases at 100% for one day.
    /// A plain percentage would flatten a month-long 20% plan and a
    /// week-long 80% plan into the same number, and they are not the same
    /// commitment.
    pub committed_percent_days: f64,
    pub allocation_count: i64,
    pub phase_count: i64,
    /// The window's total available percent-days for this technician, from
    /// their availability windows minus calendar exclusions. Zero means the
    /// technician has no declared availability in the window, which is a
    /// different state from being fully booked and must not render as 100%.
    pub available_percent_days: f64,
    /// `committed / available`, or `None` when availability is zero. A
    /// ratio with a zero denominator is the classic way a forecast reports
    /// "infinite overload" for a technician nobody scheduled.
    pub utilization: Option<f64>,
    pub over_allocated: bool,
}

/// The forecast-shaped read model: committed project load per technician
/// across a window, which is what `/dispatch/capacity` could never show
/// because it only knew about appointments.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CapacityForecastView {
    pub from: String,
    pub to: String,
    pub days: i64,
    pub entries: Vec<CapacityForecastEntryView>,
}

/// GET /api/dispatch/planning/forecast — committed project load per
/// technician over a window.
#[utoipa::path(operation_id = "dispatch_capacity_forecast", get,
    path = "/api/dispatch/planning/forecast", tag = "scheduling",
    security(("bearer" = [])), params(CapacityQuery),
    responses((status = 200, body = CapacityForecastView)))]
pub async fn capacity_forecast(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<CapacityQuery>,
) -> ApiResult<Json<CapacityForecastView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let me = authed.principal.user.id;

    let days = q.days.unwrap_or(14).clamp(1, CAPACITY_MAX_DAYS);
    let first = q.from.unwrap_or_else(|| Utc::now().date_naive());
    let window_from = first;
    let window_to = first + Duration::days(i64::from(days) - 1);

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, me)
        .await
        .map_err(ApiError::from)?;
    let members = core_db::users::list_users(&mut *tx, org_id).await?;
    let technicians: Vec<Uuid> = members
        .iter()
        .filter(|u| u.is_active)
        .map(|u| u.id)
        .collect();
    let loads = core_db::dispatch::capacity_forecast(
        &mut *tx,
        org_id,
        &technicians,
        window_from,
        window_to,
    )
    .await?;

    // Declared availability, so the forecast can say "40% committed" rather
    // than a bare percent-day number that only means something relative to
    // a capacity this endpoint would otherwise have to invent.
    let win_from_utc = window_from
        .and_hms_opt(0, 0, 0)
        .expect("midnight is valid")
        .and_utc();
    let win_to_utc = (window_to + Duration::days(1))
        .and_hms_opt(0, 0, 0)
        .expect("midnight is valid")
        .and_utc();
    let windows = core_db::scheduling::list_all_availability_windows(&mut *tx, org_id).await?;
    let exclusions = core_db::scheduling::list_calendar_exclusions_for_users_between(
        &mut *tx,
        org_id,
        &technicians,
        win_from_utc,
        win_to_utc,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;

    let mut entries = Vec::with_capacity(technicians.len());
    for user in members.iter().filter(|u| u.is_active) {
        let load = loads.iter().find(|l| l.user_id == user.id);
        let committed = load.map_or(0.0, |l| l.percent_days);
        let user_windows: Vec<_> = windows
            .iter()
            .filter(|w| w.user_id == user.id)
            .cloned()
            .collect();
        let user_exclusions: Vec<_> = exclusions
            .iter()
            .filter(|x| x.user_id == user.id)
            .cloned()
            .collect();
        // `covered_minutes` is the SAME helper the capacity horizon uses, and
        // it is the only thing that correctly expands a recurring weekly
        // window across a date span and subtracts exclusions. Hand-rolling
        // that arithmetic here (an earlier attempt treated a window as a
        // date RANGE, which it is not) is how a forecast ends up reporting
        // a technician as available for 0 minutes while the board shows
        // them bookable all week.
        //
        // The denominator is a full-time standard day, not the technician's
        // own hours: a 60%-committed technician on 4-hour days is not 100%
        // committed. 100% = one standard day per calendar day in the window,
        // so the ratio means "share of a normal full-time load" and is
        // comparable across technicians with different part-time patterns.
        const STANDARD_DAY_MINUTES: f64 = 480.0;
        let available_minutes = core_domain::scheduling::covered_minutes(
            &user_windows,
            &user_exclusions,
            win_from_utc,
            win_to_utc,
        ) as f64;
        let available_percent_days = available_minutes / STANDARD_DAY_MINUTES * 100.0;
        let utilization =
            (available_percent_days > 0.0).then(|| committed / available_percent_days);
        entries.push(CapacityForecastEntryView {
            user_id: user.id,
            display_name: user.display_name.clone(),
            committed_percent_days: committed,
            allocation_count: load.map_or(0, |l| l.allocation_count),
            phase_count: load.map_or(0, |l| l.phase_count),
            available_percent_days,
            utilization,
            over_allocated: utilization.is_some_and(|u| u > 1.0),
        });
    }
    // Most-loaded first: a forecast read top-down is a triage list, and the
    // technician who is over-committed is the one a planner opens this to
    // find.
    entries.sort_by(|a, b| {
        b.committed_percent_days
            .partial_cmp(&a.committed_percent_days)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(Json(CapacityForecastView {
        from: window_from.to_string(),
        to: window_to.to_string(),
        days: i64::from(days),
        entries,
    }))
}
