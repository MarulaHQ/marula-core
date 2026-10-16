//! Tenant suspension notice flag (TASKLIST Wave 47 lane A7,
//! `p47-org-suspension-notice`, migration 0136).
//!
//! Two endpoints under `/api/v1/orgs/{org_id}/suspension-status`:
//!
//!   * `GET .../suspension-status` — the notice state: when the org was
//!     flagged, why, when the notice starts showing, and the two links a
//!     surface needs to act on it (where to export the org's data, and the
//!     guide that explains the export).
//!   * `PATCH .../suspension-status` — set or clear the flag. Admin-only
//!     (`admin:edit`, the same permission org settings use — TASKLIST A7 (D)).
//!
//! THE POSTURE — NOTICE, NEVER A BLOCK (TASKLIST A7 (C), load-bearing).
//! This module is the whole product surface of the flag, and it does not
//! reject anything: it writes three columns on `orgs` and reads them back. No
//! middleware in this crate consults the flag, no route handler branches on
//! it, and no feature is gated. A flagged org keeps every endpoint, plugin and
//! write it had the day before. The hosted tier uses the flag so its in-app
//! banner can tell a customer that billing lapsed and offer the way out;
//! enforcement, if a hosted tier ever wants it, belongs to that tier, not to
//! this core. A later lane that adds a gate here is changing this product
//! decision, not extending it.
//!
//! AUTHORIZATION:
//!   * READ (GET): any human member of the org reads its own state; a service
//!     account (machine key) additionally needs `org_suspension:read` — the
//!     permission the `org_suspension.read` plugin capability resolves onto,
//!     and the grant the hosted-tier banner plugin holds.
//!   * MANAGE (PATCH): a human owner/admin through the Admin matrix
//!     (`admin:edit`, exactly like org settings). There is deliberately NO
//!     machine manage grant: TASKLIST A7 (D) makes setting the flag an admin
//!     action, so a machine key is refused with the same typed 403 a
//!     non-admin human gets.
//!   * A path org that is not the caller's own org is a 404, identical to
//!     "no such org", so a cross-org probe learns nothing.
//!   * The database access is RLS-scoped from the AUTHENTICATED PRINCIPAL's
//!     org (never the path value), so a future authorization slip reads and
//!     writes zero rows rather than another org's.

use crate::auth::middleware::{authed, require_permission};
use crate::auth::AuthedUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// The closed set of reasons the schema accepts (migration 0136's CHECK).
/// Validated here so a bad value is a typed 422 with the allowed set in the
/// message rather than a raw constraint violation.
const SUSPENSION_REASONS: &[&str] = &["billing_lapsed", "abuse", "other"];

/// Where a customer pulls their org's data out. This is a path, not an
/// absolute URL, on purpose: the API cannot know a deployment's public web
/// origin and no configuration carries one, so an origin-relative path
/// resolves everywhere. It names the EXISTING export surface — the same
/// collection `/settings/export` in the app drives — so a machine caller can
/// POST it directly and a web banner can prefix its own origin, exactly as it
/// already does for the export download link.
const DATA_EXPORT_URL: &str = "/api/v1/org/exports";

/// The guide that explains the export. Absolute because the docs bundle is
/// published at its own origin; the value matches the in-app help manifest's
/// `docsBase` + path (`docs/site/help-manifest.json`), which publishes
/// `docs/site/how-to/export-data.md` to this exact href, so the banner link
/// and the help drawer point at one page.
const EXPORT_DOCS_URL: &str = "https://marula.app/docs/how-to/export-data.html";

/// The human manage gate: the SAME permission org settings use — TASKLIST A7
/// (D)'s `ManageOrg` gate. Declared once so the intent is readable at the
/// call site; a machine key's permission set never carries the Admin matrix,
/// so it is refused here without a second branch.
fn admin_edit() -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action: Action::Edit,
    }
}

/// Read gate: a human member reads their own org; a machine key additionally
/// holds `org_suspension:read`. A non-own org is a 404.
fn authorize_read(authed: &AuthedUser, org_id: Uuid) -> ApiResult<()> {
    if authed.via_api_key {
        require_permission(&authed.principal, Permission::ViewOrgSuspension)?;
    }
    if authed.principal.user.org_id != org_id {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

/// Manage gate: the Admin `edit` matrix (owner/admin) for a human, nothing for
/// a machine. A non-own org is a 404.
fn authorize_manage(authed: &AuthedUser, org_id: Uuid) -> ApiResult<()> {
    require_permission(&authed.principal, admin_edit())?;
    if authed.principal.user.org_id != org_id {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

// --- wire shapes -------------------------------------------------------------

/// The notice state plus the two links a surface renders beside it.
#[derive(Debug, Serialize, ToSchema)]
pub struct SuspensionStatusView {
    pub org_id: Uuid,
    /// When the org was flagged. `null` = not flagged.
    #[schema(value_type = Option<String>, format = "date-time")]
    pub suspended_at: Option<chrono::DateTime<chrono::Utc>>,
    /// `billing_lapsed` | `abuse` | `other`. `null` = not flagged.
    pub reason: Option<String>,
    /// Whole seconds after `suspended_at` before the notice starts showing.
    /// `null` = the notice is due immediately.
    pub notify_after: Option<i64>,
    /// Where to pull the org's data out (the existing export surface).
    pub data_export_url: String,
    /// The guide that explains that export.
    pub docs_url: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetSuspensionRequest {
    /// `true` flags the org; `false` clears the flag and both companions.
    /// Setting an already-flagged org is idempotent — it keeps the original
    /// `suspended_at`, so a retry cannot restart the notice countdown.
    pub suspended: bool,
    /// Required when `suspended` is `true`, refused when it is `false`:
    /// `billing_lapsed`, `abuse`, or `other`.
    #[serde(default)]
    pub reason: Option<String>,
    /// Whole seconds after the flag is set before the notice starts showing.
    /// Omitted or `null` = due immediately.
    #[serde(default)]
    pub notify_after: Option<i64>,
}

fn view(org_id: Uuid, row: core_db::org_suspension::OrgSuspension) -> SuspensionStatusView {
    SuspensionStatusView {
        org_id,
        suspended_at: row.suspended_at,
        reason: row.reason,
        notify_after: row.notify_after_secs,
        data_export_url: DATA_EXPORT_URL.to_string(),
        docs_url: EXPORT_DOCS_URL.to_string(),
    }
}

/// Validates the write body against the closed reason set. Returns the reason
/// to store when the request flags the org, and `None` when it clears it —
/// which is what makes "clearing ignores the companions" true at the type
/// level instead of by convention.
fn validated_reason(req: &SetSuspensionRequest) -> ApiResult<Option<&'static str>> {
    if !req.suspended {
        // Clearing ignores the companions rather than refusing them, so an
        // "unflag" request that echoes the previous reason still works.
        return Ok(None);
    }
    let reason = req
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ApiError::Validation(format!(
                "reason is required when suspending; one of {}",
                SUSPENSION_REASONS.join(", ")
            ))
        })?;
    let found = SUSPENSION_REASONS
        .iter()
        .copied()
        .find(|r| *r == reason)
        .ok_or_else(|| {
            ApiError::Validation(format!(
                "reason must be one of {}",
                SUSPENSION_REASONS.join(", ")
            ))
        })?;
    Ok(Some(found))
}

// --- handlers ----------------------------------------------------------------

/// `GET /api/v1/orgs/{org_id}/suspension-status` — the org's notice state.
#[utoipa::path(operation_id = "org_suspension_status_get", get,
    path = "/api/orgs/{org_id}/suspension-status", tag = "org",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "the org; must be the caller's own org")),
    responses((status = 200, body = SuspensionStatusView),
              (status = 401, description = "not authenticated"),
              (status = 403, description = "service account without org_suspension:read"),
              (status = 404, description = "no such org for this caller")))]
pub async fn get_suspension_status(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(org_id): Path<Uuid>,
) -> ApiResult<Json<SuspensionStatusView>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    authorize_read(&authed, org_id)?;
    // RLS-scoped read (pool carries no app.org_id GUC). The scope comes from
    // the AUTHENTICATED PRINCIPAL, never the path value the caller sent: the
    // 404 gate above already established that the two are equal, so binding
    // from the principal keeps the database scope independent of request
    // input and makes the read fail closed by construction.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let row = core_db::org_suspension::get(&mut *tx, org_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(view(org_id, row)))
}

/// `PATCH /api/v1/orgs/{org_id}/suspension-status` — set or clear the flag
/// (`admin:edit`). Idempotent; emits `org.suspension_changed` + an audit row.
#[utoipa::path(operation_id = "org_suspension_status_patch", patch,
    path = "/api/orgs/{org_id}/suspension-status", tag = "org",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "the org; must be the caller's own org")),
    request_body = SetSuspensionRequest,
    responses((status = 200, body = SuspensionStatusView),
              (status = 401, description = "not authenticated"),
              (status = 403, description = "not an admin (no admin:edit)"),
              (status = 404, description = "no such org for this caller"),
              (status = 422, description = "unknown reason, or a missing one when suspending")))]
pub async fn set_suspension_status(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(org_id): Path<Uuid>,
    Json(req): Json<SetSuspensionRequest>,
) -> ApiResult<Json<SuspensionStatusView>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    authorize_manage(&authed, org_id)?;
    let reason = validated_reason(&req)?;
    let principal_org = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let notify_after = req.notify_after;

    let row = crate::org_tx::with_org_tx(&state, principal_org, user_id, |mut ctx| async move {
        let row = match reason {
            Some(reason) => {
                core_db::org_suspension::set(&mut *ctx.tx, principal_org, reason, notify_after)
                    .await?
            }
            None => core_db::org_suspension::clear(&mut *ctx.tx, principal_org).await?,
        };
        // The event payload mirrors the custom-domain lifecycle: `action` is
        // the one always-present key, and the reason/interval ride along when
        // the flag is set.
        ctx.emit(
            types::ORG_SUSPENSION_CHANGED,
            "org",
            Some(principal_org.to_string().as_str()),
            &serde_json::json!({
                "action": if reason.is_some() { "suspended" } else { "cleared" },
                "reason": row.reason,
                "notify_after_secs": row.notify_after_secs,
            }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id: principal_org,
                actor: Actor::Human(user_id),
                action: if reason.is_some() {
                    "org.suspension_flagged".to_string()
                } else {
                    "org.suspension_cleared".to_string()
                },
                entity_type: Some("org".to_string()),
                entity_id: Some(principal_org.to_string()),
                detail: serde_json::json!({
                    "reason": row.reason,
                    "notify_after_secs": row.notify_after_secs,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((ctx, row))
    })
    .await?;

    Ok(Json(view(org_id, row)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(
        suspended: bool,
        reason: Option<&str>,
        notify_after: Option<i64>,
    ) -> SetSuspensionRequest {
        SetSuspensionRequest {
            suspended,
            reason: reason.map(str::to_string),
            notify_after,
        }
    }

    #[test]
    fn suspending_requires_a_reason_from_the_closed_set() {
        assert_eq!(
            validated_reason(&req(true, Some("billing_lapsed"), None)).unwrap(),
            Some("billing_lapsed")
        );
        assert_eq!(
            validated_reason(&req(true, Some(" abuse "), None)).unwrap(),
            Some("abuse")
        );
        // A missing reason is refused: the schema's coherence constraint would
        // reject the write anyway, and a typed 422 names the allowed set.
        assert!(validated_reason(&req(true, None, None)).is_err());
        assert!(validated_reason(&req(true, Some("   "), None)).is_err());
        assert!(validated_reason(&req(true, Some("late_payment"), None)).is_err());
    }

    #[test]
    fn clearing_yields_no_reason_and_ignores_the_companions() {
        // An unflag request that echoes the previous reason still clears.
        assert_eq!(
            validated_reason(&req(false, Some("billing_lapsed"), Some(60))).unwrap(),
            None
        );
        assert_eq!(validated_reason(&req(false, None, None)).unwrap(), None);
    }

    #[test]
    fn the_two_links_are_the_export_surface_and_its_guide() {
        let row = core_db::org_suspension::OrgSuspension {
            org_id: Uuid::nil(),
            suspended_at: None,
            reason: None,
            notify_after_secs: None,
        };
        let v = view(Uuid::nil(), row);
        assert_eq!(v.data_export_url, "/api/v1/org/exports");
        assert!(v.docs_url.ends_with("/how-to/export-data.html"));
        // Not flagged: the notice fields are null, the links are still there
        // so a surface can render the "get your data out" affordance either
        // way.
        assert!(v.suspended_at.is_none());
        assert!(v.reason.is_none());
        assert!(v.notify_after.is_none());
    }
}
