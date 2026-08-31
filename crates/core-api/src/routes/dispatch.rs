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
use crate::org_tx;
use crate::routes::board_access::authorize_board_op;
use crate::routes::scheduling::{sched, validate_span};
use crate::routes::tickets::MyWorkItem;
use crate::state::AppState;
use axum::{
    extract::{Query, State},
    http::Extensions,
    Json,
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use core_domain::entities::{Appointment, AppointmentKind, AppointmentStatus};
use core_domain::events::{types, AppointmentCreated, AppointmentUpdated};
use core_domain::policy::{Action, BoardOp, Module, Permission};
use core_domain::scheduling::{
    booked_minutes, capacity_percent, conflict_indices, covered_minutes,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Tickets-module permission shorthand for the assignment leg.
fn tickets_op(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Tickets,
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

    let members = core_db::users::list_users(&state.pool, org_id).await?;
    let technicians: Vec<Uuid> = members
        .iter()
        .filter(|u| u.is_active)
        .map(|u| u.id)
        .collect();
    let (appointments, on_call) = tokio::try_join!(
        async {
            let rows = core_db::scheduling::list_dispatch_appointments_between(
                &state.pool,
                org_id,
                &technicians,
                from,
                to,
            )
            .await?;
            Ok::<_, ApiError>(rows)
        },
        async {
            let rows = core_db::scheduling::list_on_call_blocks_between(
                &state.pool,
                org_id,
                &technicians,
                from,
                to,
            )
            .await?;
            Ok::<_, ApiError>(rows)
        },
    )?;

    let mut views = Vec::with_capacity(technicians.len());
    for user in members.iter().filter(|u| u.is_active) {
        let windows =
            core_db::scheduling::list_availability_windows(&state.pool, org_id, user.id).await?;
        let exclusions = core_db::scheduling::list_calendar_exclusions_between(
            &state.pool,
            org_id,
            user.id,
            from,
            to,
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
            });
        }
        let booked = booked_minutes(
            user_appointments
                .iter()
                .map(|a: &DispatchAppointment| (a.starts_at, a.ends_at)),
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

    let unassigned = core_db::tickets::unassigned_queue_annotated(&state.pool, org_id, QUEUE_LIMIT)
        .await?
        .into_iter()
        .map(MyWorkItem::from_row)
        .collect();

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
