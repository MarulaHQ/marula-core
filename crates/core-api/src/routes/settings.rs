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
use crate::cache::{self, Caches};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{extract::State, http::Extensions, Json};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use core_domain::money::CurrencyCode;
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
// `Clone` because the cached org projection hands the same rows back on a hit;
// ownership keeps the handler free of a borrow into the cache. A plain comment,
// not a doc comment: this struct is a published OpenAPI schema, and a note about
// our caching belongs to us, not to the contract consumers read.
#[derive(Debug, Clone, Serialize, ToSchema)]
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
    // A hit returns the projection without opening a transaction: the whole
    // tx below exists for this one read. Invalidation: `org.*` and
    // `setting.updated` clear this key (crate::cache_invalidation).
    let key = Caches::ref_key(Caches::NS_ORG_SETTINGS, org_id);
    if let Some(values) = cache::caches().get_ref::<Vec<ValueView>>(&key) {
        return Ok(Json(values));
    }
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let values = effective_values(&mut tx, org_id, SettingScope::Org, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    cache::caches().put_ref(key, values.clone());
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

/// One placeholder the signature editor can insert, with the value it would
/// expand to for the caller RIGHT NOW.
#[derive(Debug, Serialize, ToSchema)]
pub struct EmailTokenView {
    /// The placeholder key, e.g. `user.name`.
    pub key: String,
    /// Human label for the picker row.
    pub label: String,
    /// What it expands to for this caller, or `None` when their record has
    /// no value for it — a token that would render empty says so here
    /// instead of looking like a field the editor forgot.
    pub value: Option<String>,
}

/// The caller's resolved email envelope: the greeting and signature that
/// would go out, which rung supplied each, the org's own signature for the
/// fallback story, the policy in force, and the insertable tokens.
///
/// This is the preview AND the composer's prefill in one call, resolved
/// through the same chain and the same dialect the send path uses. A UI
/// that previewed with its own regex would be a second implementation of
/// the rules, and the one thing a preview must never be is more generous
/// than the send.
#[derive(Debug, Serialize, ToSchema)]
pub struct EmailEnvelopeView {
    pub policy: String,
    pub greeting: Option<String>,
    pub greeting_source: String,
    pub signature: Option<String>,
    pub signature_source: String,
    /// The org's own signature (rendered), so a member can see what their
    /// messages fall back to without an admin.
    pub org_signature: Option<String>,
    /// The caller's own stored defaults, unrendered — the token form the
    /// editor edits.
    pub user_greeting: String,
    pub user_signature: String,
    pub tokens: Vec<EmailTokenView>,
}

/// GET /api/settings/user/email-envelope — the caller's resolved envelope,
/// for the signature editor's live preview and the composer's prefill.
#[utoipa::path(operation_id = "settings_user_email_envelope", get,
    path = "/api/settings/user/email-envelope", tag = "settings",
    security(("bearer" = [])),
    responses((status = 200, body = EmailEnvelopeView)))]
pub async fn user_email_envelope_get(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<EmailEnvelopeView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let (org_id, user_id) = (authed.principal.user.org_id, authed.principal.user.id);
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;

    let envelope = crate::email::resolve_envelope(
        &mut tx,
        org_id,
        None,
        Some(user_id),
        &crate::email::EnvelopeOverrides::default(),
    )
    .await?;

    // The stored forms, unrendered: the editor edits placeholders, and a
    // preview that handed back a rendered string would overwrite the
    // sender's tokens with today's values the moment they looked.
    let stored_text = |value: Option<serde_json::Value>| {
        value
            .as_ref()
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let user_greeting = stored_text(
        core_db::settings::get_user_value(&mut *tx, org_id, user_id, registry::EMAIL_GREETING)
            .await?,
    );
    let user_signature = stored_text(
        core_db::settings::get_user_value(&mut *tx, org_id, user_id, registry::EMAIL_SIGNOFF)
            .await?,
    );

    // Rendered against the CALLER's own row, which is the whole point of a
    // live preview: what you see is what the next message carries.
    let signer_ctx = crate::email::signer_context(&mut tx, org_id, Some(user_id)).await?;
    let render_ctx = signer_ctx.render_context();
    let org_signature = envelope
        .org_default_signature
        .as_deref()
        .map(|block| core_domain::email_signature::render_signature(block, &signer_ctx));
    let tokens = core_domain::templates::SIGNATURE_VARIABLE_LABELS
        .iter()
        .map(|(key, label)| EmailTokenView {
            key: (*key).to_string(),
            label: (*label).to_string(),
            value: render_ctx.get(*key).cloned(),
        })
        .collect();
    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(EmailEnvelopeView {
        policy: envelope.policy.as_str().to_string(),
        greeting: envelope.greeting,
        greeting_source: envelope.greeting_source.as_str().to_string(),
        signature: envelope.signature,
        signature_source: envelope.signature_source.as_str().to_string(),
        org_signature,
        user_greeting,
        user_signature,
        tokens,
    }))
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
    // A locked org does not merely ignore a personal signature — it
    // refuses the write, and names the policy that did it. Accepting the
    // value and then not using it would leave a stored preference the admin
    // never agreed to, waiting for the day the lock is lifted; refusing
    // with a bare "rejected" would leave the member unable to tell a
    // policy from a typo.
    if decl.key == registry::EMAIL_SIGNOFF {
        let mut probe = state.pool.begin().await.map_err(ApiError::from)?;
        core_db::set_org_and_user(&mut probe, org_id, user_id)
            .await
            .map_err(ApiError::from)?;
        let policy = crate::email::signature_policy(&mut probe, org_id).await?;
        probe.commit().await.map_err(ApiError::from)?;
        if !policy.allows_personal() {
            return Err(ApiError::Validation(format!(
                "{} is locked to the organization, so it cannot be saved. Your \
                 messages carry the workspace signature until an admin allows \
                 personal signatures again.",
                registry::EMAIL_SIGNOFF
            )));
        }
        // And the same gate the org pair takes: a stored default carrying a
        // placeholder the send path cannot expand is refused here, on the
        // field, rather than sent later as a visible placeholder.
        if let Some(text) = value.to_json().as_str() {
            crate::email::validate_signature_text(decl.key, text)?;
        }
    }
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

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OrgProfileResponse {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub ticket_prefix: String,
    /// The org's ONE base (reporting) currency (ADR 0002, FX1).
    pub base_currency: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchOrgRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub ticket_prefix: Option<String>,
    /// Present + different ⇒ a governed base-currency change (ADR 0002):
    /// refused with a typed conflict while conversion snapshots exist.
    #[serde(default)]
    pub base_currency: Option<String>,
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
    // Org identity is caller-independent, and the tx below exists for this one
    // read, so a hit skips it. Scoped under the org-settings namespace, so the
    // same `org.*` / `setting.updated` invalidation that clears the settings
    // projection clears this too.
    let key = Caches::ref_key_scoped(Caches::NS_ORG_SETTINGS, org_id, "profile");
    if let Some(profile) = cache::caches().get_ref::<OrgProfileResponse>(&key) {
        return Ok(Json(profile));
    }
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let org = core_db::orgs::get_org(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    let profile = OrgProfileResponse {
        id: org.id,
        name: org.name,
        slug: org.slug,
        ticket_prefix: org.ticket_prefix,
        base_currency: org.base_currency,
    };
    cache::caches().put_ref(key, profile.clone());
    Ok(Json(profile))
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
    let base_currency = match req.base_currency.as_deref().map(str::trim) {
        None => None,
        Some(code) => Some(
            CurrencyCode::new(code)
                .map_err(|_| ApiError::Validation("base_currency must be an ISO 4217 code".into()))?
                .to_string(),
        ),
    };

    crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        // Base-currency change is governed (ADR 0002): refused with a
        // typed conflict while conversion snapshots exist. Emit the
        // dedicated event so caches re-hydrate every money view.
        if let Some(new_base) = base_currency.as_deref() {
            let current = core_db::orgs::get_org(&mut *ctx.tx, org_id).await?;
            if current.base_currency != new_base {
                let org =
                    core_db::orgs::change_base_currency(&mut ctx.tx, org_id, new_base).await?;
                ctx.emit(
                    types::FX_BASE_CURRENCY_CHANGED,
                    "fx",
                    Some(org_id.to_string().as_str()),
                    &serde_json::json!({
                        "org_id": org_id,
                        "from": current.base_currency,
                        "to": org.base_currency,
                        "changed_at": chrono::Utc::now(),
                    }),
                )
                .await?;
                core_db::audit::insert(
                    &mut *ctx.tx,
                    &AuditEvent {
                        id: Uuid::now_v7(),
                        org_id,
                        actor: Actor::Human(user_id),
                        action: "fx.base_currency_changed".to_string(),
                        entity_type: Some("org".to_string()),
                        entity_id: Some(org_id.to_string()),
                        detail: serde_json::json!({
                            "from": current.base_currency,
                            "to": org.base_currency,
                        }),
                        ip: None,
                        occurred_at: chrono::Utc::now(),
                    },
                )
                .await?;
            }
        }
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
                base_currency: org.base_currency,
            },
        ))
    })
    .await
    .map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{self, Caches};

    /// The cache singleton is process-wide, so cases here that insert into it
    /// must not interleave. Locking keeps them serial inside the binary,
    /// matching the guard `cache_invalidation`'s tests use.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn only_one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn sample_values() -> Vec<ValueView> {
        vec![
            ValueView {
                key: "ai.features_enabled".to_string(),
                value: serde_json::json!(true),
            },
            ValueView {
                key: "email.signoff".to_string(),
                value: serde_json::json!("— The team"),
            },
        ]
    }

    fn sample_profile(org: Uuid) -> OrgProfileResponse {
        OrgProfileResponse {
            id: org,
            name: "Marula".to_string(),
            slug: "marula".to_string(),
            ticket_prefix: "MAR".to_string(),
            base_currency: "USD".to_string(),
        }
    }

    fn frame(event_type: &str, org_id: Uuid) -> crate::realtime::StreamFrame {
        crate::realtime::StreamFrame {
            event_type: event_type.to_string(),
            org_id,
            entity: "setting".to_string(),
            entity_id: None,
            seq: Some(1),
            at: chrono::Utc::now(),
            viewers: None,
            id: Some(Uuid::new_v4()),
        }
    }

    #[test]
    fn org_settings_and_profile_ride_one_invalidated_prefix() {
        let _guard = only_one_at_a_time();
        let caches = cache::__enable_for_tests();
        let org = Uuid::new_v4();
        let other_org = Uuid::new_v4();

        let values_key = Caches::ref_key(Caches::NS_ORG_SETTINGS, org);
        let profile_key = Caches::ref_key_scoped(Caches::NS_ORG_SETTINGS, org, "profile");
        let other_values_key = Caches::ref_key(Caches::NS_ORG_SETTINGS, other_org);

        // The profile caches under a discriminator, but still starts with the
        // plain key — which is the prefix `org.*` / `setting.updated` clears,
        // so a single invalidation drops both families.
        assert!(
            profile_key.starts_with(values_key.as_str()),
            "{profile_key} must sit under {values_key}"
        );

        caches.put_ref(values_key.clone(), sample_values());
        caches.put_ref(profile_key.clone(), sample_profile(org));
        caches.put_ref(other_values_key.clone(), sample_values());

        // A second read is served from the cache (the value survives typed).
        assert_eq!(
            caches
                .get_ref::<Vec<ValueView>>(&values_key)
                .map(|v| v.len()),
            Some(2)
        );
        assert_eq!(
            caches
                .get_ref::<OrgProfileResponse>(&profile_key)
                .map(|p| p.slug),
            Some("marula".to_string())
        );

        // After the org-settings invalidation fires — `setting.updated` is the
        // event `org_value_put` emits — both entries are gone and another org's
        // settings are untouched.
        crate::cache_invalidation::invalidate_for(&frame("setting.updated", org));
        assert!(caches.get_ref::<Vec<ValueView>>(&values_key).is_none());
        assert!(caches.get_ref::<OrgProfileResponse>(&profile_key).is_none());
        assert!(caches
            .get_ref::<Vec<ValueView>>(&other_values_key)
            .is_some());
    }
}
