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
use chrono::{DateTime, Timelike, Utc};
use core_domain::entities::{AvailabilityWindow, CalendarExclusion, OnCallBlock};
use core_domain::events::{types, AvailabilityChanged, CalendarExclusionChanged, OnCallChanged};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// The one spelling of "through midnight" the availability API accepts. It is
/// not a `chrono::NaiveTime`, so the two bounds treat it differently: an end
/// may say it, a start may not.
const END_OF_DAY_HHMM: &str = "24:00";

fn parse_hhmm(s: &str) -> ApiResult<chrono::NaiveTime> {
    chrono::NaiveTime::parse_from_str(s, "%H:%M")
        .map_err(|_| ApiError::Validation(format!("bad time '{s}' (expected HH:MM)")))
}

/// `start_local` bound. `00:00` is how a caller says start-of-day; `24:00` is
/// rejected here because a window that opens at midnight closes on the *next*
/// day, which weekly templates cannot express — the half-open window is
/// `[start, end)` inside one local day.
fn parse_start_hhmm(s: &str) -> ApiResult<chrono::NaiveTime> {
    if s == END_OF_DAY_HHMM {
        return Err(ApiError::Validation(format!(
            "bad time '{s}' (start must be before end; {END_OF_DAY_HHMM} is only valid as an end)"
        )));
    }
    parse_hhmm(s)
}

/// `end_local` bound. `24:00` means end-of-day, which no `NaiveTime` can hold,
/// so it maps to the domain's own bound (`core_domain::scheduling::end_of_day`)
/// rather than to a second sentinel here: `ends_at_end_of_day` must keep
/// recognising exactly one definition. Every other string keeps its meaning
/// byte for byte — `23:59` is `23:59:00`, a minute short of midnight, not
/// end-of-day.
fn parse_end_hhmm(s: &str) -> ApiResult<chrono::NaiveTime> {
    if s == END_OF_DAY_HHMM {
        return Ok(end_of_day_at_column_precision());
    }
    parse_hhmm(s)
}

/// The `time` columns keep microseconds, so the nanosecond sentinel cannot
/// survive a write. `24:00` is therefore mapped onto the last microsecond the
/// column can hold: a create response and a later read then carry the same
/// string, where echoing the sentinel would have answered `23:59:59.999999999`
/// once and `23:59:59.999999` from then on. Truncating at the boundary keeps
/// one value rather than two, and `ends_at_end_of_day` accepts it because the
/// tolerance is the final second.
fn end_of_day_at_column_precision() -> chrono::NaiveTime {
    let end = core_domain::scheduling::end_of_day();
    chrono::NaiveTime::from_hms_micro_opt(
        end.hour(),
        end.minute(),
        end.second(),
        end.nanosecond() / 1_000,
    )
    .expect("the truncated end-of-day bound is valid")
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
    /// Local wall-clock "HH:MM" where the window opens (the window is
    /// `[start, end)`). `00:00` means start-of-day; `24:00` is rejected here,
    /// it is only valid as an end.
    pub start_local: String,
    /// Local wall-clock "HH:MM" where the window closes. `24:00` is accepted
    /// and means end-of-day (through local midnight), so a round-the-clock
    /// window is `00:00`–`24:00`. Every other value keeps its exact second:
    /// `23:59` still means 23:59:00.
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
        start_local: parse_start_hhmm(&req.start_local)?,
        end_local: parse_end_hhmm(&req.end_local)?,
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

/// The `HH:MM` bound parsers are pure, so these run without a database.
/// They pin the property the write path protects: `24:00` is end-of-day and
/// only an end, while `23:59` stays exactly `23:59:00` — a window an operator
/// deliberately closed one second before midnight is not silently widened.
#[cfg(test)]
mod tests {
    use super::*;

    fn validation_message(err: ApiError) -> String {
        match err {
            ApiError::Validation(m) => m,
            other => panic!("expected a validation error, got {other:?}"),
        }
    }

    /// `24:00` maps onto the domain's end-of-day bound, truncated to what the
    /// `time` column keeps, so the create response and a later read agree.
    #[test]
    fn an_end_of_24_00_is_the_end_of_day_at_column_precision() {
        let end = parse_end_hhmm("24:00").expect("24:00 is a valid end");
        assert_eq!(
            end,
            chrono::NaiveTime::from_hms_micro_opt(23, 59, 59, 999_999).unwrap()
        );
        let sentinel = core_domain::scheduling::end_of_day();
        assert!(sentinel - end < chrono::Duration::microseconds(1), "{end}");
    }

    /// The property the truncation exists for: what a create echoes and what a
    /// read returns are the same string, while the nanosecond sentinel — which
    /// no read can ever return — is not.
    #[test]
    fn an_end_of_24_00_round_trips_through_the_column_representation() {
        let echoed = serde_json::to_string(&parse_end_hhmm("24:00").unwrap()).unwrap();
        let as_stored = serde_json::to_string(
            &chrono::NaiveTime::from_hms_micro_opt(23, 59, 59, 999_999).unwrap(),
        )
        .unwrap();
        let as_sentinel = serde_json::to_string(&core_domain::scheduling::end_of_day()).unwrap();
        assert_eq!(echoed, as_stored, "a create echoes what the column returns");
        assert_ne!(
            as_sentinel, as_stored,
            "the sentinel is not a value a read can return"
        );
    }

    #[test]
    fn a_start_of_24_00_is_rejected_and_names_the_rule() {
        let msg = validation_message(parse_start_hhmm("24:00").unwrap_err());
        assert!(msg.contains("24:00"), "{msg}");
        assert!(msg.contains("start must be before end"), "{msg}");
        assert!(msg.contains("only valid as an end"), "{msg}");
    }

    #[test]
    fn start_of_day_is_00_00() {
        assert_eq!(
            parse_start_hhmm("00:00").unwrap(),
            chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()
        );
    }

    /// The ruling's protected property: the latest ordinary end stays a
    /// minute short of midnight, nowhere near the end-of-day bound.
    #[test]
    fn an_end_of_23_59_is_exactly_23_59_00() {
        assert_eq!(
            parse_end_hhmm("23:59").unwrap(),
            chrono::NaiveTime::from_hms_opt(23, 59, 0).unwrap()
        );
        assert_ne!(
            parse_end_hhmm("23:59").unwrap(),
            core_domain::scheduling::end_of_day()
        );
    }

    #[test]
    fn other_bounds_keep_their_seconds() {
        assert_eq!(
            parse_end_hhmm("09:30").unwrap(),
            chrono::NaiveTime::from_hms_opt(9, 30, 0).unwrap()
        );
        assert_eq!(
            parse_start_hhmm("00:00").unwrap(),
            parse_end_hhmm("00:00").unwrap()
        );
    }

    #[test]
    fn garbage_still_errors_with_the_existing_message_shape() {
        for (parsed, bad) in [
            (parse_start_hhmm("garbage").unwrap_err(), "garbage"),
            (parse_end_hhmm("garbage").unwrap_err(), "garbage"),
            (parse_start_hhmm("24:00:00").unwrap_err(), "24:00:00"),
            (parse_end_hhmm("24:00:00").unwrap_err(), "24:00:00"),
            (parse_end_hhmm("23:60").unwrap_err(), "23:60"),
            (parse_end_hhmm("").unwrap_err(), ""),
        ] {
            assert_eq!(
                validation_message(parsed),
                format!("bad time '{bad}' (expected HH:MM)")
            );
        }
    }
}
