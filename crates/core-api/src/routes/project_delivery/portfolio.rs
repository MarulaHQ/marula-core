//! Portfolio quarter: every project on one axis, grouped, with named reasons.
//!
//! The risk read is deterministic and its reason is a sentence, never a colour
//! (S7.4). A project with no dated phase is "not enough movement to assess",
//! not healthy, and a project with no dates waits in the tray rather than being
//! drawn somewhere it was never scheduled (S7.8).

use super::*;
use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PortfolioPhase {
    pub id: Uuid,
    pub name: String,
    pub status: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PortfolioProject {
    pub id: Uuid,
    pub name: String,
    pub status: String,
    pub client_id: Option<Uuid>,
    pub client_name: Option<String>,
    pub start_date: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
    pub at_risk: bool,
    pub risk_reasons: Vec<String>,
    pub has_dated_phase: bool,
    pub phases: Vec<PortfolioPhase>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PortfolioGroup {
    pub key: String,
    pub label: String,
    pub project_count: i32,
    pub at_risk_count: i32,
    pub projects: Vec<PortfolioProject>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PortfolioWeek {
    pub start: NaiveDate,
    pub label: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UnscheduledProject {
    pub id: Uuid,
    pub name: String,
    pub status: String,
    pub start_date: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PortfolioView {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub zoom: String,
    pub group_by: String,
    pub weeks: Vec<PortfolioWeek>,
    pub groups: Vec<PortfolioGroup>,
    pub unscheduled: Vec<UnscheduledProject>,
}

#[derive(Debug, Deserialize)]
pub struct QuarterQuery {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    #[serde(default = "default_group")]
    pub group_by: String,
    #[serde(default)]
    pub at_risk_only: bool,
    #[serde(default = "default_zoom")]
    pub zoom: String,
}

fn default_group() -> String {
    "client".into()
}

fn default_zoom() -> String {
    "quarter".into()
}

/// The named reasons a project is at risk. A reason is a sentence a
/// coordinator can act on, and the rule is deliberately small: overdue, on
/// hold, or active with nothing scheduled.
pub fn risk_reasons(
    status: &str,
    due_date: Option<&str>,
    has_dated_phase: bool,
    is_active: bool,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if status == "on_hold" {
        reasons.push("On hold and not moving".to_string());
    }
    if let Some(due) = due_date {
        if let Ok(due) = NaiveDate::parse_from_str(due, "%Y-%m-%d") {
            if due < Utc::now().date_naive() && status != "completed" {
                reasons.push(format!(
                    "Past its due date ({} days)",
                    (Utc::now().date_naive() - due).num_days()
                ));
            }
        }
    }
    if is_active && !has_dated_phase {
        reasons.push("Active but nothing is scheduled".to_string());
    }
    reasons
}

fn week_label(start: NaiveDate) -> String {
    let month = start.format("%b");
    format!("Wk of {month} {}", start.day())
}

fn built_weeks(from: NaiveDate, to: NaiveDate, zoom: &str) -> Vec<PortfolioWeek> {
    let step = 7;
    let max = if zoom == "weeks" { 4 } else { 26 };
    let mut weeks = Vec::new();
    let mut cursor = from;
    while cursor <= to && weeks.len() < max {
        weeks.push(PortfolioWeek {
            start: cursor,
            label: week_label(cursor),
        });
        cursor += Duration::days(step);
    }
    weeks
}

struct Built {
    view: PortfolioView,
}

async fn build(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    query: &QuarterQuery,
) -> ApiResult<Built> {
    let today = Utc::now().date_naive();
    let from = query.from.unwrap_or_else(|| {
        let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
        monday - Duration::days(28)
    });
    let to = query
        .to
        .unwrap_or_else(|| from + Duration::days(7 * 12 - 1));
    if to < from {
        return Err(ApiError::Validation("to must not precede from".into()));
    }

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::project_delivery::list_portfolio_rows(&mut *tx, org_id, from, to).await?;
    let unscheduled =
        core_db::project_delivery::list_unscheduled_projects(&mut *tx, org_id, from, to).await?;
    let clients = core_db::clients::list_clients(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let client_names: std::collections::HashMap<Uuid, String> =
        clients.into_iter().map(|c| (c.id, c.name)).collect();

    // Fold the flat rows into one entry per project, preserving first-seen
    // order so the chart reads like the list.
    let mut order: Vec<Uuid> = Vec::new();
    let mut projects: std::collections::HashMap<Uuid, PortfolioProject> =
        std::collections::HashMap::new();
    for row in rows {
        let entry = projects.entry(row.project_id).or_insert_with(|| {
            order.push(row.project_id);
            let reasons = risk_reasons(
                &row.project_status,
                row.due_date.map(|d| d.to_string()).as_deref(),
                false,
                row.project_status == "active",
            );
            PortfolioProject {
                id: row.project_id,
                name: row.project_name.clone(),
                status: row.project_status.clone(),
                client_id: row.client_id,
                client_name: row.client_id.and_then(|id| client_names.get(&id).cloned()),
                start_date: row.start_date,
                due_date: row.due_date,
                at_risk: !reasons.is_empty(),
                risk_reasons: reasons,
                has_dated_phase: false,
                phases: Vec::new(),
            }
        });
        if let (Some(id), Some(name), Some(status), Some(start), Some(end)) = (
            row.phase_id,
            row.phase_name,
            row.phase_status,
            row.phase_start,
            row.phase_end,
        ) {
            entry.has_dated_phase = true;
            entry.phases.push(PortfolioPhase {
                id,
                name,
                status,
                start_date: start,
                end_date: end,
            });
        }
    }

    // Recompute risk now that dated phases are known.
    for project in projects.values_mut() {
        project.risk_reasons = risk_reasons(
            &project.status,
            project.due_date.map(|d| d.to_string()).as_deref(),
            project.has_dated_phase,
            project.status == "active",
        );
        project.at_risk = !project.risk_reasons.is_empty();
    }

    let mut list: Vec<PortfolioProject> = order
        .into_iter()
        .filter_map(|id| projects.remove(&id))
        .filter(|p| !query.at_risk_only || p.at_risk)
        .collect();

    // Group.
    let mut grouped: std::collections::BTreeMap<String, PortfolioGroup> =
        std::collections::BTreeMap::new();
    for project in list.drain(..) {
        let (key, label) = match query.group_by.as_str() {
            "status" => (project.status.clone(), status_label(&project.status)),
            "none" => ("all".into(), "All projects".into()),
            _ => (
                project
                    .client_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "none".into()),
                project
                    .client_name
                    .clone()
                    .unwrap_or_else(|| "No client".into()),
            ),
        };
        let group = grouped
            .entry(key.clone())
            .or_insert_with(|| PortfolioGroup {
                key,
                label,
                project_count: 0,
                at_risk_count: 0,
                projects: Vec::new(),
            });
        group.project_count += 1;
        if project.at_risk {
            group.at_risk_count += 1;
        }
        group.projects.push(project);
    }

    Ok(Built {
        view: PortfolioView {
            from,
            to,
            zoom: query.zoom.clone(),
            group_by: query.group_by.clone(),
            weeks: built_weeks(from, to, &query.zoom),
            groups: grouped.into_values().collect(),
            unscheduled: unscheduled
                .into_iter()
                .map(
                    |(id, name, status, start_date, due_date)| UnscheduledProject {
                        id,
                        name,
                        status,
                        start_date,
                        due_date,
                    },
                )
                .collect(),
        },
    })
}

fn status_label(status: &str) -> String {
    match status {
        "planning" => "Planning".into(),
        "active" => "Active".into(),
        "on_hold" => "On hold".into(),
        "completed" => "Completed".into(),
        other => other.to_string(),
    }
}

/// GET /api/projects/portfolio
#[utoipa::path(operation_id = "project_portfolio_quarter", get, path = "/api/projects/portfolio", tag = "projects", security(("bearer" = [])),
    params(("from" = Option<NaiveDate>, Query), ("to" = Option<NaiveDate>, Query),
           ("group_by" = Option<String>, Query), ("at_risk_only" = Option<bool>, Query),
           ("zoom" = Option<String>, Query)),
    responses((status = 200, body = PortfolioView)))]
pub async fn quarter(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(query): Query<QuarterQuery>,
) -> ApiResult<Json<PortfolioView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let built = build(
        &state,
        authed.principal.user.org_id,
        authed.principal.user.id,
        &query,
    )
    .await?;
    Ok(Json(built.view))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ShareView {
    pub token: String,
    pub path: String,
}

/// POST /api/projects/portfolio/share — a link reproduces this exact view.
#[utoipa::path(operation_id = "project_portfolio_share", post, path = "/api/projects/portfolio/share", tag = "projects", security(("bearer" = [])),
    responses((status = 200, body = ShareView)))]
pub async fn share(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(query): Query<QuarterQuery>,
) -> ApiResult<Json<ShareView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let built = build(&state, org_id, user_id, &query).await?;
    let spec = serde_json::json!({
        "from": built.view.from,
        "to": built.view.to,
        "group_by": built.view.group_by,
        "zoom": built.view.zoom,
    });
    let snapshot = serde_json::to_value(&built.view).unwrap_or_default();
    let token = format!("{}{}", Uuid::now_v7().simple(), Uuid::now_v7().simple());

    let share_token = token.clone();
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::project_delivery::insert_portfolio_share(
            &mut ctx.tx,
            org_id,
            &share_token,
            &spec,
            &snapshot,
            Some(user_id),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;

    Ok(Json(ShareView {
        path: format!("/projects/portfolio?share={token}"),
        token,
    }))
}

/// GET /api/projects/portfolio/shared/{token} — the picture that was sent,
/// not a live re-query.
#[utoipa::path(operation_id = "project_portfolio_shared", get, path = "/api/projects/portfolio/shared/{token}", tag = "projects", security(("bearer" = [])),
    params(("token" = String, Path)),
    responses((status = 200, body = PortfolioView),
              (status = 404, description = "the link is unknown or expired")))]
pub async fn shared(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(token): Path<String>,
) -> ApiResult<Json<PortfolioView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let share = core_db::project_delivery::get_portfolio_share(&mut *tx, org_id, &token).await?;
    tx.commit().await.map_err(ApiError::from)?;
    let share = share.ok_or(ApiError::NotFound)?;
    let view: PortfolioView = serde_json::from_value(share.snapshot)
        .map_err(|_| ApiError::Validation("the shared snapshot is unreadable".into()))?;
    Ok(Json(view))
}
