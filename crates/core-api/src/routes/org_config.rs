//! Org-scoped configuration endpoints (Wave 12 lane 6).
//!
//! Purpose: the server store for the ticket-workspace admin per-RBAC
//! pod-visibility matrix (`org_pod_visibility`, migration 0064). The
//! matrix was previously device-local via the panel-state engine; an org
//! config endpoint makes it real for every member. Reads are any-member
//! (pods are visible config, not secrets); writes require the Admin:edit
//! matrix grant and emit `org.pod_visibility_updated` in the same tx.
//!
//! Non-purpose: the typed settings registry (pod visibility is a matrix —
//! a richer structure, therefore an entity, not a setting) and pod state
//! per user (hidden/order stays per-user client state). Shape validation
//! lives here so the stored document is always a bounded, level-scoped
//! matrix; the pod/level vocabularies stay client registries so pods can
//! be added without backend changes.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{extract::State, http::Extensions, Json};
use chrono::{DateTime, Utc};
use core_domain::entities::AuditEvent;
use core_domain::events::{self, types};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

/// Valid RBAC levels (the org-users lane's builtin roles). Levels outside
/// this set cannot exist in the stored matrix.
const RBAC_LEVELS: &[&str] = &["owner", "admin", "dispatcher", "technician", "read_only"];

const MAX_POD_ENTRIES: usize = 64;
const MAX_POD_KEY_LEN: usize = 64;
/// Safety cap for the stored document; even a fully-populated matrix is
/// a few KiB, so anything larger is a malformed write, not a config.
const MAX_CONFIG_BYTES: usize = 32 * 1024;

/// GET /api/org/pod-visibility — the org's stored matrix; `config` is
/// `{}` when the org never customized (clients fall back to their
/// registry defaults, defaults-first).
#[utoipa::path(operation_id = "org_pod_visibility_get", get,
    path = "/api/org/pod-visibility", tag = "settings", security(("bearer" = [])),
    responses((status = 200, body = PodVisibilityView)))]
pub async fn pod_visibility_get(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<PodVisibilityView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let row = core_db::org_config::get_pod_visibility(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(match row {
        Some(row) => PodVisibilityView {
            config: row.config,
            updated_by: row.updated_by,
            updated_at: row.updated_at,
        },
        // Defaults-first: an untouched org reports the empty matrix.
        None => PodVisibilityView {
            config: serde_json::json!({}),
            updated_by: None,
            updated_at: chrono::DateTime::<Utc>::UNIX_EPOCH,
        },
    }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PodVisibilityView {
    /// Pod id → RBAC level → visible. Absent entries fall back to the
    /// client registry defaults.
    pub config: serde_json::Value,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub updated_by: Option<Uuid>,
    #[schema(value_type = String, format = "date-time")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PutPodVisibilityRequest {
    /// The full matrix; the store is whole-document (admins edit the
    /// matrix in one pass). Must satisfy [`validate_matrix`].
    pub config: serde_json::Value,
}

/// Validates the matrix document: an object of `pod_id → level → bool`
/// with whitelisted levels, bounded keys, and a bounded serialized size.
fn validate_matrix(config: &serde_json::Value) -> ApiResult<()> {
    let obj = config.as_object().ok_or_else(|| {
        ApiError::Validation("config must be an object of pod → level → boolean".into())
    })?;
    if obj.len() > MAX_POD_ENTRIES {
        return Err(ApiError::Validation(format!(
            "config carries more than {MAX_POD_ENTRIES} pod entries"
        )));
    }
    if serde_json::to_vec(config).map_or(true, |v| v.len() > MAX_CONFIG_BYTES) {
        return Err(ApiError::Validation("config document too large".into()));
    }
    for (pod_id, levels) in obj {
        if pod_id.is_empty()
            || pod_id.len() > MAX_POD_KEY_LEN
            || !pod_id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(ApiError::Validation(format!(
                "invalid pod id: {pod_id:?} (lowercase letters/digits/-/_ only)"
            )));
        }
        let levels_obj = levels.as_object().ok_or_else(|| {
            ApiError::Validation(format!("pod {pod_id:?} must map levels to booleans"))
        })?;
        for (level, visible) in levels_obj {
            if !RBAC_LEVELS.contains(&level.as_str()) {
                return Err(ApiError::Validation(format!(
                    "unknown RBAC level {level:?} (one of: {})",
                    RBAC_LEVELS.join(", ")
                )));
            }
            if !visible.is_boolean() {
                return Err(ApiError::Validation(format!(
                    "pod {pod_id:?} level {level:?} must be a boolean"
                )));
            }
        }
    }
    Ok(())
}

/// PUT /api/org/pod-visibility — rewrites the org's matrix (Admin:edit).
/// Emits `org.pod_visibility_updated` + audit row in the same tx.
#[utoipa::path(operation_id = "org_pod_visibility_put", put,
    path = "/api/org/pod-visibility", tag = "settings", security(("bearer" = [])),
    request_body = PutPodVisibilityRequest,
    responses((status = 200, body = PodVisibilityView),
              (status = 403), (status = 422, description = "invalid matrix shape")))]
pub async fn pod_visibility_put(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PutPodVisibilityRequest>,
) -> ApiResult<Json<PodVisibilityView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    validate_matrix(&req.config)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::org_config::upsert_pod_visibility(&mut ctx.tx, org_id, &req.config, user_id)
            .await?;
        ctx.emit(
            types::POD_VISIBILITY_UPDATED,
            "org",
            Some(org_id.to_string().as_str()),
            &serde_json::to_value(events::PodVisibilityUpdated {
                config: req.config.clone(),
                updated_by: Some(user_id),
            })
            .unwrap_or_default(),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: "org.pod_visibility_updated".to_string(),
                entity_type: Some("org".to_string()),
                entity_id: Some(org_id.to_string()),
                detail: serde_json::json!({ "config": req.config }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((
            ctx,
            PodVisibilityView {
                config: req.config,
                updated_by: Some(user_id),
                updated_at: chrono::Utc::now(),
            },
        ))
    })
    .await
    .map(Json)
}
