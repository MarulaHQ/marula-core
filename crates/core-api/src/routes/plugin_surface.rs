//! Plugin SURFACE registry reads (Wave 34 lane 4, §4.6): the org's
//! registered plugin contributions to first-party surfaces — dashboard /
//! ticket-pane widgets, automation action kinds, report render kinds,
//! and custom-field validators.
//!
//! Posture: entries are non-secret DECLARATIONS (closed render kinds,
//! typed param schemas, JSON-Schema validators) — no org data rides in
//! them, so any authed org member may list; each CONSUMING surface keeps
//! its own data gates (widgets show report data only to viewers who pass
//! the reporting gate, plugin automation actions execute only through
//! the rule engine's effect authorization, validators only shape
//! writes). §4.8 holds by construction: nothing here is executable
//! content.

use crate::auth::middleware::authed;
use crate::error::ApiResult;
use crate::state::AppState;
use axum::{extract::State, http::Extensions, Json};
use core_db::plugin_surface::SurfaceEntryRow;
use serde::Serialize;
use uuid::Uuid;

/// One registered entry as the web consumes it.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SurfaceEntryView {
    #[schema(value_type = uuid::Uuid)]
    pub install_id: Uuid,
    pub entry_key: String,
    pub label: String,
    pub definition: serde_json::Value,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SurfaceEntryList {
    pub entries: Vec<SurfaceEntryView>,
}

fn to_views(rows: Vec<SurfaceEntryRow>) -> Vec<SurfaceEntryView> {
    rows.into_iter()
        .map(|r| SurfaceEntryView {
            install_id: r.install_id,
            entry_key: r.entry_key,
            label: r.label,
            definition: r.definition,
        })
        .collect()
}

/// Org-RLS-scoped read (the `set_org` posture every org-scoped read
/// here uses): transaction, org context, query, commit.
async fn org_entries(
    state: &AppState,
    org_id: Uuid,
    surface: &str,
) -> ApiResult<Vec<SurfaceEntryRow>> {
    let mut tx = state.pool.begin().await.map_err(crate::error::ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let rows = core_db::plugin_surface::list_entries(&mut *tx, org_id, surface).await?;
    tx.commit().await.map_err(crate::error::ApiError::from)?;
    Ok(rows)
}

/// GET /api/plugins/surface/widgets — the org's registered plugin
/// widgets (dashboard + ticket-pane slots), enabled installs only.
#[utoipa::path(operation_id = "plugin_surface_widgets", get, path = "/api/plugins/surface/widgets", tag = "plugins", security(("bearer" = [])),
    responses((status = 200, body = SurfaceEntryList)))]
pub async fn list_widgets(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<SurfaceEntryList>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let rows = org_entries(&state, org_id, "widget").await?;
    Ok(Json(SurfaceEntryList {
        entries: to_views(rows),
    }))
}

/// GET /api/plugins/surface/automation-actions — the org's registered
/// plugin automation action kinds (the automations editor lists them
/// alongside the built-ins).
#[utoipa::path(operation_id = "plugin_surface_automation_actions", get, path = "/api/plugins/surface/automation-actions", tag = "plugins", security(("bearer" = [])),
    responses((status = 200, body = SurfaceEntryList)))]
pub async fn list_automation_actions(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<SurfaceEntryList>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let rows = org_entries(&state, org_id, "automation_action").await?;
    Ok(Json(SurfaceEntryList {
        entries: to_views(rows),
    }))
}

/// GET /api/plugins/surface/report-renders — the org's registered
/// plugin render kinds for the report builder.
#[utoipa::path(operation_id = "plugin_surface_report_renders", get, path = "/api/plugins/surface/report-renders", tag = "plugins", security(("bearer" = [])),
    responses((status = 200, body = SurfaceEntryList)))]
pub async fn list_report_renders(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<SurfaceEntryList>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let rows = org_entries(&state, org_id, "report_render").await?;
    Ok(Json(SurfaceEntryList {
        entries: to_views(rows),
    }))
}

/// GET /api/plugins/surface/field-validators — the org's registered
/// plugin schema validators for board custom fields.
#[utoipa::path(operation_id = "plugin_surface_field_validators", get, path = "/api/plugins/surface/field-validators", tag = "plugins", security(("bearer" = [])),
    responses((status = 200, body = SurfaceEntryList)))]
pub async fn list_field_validators(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<SurfaceEntryList>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let rows = org_entries(&state, org_id, "field_validator").await?;
    Ok(Json(SurfaceEntryList {
        entries: to_views(rows),
    }))
}
