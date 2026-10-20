//! Dispatch connector API v1 (the private build tracker Phase 3, Wave 9 lane 9): the
//! machine-scoped surface external field-service systems integrate with.
//!
//! Purpose: let a third-party dispatch/GPS/telematics system READ the
//! org's schedule (bookings with technician, ticket, client coordinates)
//! and PUSH field-service status updates (en_route, arrived,
//! in_progress, completed, blocked, delayed) against one booking.
//!
//! Gating mirrors the D3 migration connector's contract with its own
//! separate scope family: machine keys need `dispatch:write` to push and
//! `dispatch:read` (or `write`) to read; humans must be org owner/admin.
//! Every request is rate-limited per principal (fixed one-minute window,
//! `Retry-After` on 429), with its own config knob.
//!
//! Provenance: a pushed status is an IF-envelope write — the idempotency
//! claim, the append-only `dispatch_status_updates` ledger row, the
//! `dispatch.status_updated` event, and the write-trail row all commit
//! atomically, with the trail recording actor_kind `connector` keyed by
//! the api-key id. The appointment's own lifecycle status is untouched:
//! the ledger is the connector-visible ground truth, and the staff day
//! view shades blocks from it additively.
//!
//! Non-purpose: no booking creation/rescheduling (the staff dispatch
//! surface owns mutations to the calendar), no per-key audit dashboards
//! (the ledger + write trail + audit are the provenance queries), no
//! fan-out delivery of statuses to the connector (poll now, push later).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome, MutationParams};
use crate::org_tx;
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use core_domain::entities::AppointmentStatus;
use core_domain::events::{types, DispatchStatusUpdated};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Instant;
use utoipa::ToSchema;
use uuid::Uuid;

// --- rate limiting ----------------------------------------------------------------

/// Fixed rate-limit window for this surface only (separate bucket family
/// from the D3 limiter — the two connectors have different demand).
const RATE_WINDOW_SECS: u64 = 60;

struct RateWindow {
    started: Instant,
    count: u32,
}

static RATE_WINDOWS: OnceLock<std::sync::Mutex<HashMap<Uuid, RateWindow>>> = OnceLock::new();

/// Per-principal fixed-window limiter. `limit == 0` disables limiting
/// (documented escape hatch); the breach carries the seconds until the
/// window resets (429 body + `Retry-After` header).
fn check_rate_limit(limit_per_min: u32, principal: Uuid) -> ApiResult<()> {
    if limit_per_min == 0 {
        return Ok(());
    }
    let buckets = RATE_WINDOWS.get_or_init(Default::default);
    let mut map = buckets
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let now = Instant::now();
    let window = map.entry(principal).or_insert(RateWindow {
        started: now,
        count: 0,
    });
    if now.duration_since(window.started).as_secs() >= RATE_WINDOW_SECS {
        window.started = now;
        window.count = 0;
    }
    window.count += 1;
    if window.count > limit_per_min {
        let elapsed = now.duration_since(window.started).as_secs();
        let retry_after = RATE_WINDOW_SECS.saturating_sub(elapsed).max(1);
        return Err(ApiError::RateLimited {
            retry_after_secs: i64::try_from(retry_after).unwrap_or(i64::MAX),
        });
    }
    Ok(())
}

// --- gating -----------------------------------------------------------------------

/// Machine keys need the Scheduling-matrix grants their `dispatch:*`
/// scopes resolve onto (read = view; write = view + create + edit — the
/// push writes against the booking calendar's module). Humans must be
/// org owner/admin, exactly like the D3 connector gate.
fn require_dispatch_gate(principal: &crate::auth::AuthedUser, write: bool) -> ApiResult<()> {
    let sched = |a: Action| Permission::Matrix {
        module: Module::Scheduling,
        action: a,
    };
    if principal.via_api_key {
        require_permission(&principal.principal, sched(Action::View))?;
        if write {
            require_permission(&principal.principal, sched(Action::Create))?;
            require_permission(&principal.principal, sched(Action::Edit))?;
        }
        return Ok(());
    }
    if principal.principal.is_owner_or_admin() {
        return Ok(());
    }
    Err(ApiError::Forbidden(
        "dispatch connector is limited to org admins or dispatch-scoped keys".into(),
    ))
}

// --- wire types ---------------------------------------------------------------------

/// GET /api/dispatch-connector/schedule query. The window is bounded at
/// 62 days so one pull cannot scan the whole calendar.
#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct ConnectorScheduleQuery {
    #[schema(value_type = Option<String>, format = "date")]
    pub from: Option<chrono::NaiveDate>,
    /// Window length in days (1..=62, default 7).
    pub days: Option<u32>,
}

/// One booking as the connector sees it.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConnectorAppointment {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub technician_user_id: Uuid,
    pub technician_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub client_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    pub title: String,
    #[schema(value_type = String, format = "date-time")]
    pub starts_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub ends_at: DateTime<Utc>,
    pub kind: String,
    pub status: String,
    pub travel_minutes: i32,
    /// Latest pushed field-service status, when any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatch_status: Option<ConnectorStatusView>,
}

/// The latest status push for one booking.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConnectorStatusView {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// `connector` | `staff`.
    pub source: String,
    #[schema(value_type = String, format = "date-time")]
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ConnectorScheduleView {
    #[schema(value_type = String, format = "date")]
    pub from: chrono::NaiveDate,
    pub days: u32,
    pub appointments: Vec<ConnectorAppointment>,
}

/// GET /api/dispatch-connector/technicians entry.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConnectorTechnician {
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    pub display_name: String,
    /// Current (non-expired) skills — lets the external planner respect
    /// certifications before proposing a dispatch.
    pub skills: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ConnectorTechniciansView {
    pub technicians: Vec<ConnectorTechnician>,
}

/// POST /api/dispatch-connector/appointments/{id}/status body.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PushStatusRequest {
    /// Field-service status: en_route | arrived | in_progress | completed
    /// | blocked | delayed.
    pub status: String,
    #[serde(default)]
    pub note: Option<String>,
    /// When the status was observed; defaults to now (server clock).
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "date-time")]
    pub occurred_at: Option<DateTime<Utc>>,
    #[serde(flatten)]
    pub instant: MutationParams,
}

/// The pushed row (entity fields; the mutation envelope flattens it).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PushStatusResponse {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub appointment_id: Uuid,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub source: String,
    #[schema(value_type = String, format = "date-time")]
    pub occurred_at: DateTime<Utc>,
}

/// 201 body: the pushed row plus the read-your-write metadata (IF2).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PushStatusMutationResponse {
    #[serde(flatten)]
    pub pushed: PushStatusResponse,
    pub mutation: instant::MutationMeta,
}

/// The connector status vocabulary (mirrors the DB CHECK).
const STATUSES: &[&str] = &[
    "en_route",
    "arrived",
    "in_progress",
    "completed",
    "blocked",
    "delayed",
];

// --- router ------------------------------------------------------------------------

/// The dispatch-connector slice of the `/api` surface, built per
/// generation so both mounts register identical routes through one merge
/// line. Key acceptance rides the `API_KEY_PATHS` allowlist
/// (`/api/dispatch-connector`).
pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/dispatch-connector/schedule"),
            get(schedule),
        )
        .route(
            &format!("{base}/dispatch-connector/technicians"),
            get(technicians),
        )
        .route(
            &format!("{base}/dispatch-connector/appointments/{{id}}/status"),
            post(push_status),
        )
}

// --- handlers ---------------------------------------------------------------------

/// GET /api/dispatch-connector/schedule — bookings in a bounded window
/// with technician/ticket/client coordinates and the latest pushed
/// field-service status per booking.
#[utoipa::path(operation_id = "dispatch_connector_schedule", get, path = "/api/dispatch-connector/schedule", tag = "dispatch-connector", security(("bearer" = []), ("api_key" = [])),
    params(ConnectorScheduleQuery),
    responses((status = 200, body = ConnectorScheduleView),
              (status = 403, description = "missing dispatch:read scope (keys) or not an org admin (humans)")))]
pub async fn schedule(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ConnectorScheduleQuery>,
) -> ApiResult<Json<ConnectorScheduleView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_dispatch_gate(&authed, false)?;
    check_rate_limit(
        state.config.dispatch_rate_limit_per_min,
        authed.principal.user.id,
    )?;
    let org_id = authed.principal.user.org_id;

    let days = q.days.unwrap_or(7).clamp(1, 62);
    let from_date = q.from.unwrap_or_else(|| Utc::now().date_naive());
    let from = from_date
        .and_hms_opt(0, 0, 0)
        .expect("midnight is a valid local time")
        .and_utc();
    let to = from + chrono::Duration::days(i64::from(days));

    // RLS-scoped read (pool carries no app.org_id GUC): one tx around
    // the whole multi-query body.
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
    let names: HashMap<Uuid, String> = members
        .iter()
        .map(|u| (u.id, u.display_name.clone()))
        .collect();

    let rows = core_db::scheduling::list_dispatch_appointments_between(
        &mut *tx,
        org_id,
        &technicians,
        from,
        to,
    )
    .await?;
    let ids: Vec<Uuid> = rows.iter().map(|a| a.id).collect();
    let latest = core_db::dispatch::latest_status_by_appointment(&mut *tx, org_id, &ids)
        .await?
        .into_iter()
        .map(|row| (row.appointment_id, row))
        .collect::<HashMap<Uuid, _>>();

    let mut appointments = Vec::with_capacity(rows.len());
    for a in rows {
        let shade = latest.get(&a.id).map(|row| ConnectorStatusView {
            status: row.status.clone(),
            note: row.note.clone(),
            source: row.source.clone(),
            occurred_at: row.occurred_at,
        });
        // The booking's owner is resolved even if the technician was
        // deactivated after the booking was made (schedule is truth).
        let technician_name = names
            .get(&a.user_id)
            .cloned()
            .unwrap_or_else(|| "inactive technician".into());
        appointments.push(ConnectorAppointment {
            id: a.id,
            technician_user_id: a.user_id,
            technician_name,
            ticket_id: a.ticket_id,
            ticket_number: a.ticket_number,
            client_id: a.client_id,
            client_name: a.client_name,
            title: a.title,
            starts_at: a.starts_at,
            ends_at: a.ends_at,
            kind: a.kind,
            status: a.status,
            travel_minutes: a.travel_minutes,
            dispatch_status: shade,
        });
    }
    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(ConnectorScheduleView {
        from: from_date,
        days,
        appointments,
    }))
}

/// GET /api/dispatch-connector/technicians — active technicians with
/// their current (non-expired) skills.
#[utoipa::path(operation_id = "dispatch_connector_technicians", get, path = "/api/dispatch-connector/technicians", tag = "dispatch-connector", security(("bearer" = []), ("api_key" = [])),
    responses((status = 200, body = ConnectorTechniciansView),
              (status = 403, description = "missing dispatch:read scope (keys) or not an org admin (humans)")))]
pub async fn technicians(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<ConnectorTechniciansView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_dispatch_gate(&authed, false)?;
    check_rate_limit(
        state.config.dispatch_rate_limit_per_min,
        authed.principal.user.id,
    )?;
    let org_id = authed.principal.user.org_id;

    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let members = core_db::users::list_users(&mut *tx, org_id).await?;
    let skills = core_db::dispatch::list_org_skills(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    let now = Utc::now();
    let mut by_user: HashMap<Uuid, Vec<String>> = HashMap::new();
    for row in skills {
        if row.expires_at.is_none_or(|t| t > now) {
            by_user.entry(row.user_id).or_default().push(row.skill);
        }
    }

    Ok(Json(ConnectorTechniciansView {
        technicians: members
            .iter()
            .filter(|u| u.is_active)
            .map(|u| ConnectorTechnician {
                user_id: u.id,
                display_name: u.display_name.clone(),
                skills: by_user.get(&u.id).cloned().unwrap_or_default(),
            })
            .collect(),
    }))
}

/// POST /api/dispatch-connector/appointments/{id}/status — push one
/// field-service status update. Idempotent via the instant envelope: the
/// same key + payload replays the same stored row (never double-pushes);
/// a foreign-org appointment id is a bare 404.
#[utoipa::path(operation_id = "dispatch_connector_push_status", post, path = "/api/dispatch-connector/appointments/{id}/status", tag = "dispatch-connector", security(("bearer" = []), ("api_key" = [])),
    params(("id" = Uuid, Path)),
    request_body = PushStatusRequest,
    responses((status = 201, body = PushStatusResponse),
              (status = 404, description = "appointment not visible in scope"),
              (status = 409, description = "idempotency key reused with a different payload"),
              (status = 422, description = "unknown status vocabulary"),
              (status = 429, description = "rate limit exceeded (Retry-After header)")))]
pub async fn push_status(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(appointment_id): Path<Uuid>,
    Json(req): Json<PushStatusRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<PushStatusMutationResponse>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_dispatch_gate(&authed, true)?;
    check_rate_limit(
        state.config.dispatch_rate_limit_per_min,
        authed.principal.user.id,
    )?;
    if !STATUSES.contains(&req.status.as_str()) {
        return Err(ApiError::Validation(format!(
            "unknown status {:?}; known: {}",
            req.status,
            STATUSES.join(", ")
        )));
    }
    let note_len = req.note.as_ref().map(String::len).unwrap_or(0);
    if note_len > 500 {
        return Err(ApiError::Validation(
            "note must be at most 500 characters".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "dispatch_status",
                entity_id: Some(appointment_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // Org-scoped existence check: a foreign id is NotFound.
        let appointment =
            core_db::scheduling::get_appointment(&mut *ctx.tx, ctx.org_id, appointment_id).await?;
        if appointment.status == AppointmentStatus::Cancelled {
            return Err(ApiError::Validation(
                "cancelled bookings accept no status pushes".into(),
            ));
        }
        let row = core_db::dispatch::StatusUpdateRow {
            id: Uuid::now_v7(),
            org_id: ctx.org_id,
            appointment_id,
            status: req.status.clone(),
            note: req.note.clone(),
            actor_id: actor,
            source: "connector".into(),
            occurred_at: req.occurred_at.unwrap_or_else(Utc::now),
            created_at: Utc::now(),
        };
        core_db::dispatch::insert_status_update(&mut ctx.tx, &row).await?;
        ctx.emit(
            types::DISPATCH_STATUS_UPDATED,
            "appointment",
            Some(appointment_id.to_string().as_str()),
            &serde_json::to_value(DispatchStatusUpdated {
                appointment_id,
                status: row.status.clone(),
                note: row.note.clone(),
                source: row.source.clone(),
                actor_id: actor,
                occurred_at: row.occurred_at,
            })
            .map_err(|e| ApiError::Internal(e.into()))?,
        )
        .await?;
        let view = PushStatusResponse {
            id: row.id,
            appointment_id,
            status: row.status.clone(),
            note: row.note.clone(),
            source: row.source.clone(),
            occurred_at: row.occurred_at,
        };
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "dispatch_status",
                entity_id: row.id,
                version: None,
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "dispatch_status",
                entity_id: row.id,
                op: "status_push",
                seq,
                before: None,
                after: serde_json::to_value(&row).unwrap_or(Value::Null),
                updated_at: row.occurred_at,
                key: key.as_deref(),
                payload: &view,
                // Connector provenance: the trail names the api key as
                // the actor, so "who did this" resolves through the key.
                provenance: instant::ProvenanceContext {
                    actor_kind: Some("connector"),
                    ..Default::default()
                },
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(PushStatusMutationResponse {
                pushed: view,
                mutation,
            }),
        ))
    })
    .await?;

    let response = outcome.into_response()?;
    Ok((axum::http::StatusCode::CREATED, Json(response)))
}
