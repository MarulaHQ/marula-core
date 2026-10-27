//! Calendar view endpoints (the private build tracker 2.12): personal day/week views and
//! the team-wide range view, merging appointments and on-call blocks into
//! one ordered item list.
//!
//! Purpose: read-only projections for the /calendar surface. Personal
//! views resolve the target user's LOCAL day/week through their IANA
//! timezone (DST-safe via `local_day_window`) and attach that user's
//! availability templates + time-off so the UI can shade coverage.
//!
//! Non-purpose: no writes, no conflict logic (routes/scheduling.rs), no
//! dispatch capacity math (Stage-2 dispatch board).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::routes::scheduling::{sched, target_user};
use crate::state::AppState;
use axum::{
    extract::{Query, State},
    http::Extensions,
    Json,
};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use core_domain::entities::{
    AppointmentKind, AppointmentStatus, AvailabilityWindow, CalendarExclusion, OnCallBlock,
};
use core_domain::policy::Action;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Flat merged entry so calendars render one ordered list without caring
/// which table produced it.
#[derive(Debug, Serialize, ToSchema)]
pub struct CalendarItem {
    /// "appointment" | "on_call"
    pub item_type: String,
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    #[schema(value_type = String, format = "date-time")]
    pub starts_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub ends_at: DateTime<Utc>,
    pub title: Option<String>,
    /// Appointment-only fields.
    pub kind: Option<AppointmentKind>,
    pub status: Option<AppointmentStatus>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    /// Appointment-only (Wave 32 lane 7): booked travel minutes sitting
    /// immediately BEFORE the appointment — the calendar renders them
    /// visually distinct from the booking block.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub travel_minutes: Option<i32>,
    /// `heuristic` | `measured` when the estimate carries provenance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub travel_quality: Option<String>,
    /// On-call-only field.
    pub category: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CalendarViewResponse {
    /// None on team-wide views.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub user_id: Option<Uuid>,
    #[schema(value_type = String, format = "date-time")]
    pub from: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub to: DateTime<Utc>,
    pub timezone: String,
    /// Merged appointments + on-call, ordered by starts_at then id.
    pub items: Vec<CalendarItem>,
    pub exclusions: Vec<CalendarExclusion>,
    /// Present on personal views only (templates are per-user).
    pub availability_windows: Vec<AvailabilityWindow>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct DayWeekQuery {
    /// Whose calendar. Defaults to the caller; another user requires
    /// owner/admin.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub user_id: Option<Uuid>,
    /// Local date in the TARGET user's timezone. Day view: the day itself.
    /// Week view: any date inside the week (Monday-start). Defaults to
    /// today in that timezone.
    #[schema(value_type = Option<String>)]
    pub date: Option<NaiveDate>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct TeamRangeQuery {
    /// Inclusive window start (default: today's UTC midnight minus 1 day).
    #[schema(value_type = Option<String>, format = "date-time")]
    pub from: Option<DateTime<Utc>>,
    /// Exclusive window end (default: `from` + 8 days).
    #[schema(value_type = Option<String>, format = "date-time")]
    pub to: Option<DateTime<Utc>>,
}

fn appt_item(a: &core_domain::entities::Appointment) -> CalendarItem {
    CalendarItem {
        item_type: "appointment".into(),
        id: a.id,
        user_id: a.user_id,
        starts_at: a.starts_at,
        ends_at: a.ends_at,
        title: Some(a.title.clone()),
        kind: Some(a.kind),
        status: Some(a.status),
        ticket_id: a.ticket_id,
        travel_minutes: Some(a.travel_minutes),
        travel_quality: a.travel_quality.clone(),
        category: None,
    }
}

fn on_call_item(b: &OnCallBlock) -> CalendarItem {
    CalendarItem {
        item_type: "on_call".into(),
        id: b.id,
        user_id: b.user_id,
        starts_at: b.starts_at,
        ends_at: b.ends_at,
        title: None,
        kind: None,
        status: None,
        ticket_id: None,
        travel_minutes: None,
        travel_quality: None,
        category: Some(b.category.clone()),
    }
}

async fn merged_items(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    users: &[Uuid],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> ApiResult<Vec<CalendarItem>> {
    let appts =
        core_db::scheduling::list_appointments_between(&mut *conn, org_id, users, from, to).await?;
    let on_call =
        core_db::scheduling::list_on_call_blocks_between(&mut *conn, org_id, users, from, to)
            .await?;
    let mut items: Vec<CalendarItem> = appts
        .iter()
        .map(appt_item)
        .chain(on_call.iter().map(on_call_item))
        .collect();
    items.sort_by_key(|x| (x.starts_at, x.id));
    Ok(items)
}

/// GET /api/calendar/day — one local day for one user.
#[utoipa::path(operation_id = "calendar_day", get, path = "/api/calendar/day", tag = "scheduling", security(("bearer" = [])),
    params(DayWeekQuery),
    responses((status = 200, body = CalendarViewResponse)))]
pub async fn calendar_day(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<DayWeekQuery>,
) -> ApiResult<Json<CalendarViewResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let user_id = target_user(&authed.principal, q.user_id, "calendar reads")?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let user = core_db::users::get_user(&mut *tx, org_id, user_id)
        .await
        .map_err(|_| ApiError::NotFound)?;
    let tz: Tz = user
        .timezone
        .parse()
        .map_err(|_| ApiError::Validation(format!("unknown timezone: {}", user.timezone)))?;
    let date = q
        .date
        .unwrap_or_else(|| Utc::now().with_timezone(&tz).date_naive());
    let (from, to) = core_domain::time_audit::local_day_window(date, tz);
    let view = build_personal(&mut tx, org_id, user_id, from, to, &user.timezone).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(view))
}

/// GET /api/calendar/week — Monday-start local week for one user.
#[utoipa::path(operation_id = "calendar_week", get, path = "/api/calendar/week", tag = "scheduling", security(("bearer" = [])),
    params(DayWeekQuery),
    responses((status = 200, body = CalendarViewResponse)))]
pub async fn calendar_week(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<DayWeekQuery>,
) -> ApiResult<Json<CalendarViewResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let user_id = target_user(&authed.principal, q.user_id, "calendar reads")?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let user = core_db::users::get_user(&mut *tx, org_id, user_id)
        .await
        .map_err(|_| ApiError::NotFound)?;
    let tz: Tz = user
        .timezone
        .parse()
        .map_err(|_| ApiError::Validation(format!("unknown timezone: {}", user.timezone)))?;
    let date = q
        .date
        .unwrap_or_else(|| Utc::now().with_timezone(&tz).date_naive());
    // Snap to Monday of this week (num_days_from_monday: Mon=0..Sun=6).
    let monday = date - Duration::days(i64::from(date.weekday().num_days_from_monday()));
    let (from, _) = core_domain::time_audit::local_day_window(monday, tz);
    let (_, to) = core_domain::time_audit::local_day_window(
        monday + Duration::weeks(1) - Duration::days(1),
        tz,
    );
    let view = build_personal(&mut tx, org_id, user_id, from, to, &user.timezone).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(view))
}

/// GET /api/calendar/team — every org member over an absolute UTC range.
#[utoipa::path(operation_id = "calendar_team", get, path = "/api/calendar/team", tag = "scheduling", security(("bearer" = [])),
    params(TeamRangeQuery),
    responses((status = 200, body = CalendarViewResponse)))]
pub async fn calendar_team(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<TeamRangeQuery>,
) -> ApiResult<Json<CalendarViewResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let members = core_db::users::list_users(&mut *tx, org_id).await?;
    let ids: Vec<Uuid> = members.iter().map(|u| u.id).collect();
    let to_default = q.from.unwrap_or(Utc::now()) + Duration::days(8);
    let from = q.from.unwrap_or_else(|| Utc::now() - Duration::days(1));
    let to = q.to.unwrap_or(to_default);
    if to <= from || to - from > Duration::days(62) {
        return Err(ApiError::Validation("range must be within 62 days".into()));
    }
    let items = merged_items(&mut tx, org_id, &ids, from, to).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(CalendarViewResponse {
        user_id: None,
        from,
        to,
        timezone: "UTC".into(),
        items,
        exclusions: Vec::new(),
        availability_windows: Vec::new(),
    }))
}

async fn build_personal(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    tz_label: &str,
) -> ApiResult<CalendarViewResponse> {
    let exclusions = core_db::scheduling::list_calendar_exclusions_between(
        &mut *conn, org_id, user_id, from, to,
    )
    .await?;
    let users = [user_id];
    let items = merged_items(&mut *conn, org_id, &users, from, to).await?;
    let windows = core_db::scheduling::list_availability_windows(&mut *conn, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    Ok(CalendarViewResponse {
        user_id: Some(user_id),
        from,
        to,
        timezone: tz_label.to_string(),
        items,
        exclusions,
        availability_windows: windows,
    })
}

// --- org-wide ticket deadline read model (Wave 17 lane 13, migration 0089) -----

/// One deadline the overlay renders. Hydration lives in the row.
#[derive(Debug, Serialize, ToSchema)]
pub struct CalendarDeadlineEntry {
    #[schema(value_type = uuid::Uuid)]
    pub ticket_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub board_id: Uuid,
    pub number: String,
    pub title: String,
    #[schema(value_type = String)]
    pub due_at: DateTime<Utc>,
    /// Semantic category (informational — closed tickets never arrive).
    pub category: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub assignee_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub client_id: Option<Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CalendarDeadlinesResponse {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub deadlines: Vec<CalendarDeadlineEntry>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct DeadlineRangeQuery {
    /// Absolute UTC window [from, to); capped at 62 days like the team
    /// view (a deadline overlay never walks quarterly ranges).
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    /// Optional per-person narrowing (personal mode rides the same model).
    #[schema(value_type = Option<uuid::Uuid>)]
    pub assignee_id: Option<Uuid>,
}

/// GET /api/calendar/deadlines — the ORG-WIDE ticket-deadline read model
/// (Wave 17 lane 13): /calendar team mode overlays THIS instead of the
/// bounded page-1 tickets fetch. Alive tickets only (category not closed)
/// whose due date falls in the window; hydrated in the row itself.
#[utoipa::path(operation_id = "calendar_deadlines", get, path = "/api/calendar/deadlines", tag = "scheduling", security(("bearer" = [])),
    params(DeadlineRangeQuery),
    responses((status = 200, body = CalendarDeadlinesResponse),
              (status = 422, description = "validation failed")))]
pub async fn deadlines(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<DeadlineRangeQuery>,
) -> ApiResult<Json<CalendarDeadlinesResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let to_default = q.from.unwrap_or(Utc::now()) + Duration::days(8);
    let from = q.from.unwrap_or_else(|| Utc::now() - Duration::days(1));
    let to = q.to.unwrap_or(to_default);
    if to <= from || to - from > Duration::days(62) {
        return Err(ApiError::Validation("range must be within 62 days".into()));
    }
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::scheduling::list_ticket_deadlines_between(
        &mut *tx,
        org_id,
        from,
        to,
        q.assignee_id,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(CalendarDeadlinesResponse {
        from,
        to,
        deadlines: rows
            .into_iter()
            .map(|r| CalendarDeadlineEntry {
                ticket_id: r.ticket_id,
                board_id: r.board_id,
                number: r.number,
                title: r.title,
                due_at: r.due_at,
                category: r.category,
                assignee_id: r.assignee_id,
                client_id: r.client_id,
            })
            .collect(),
    }))
}
