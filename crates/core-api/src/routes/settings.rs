//! Settings hub endpoints: the typed registry projection plus org- and
//! user-scoped value read/write (docs/10-agent-platform.md §2).
//!
//! Purpose: GET /api/settings/registry serves the declarations so any UI
//! renders settings pages FROM the registry; /api/settings/org and
//! /api/settings/user resolve effective values defaults-first and accept
//! validated overrides. Org writes require the Admin:edit matrix grant;
//! user writes are self-scoped. Every accepted change emits a `setting.updated`
//! outbox event AND an append-only audit record in the same transaction.
//! The org profile endpoints here back General settings and the Tier-1
//! first-run wizard.
//!
//! Non-purpose: notification preferences (dedicated surface), email inbound
//! routing, OIDC providers — those keep their own admin endpoints; no
//! per-setting history beyond audit_events.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{extract::State, http::Extensions, Json};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use core_domain::settings::{self as registry, SettingDeclaration, SettingScope, SettingValue};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

// --- registry projection -----------------------------------------------------

/// Wire shape of one declaration. Kinds/options are flattened so generated
/// clients can render editors directly from this payload.
#[derive(Debug, Serialize, ToSchema)]
pub struct DeclarationView {
    pub key: String,
    pub kind: registry::SettingKind,
    pub default: SettingValue,
    /// "org" | "user"
    pub scope: String,
    /// Settings-hub section this declaration renders under.
    pub section: String,
    /// "ui" renders an editable row; "hidden" is state readable via values.
    pub visibility: String,
    pub label: String,
    pub description: String,
    /// Closed option set for enumerations; empty otherwise.
    pub options: Vec<String>,
}

impl From<&'static SettingDeclaration> for DeclarationView {
    fn from(d: &'static SettingDeclaration) -> Self {
        Self {
            key: d.key.to_string(),
            kind: d.kind.clone(),
            default: d.default.clone(),
            scope: d.scope.as_str().to_string(),
            section: d.section.as_str().to_string(),
            visibility: match d.visibility {
                registry::SettingVisibility::Ui => "ui",
                registry::SettingVisibility::Hidden => "hidden",
            }
            .to_string(),
            label: d.label.to_string(),
            description: d.description.to_string(),
            options: d
                .options()
                .map(|opts| opts.iter().map(|o| o.to_string()).collect())
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RegistryResponse {
    pub settings: Vec<DeclarationView>,
}

/// One effective value: registry default overlaid by any stored override.
#[derive(Debug, Serialize, ToSchema)]
pub struct ValueView {
    pub key: String,
    /// JSON payload typed by the declaration's kind.
    pub value: serde_json::Value,
}

/// GET /api/settings/registry — every declared setting with kind, default,
/// scope and validation shape. Any authenticated member may read it.
#[utoipa::path(operation_id = "settings_registry", get, path = "/api/settings/registry",
    tag = "settings", security(("bearer" = [])),
    responses((status = 200, body = RegistryResponse)))]
pub async fn get_registry() -> ApiResult<Json<RegistryResponse>> {
    Ok(Json(RegistryResponse {
        settings: registry::declarations()
            .iter()
            .map(DeclarationView::from)
            .collect(),
    }))
}

// --- shared plumbing ---------------------------------------------------------

fn audit_event(
    org_id: Uuid,
    actor_id: Uuid,
    entity_id: String,
    detail: serde_json::Value,
) -> AuditEvent {
    AuditEvent {
        id: Uuid::now_v7(),
        org_id,
        actor: Actor::Human(actor_id),
        action: "settings.updated".to_string(),
        entity_type: Some("setting".to_string()),
        entity_id: Some(entity_id),
        detail,
        ip: None,
        occurred_at: chrono::Utc::now(),
    }
}

/// Parses + validates an incoming {key, value} pair against the registry.
fn resolve_update(
    key: &str,
    raw: &serde_json::Value,
) -> ApiResult<(&'static SettingDeclaration, SettingValue)> {
    let decl = registry::declaration(key)
        .ok_or_else(|| ApiError::Validation(format!("unknown setting key: {key}")))?;
    let value =
        SettingValue::from_json(raw).map_err(|e| ApiError::Validation(format!("{key}: {e}")))?;
    decl.validate(&value)
        .map_err(|e| ApiError::Validation(format!("{key}: {e}")))?;
    Ok((decl, value))
}

async fn effective_values(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    scope: SettingScope,
    owner_id: Uuid,
) -> ApiResult<Vec<ValueView>> {
    let stored = match scope {
        SettingScope::Org => core_db::settings::list_org_overrides(&mut *conn, org_id).await?,
        SettingScope::User => {
            core_db::settings::list_user_overrides(&mut *conn, org_id, owner_id).await?
        }
    };
    let mut out = Vec::new();
    for decl in registry::declarations_for(scope) {
        let value = stored
            .iter()
            .find(|o| o.key == decl.key)
            .map(|o| o.value.clone())
            .unwrap_or_else(|| decl.default.to_json());
        out.push(ValueView {
            key: decl.key.to_string(),
            value,
        });
    }
    Ok(out)
}

// --- org-scope values --------------------------------------------------------

/// GET /api/settings/org — effective org settings (defaults overlaid by
/// stored overrides). Visible to every member.
#[utoipa::path(operation_id = "settings_org_get", get, path = "/api/settings/org",
    tag = "settings", security(("bearer" = [])),
    responses((status = 200, body = [ValueView])))]
pub async fn org_values_get(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<ValueView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let values = effective_values(&mut tx, org_id, SettingScope::Org, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(values))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSettingRequest {
    pub key: String,
    /// Must satisfy the declaration's kind or the request is rejected 422.
    pub value: serde_json::Value,
}

/// PUT /api/settings/org — upserts one org-scope override after registry
/// validation (Admin:edit required). Emits `setting.updated` + audit row.
#[utoipa::path(operation_id = "settings_org_put", put, path = "/api/settings/org",
    tag = "settings", security(("bearer" = [])), request_body = UpdateSettingRequest,
    responses((status = 200, body = ValueView),
              (status = 403), (status = 422, description = "unknown key or invalid value")))]
pub async fn org_value_put(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<UpdateSettingRequest>,
) -> ApiResult<Json<ValueView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let (decl, value) = resolve_update(&req.key, &req.value)?;
    if decl.scope != SettingScope::Org {
        return Err(ApiError::Validation(format!(
            "{} is a {}-scope setting",
            req.key,
            decl.scope.as_str()
        )));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::settings::upsert_org_setting(
            &mut ctx.tx,
            org_id,
            decl.key,
            &value.to_json(),
            user_id,
        )
        .await?;
        ctx.emit(
            types::SETTING_UPDATED,
            "setting",
            Some(decl.key),
            &serde_json::json!({ "scope": "org", "key": decl.key, "value": value.to_json() }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &audit_event(
                org_id,
                user_id,
                format!("org:{}", decl.key),
                serde_json::json!({ "key": decl.key, "value": value.to_json() }),
            ),
        )
        .await?;
        Ok((
            ctx,
            ValueView {
                key: decl.key.to_string(),
                value: value.to_json(),
            },
        ))
    })
    .await
    .map(Json)
}

// --- user-scope values -------------------------------------------------------

/// GET /api/settings/user — the caller's effective personal settings.
#[utoipa::path(operation_id = "settings_user_get", get, path = "/api/settings/user",
    tag = "settings", security(("bearer" = [])),
    responses((status = 200, body = [ValueView])))]
pub async fn user_values_get(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<ValueView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let (org_id, user_id) = (authed.principal.user.org_id, authed.principal.user.id);
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let values = effective_values(&mut tx, org_id, SettingScope::User, user_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(values))
}

/// PUT /api/settings/user — upserts one of the CALLER's own user-scope
/// overrides. No role requirement: everyone may tune their own preferences.
#[utoipa::path(operation_id = "settings_user_put", put, path = "/api/settings/user",
    tag = "settings", security(("bearer" = [])), request_body = UpdateSettingRequest,
    responses((status = 200, body = ValueView),
              (status = 422, description = "unknown key or invalid value")))]
pub async fn user_value_put(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<UpdateSettingRequest>,
) -> ApiResult<Json<ValueView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let (decl, value) = resolve_update(&req.key, &req.value)?;
    if decl.scope != SettingScope::User {
        return Err(ApiError::Validation(format!(
            "{} is an org-scope setting; use /api/settings/org",
            req.key
        )));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::settings::upsert_user_setting(
            &mut ctx.tx,
            org_id,
            user_id,
            decl.key,
            &value.to_json(),
        )
        .await?;
        ctx.emit(
            types::SETTING_UPDATED,
            "setting",
            Some(decl.key),
            &serde_json::json!({ "scope": "user", "user_id": user_id, "key": decl.key, "value": value.to_json() }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &audit_event(
                org_id,
                user_id,
                format!("user:{}:{}", user_id, decl.key),
                serde_json::json!({ "key": decl.key, "value": value.to_json() }),
            ),
        )
        .await?;
        Ok((
            ctx,
            ValueView {
                key: decl.key.to_string(),
                value: value.to_json(),
            },
        ))
    })
    .await
    .map(Json)
}

// --- org profile -------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct OrgProfileResponse {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub ticket_prefix: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchOrgRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub ticket_prefix: Option<String>,
}

/// GET /api/org — profile fields shown on General settings / the wizard.
#[utoipa::path(operation_id = "org_profile_get", get, path = "/api/org",
    tag = "settings", security(("bearer" = [])),
    responses((status = 200, body = OrgProfileResponse)))]
pub async fn org_profile_get(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<OrgProfileResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let org = core_db::orgs::get_org(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(OrgProfileResponse {
        id: org.id,
        name: org.name,
        slug: org.slug,
        ticket_prefix: org.ticket_prefix,
    }))
}

/// PATCH /api/org — updates name and/or ticket prefix (Admin:edit).
/// Idempotent; emits `org.updated` + audit row.
#[utoipa::path(operation_id = "org_profile_patch", patch, path = "/api/org",
    tag = "settings", security(("bearer" = [])), request_body = PatchOrgRequest,
    responses((status = 200, body = OrgProfileResponse), (status = 422)))]
pub async fn org_profile_patch(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PatchOrgRequest>,
) -> ApiResult<Json<OrgProfileResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(core_domain::policy::Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let name = req.name.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let prefix = req
        .ticket_prefix
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(p) = prefix {
        let ok = !p.is_empty()
            && p.len() <= 8
            && p.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
        if !ok {
            return Err(ApiError::Validation(
                "ticket_prefix must be 1–8 uppercase letters/digits".into(),
            ));
        }
    }

    crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let org = core_db::orgs::update_profile(&mut *ctx.tx, org_id, name, prefix).await?;
        ctx.emit(
            types::ORG_UPDATED,
            "org",
            Some(org_id.to_string().as_str()),
            &serde_json::json!({ "name": org.name, "ticket_prefix": org.ticket_prefix }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "org.updated".to_string(),
                entity_type: Some("org".to_string()),
                entity_id: Some(org_id.to_string()),
                detail: serde_json::json!({
                    "name": org.name,
                    "ticket_prefix": org.ticket_prefix,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((
            ctx,
            OrgProfileResponse {
                id: org.id,
                name: org.name,
                slug: org.slug,
                ticket_prefix: org.ticket_prefix,
            },
        ))
    })
    .await
    .map(Json)
}
