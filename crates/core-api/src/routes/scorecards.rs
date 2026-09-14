//! Coaching scorecard endpoints (Wave 14 lane 3, R5): per-tech /
//! service-desk scorecards — SLA attainment, response/resolution latency,
//! CSAT inputs, and workload mix — aggregated over the versioned
//! read-model views (`ops.v1_*`) by [`core_db::scorecards`].
//!
//! Privacy model (R5, private-by-default): any staff member with
//! `reporting:view` reads their OWN scorecard; the team view and other
//! members' cards are owner/admin only. Gamification is strictly opt-in —
//! the toggle lives in `scorecard_settings` (0075) and defaults false;
//! flipping it emits `scorecard.settings_updated` from the same
//! transaction as the write.
//!
//! Reads run inside an org-bound transaction (the RLS pool-read gate) so
//! the security-invoker views evaluate every underlying table's RLS with
//! the caller's binding; the queries themselves never touch physical
//! tables (ADR 0001 seam).
//!
//! Non-purpose: no CSAT data (the surveys-on-resolution source is a
//! later TASKLIST item — the payload reserves the slot as
//! `available: false`), no gamification math (opt-in presentation is
//! computed client-side over these windows), no scheduled delivery.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

fn reporting_view() -> Permission {
    Permission::Matrix {
        module: Module::Reporting,
        action: Action::View,
    }
}

/// Shared window query surface; defaults to the 30 trailing UTC days,
/// clamped to the reporting engine's 366-day maximum (same rules as
/// `/api/reports/*` so every time-windowed surface behaves identically).
#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct ScorecardQuery {
    /// Inclusive window start (UTC day, YYYY-MM-DD). Default: 30 trailing days.
    pub from: Option<chrono::NaiveDate>,
    /// Inclusive window end (UTC day). Max span with `from`: 366 days.
    pub to: Option<chrono::NaiveDate>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ScorecardWindow {
    /// Inclusive start (UTC midnight).
    pub from: chrono::DateTime<chrono::Utc>,
    /// Exclusive end (UTC midnight of the day after `to`).
    pub to: chrono::DateTime<chrono::Utc>,
}

/// SLA attainment block. `response_total`/`resolution_total` count only
/// tickets where BOTH the SLA target and the actual instant exist (the
/// three-valued metrics from `ops.v1_sla_outcomes`); attainment is the
/// met/total ratio, `None` when the total is zero.
#[derive(Debug, Serialize, ToSchema)]
pub struct ScorecardSla {
    pub response_total: i64,
    pub response_met: i64,
    pub resolution_total: i64,
    pub resolution_met: i64,
    pub breached: i64,
    pub response_attainment: Option<f64>,
    pub resolution_attainment: Option<f64>,
}

fn ratio(met: i64, total: i64) -> Option<f64> {
    if total == 0 {
        None
    } else {
        Some(met as f64 / total as f64)
    }
}

/// First-staff-response latency block (seconds; staff-authored comments
/// only — portal replies are structurally excluded by the view).
#[derive(Debug, Serialize, ToSchema)]
pub struct ScorecardLatency {
    pub count: i64,
    pub median_seconds: Option<f64>,
    pub p95_seconds: Option<f64>,
}

/// Workload mix over APPROVED time entries only — drafts and rejected
/// entries are never scorecard truth (Wave 14 lane-1 approval contract).
#[derive(Debug, Serialize, ToSchema)]
pub struct ScorecardWorkload {
    pub approved_entries: i64,
    pub approved_minutes: i64,
    pub approved_billable_minutes: i64,
    /// billable / total approved minutes; `None` with no approved work.
    pub billable_ratio: Option<f64>,
    /// APPROVED minutes per work type; `work_type: null` groups
    /// unclassified entries.
    pub by_work_type: Vec<WorkTypeSlice>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkTypeSlice {
    /// `null` = unclassified entries.
    pub work_type: Option<String>,
    pub minutes: i64,
}

/// CSAT inputs slot. The surveys-on-resolution source does not exist yet
/// (later TASKLIST item); the scorecard reserves the field and reports
/// `available: false` rather than inventing data.
#[derive(Debug, Serialize, ToSchema)]
pub struct CsatSection {
    pub available: bool,
}

/// One staff member's scorecard over a window.
#[derive(Debug, Serialize, ToSchema)]
pub struct Scorecard {
    pub user_id: Uuid,
    pub display_name: String,
    pub window: ScorecardWindow,
    pub sla: ScorecardSla,
    pub response: ScorecardLatency,
    pub resolution: ScorecardLatency,
    pub workload: ScorecardWorkload,
    pub csat: CsatSection,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ScorecardResponse {
    #[serde(flatten)]
    pub scorecard: Scorecard,
}

/// The team view: one scorecard per staff member, owner/admin only.
#[derive(Debug, Serialize, ToSchema)]
pub struct TeamScorecardsResponse {
    pub window: ScorecardWindow,
    pub scorecards: Vec<Scorecard>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ScorecardSettingsResponse {
    pub user_id: Uuid,
    pub gamification_opt_in: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateScorecardSettingsRequest {
    /// R5's opt-in gamification toggle (streaks / leaderboards). Default
    /// and privacy-preserving value: false.
    pub gamification_opt_in: bool,
}

fn resolve_window(
    q: &ScorecardQuery,
    now: chrono::DateTime<chrono::Utc>,
) -> ApiResult<core_reports::Window> {
    core_reports::resolve_window(q.from, q.to, now).map_err(|e| ApiError::Validation(e.to_string()))
}

fn build_scorecard(
    row: core_db::scorecards::ScorecardRow,
    mix: Vec<core_db::scorecards::WorkTypeMixRow>,
    window: core_reports::Window,
) -> Scorecard {
    let mix = mix
        .into_iter()
        .map(|m| WorkTypeSlice {
            work_type: m.work_type,
            minutes: m.minutes,
        })
        .collect();
    let sla = ScorecardSla {
        response_total: row.response_total,
        response_met: row.response_met,
        resolution_total: row.resolution_total,
        resolution_met: row.resolution_met,
        breached: row.breached,
        response_attainment: ratio(row.response_met, row.response_total),
        resolution_attainment: ratio(row.resolution_met, row.resolution_total),
    };
    Scorecard {
        user_id: row.user_id,
        display_name: row.display_name,
        window: ScorecardWindow {
            from: window.start,
            to: window.end,
        },
        sla,
        response: ScorecardLatency {
            count: row.responses,
            median_seconds: row.median_response_secs,
            p95_seconds: row.p95_response_secs,
        },
        resolution: ScorecardLatency {
            count: row.resolutions,
            median_seconds: row.median_resolution_secs,
            p95_seconds: row.p95_resolution_secs,
        },
        workload: ScorecardWorkload {
            approved_entries: row.approved_entries,
            approved_minutes: row.approved_minutes,
            approved_billable_minutes: row.approved_billable_minutes,
            billable_ratio: ratio(row.approved_billable_minutes, row.approved_minutes),
            by_work_type: mix,
        },
        csat: CsatSection { available: false },
    }
}

/// GET /api/scorecards/me?from&to — the caller's own scorecard.
#[utoipa::path(operation_id = "scorecard_get_own", get, path = "/api/scorecards/me", tag = "reports", security(("bearer" = [])),
    params(ScorecardQuery),
    responses((status = 200, body = ScorecardResponse),
              (status = 403, description = "missing reporting:view"),
              (status = 422, description = "invalid window")))]
pub async fn me(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ScorecardQuery>,
) -> ApiResult<Json<ScorecardResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, reporting_view())?;
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let window = resolve_window(&q, chrono::Utc::now())?;

    let (row, mix) = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| {
        let user_ids = vec![user_id];
        async move {
            let row = core_db::scorecards::scorecard_for_users(
                &mut *ctx.tx,
                org_id,
                &user_ids,
                window.start,
                window.end,
            )
            .await?
            .into_iter()
            .next()
            .ok_or(ApiError::RlsSparse)?;
            let mix = core_db::scorecards::work_type_mix(
                &mut *ctx.tx,
                org_id,
                &user_ids,
                window.start,
                window.end,
            )
            .await?;
            Ok((ctx, (row, mix)))
        }
    })
    .await?;

    Ok(Json(ScorecardResponse {
        scorecard: build_scorecard(row, mix, window),
    }))
}

/// GET /api/scorecards?from&to — every active staff member's scorecard in
/// one org-wide read (owner/admin only; R5 private-by-default means a
/// plain tech never sees anyone else's card here, including via query
/// tricks — the scope is derived from the principal, never the request).
#[utoipa::path(operation_id = "scorecard_list_team", get, path = "/api/scorecards", tag = "reports", security(("bearer" = [])),
    params(ScorecardQuery),
    responses((status = 200, body = TeamScorecardsResponse),
              (status = 403, description = "team scorecards are owner/admin only"),
              (status = 422, description = "invalid window")))]
pub async fn team(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ScorecardQuery>,
) -> ApiResult<Json<TeamScorecardsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, reporting_view())?;
    if !authed.principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "team scorecards are visible to owners and admins only".into(),
        ));
    }
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let window = resolve_window(&q, chrono::Utc::now())?;

    let (_, rows, mixes) = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| {
        async move {
            // Active staff only: invited-but-not-redeemed and deactivated
            // accounts carry no coaching signal.
            let roster: Vec<Uuid> = sqlx::query_scalar!(
                r#"SELECT id FROM users WHERE org_id = $1 AND is_active ORDER BY display_name, id"#,
                org_id
            )
            .fetch_all(&mut *ctx.tx)
            .await?;
            let rows = core_db::scorecards::scorecard_for_users(
                &mut *ctx.tx,
                org_id,
                &roster,
                window.start,
                window.end,
            )
            .await?;
            let mixes = core_db::scorecards::work_type_mix(
                &mut *ctx.tx,
                org_id,
                &roster,
                window.start,
                window.end,
            )
            .await?;
            Ok((ctx, (roster, rows, mixes)))
        }
    })
    .await?;

    let mut by_user: std::collections::HashMap<Uuid, Vec<core_db::scorecards::WorkTypeMixRow>> =
        std::collections::HashMap::new();
    for m in mixes {
        by_user.entry(m.user_id).or_default().push(m);
    }
    let scorecards = rows
        .into_iter()
        .map(|r| {
            let mix = by_user.remove(&r.user_id).unwrap_or_default();
            build_scorecard(r, mix, window)
        })
        .collect();

    Ok(Json(TeamScorecardsResponse {
        window: ScorecardWindow {
            from: window.start,
            to: window.end,
        },
        scorecards,
    }))
}

/// GET /api/scorecards/{user_id}?from&to — one member's scorecard; a
/// member may read their own, owner/admin may read anyone's.
#[utoipa::path(operation_id = "scorecard_get_user", get, path = "/api/scorecards/{user_id}", tag = "reports", security(("bearer" = [])),
    params(ScorecardQuery, ("user_id" = Uuid, Path)),
    responses((status = 200, body = ScorecardResponse),
              (status = 403, description = "other members' scorecards are owner/admin only"),
              (status = 404, description = "member not in scope (rls_sparse)"),
              (status = 422, description = "invalid window")))]
pub async fn get_user(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(target): Path<Uuid>,
    Query(q): Query<ScorecardQuery>,
) -> ApiResult<Json<ScorecardResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, reporting_view())?;
    let caller = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    if target != caller && !authed.principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "other members' scorecards are visible to owners and admins only".into(),
        ));
    }
    let window = resolve_window(&q, chrono::Utc::now())?;

    let (row, mix) = org_tx::with_org_tx(&state, org_id, caller, |mut ctx| {
        let user_ids = vec![target];
        async move {
            let row = core_db::scorecards::scorecard_for_users(
                &mut *ctx.tx,
                org_id,
                &user_ids,
                window.start,
                window.end,
            )
            .await?
            .into_iter()
            .next()
            .ok_or(ApiError::RlsSparse)?;
            let mix = core_db::scorecards::work_type_mix(
                &mut *ctx.tx,
                org_id,
                &user_ids,
                window.start,
                window.end,
            )
            .await?;
            Ok((ctx, (row, mix)))
        }
    })
    .await?;

    Ok(Json(ScorecardResponse {
        scorecard: build_scorecard(row, mix, window),
    }))
}

/// GET /api/scorecards/me/settings — the caller's scorecard settings
/// (absent row reads as the private-by-defaults).
#[utoipa::path(operation_id = "scorecard_get_settings", get, path = "/api/scorecards/me/settings", tag = "reports", security(("bearer" = [])),
    responses((status = 200, body = ScorecardSettingsResponse),
              (status = 403, description = "missing reporting:view")))]
pub async fn get_settings(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<ScorecardSettingsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, reporting_view())?;
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;

    let settings = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let settings = core_db::scorecards::settings::get(&mut *ctx.tx, org_id, user_id).await?;
        Ok((ctx, settings))
    })
    .await?;

    Ok(Json(ScorecardSettingsResponse {
        user_id: settings.user_id,
        gamification_opt_in: settings.gamification_opt_in,
    }))
}

/// PATCH /api/scorecards/me/settings — flip the R5 opt-in gamification
/// toggle. Only ever touches the CALLER's row (no user_id in the body by
/// design); emits `scorecard.settings_updated` in the same transaction.
#[utoipa::path(operation_id = "scorecard_update_settings", patch, path = "/api/scorecards/me/settings", tag = "reports", security(("bearer" = [])),
    request_body = UpdateScorecardSettingsRequest,
    responses((status = 200, body = ScorecardSettingsResponse),
              (status = 403, description = "missing reporting:view")))]
pub async fn update_settings(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<UpdateScorecardSettingsRequest>,
) -> ApiResult<Json<ScorecardSettingsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, reporting_view())?;
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let now = chrono::Utc::now();

    let settings = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let settings = core_db::scorecards::settings::set_gamification(
            &mut *ctx.tx,
            org_id,
            user_id,
            req.gamification_opt_in,
            now,
        )
        .await?;
        ctx.emit(
            types::SCORECARD_SETTINGS_UPDATED,
            "scorecard_settings",
            Some(user_id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ScorecardSettingsUpdated {
                user_id,
                gamification_opt_in: settings.gamification_opt_in,
                occurred_at: now,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, settings))
    })
    .await?;

    Ok(Json(ScorecardSettingsResponse {
        user_id: settings.user_id,
        gamification_opt_in: settings.gamification_opt_in,
    }))
}
