//! OIDC SSO routes: admin provider management, the authorization-code
//! redirect + callback flow (state+nonce+PKCE validated), conservative
//! account linking, JIT provisioning (default OFF), and the one-time
//! token-exchange endpoint the web app calls after callback.
//!
//! Security posture: linking happens ONLY via an explicit authenticated
//! link flow whose email claim matches the linking user; JIT requires an
//! org-level opt-in AND a verified email claim. Provider secrets are
//! decrypted in-memory for the exchange and never returned after write.

use crate::auth::federation::denial_reasons;
use crate::auth::handlers::issue_tokens;
use crate::auth::middleware::{authed, require_permission};
use crate::auth::oidc_client::{
    decide_sso, issue_state_token, new_nonce, new_verifier, verify_state_token, SsoOutcome,
};
use crate::auth::presets;
use crate::auth::{oidc_client, AuthedUser};
use crate::error::{ApiError, ApiResult};
use crate::routes::openapi::TokenPair;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    response::{IntoResponse, Redirect, Response},
    Json,
};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

const SSO_CODE_TTL_SECS: i64 = 60;

fn require_sso_admin(authed: &AuthedUser) -> ApiResult<()> {
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Admin,
            action: Action::Edit,
        },
    )
}

async fn org_tx(
    state: &AppState,
    org_id: Uuid,
) -> ApiResult<sqlx::Transaction<'static, sqlx::Postgres>> {
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    Ok(tx)
}

// ---------------------------------------------------------------------------
// Admin provider management
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertProviderRequest {
    pub display_name: String,
    pub issuer: String,
    /// Typically `{issuer}/.well-known/openid-configuration`.
    pub discovery_url: String,
    pub client_id: String,
    /// Write-only: stored encrypted at rest, never returned.
    pub client_secret: String,
    #[serde(default)]
    pub jit_provisioning: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Declarative first-party preset (`google` | `entra` | `okta`).
    /// When set, issuer + discovery_url are DERIVED from the preset
    /// template + [`Self::preset_params`]; request values, when also
    /// present, must match the derivation exactly (mix-up defense).
    pub preset: Option<String>,
    /// Template parameters for the preset (e.g. `tenant` for Entra,
    /// `domain` for Okta).
    #[serde(default)]
    pub preset_params: Option<HashMap<String, String>>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OidcProviderInfo {
    #[schema(value_type = uuid::Uuid)]
    pub id: uuid::Uuid,
    /// URL path segment used by login/link routes.
    pub name: String,
    pub display_name: String,
    pub issuer: String,
    pub discovery_url: String,
    pub client_id: String,
    pub jit_provisioning: bool,
    pub enabled: bool,
    /// Declarative preset this provider was created from, when any.
    pub preset: Option<String>,
}

/// One entry of the declarative first-party preset catalog.
#[derive(Debug, Serialize, ToSchema)]
pub struct OidcPresetInfo {
    pub name: String,
    pub display_name: String,
    /// `{param}`-templated issuer the preset pins.
    pub issuer_template: String,
    pub discovery_template: String,
    pub scopes: String,
    /// Parameters the admin MUST provide (e.g. `tenant`, `domain`).
    pub required_params: Vec<String>,
    /// Where to register the redirect URI on the provider side.
    pub redirect_guidance: String,
}

/// GET /api/auth/oidc/presets — the declarative preset catalog that
/// drives the admin "add provider" UI. Pure data; no configuration.
#[utoipa::path(operation_id = "oidc_presets_list", get, path = "/api/auth/oidc/presets",
    tag = "auth", security(("bearer" = [])),
    responses((status = 200, body = [OidcPresetInfo])))]
pub async fn presets_list(extensions: Extensions) -> ApiResult<Json<Vec<OidcPresetInfo>>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    require_sso_admin(&authed)?;
    Ok(Json(
        presets::OIDC_PRESETS
            .iter()
            .map(|p| OidcPresetInfo {
                name: p.name.to_string(),
                display_name: p.display_name.to_string(),
                issuer_template: p.issuer_template.to_string(),
                discovery_template: p.discovery_template.to_string(),
                scopes: p.scopes.to_string(),
                required_params: p.required_params.iter().map(|s| s.to_string()).collect(),
                redirect_guidance: p.redirect_guidance.to_string(),
            })
            .collect(),
    ))
}

/// GET /api/auth/oidc/providers — admin listing. Secrets excluded by type.
#[utoipa::path(operation_id = "oidc_providers_list", get, path = "/api/auth/oidc/providers",
    tag = "auth", security(("bearer" = [])),
    responses((status = 200, body = [OidcProviderInfo]), (status = 403)))]
pub async fn providers_list(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<OidcProviderInfo>>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    require_sso_admin(&authed)?;
    let mut tx = org_tx(&state, authed.principal.user.org_id).await?;
    let rows = core_db::oidc::list_providers(&mut *tx, authed.principal.user.org_id).await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(rows.into_iter().map(provider_meta_to_info).collect()))
}

fn provider_meta_to_info(p: core_db::oidc::ProviderMeta) -> OidcProviderInfo {
    OidcProviderInfo {
        id: p.id,
        name: p.name,
        display_name: p.display_name,
        issuer: p.issuer,
        discovery_url: p.discovery_url,
        client_id: p.client_id,
        jit_provisioning: p.jit_provisioning,
        enabled: p.enabled,
        preset: p.preset,
    }
}

/// PUT /api/auth/oidc/providers/{name} — create or replace a provider.
/// The client secret is required on every write (rotate-friendly).
#[utoipa::path(operation_id = "oidc_provider_upsert", put, path = "/api/auth/oidc/providers/{name}",
    tag = "auth", security(("bearer" = [])), params(("name" = String, Path)),
    request_body = UpsertProviderRequest,
    responses((status = 200, body = OidcProviderInfo), (status = 403)))]
pub async fn providers_upsert(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(name): Path<String>,
    Json(req): Json<UpsertProviderRequest>,
) -> ApiResult<Json<OidcProviderInfo>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    require_sso_admin(&authed)?;
    let user = &authed.principal.user;

    let name_ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !name_ok || req.client_secret.is_empty() {
        return Err(ApiError::Validation(
            "provider name must be lowercase alphanumeric/'-' and client_secret required".into(),
        ));
    }

    // Declarative presets (ID1): a preset DERIVES issuer + discovery_url
    // from its templates; a mismatch between the request values and the
    // derivation is refused rather than silently trusted.
    let empty = HashMap::new();
    let params = req.preset_params.as_ref().unwrap_or(&empty);
    let (issuer, discovery_url, preset_name) = match &req.preset {
        Some(preset) => {
            let (issuer, discovery) =
                presets::resolve_preset(preset, params).map_err(ApiError::Validation)?;
            if !req.issuer.trim().is_empty() && req.issuer.trim_end_matches('/') != issuer {
                return Err(ApiError::Validation(
                    "issuer does not match the preset derivation; omit it or fix the preset params"
                        .into(),
                ));
            }
            if !req.discovery_url.trim().is_empty() && req.discovery_url.trim() != discovery {
                return Err(ApiError::Validation(
                    "discovery_url does not match the preset derivation; omit it or fix the preset params"
                        .into(),
                ));
            }
            (issuer, discovery, Some(preset.clone()))
        }
        None => {
            if req.issuer.trim().is_empty() || req.discovery_url.trim().is_empty() {
                return Err(ApiError::Validation(
                    "issuer and discovery_url are required without a preset".into(),
                ));
            }
            (
                req.issuer.trim_end_matches('/').to_string(),
                req.discovery_url.trim().to_string(),
                None,
            )
        }
    };

    let sealed = crate::crypto::seal(&state.config.secret_key, req.client_secret.as_bytes())
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("{e}")))?;

    let mut tx = org_tx(&state, user.org_id).await?;
    let id = core_db::oidc::upsert_provider(
        &mut *tx,
        Uuid::now_v7(),
        user.org_id,
        core_db::oidc::NewOidcProvider {
            name: &name,
            display_name: &req.display_name,
            issuer: &issuer,
            discovery_url: &discovery_url,
            client_id: &req.client_id,
            client_secret_enc: &sealed,
            jit_provisioning: req.jit_provisioning,
            preset: preset_name.as_deref(),
        },
        user.id,
    )
    .await?;
    let meta = core_db::oidc::get_provider_meta(&mut *tx, user.org_id, id).await?;
    core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id: user.org_id,
            actor: Actor::Human(user.id),
            action: "auth.oidc_provider_saved".into(),
            entity_type: Some("oidc_provider".into()),
            entity_id: Some(id.to_string()),
            detail: serde_json::json!({
                "name": name,
                "preset": preset_name,
                "jit": req.jit_provisioning,
                "enabled": meta.enabled,
                "secret_rotated": true,
            }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(provider_meta_to_info(meta)))
}

/// DELETE /api/auth/oidc/providers/{name}.
#[utoipa::path(operation_id = "oidc_provider_delete", delete, path = "/api/auth/oidc/providers/{name}",
    tag = "auth", security(("bearer" = [])), params(("name" = String, Path)),
    responses((status = 204), (status = 404), (status = 403)))]
pub async fn providers_delete(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(name): Path<String>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<AuthedUser>(&extensions)?;
    require_sso_admin(&authed)?;
    let user = &authed.principal.user;

    let mut tx = org_tx(&state, user.org_id).await?;
    // Resolve id first so the audit event can name it.
    let rows = core_db::oidc::list_providers(&mut *tx, user.org_id).await?;
    let target = rows
        .iter()
        .find(|p| p.name == name)
        .ok_or(ApiError::NotFound)?;
    let deleted = core_db::oidc::delete_provider(&mut *tx, user.org_id, target.id).await?;
    if deleted == 0 {
        return Err(ApiError::NotFound);
    }
    core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id: user.org_id,
            actor: Actor::Human(user.id),
            action: "auth.oidc_provider_deleted".into(),
            entity_type: Some("oidc_provider".into()),
            entity_id: Some(target.id.to_string()),
            detail: serde_json::json!({ "name": name }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Public discovery + flows
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct PublicProvider {
    /// URL path segment for /api/auth/oidc/{name}/login.
    pub name: String,
    pub display_name: String,
}

/// GET /api/auth/oidc/providers/public — drives config-driven SSO buttons
/// on the login screen. No configuration detail beyond names.
#[utoipa::path(operation_id = "oidc_providers_public", get,
    path = "/api/auth/oidc/providers/public", tag = "auth",
    responses((status = 200, body = [PublicProvider])))]
pub async fn providers_public(
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<PublicProvider>>> {
    let rows: Vec<(uuid::Uuid, String, String)> =
        sqlx::query_as("SELECT org_id, name, display_name FROM app_list_enabled_oidc_providers()")
            .fetch_all(&state.pool)
            .await
            .map_err(ApiError::from)?;
    Ok(Json(
        rows.into_iter()
            .map(|(_, name, display_name)| PublicProvider { name, display_name })
            .collect(),
    ))
}

struct ResolvedProvider {
    provider_id: Uuid,
    org_id: Uuid,
    issuer: String,
    discovery_url: String,
    client_id: String,
}

/// SECURITY DEFINER bridge: resolves an enabled provider pre-authentication.
async fn resolve_provider_by_name(
    pool: &sqlx::PgPool,
    name: &str,
) -> ApiResult<Option<ResolvedProvider>> {
    let row: Option<(Uuid, Uuid, String, String, String, bool)> = sqlx::query_as(
        "SELECT provider_id, org_id, issuer, discovery_url, client_id, jit_provisioning \
         FROM app_resolve_oidc_provider($1)",
    )
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from)?;
    Ok(row.map(
        |(provider_id, org_id, issuer, discovery_url, client_id, _jit)| ResolvedProvider {
            provider_id,
            org_id,
            issuer,
            discovery_url,
            client_id,
        },
    ))
}

/// GET /api/auth/oidc/{name}/login — 303 to the IdP with signed state,
/// nonce, and PKCE S256 bound together.
#[utoipa::path(operation_id = "oidc_login_redirect", get, path = "/api/auth/oidc/{name}/login",
    tag = "auth", params(("name" = String, Path)),
    responses((status = 303, description = "redirect to provider"), (status = 404)))]
pub async fn login_redirect(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<Response> {
    let provider = resolve_provider_by_name(&state.pool, &name)
        .await?
        .ok_or(ApiError::NotFound)?;

    let discovery = state
        .oidc
        .discover(&provider.discovery_url, &provider.issuer)
        .await
        .map_err(map_oidc_err)?;

    let nonce = oidc_client::new_nonce();
    let verifier = new_verifier();
    let state_jwt = issue_state_token(&state.config, provider.provider_id, &nonce, &verifier, None)
        .map_err(|e| ApiError::Internal(e.into()))?;

    let redirect_uri = format!(
        "{}/api/auth/oidc/{}/callback",
        state.config.public_base_url.trim_end_matches('/'),
        name
    );
    let url = crate::auth::oidc_client::OidcHttpClient::authorize_url(
        &discovery,
        &provider.client_id,
        &redirect_uri,
        &state_jwt,
        &nonce,
        &verifier,
    );
    Ok(Redirect::to(url.as_str()).into_response())
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// Outcome payload parked behind a one-time code for the web app to pick up.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum SsoHandoff {
    Tokens(TokenPair),
    /// Org MFA policy applies to federated logins too (ID4): the same
    /// `mfa_pending` bridge password login returns; POST
    /// /api/auth/mfa/verify completes it.
    MfaRequired {
        mfa_required: bool,
        mfa_token: String,
        methods: Vec<String>,
    },
    Linked {
        linked: bool,
    },
}

/// GET /api/auth/oidc/{name}/callback — validates state signature/expiry/
/// provider binding and nonce echo, runs the conservative decision, then
/// hands tokens (or link confirmation) to the web app via a single-use code.
#[utoipa::path(operation_id = "oidc_callback", get, path = "/api/auth/oidc/{name}/callback",
    tag = "auth", params(("name" = String, Path)),
    responses((status = 303, description = "redirect to web app")))]
pub async fn callback(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<CallbackQuery>,
) -> ApiResult<Response> {
    let fail = |reason: &str| sso_error_redirect(&state.config.web_base_url, reason);

    if q.error.is_some() || q.code.is_none() || q.state.is_none() {
        return Ok(fail("provider_denied"));
    }

    let claims_state =
        match verify_state_token(&state.config, q.state.as_deref().expect("checked above")) {
            Ok(c) => c,
            Err(_) => return Ok(fail("bad_state")),
        };

    let provider = match resolve_provider_by_name(&state.pool, &name).await? {
        Some(p) => p,
        None => return Ok(fail("unknown_provider")),
    };
    if provider.provider_id != claims_state.pid {
        return Ok(fail("provider_mismatch"));
    }

    let mut tx = org_tx(&state, provider.org_id).await?;
    let full =
        core_db::oidc::find_provider_with_secret(&mut *tx, provider.org_id, provider.provider_id)
            .await
            .map_err(|_| ApiError::NotFound)?;
    let secret = String::from_utf8(
        crate::crypto::open(&state.config.secret_key, &full.client_secret_enc)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("{e}")))?,
    )
    .map_err(|_| ApiError::Internal(anyhow::anyhow!("stored secret not utf8")))?;

    let discovery = state
        .oidc
        .discover(&full.discovery_url, &full.issuer)
        .await
        .map_err(map_oidc_err)?;

    let redirect_uri = format!(
        "{}/api/auth/oidc/{}/callback",
        state.config.public_base_url.trim_end_matches('/'),
        name
    );
    let id_token = state
        .oidc
        .exchange_code(
            &discovery.token_endpoint,
            &full.client_id,
            &secret,
            &redirect_uri,
            q.code.as_deref().expect("checked above"),
            &claims_state.ver,
        )
        .await
        .map_err(|e| {
            audit_sso_failed(&state, provider.org_id, "exchange");
            map_oidc_err(e)
        })?;

    let mut id_claims = state
        .oidc
        .verify_id_token(
            &id_token,
            &discovery.jwks_uri,
            &full.issuer,
            &full.client_id,
        )
        .await
        .map_err(|e| {
            audit_sso_failed(&state, provider.org_id, "id_token");
            map_oidc_err(e)
        })?;
    if id_claims.nonce.as_deref() != Some(claims_state.nce.as_str()) {
        audit_sso_failed(&state, provider.org_id, "nonce");
        return Err(ApiError::Unauthorized("nonce mismatch".into()));
    }
    // Canonicalize email case before matching/linking.
    if let Some(email) = &mut id_claims.email {
        *email = email.to_lowercase();
    }
    // Declarative preset attribute mapping (ID1): normalize the
    // provider's claim shape into the canonical IdClaims BEFORE the
    // conservative decision runs.
    if let Some(preset) = full.preset.as_deref().and_then(presets::get_preset) {
        (preset.map_claims)(&mut id_claims);
    }

    // --- decision ---------------------------------------------------------
    enum Path_ {
        TokensFor(Uuid, Option<&'static str>),
        LinkCreated(Uuid),
    }
    let outcome: Path_ = if let Some(link_user) = claims_state.link {
        let local = match core_db::users::get_user(&mut *tx, provider.org_id, link_user).await {
            Ok(u) => u,
            Err(_) => return Ok(fail("link_user_missing")),
        };
        match decide_sso(
            None,
            false,
            &id_claims,
            Some((local.id, local.email.clone())),
        ) {
            Ok(SsoOutcome::Login { .. }) => {
                let email = id_claims
                    .email
                    .clone()
                    .unwrap_or_else(|| local.email.clone());
                core_db::oidc::create_link(
                    &mut *tx,
                    Uuid::now_v7(),
                    provider.org_id,
                    provider.provider_id,
                    link_user,
                    core_db::oidc::IdpIdentity {
                        subject: &id_claims.sub,
                        email: &email,
                    },
                    Some(link_user),
                )
                .await?;
                core_db::audit::insert(
                    &mut *tx,
                    &AuditEvent {
                        id: Uuid::now_v7(),
                        org_id: provider.org_id,
                        actor: Actor::Human(link_user),
                        action: "auth.oidc_account_linked".into(),
                        entity_type: Some("oidc_provider".into()),
                        entity_id: Some(provider.provider_id.to_string()),
                        detail: serde_json::json!({ "subject_bound": true }),
                        ip: None,
                        occurred_at: chrono::Utc::now(),
                    },
                )
                .await?;
                Path_::LinkCreated(link_user)
            }
            Ok(SsoOutcome::LinkRequired) | Ok(SsoOutcome::Provision { .. }) => {
                return Ok(fail("link_policy"));
            }
            Err(reason) => return Ok(fail(reason)),
        }
    } else {
        let existing = core_db::oidc::find_link(&mut *tx, provider.provider_id, &id_claims.sub)
            .await?
            .map(|l| l.user_id);
        match decide_sso(existing, full.jit_provisioning, &id_claims, None) {
            Ok(SsoOutcome::Login { user_id }) => {
                core_db::oidc::touch_link_login(&mut *tx, provider.provider_id, &id_claims.sub)
                    .await?;
                Path_::TokensFor(user_id, None)
            }
            Ok(SsoOutcome::Provision {
                email,
                display_name,
            }) => {
                let user_id =
                    provision_jit_user(&mut tx, provider.org_id, &email, &display_name).await?;
                core_db::oidc::create_link(
                    &mut *tx,
                    Uuid::now_v7(),
                    provider.org_id,
                    provider.provider_id,
                    user_id,
                    core_db::oidc::IdpIdentity {
                        subject: &id_claims.sub,
                        email: &email,
                    },
                    None,
                )
                .await?;
                core_db::audit::insert(
                    &mut *tx,
                    &AuditEvent {
                        id: Uuid::now_v7(),
                        org_id: provider.org_id,
                        actor: Actor::System,
                        action: "auth.oidc_jit_provisioned".into(),
                        entity_type: Some("user".into()),
                        entity_id: Some(user_id.to_string()),
                        detail: serde_json::json!({ "provider": name }),
                        ip: None,
                        occurred_at: chrono::Utc::now(),
                    },
                )
                .await?;
                Path_::TokensFor(user_id, Some("provisioned"))
            }
            Ok(SsoOutcome::LinkRequired) => {
                audit_sso_failed(&state, provider.org_id, "link_required");
                tx.commit()
                    .await
                    .map_err(|e| ApiError::Internal(e.into()))?;
                return Ok(fail("link_required"));
            }
            Err(reason) => {
                audit_sso_failed(&state, provider.org_id, reason);
                return Ok(fail(reason));
            }
        }
    };

    // --- handoff ----------------------------------------------------------
    // Decision writes (links, JIT users) must be durable BEFORE sessions are
    // issued against them (issue_tokens uses a separate connection).
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let (handoff, user_id, _note) = match outcome {
        Path_::LinkCreated(uid) => (
            SsoHandoff::Linked { linked: true },
            uid,
            None::<&'static str>,
        ),
        Path_::TokensFor(uid, note) => {
            // Org MFA policy applies regardless of identity source
            // (ID4): the SAME gate password login uses, BEFORE tokens
            // exist. A federated user with an enrolled second factor
            // gets the mfa_pending bridge, never tokens directly.
            if let Some((mfa_token, methods)) =
                crate::auth::handlers::federated_mfa_gate(&state, uid, provider.org_id).await?
            {
                (
                    SsoHandoff::MfaRequired {
                        mfa_required: true,
                        mfa_token,
                        methods,
                    },
                    uid,
                    note,
                )
            } else {
                let tokens = issue_tokens(&state, uid, provider.org_id, None).await?;
                let mut tx = org_tx(&state, provider.org_id).await?;
                core_db::audit::insert(
                    &mut *tx,
                    &AuditEvent {
                        id: Uuid::now_v7(),
                        org_id: provider.org_id,
                        actor: Actor::Human(uid),
                        action: "auth.oidc_login_success".into(),
                        entity_type: Some("user".into()),
                        entity_id: Some(uid.to_string()),
                        detail: serde_json::json!({
                            "provider": name,
                            "jit": note.map(|n| n == "provisioned").unwrap_or(false),
                        }),
                        ip: None,
                        occurred_at: chrono::Utc::now(),
                    },
                )
                .await?;
                (SsoHandoff::Tokens(tokens), uid, note)
            }
        }
    };

    let code = Uuid::now_v7();
    let mut tx = org_tx(&state, provider.org_id).await?;
    core_db::mfa::insert_challenge(
        &mut *tx,
        code,
        provider.org_id,
        user_id,
        "sso_code",
        &serde_json::to_value(&handoff).map_err(|e| ApiError::Internal(e.into()))?,
        chrono::Utc::now() + chrono::Duration::seconds(SSO_CODE_TTL_SECS),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let mut location = url::Url::parse(&format!(
        "{}/sso",
        state.config.web_base_url.trim_end_matches('/')
    ))
    .map_err(|e| ApiError::Internal(anyhow::anyhow!("{e}")))?;
    location
        .query_pairs_mut()
        .append_pair("code", &code.to_string());
    Ok(Redirect::to(location.as_str()).into_response())
}

/// Creates a disabled-password user under the org's default Technician role.
async fn provision_jit_user(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    org_id: Uuid,
    email: &str,
    display_name: &str,
) -> ApiResult<Uuid> {
    use rand::RngCore;
    let mut random = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut random);
    let unusable_password_hash = crate::auth::password::hash(&hex::encode(random))
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("argon2 failed: {e}")))?;
    let user_id = Uuid::now_v7();
    core_db::users::create_user(
        &mut **tx,
        user_id,
        org_id,
        email,
        display_name,
        &unusable_password_hash,
        "UTC",
    )
    .await?;
    let role_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM roles WHERE org_id = $1 AND builtin = 'technician' LIMIT 1",
    )
    .bind(org_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::from)?;
    if let Some(role_id) = role_id {
        core_db::users::assign_role(&mut **tx, user_id, role_id).await?;
    }
    Ok(user_id)
}

fn audit_sso_failed(state: &AppState, org_id: Uuid, reason: &str) {
    let state = state.clone();
    let reason = reason.to_string();
    tokio::spawn(async move {
        // RLS-scoped write (pool carries no app.org_id GUC); best effort.
        let result = async {
            let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
            core_db::set_org(&mut tx, org_id).await?;
            core_db::audit::insert(
                &mut *tx,
                &AuditEvent {
                    id: Uuid::now_v7(),
                    org_id,
                    actor: Actor::System,
                    action: "auth.oidc_login_failed".into(),
                    entity_type: None,
                    entity_id: None,
                    detail: serde_json::json!({ "reason": reason }),
                    ip: None,
                    occurred_at: chrono::Utc::now(),
                },
            )
            .await?;
            tx.commit().await.map_err(ApiError::from)?;
            Ok::<(), ApiError>(())
        }
        .await;
        let _ = result;
    });
}

fn map_oidc_err(e: crate::auth::oidc_client::OidcError) -> ApiError {
    tracing::warn!(error = %e, "oidc flow failed");
    ApiError::Unauthorized(format!("sso failed: {e}"))
}

fn sso_error_redirect(web_base: &str, reason: &str) -> Response {
    let mut location = url::Url::parse(&format!("{}/sso", web_base.trim_end_matches('/')))
        .unwrap_or_else(|_| url::Url::parse("http://localhost:3000/sso").expect("static fallback"));
    location.query_pairs_mut().append_pair("error", reason);
    Redirect::to(location.as_str()).into_response()
}

// --- authenticated link begin ----------------------------------------------

/// POST /api/auth/oidc/{name}/link — begins the EXPLICIT account-link flow
/// for the current user. Returns the authorize URL to navigate to.
#[utoipa::path(operation_id = "oidc_link_begin", post, path = "/api/auth/oidc/{name}/link",
    tag = "auth", security(("bearer" = [])), params(("name" = String, Path)),
    responses((status = 200, body = LinkBeginResponse), (status = 404)))]
pub async fn link_begin(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(name): Path<String>,
) -> ApiResult<Json<LinkBeginResponse>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    let user = &authed.principal.user;

    let mut tx = org_tx(&state, user.org_id).await?;
    let rows = core_db::oidc::list_providers(&mut *tx, user.org_id).await?;
    let provider = rows
        .into_iter()
        .find(|p| p.name == name && p.enabled)
        .ok_or(ApiError::NotFound)?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let discovery = state
        .oidc
        .discover(&provider.discovery_url, &provider.issuer)
        .await
        .map_err(map_oidc_err)?;

    let nonce = new_nonce();
    let verifier = new_verifier();
    let state_jwt = issue_state_token(&state.config, provider.id, &nonce, &verifier, Some(user.id))
        .map_err(|e| ApiError::Internal(e.into()))?;

    let redirect_uri = format!(
        "{}/api/auth/oidc/{}/callback",
        state.config.public_base_url.trim_end_matches('/'),
        name
    );
    let url = crate::auth::oidc_client::OidcHttpClient::authorize_url(
        &discovery,
        &provider.client_id,
        &redirect_uri,
        &state_jwt,
        &nonce,
        &verifier,
    );
    Ok(Json(LinkBeginResponse {
        authorize_url: url.to_string(),
    }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LinkBeginResponse {
    pub authorize_url: String,
}

// --- one-time exchange ------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct SsoExchangeRequest {
    pub code: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(untagged)]
pub enum SsoExchangeResponse {
    Tokens(TokenPair),
    /// Org MFA policy applies to federated logins (Wave 13 lane 7):
    /// complete via POST /api/auth/mfa/verify with the bridge token.
    MfaRequired {
        mfa_required: bool,
        mfa_token: String,
        methods: Vec<String>,
    },
    Linked {
        linked: bool,
    },
}

/// Typed 401 for a handoff code the exchange cannot redeem: unknown,
/// expired, or already consumed — one reason code for all three so an
/// attacker cannot fingerprint which check tripped.
fn sso_handoff_denied() -> ApiError {
    ApiError::AuthDenied {
        reason: denial_reasons::SSO_HANDOFF_INVALID,
        message: "invalid or expired sso code",
    }
}

/// POST /api/auth/sso/exchange — swaps the single-use callback code for
/// tokens (or a link confirmation). Codes die on first use or after 60s.
#[utoipa::path(operation_id = "sso_exchange", post, path = "/api/sso/exchange", tag = "auth",
    request_body = SsoExchangeRequest,
    responses((status = 200, body = SsoExchangeResponse), (status = 401)))]
pub async fn sso_exchange(
    State(state): State<AppState>,
    Json(req): Json<SsoExchangeRequest>,
) -> ApiResult<Json<SsoExchangeResponse>> {
    // The code's owning org is resolved pre-scope via the SECURITY DEFINER
    // bridge (NULL when unknown/expired), then the row is consumed under RLS.
    let org_id: Option<Uuid> = sqlx::query_scalar("SELECT app_resolve_sso_code($1)")
        .bind(req.code)
        .fetch_one(&state.pool)
        .await
        .map_err(ApiError::from)?;
    let Some(org_id) = org_id else {
        return Err(sso_handoff_denied());
    };

    let mut tx = org_tx(&state, org_id).await?;
    let payload = core_db::mfa::take_sso_code(&mut *tx, req.code)
        .await?
        .ok_or_else(sso_handoff_denied)?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let handoff: SsoHandoff = serde_json::from_value(payload).map_err(|_| sso_handoff_denied())?;
    Ok(Json(match handoff {
        SsoHandoff::Tokens(tokens) => SsoExchangeResponse::Tokens(tokens),
        SsoHandoff::MfaRequired {
            mfa_required,
            mfa_token,
            methods,
        } => SsoExchangeResponse::MfaRequired {
            mfa_required,
            mfa_token,
            methods,
        },
        SsoHandoff::Linked { linked } => SsoExchangeResponse::Linked { linked },
    }))
}
