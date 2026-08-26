//! Time-discipline endpoints (TASKLIST 2.9/2.10): org time settings and
//! the missing-time gap reports.
//!
//! Purpose: admin read/write of the org's EOD-nudge knobs, the caller's
//! own day-level gap report, and the manager-scoped report for any member.
//! Reports compute live from ticket activity + time entries through the
//! shared core-domain rule, then enrich with the audit ledger (when the
//! EOD run nudged).
//!
//! Non-purpose: no timer lifecycle (routes/timers.rs), no notification
//! delivery (the nudge run inserts through the notification center), no
//! approval workflows (Stage 4).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    extract::{Query, State},
    http::Extensions,
    Json,
};
use chrono::{NaiveDate, Utc};
use chrono_tz::Tz;
use core_domain::policy::{Action, Module, Permission};
use core_domain::time_audit::{day_series, evaluate_day, local_day_window, previous_local_day};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

fn time_perm(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Time,
        action,
    }
}

fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

// --- org settings -----------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct TimeSettingsResponse {
    pub eod_nudges_enabled: bool,
    /// Local hour (member timezones) the EOD audit fires at.
    pub eod_nudge_hour: i16,
    /// Active days logging under this many minutes are gap days.
    pub time_gap_min_minutes: i32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateTimeSettingsRequest {
    pub eod_nudges_enabled: bool,
    pub eod_nudge_hour: i16,
    pub time_gap_min_minutes: i32,
}

/// GET /api/time-settings — org time-discipline knobs (admin).
#[utoipa::path(operation_id = "time_settings_get", get, path = "/api/time-settings", tag = "timers", security(("bearer" = [])),
    responses((status = 200, body = TimeSettingsResponse)))]
pub async fn get_settings(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<TimeSettingsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let s =
        core_db::time_audit::org_time_settings(&state.pool, authed.principal.user.org_id).await?;
    Ok(Json(TimeSettingsResponse {
        eod_nudges_enabled: s.eod_nudges_enabled,
        eod_nudge_hour: s.eod_nudge_hour,
        time_gap_min_minutes: s.time_gap_min_minutes,
    }))
}

/// PUT /api/time-settings — update the org's time-discipline knobs (admin).
#[utoipa::path(operation_id = "time_settings_put", put, path = "/api/time-settings", tag = "timers", security(("bearer" = [])),
    request_body = UpdateTimeSettingsRequest,
    responses((status = 200, body = TimeSettingsResponse)))]
pub async fn put_settings(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<UpdateTimeSettingsRequest>,
) -> ApiResult<Json<TimeSettingsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    if !(0..=23).contains(&req.eod_nudge_hour) {
        return Err(ApiError::Validation("eod_nudge_hour must be 0–23".into()));
    }
    if !(0..=1440).contains(&req.time_gap_min_minutes) {
        return Err(ApiError::Validation(
            "time_gap_min_minutes must be 0–1440".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let settings = core_db::time_audit::OrgTimeSettings {
        eod_nudges_enabled: req.eod_nudges_enabled,
        eod_nudge_hour: req.eod_nudge_hour,
        time_gap_min_minutes: req.time_gap_min_minutes,
    };
    core_db::time_audit::update_org_time_settings(&state.pool, org_id, &settings).await?;
    Ok(Json(TimeSettingsResponse {
        eod_nudges_enabled: settings.eod_nudges_enabled,
        eod_nudge_hour: settings.eod_nudge_hour,
        time_gap_min_minutes: settings.time_gap_min_minutes,
    }))
}

// --- gap reports --------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct GapDayView {
    /// Local calendar day (YYYY-MM-DD).
    pub day: String,
    pub activity: bool,
    pub logged_minutes: i64,
    pub expected_minutes: i64,
    pub has_gap: bool,
    pub unaccounted_minutes: i64,
    /// When the EOD run nudged for this day, if it did.
    pub nudge_sent_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct GapReportResponse {
    pub user_id: Uuid,
    pub timezone: String,
    pub expected_minutes: i64,
    pub days: Vec<GapDayView>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct GapReportQuery {
    /// Inclusive local-day start (YYYY-MM-DD). Defaults to yesterday.
    pub from: Option<NaiveDate>,
    /// Inclusive local-day end (YYYY-MM-DD). Defaults to `from`.
    pub to: Option<NaiveDate>,
    /// Manager report only: whose days. Requires owner/admin.
    pub user_id: Option<Uuid>,
}

/// GET /api/time-gaps/me — the caller's day-level gap report (defaults to
/// yesterday in the caller's timezone).
#[utoipa::path(operation_id = "time_gaps_me", get, path = "/api/time-gaps/me", tag = "timers", security(("bearer" = [])),
    params(GapReportQuery),
    responses((status = 200, body = GapReportResponse)))]
pub async fn my_gaps(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<GapReportQuery>,
) -> ApiResult<Json<GapReportResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(Action::View))?;
    let report = build_report(
        &state,
        authed.principal.user.org_id,
        authed.principal.user.id,
        &authed.principal.user.timezone,
        q.from,
        q.to,
    )
    .await?;
    Ok(Json(report))
}

/// GET /api/time-gaps?user_id= — manager-scoped report for one member.
/// Owner/admin only; everyone else uses `/api/time-gaps/me`.
#[utoipa::path(operation_id = "time_gaps_user", get, path = "/api/time-gaps", tag = "timers", security(("bearer" = [])),
    params(GapReportQuery),
    responses((status = 200, body = GapReportResponse)))]
pub async fn user_gaps(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<GapReportQuery>,
) -> ApiResult<Json<GapReportResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(Action::View))?;
    if !authed.principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "gap reports for others require owner or admin".into(),
        ));
    }
    let user_id = q.user_id.ok_or_else(|| {
        ApiError::Validation("user_id is required (use /api/time-gaps/me for yourself)".into())
    })?;
    let user = core_db::users::get_user(&state.pool, authed.principal.user.org_id, user_id)
        .await
        .map_err(|_| ApiError::NotFound)?;
    let report = build_report(
        &state,
        authed.principal.user.org_id,
        user.id,
        &user.timezone,
        q.from,
        q.to,
    )
    .await?;
    Ok(Json(report))
}

/// Shared report builder: live computation over the day range, enriched
/// with ledger nudge timestamps. Ranges clamp to 62 days.
async fn build_report(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    timezone: &str,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
) -> ApiResult<GapReportResponse> {
    let tz: Tz = timezone
        .parse()
        .map_err(|_| ApiError::Validation(format!("unknown timezone: {timezone}")))?;
    let yesterday = previous_local_day(Utc::now(), tz);
    let from = from.unwrap_or(yesterday);
    let to = to.unwrap_or(from);
    if to < from {
        return Err(ApiError::Validation("to must not precede from".into()));
    }
    let settings = core_db::time_audit::org_time_settings(&state.pool, org_id).await?;
    let expected = i64::from(settings.time_gap_min_minutes);

    let days = day_series(from, to);
    let ledger =
        core_db::time_audit::list_gap_audits(&state.pool, org_id, user_id, from, to).await?;
    let mut views = Vec::with_capacity(days.len());
    for day in days {
        let (start, end) = local_day_window(day, tz);
        let activity =
            core_db::time_audit::day_activity(&state.pool, org_id, user_id, start, end).await?;
        let logged =
            core_db::timers::minutes_logged_on(&state.pool, user_id, org_id, start, end).await?;
        let gap = evaluate_day(day, activity, logged, expected);
        views.push(GapDayView {
            day: day.to_string(),
            activity: gap.activity,
            logged_minutes: gap.logged_minutes,
            expected_minutes: gap.expected_minutes,
            has_gap: gap.has_gap(),
            unaccounted_minutes: gap.unaccounted_minutes(),
            nudge_sent_at: ledger
                .iter()
                .find(|a| a.day == day)
                .and_then(|a| a.nudge_sent_at),
        });
    }
    Ok(GapReportResponse {
        user_id,
        timezone: timezone.to_string(),
        expected_minutes: expected,
        days: views,
    })
}
