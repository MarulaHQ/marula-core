//! Customization-manifest export (TASKLIST U5, upgrade-compatibility-policy
//! .md §5).
//!
//! Purpose: one admin-gated GET returning the org's canonical, schema-
//! versioned [`CustomizationManifest`] — the read-only inventory of
//! everything the org has customized (custom field definitions, per-board
//! workflow statuses/transitions, automation rules, webhook subscriptions
//! with filters, API-key metadata minus secrets, org settings-registry
//! overrides, SLA calendars/policies/ladders).
//!
//! Consumers: backup documentation, the upgrade preflight (`meridian
//! upgrade --check`, U6) and the org-data export (D1, which generalizes
//! this envelope — same format, wider scope). The envelope evolution
//! contract lives on the type: sections grow additively, `manifest_version`
//! bumps only for breaking reshapes, consumers tolerate unknown fields.
//!
//! Non-purpose: no import/apply path (the manifest documents, it does not
//! mutate), no operational data (D1), no filtering/query params (an org's
//! manifest is one canonical document — deterministic by the loader
//! contract in core-db `manifest`).

use crate::auth::middleware::authed;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{extract::State, http::Extensions, Json};
use core_domain::entities::manifest::CustomizationManifest;

/// GET /api/org/manifest — the org's customization manifest.
///
/// Owner/admin gated (policy §5: the manifest is an org-level
/// administrative artifact; technicians and dispatchers have no business
/// reading the full customization inventory, e.g. webhook URLs).
#[utoipa::path(operation_id = "org_manifest", get, path = "/api/org/manifest", tag = "org", security(("bearer" = [])),
    responses((status = 200, body = CustomizationManifest)))]
pub async fn get(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<CustomizationManifest>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    if !authed.principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "the customization manifest is limited to org admins".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let manifest =
        core_db::manifest::load_customization_manifest(&mut tx, org_id, chrono::Utc::now()).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(manifest))
}
