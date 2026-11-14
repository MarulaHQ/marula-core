//! Labour cost rate per user (migration 0188) — the write surface the
//! client-margin report was missing.
//!
//! The report could already cost labour (`ops.v1_client_margin` reads
//! `COALESCE(users.cost_rate_minor, orgs.default_cost_rate_minor)`), but
//! nothing in the API could write either number, so every org's margin came
//! back as "cost unknown". This module is the narrow pair that fixes that:
//! one user's rate, and (in `settings::org_profile_patch`) the org default.
//!
//! ## The encoding: absent ≠ null ≠ value
//!
//! `PATCH` bodies here follow the house tri-state, `Option<Option<T>>` with
//! `deserialize_with = "double_option"` (`routes::double_option`), the same
//! shape `alerts.rs`, `labels.rs`, `purchasing.rs` and `email_admin.rs` use:
//!
//! | body                          | meaning                          |
//! |-------------------------------|----------------------------------|
//! | `{"cost_rate_minor": 4500}`   | set 4500 minor units per hour    |
//! | `{"cost_rate_minor": null}`   | clear — fall back to org default |
//! | `{}` (field absent)           | refused 422 (see below)          |
//!
//! The `deserialize_with` is what keeps the last two apart: serde's plain
//! `Option<Option<T>>` reads a JSON `null` as `None`, which is the absent
//! shape, so without the helper "clear my rate" would be indistinguishable
//! from "leave my rate alone". On this route an absent field is refused
//! rather than read as a no-op: the body carries exactly one field, so a
//! PATCH without it is a client bug, and answering 200 would hide it.
//!
//! The org route keeps the "absent = leave unchanged" half, because it
//! patches several fields at once — one encoding, two coherent readings.
//!
//! ## Permissions
//!
//! A user may read their OWN rate (it is their payroll number, and the settings
//! page shows it). Reading anybody else's is a financial read, so it takes
//! `ViewFinancials` — the existing money-read grant used by cost lines and
//! purchasing, not a new permission. Writing is user administration and takes
//! `ManageUsers`, exactly like the neighbouring `set_status` / `set_role`
//! routes on `/api/org/users`.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::routes::double_option;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_domain::policy::Permission;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// The wire shape of a rate: minor units per hour, `null` = unset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UserCostRateResponse {
    /// `null` means "no personal rate; the org default applies". It is not
    /// zero: `0` is a rate that says this user's hour costs nothing.
    #[schema(nullable = true)]
    pub cost_rate_minor: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchUserCostRateRequest {
    /// `{"cost_rate_minor": 4500}` sets; `{"cost_rate_minor": null}` clears
    /// (fall back to the org's default). Omitting the field is refused with
    /// 422 — this route patches one field, so absence is a client bug.
    /// Integer minor units per hour; negative and out-of-range values are
    /// refused with 422.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<i64>, nullable = true)]
    pub cost_rate_minor: Option<Option<i64>>,
}

/// The org's stored default, in the unit the column holds.
pub(crate) fn stored_rate(rate: i64) -> ApiResult<i32> {
    if rate < 0 {
        return Err(ApiError::Validation(
            "cost_rate_minor cannot be negative: it is minor units per hour".into(),
        ));
    }
    i32::try_from(rate).map_err(|_| {
        ApiError::Validation(format!(
            "cost_rate_minor is out of range: {rate} does not fit the column \
             (max {} minor units per hour)",
            i32::MAX
        ))
    })
}

fn view(rate: Option<i32>) -> Json<UserCostRateResponse> {
    Json(UserCostRateResponse {
        cost_rate_minor: rate.map(i64::from),
    })
}

/// GET /api/org/users/{id}/cost-rate
#[utoipa::path(operation_id = "user_cost_rate_get", get, path = "/api/org/users/{id}/cost-rate",
    tag = "org-users", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = UserCostRateResponse),
              (status = 403, description = "another user's rate needs `financial:read`"),
              (status = 404, description = "no such user in the caller's org")))]
pub async fn get(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<UserCostRateResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    // Own rate: yes. Anybody else's: the money-read grant.
    if id != authed.principal.user.id {
        require_permission(&authed.principal, Permission::ViewFinancials)?;
    }

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rate = core_db::cost_rates::user_cost_rate(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(view(rate.cost_rate_minor))
}

/// PATCH /api/org/users/{id}/cost-rate
#[utoipa::path(operation_id = "user_cost_rate_patch", patch, path = "/api/org/users/{id}/cost-rate",
    tag = "org-users", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = PatchUserCostRateRequest,
    responses((status = 200, body = UserCostRateResponse),
              (status = 403, description = "user administration requires `manage_users`"),
              (status = 404, description = "no such user in the caller's org"),
              (status = 422, description = "negative or out-of-range rate, or the field is absent")))]
pub async fn patch(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchUserCostRateRequest>,
) -> ApiResult<Json<UserCostRateResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManageUsers)?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    // Absent is not "no change" here: refuse it, so a client that forgets the
    // field hears about it instead of believing it cleared a rate.
    let Some(requested) = req.cost_rate_minor else {
        return Err(ApiError::Validation(
            "cost_rate_minor is required: send an integer to set it, or null to \
             clear it (fall back to the org default)"
                .into(),
        ));
    };
    let stored = requested.map(stored_rate).transpose()?;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, actor_id)
        .await
        .map_err(ApiError::from)?;
    let before = core_db::cost_rates::user_cost_rate(&mut *tx, org_id, id).await?;
    let after = core_db::cost_rates::set_user_cost_rate(&mut *tx, org_id, id, stored).await?;
    core_db::audit::insert(
        &mut *tx,
        &core_domain::entities::AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: core_domain::entities::Actor::Human(actor_id),
            action: "user.cost_rate_changed".to_string(),
            entity_type: Some("user".to_string()),
            entity_id: Some(id.to_string()),
            // Both values, so a payroll correction is readable after the
            // fact without joining the audit log to anything.
            detail: serde_json::json!({
                "from": before.cost_rate_minor.map(i64::from),
                "to": after.cost_rate_minor.map(i64::from),
            }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(view(after.cost_rate_minor))
}
