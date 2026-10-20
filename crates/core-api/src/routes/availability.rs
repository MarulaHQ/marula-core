//! Availability-window, on-call-block, and time-off-exclusion endpoints
//! (the private build tracker 2.12).
//!
//! Purpose: self-service CRUD for a user's recurring working templates,
//! on-call rotations, and absolute time-off. Writes naming another user
//! require owner/admin (manager override), mirroring the booking rule.
//! Every mutation emits its lightweight change event in the same
//! transaction so consumers refetch that user's windows.
//!
//! Non-purpose: occurrence expansion or containment math
//! (core_domain::scheduling), appointment lifecycle (routes/scheduling.rs),
//! per-org business-hours calendars (SLA lane's sla_calendars).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::routes::scheduling::{sched, target_user, validate_span};
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use chrono::{DateTime, Utc};
use core_domain::entities::{AvailabilityWindow, CalendarExclusion, OnCallBlock};
use core_domain::events::{types, AvailabilityChanged, CalendarExclusionChanged, OnCallChanged};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

fn parse_hhmm(s: &str) -> ApiResult<chrono::NaiveTime> {
    chrono::NaiveTime::parse_from_str(s, "%H:%M")
        .map_err(|_| ApiError::Validation(format!("bad time '{s}' (expected HH:MM)")))
}

// --- availability windows ------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAvailabilityRequest {
    /// Whose calendar. Defaults to the caller; another user requires
    /// owner/admin.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub user_id: Option<Uuid>,
    /// 0 = Monday … 6 = Sunday.
    pub weekday: i16,
    /// Local wall-clock "HH:MM" (window is `[start, end)`).
    pub start_local: String,
    pub end_local: String,
    /// IANA timezone the wall-clock bounds live in.
    pub timezone: String,
    #[schema(value_type = Option<String>)]
    pub effective_from: Option<chrono::NaiveDate>,
    #[schema(value_type = Option<String>)]
    pub effective_until: Option<chrono::NaiveDate>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AvailabilityListResponse {
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    pub windows: Vec<AvailabilityWindow>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct AvailabilityQuery {
    /// Whose windows. Defaults to the caller; another user requires owner/admin.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub user_id: Option<Uuid>,
}

/// GET /api/availability — list one user's weekly templates.
#[utoipa::path(operation_id = "availability_list", get, path = "/api/availability", tag = "scheduling", security(("bearer" = [])),
    params(AvailabilityQuery),
    responses((status = 200, body = AvailabilityListResponse)))]
pub async fn list_availability(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<AvailabilityQuery>,
) -> ApiResult<Json<AvailabilityListResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(core_domain::policy::Action::View))?;
    let user_id = target_user(&authed.principal, q.user_id, "availability reads")?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let windows = core_db::scheduling::list_availability_windows(&mut *tx, org_id, user_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(AvailabilityListResponse { user_id, windows }))
}

/// POST /api/availability — add a weekly template for a user.
#[utoipa::path(operation_id = "availability_create", post, path = "/api/availability", tag = "scheduling", security(("bearer" = [])),
    request_body = CreateAvailabilityRequest,
    responses((status = 201, body = AvailabilityWindow)))]
pub async fn create_availability(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateAvailabilityRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<AvailabilityWindow>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(core_domain::policy::Action::Edit))?;
    let user_id = target_user(&authed.principal, req.user_id, "availability writes")?;
    let window = AvailabilityWindow {
        id: Uuid::now_v7(),
        org_id: authed.principal.user.org_id,
        user_id,
        weekday: req.weekday,
        start_local: parse_hhmm(&req.start_local)?,
        end_local: parse_hhmm(&req.end_local)?,
        timezone: req.timezone.clone(),
        effective_from: req.effective_from,
        effective_until: req.effective_until,
        created_at: Utc::now(),
    };
    window.validate()?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        core_db::scheduling::create_availability_window(&mut *tx.tx, &window).await?;
        tx.emit(
            types::AVAILABILITY_UPDATED,
            "availability",
            Some(&user_id.to_string()),
            &serde_json::to_value(AvailabilityChanged { user_id })
                .map_err(|e| ApiError::Internal(e.into()))?,
        )
        .await?;
        Ok((tx, window))
    })
    .await
    .map(|w| (axum::http::StatusCode::CREATED, Json(w)))
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct OwnedDeleteQuery {
    /// Owner of the row being deleted (must match self-or-admin rule).
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
}

/// DELETE /api/availability/{id}?user_id= — remove one template.
#[utoipa::path(operation_id = "availability_delete", delete, path = "/api/availability/{id}", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path), OwnedDeleteQuery),
    responses((status = 204), (status = 404)))]
pub async fn delete_availability(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Query(q): Query<OwnedDeleteQuery>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(core_domain::policy::Action::Edit))?;
    target_user(&authed.principal, Some(q.user_id), "availability writes")?;
    let org_id = authed.principal.user.org_id;
    let user_id = q.user_id;
    let actor = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        core_db::scheduling::delete_availability_window(&mut tx.tx, org_id, id, user_id).await?;
        tx.emit(
            types::AVAILABILITY_UPDATED,
            "availability",
            Some(&user_id.to_string()),
            &serde_json::to_value(AvailabilityChanged { user_id })
                .map_err(|e| ApiError::Internal(e.into()))?,
        )
        .await?;
        Ok((tx, ()))
    })
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// --- on-call blocks -------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateOnCallRequest {
    #[schema(value_type = Option<uuid::Uuid>)]
    pub user_id: Option<Uuid>,
    #[schema(value_type = String, format = "date-time")]
    pub starts_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub ends_at: DateTime<Utc>,
    /// Rotation label ('primary', 'secondary', 'escalation', …).
    pub category: String,
    pub notes: Option<String>,
}

/// GET /api/on-call-blocks?user_id=&from=&to=
#[utoipa::path(operation_id = "on_call_list", get, path = "/api/on-call-blocks", tag = "scheduling", security(("bearer" = [])),
    params(crate::routes::calendar::DayWeekQuery),
    responses((status = 200, body = Vec<OnCallBlock>)))]
pub async fn list_on_call(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<RangeParams>,
) -> ApiResult<Json<Vec<OnCallBlock>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(core_domain::policy::Action::View))?;
    let user_id = target_user(&authed.principal, q.user_id, "on-call reads")?;
    let from = q
        .from
        .unwrap_or_else(|| Utc::now() - chrono::Duration::days(1));
    let to =
        q.to.unwrap_or_else(|| Utc::now() + chrono::Duration::days(14));
    validate_span(from, to)?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let blocks =
        core_db::scheduling::list_on_call_blocks_between(&mut *tx, org_id, &[user_id], from, to)
            .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(blocks))
}

/// POST /api/on-call-blocks — add a rotation segment (409 when stacking).
#[utoipa::path(operation_id = "on_call_create", post, path = "/api/on-call-blocks", tag = "scheduling", security(("bearer" = [])),
    request_body = CreateOnCallRequest,
    responses((status = 201, body = OnCallBlock), (status = 409, description = "overlapping rotation")))]
pub async fn create_on_call(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateOnCallRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<OnCallBlock>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(core_domain::policy::Action::Edit))?;
    let user_id = target_user(&authed.principal, req.user_id, "on-call writes")?;
    validate_span(req.starts_at, req.ends_at)?;
    if req.category.trim().is_empty() {
        return Err(ApiError::Validation("category must not be empty".into()));
    }
    let block = OnCallBlock {
        id: Uuid::now_v7(),
        org_id: authed.principal.user.org_id,
        user_id,
        starts_at: req.starts_at,
        ends_at: req.ends_at,
        category: req.category.trim().to_string(),
        notes: req.notes.clone(),
        created_at: Utc::now(),
    };
    let org_id = block.org_id;
    let actor = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        // Pre-check names the conflict; the DB exclusion enforces it.
        let existing = core_db::scheduling::list_on_call_blocks_between(
            &mut *tx.tx,
            org_id,
            &[user_id],
            req.starts_at,
            req.ends_at,
        )
        .await?;
        if existing
            .iter()
            .any(|b| b.starts_at < req.ends_at && req.starts_at < b.ends_at)
        {
            return Err(ApiError::Conflict("overlapping on-call rotation".into()));
        }
        core_db::scheduling::create_on_call_block(&mut *tx.tx, &block).await?;
        tx.emit(
            types::ON_CALL_UPDATED,
            "on_call",
            Some(&user_id.to_string()),
            &serde_json::to_value(OnCallChanged { user_id })
                .map_err(|e| ApiError::Internal(e.into()))?,
        )
        .await?;
        Ok((tx, block))
    })
    .await
    .map(|b| (axum::http::StatusCode::CREATED, Json(b)))
}

/// DELETE /api/on-call-blocks/{id}?user_id=
#[utoipa::path(operation_id = "on_call_delete", delete, path = "/api/on-call-blocks/{id}", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path), OwnedDeleteQuery),
    responses((status = 204), (status = 404)))]
pub async fn delete_on_call(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Query(q): Query<OwnedDeleteQuery>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(core_domain::policy::Action::Edit))?;
    target_user(&authed.principal, Some(q.user_id), "on-call writes")?;
    let org_id = authed.principal.user.org_id;
    let user_id = q.user_id;
    let actor = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        core_db::scheduling::delete_on_call_block(&mut tx.tx, org_id, id, user_id).await?;
        tx.emit(
            types::ON_CALL_UPDATED,
            "on_call",
            Some(&user_id.to_string()),
            &serde_json::to_value(OnCallChanged { user_id })
                .map_err(|e| ApiError::Internal(e.into()))?,
        )
        .await?;
        Ok((tx, ()))
    })
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// --- calendar exclusions (time-off) ----------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateExclusionRequest {
    #[schema(value_type = Option<uuid::Uuid>)]
    pub user_id: Option<Uuid>,
    #[schema(value_type = String, format = "date-time")]
    pub starts_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub ends_at: DateTime<Utc>,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct RangeParams {
    #[schema(value_type = Option<uuid::Uuid>)]
    pub user_id: Option<Uuid>,
    #[schema(value_type = Option<String>, format = "date-time")]
    pub from: Option<DateTime<Utc>>,
    #[schema(value_type = Option<String>, format = "date-time")]
    pub to: Option<DateTime<Utc>>,
}

/// GET /api/calendar-exclusions?user_id=&from=&to=
#[utoipa::path(operation_id = "calendar_exclusion_list", get, path = "/api/calendar-exclusions", tag = "scheduling", security(("bearer" = [])),
    params(RangeParams),
    responses((status = 200, body = Vec<CalendarExclusion>)))]
pub async fn list_exclusions(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<RangeParams>,
) -> ApiResult<Json<Vec<CalendarExclusion>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(core_domain::policy::Action::View))?;
    let user_id = target_user(&authed.principal, q.user_id, "time-off reads")?;
    let from = q
        .from
        .unwrap_or_else(|| Utc::now() - chrono::Duration::days(30));
    let to =
        q.to.unwrap_or_else(|| Utc::now() + chrono::Duration::days(180));
    validate_span(from, to)?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let exclusions =
        core_db::scheduling::list_calendar_exclusions_between(&mut *tx, org_id, user_id, from, to)
            .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(exclusions))
}

/// POST /api/calendar-exclusions — record time-off.
#[utoipa::path(operation_id = "calendar_exclusion_create", post, path = "/api/calendar-exclusions", tag = "scheduling", security(("bearer" = [])),
    request_body = CreateExclusionRequest,
    responses((status = 201, body = CalendarExclusion)))]
pub async fn create_exclusion(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateExclusionRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<CalendarExclusion>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(core_domain::policy::Action::Edit))?;
    let user_id = target_user(&authed.principal, req.user_id, "time-off writes")?;
    validate_span(req.starts_at, req.ends_at)?;
    let exclusion = CalendarExclusion {
        id: Uuid::now_v7(),
        org_id: authed.principal.user.org_id,
        user_id,
        starts_at: req.starts_at,
        ends_at: req.ends_at,
        reason: req.reason.clone(),
        created_at: Utc::now(),
    };
    let org_id = exclusion.org_id;
    let actor = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        core_db::scheduling::create_calendar_exclusion(&mut *tx.tx, &exclusion).await?;
        tx.emit(
            types::CALENDAR_EXCLUSION_UPDATED,
            "calendar_exclusion",
            Some(&user_id.to_string()),
            &serde_json::to_value(CalendarExclusionChanged { user_id })
                .map_err(|e| ApiError::Internal(e.into()))?,
        )
        .await?;
        Ok((tx, exclusion))
    })
    .await
    .map(|x| (axum::http::StatusCode::CREATED, Json(x)))
}

/// DELETE /api/calendar-exclusions/{id}?user_id=
#[utoipa::path(operation_id = "calendar_exclusion_delete", delete, path = "/api/calendar-exclusions/{id}", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path), OwnedDeleteQuery),
    responses((status = 204), (status = 404)))]
pub async fn delete_exclusion(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Query(q): Query<OwnedDeleteQuery>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(core_domain::policy::Action::Edit))?;
    target_user(&authed.principal, Some(q.user_id), "time-off writes")?;
    let org_id = authed.principal.user.org_id;
    let user_id = q.user_id;
    let actor = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        core_db::scheduling::delete_calendar_exclusion(&mut tx.tx, org_id, id, user_id).await?;
        tx.emit(
            types::CALENDAR_EXCLUSION_UPDATED,
            "calendar_exclusion",
            Some(&user_id.to_string()),
            &serde_json::to_value(CalendarExclusionChanged { user_id })
                .map_err(|e| ApiError::Internal(e.into()))?,
        )
        .await?;
        Ok((tx, ()))
    })
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
