//! Edge-assertion federation (the private build tracker ID2/ID3/ID4, Wave 13 lane 7):
//! ZTNA reverse-proxy authentication — the Cloudflare Access pattern.
//! A proxy injects a SIGNED JWT (`Cf-Access-Jwt-Assertion`) into
//! requests to the origin; the exchange endpoint validates that
//! assertion against the org's pinned team JWKS with issuer AND
//! audience pinned byte-for-byte, then resolves the asserted identity
//! under the same conservative linking rules as every other bundle.
//!
//! Security posture (non-negotiable):
//! * Raw header trust is never valid — there is no mode that skips
//!   signature verification (the config table has no column for one).
//! * All invalid assertions (missing/garbled/bad signature/wrong
//!   issuer/wrong audience/expired) collapse to ONE typed reason,
//!   `edge_assertion_invalid`; which check tripped is logged
//!   server-side only.
//! * Deny-by-default: sources are created DISABLED, only an admin can
//!   enable them, and the pre-auth bridge resolves only ENABLED
//!   sources — unknown and disabled are indistinguishable 401s.
//! * NO JIT provisioning for edge sources: the proxy JWT carries no
//!   verified-email claim, so there is nothing verified to provision
//!   from. Login requires an admin-created link.
//! * Sessions are the SAME machinery as password login: tokens come
//!   from `handlers::issue_tokens` (auth_sessions + refresh chain),
//!   and the org MFA gate applies before issuance.

use crate::auth::federation::{denial_reasons, kind};
use crate::auth::handlers::{audit_best_effort, federated_mfa_gate, issue_tokens};
use crate::auth::middleware::{authed, require_permission};
use crate::auth::AuthedUser;
use crate::error::{ApiError, ApiResult};
use crate::routes::openapi::TokenPair;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::{Extensions, HeaderMap},
    Json,
};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

const BUNDLE_KIND: &str = kind::EDGE_ASSERTION;
const CF_ASSERTION_HEADER: &str = "cf-access-jwt-assertion";

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
// Admin source management
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertEdgeSourceRequest {
    pub display_name: String,
    /// PINNED issuer — the assertion's `iss` must equal this exactly.
    /// For Cloudflare Access: `https://<team>.cloudflareaccess.com`.
    pub issuer: String,
    /// PINNED audience — the assertion's `aud` must equal this exactly
    /// (the Access application's AUD tag).
    pub audience: String,
    /// Team JWKS URL the assertion signature is verified against.
    pub jwks_url: String,
    /// Deny-by-default: omitted or false creates/keeps the source
    /// DISABLED. Only an explicit admin `true` enables authentication.
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EdgeSourceInfo {
    #[schema(value_type = uuid::Uuid)]
    pub id: uuid::Uuid,
    /// URL path segment used by the exchange + link routes.
    pub name: String,
    pub display_name: String,
    pub issuer: String,
    pub audience: String,
    pub jwks_url: String,
    pub enabled: bool,
}

fn meta_to_info(m: core_db::federation::EdgeSourceMeta) -> EdgeSourceInfo {
    EdgeSourceInfo {
        id: m.id,
        name: m.name,
        display_name: m.display_name,
        issuer: m.issuer,
        audience: m.audience,
        jwks_url: m.jwks_url,
        enabled: m.enabled,
    }
}

/// GET /api/auth/edge/sources — admin listing of edge-assertion sources.
#[utoipa::path(operation_id = "edge_sources_list", get, path = "/api/auth/edge/sources",
    tag = "auth", security(("bearer" = [])),
    responses((status = 200, body = [EdgeSourceInfo]), (status = 403)))]
pub async fn sources_list(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<EdgeSourceInfo>>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    require_sso_admin(&authed)?;
    let org_id = authed.principal.user.org_id;
    let mut tx = org_tx(&state, org_id).await?;
    let rows = core_db::federation::list_edge_sources(&mut *tx, org_id).await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(rows.into_iter().map(meta_to_info).collect()))
}

/// PUT /api/auth/edge/sources/{name} — create or replace an edge
/// source. Pins are stored verbatim and enforced byte-for-byte at
/// verification time; every write is audited.
#[utoipa::path(operation_id = "edge_source_upsert", put, path = "/api/auth/edge/sources/{name}",
    tag = "auth", security(("bearer" = [])), params(("name" = String, Path)),
    request_body = UpsertEdgeSourceRequest,
    responses((status = 200, body = EdgeSourceInfo), (status = 403)))]
pub async fn sources_upsert(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(name): Path<String>,
    Json(req): Json<UpsertEdgeSourceRequest>,
) -> ApiResult<Json<EdgeSourceInfo>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    require_sso_admin(&authed)?;
    let user = &authed.principal.user;

    let name_ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !name_ok
        || req.display_name.trim().is_empty()
        || req.issuer.trim().is_empty()
        || req.audience.trim().is_empty()
        || req.jwks_url.trim().is_empty()
    {
        return Err(ApiError::Validation(
            "source name must be lowercase alphanumeric/'-' and issuer, audience, jwks_url are required"
                .into(),
        ));
    }
    // https only, EXCEPT loopback (self-hosted dev/test JWKS on
    // 127.0.0.1/localhost). A remote plaintext JWKS would let a
    // network attacker swap the trust set — deny, don't downgrade.
    let loopback_jwks = url::Url::parse(req.jwks_url.trim())
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .is_some_and(|h| h == "127.0.0.1" || h == "localhost" || h == "::1");
    if !req.jwks_url.starts_with("https://") && !loopback_jwks {
        return Err(ApiError::Validation("jwks_url must be https".into()));
    }

    let mut tx = org_tx(&state, user.org_id).await?;
    let id = core_db::federation::upsert_edge_source(
        &mut *tx,
        Uuid::now_v7(),
        user.org_id,
        core_db::federation::NewEdgeSource {
            name: &name,
            display_name: req.display_name.trim(),
            issuer: req.issuer.trim(),
            audience: req.audience.trim(),
            jwks_url: req.jwks_url.trim(),
            // Deny-by-default: the enabled decision is the admin's
            // explicit field, never inferred from presence.
            enabled: req.enabled,
        },
        user.id,
    )
    .await?;
    let meta = core_db::federation::list_edge_sources(&mut *tx, user.org_id)
        .await?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or(ApiError::NotFound)?;
    core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id: user.org_id,
            actor: Actor::Human(user.id),
            action: "auth.edge_source_saved".into(),
            entity_type: Some("edge_assertion_source".into()),
            entity_id: Some(id.to_string()),
            detail: serde_json::json!({
                "name": name,
                "issuer": meta.issuer,
                "audience": meta.audience,
                "enabled": meta.enabled,
            }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(meta_to_info(meta)))
}

/// DELETE /api/auth/edge/sources/{name}.
#[utoipa::path(operation_id = "edge_source_delete", delete, path = "/api/auth/edge/sources/{name}",
    tag = "auth", security(("bearer" = [])), params(("name" = String, Path)),
    responses((status = 204), (status = 404), (status = 403)))]
pub async fn sources_delete(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(name): Path<String>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<AuthedUser>(&extensions)?;
    require_sso_admin(&authed)?;
    let user = &authed.principal.user;

    let mut tx = org_tx(&state, user.org_id).await?;
    let rows = core_db::federation::list_edge_sources(&mut *tx, user.org_id).await?;
    let target = rows
        .iter()
        .find(|s| s.name == name)
        .ok_or(ApiError::NotFound)?;
    let deleted = core_db::federation::delete_edge_source(&mut *tx, user.org_id, target.id).await?;
    if deleted == 0 {
        return Err(ApiError::NotFound);
    }
    core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id: user.org_id,
            actor: Actor::Human(user.id),
            action: "auth.edge_source_deleted".into(),
            entity_type: Some("edge_assertion_source".into()),
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
// Admin-explicit links (NO JIT for edge sources)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct EdgeLinkInfo {
    #[schema(value_type = uuid::Uuid)]
    pub link_id: uuid::Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub user_id: uuid::Uuid,
    pub subject: String,
    pub email: String,
    pub last_login_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateEdgeLinkRequest {
    /// Local Marula user to bind. The admin is the verified-email
    /// authority here: linking asserts "this local account is this
    /// edge identity".
    #[schema(value_type = uuid::Uuid)]
    pub user_id: uuid::Uuid,
    /// The edge assertion's `sub` (Cloudflare Access user id).
    pub subject: String,
    /// The identity's email, recorded for audit/display only.
    pub email: String,
}

/// GET /api/auth/edge/sources/{name}/links — admin link listing.
#[utoipa::path(operation_id = "edge_links_list", get,
    path = "/api/auth/edge/sources/{name}/links",
    tag = "auth", security(("bearer" = [])), params(("name" = String, Path)),
    responses((status = 200, body = [EdgeLinkInfo]), (status = 404), (status = 403)))]
pub async fn links_list(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(name): Path<String>,
) -> ApiResult<Json<Vec<EdgeLinkInfo>>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    require_sso_admin(&authed)?;
    let user = &authed.principal.user;
    let mut tx = org_tx(&state, user.org_id).await?;
    let source = find_source_by_name(&mut tx, user.org_id, &name).await?;
    let rows =
        core_db::federation::list_links(&mut *tx, user.org_id, BUNDLE_KIND, source.id).await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(
        rows.into_iter()
            .map(|l| EdgeLinkInfo {
                link_id: l.id,
                user_id: l.user_id,
                subject: l.subject,
                email: l.email,
                last_login_at: l.last_login_at,
            })
            .collect(),
    ))
}

/// POST /api/auth/edge/sources/{name}/links — create an explicit link.
/// Audited as `auth.federated_account_linked`.
#[utoipa::path(operation_id = "edge_link_create", post,
    path = "/api/auth/edge/sources/{name}/links",
    tag = "auth", security(("bearer" = [])), params(("name" = String, Path)),
    request_body = CreateEdgeLinkRequest,
    responses((status = 200, body = EdgeLinkInfo), (status = 404), (status = 403)))]
pub async fn links_create(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(name): Path<String>,
    Json(req): Json<CreateEdgeLinkRequest>,
) -> ApiResult<Json<EdgeLinkInfo>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    require_sso_admin(&authed)?;
    let user = &authed.principal.user;
    if req.subject.trim().is_empty() || req.email.trim().is_empty() {
        return Err(ApiError::Validation(
            "subject and email are required".into(),
        ));
    }

    let mut tx = org_tx(&state, user.org_id).await?;
    let source = find_source_by_name(&mut tx, user.org_id, &name).await?;
    // The target user must exist inside this org (RLS makes cross-org
    // ids invisible → NotFound).
    core_db::users::get_user(&mut *tx, user.org_id, req.user_id).await?;
    let link_id = Uuid::now_v7();
    core_db::federation::create_link(
        &mut *tx,
        link_id,
        user.org_id,
        BUNDLE_KIND,
        source.id,
        req.user_id,
        core_db::federation::IdpIdentity {
            subject: req.subject.trim(),
            email: req.email.trim().to_lowercase().as_str(),
        },
        user.id,
    )
    .await?;
    core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id: user.org_id,
            actor: Actor::Human(user.id),
            action: "auth.federated_account_linked".into(),
            entity_type: Some("federation_link".into()),
            entity_id: Some(link_id.to_string()),
            detail: serde_json::json!({
                "bundle": BUNDLE_KIND,
                "source": name,
                "subject_bound": true,
                "linked_user": req.user_id,
            }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(EdgeLinkInfo {
        link_id,
        user_id: req.user_id,
        subject: req.subject.trim().to_string(),
        email: req.email.trim().to_string(),
        last_login_at: None,
    }))
}

/// DELETE /api/auth/edge/sources/{name}/links/{link_id}. Audited as
/// `auth.federated_account_unlinked`.
#[utoipa::path(operation_id = "edge_link_delete", delete,
    path = "/api/auth/edge/sources/{name}/links/{link_id}",
    tag = "auth", security(("bearer" = [])),
    params(("name" = String, Path), ("link_id" = uuid::Uuid, Path)),
    responses((status = 204), (status = 404), (status = 403)))]
pub async fn links_delete(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((name, link_id)): Path<(String, Uuid)>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<AuthedUser>(&extensions)?;
    require_sso_admin(&authed)?;
    let user = &authed.principal.user;

    let mut tx = org_tx(&state, user.org_id).await?;
    let source = find_source_by_name(&mut tx, user.org_id, &name).await?;
    let deleted =
        core_db::federation::delete_link(&mut *tx, user.org_id, link_id, BUNDLE_KIND, source.id)
            .await?;
    if deleted == 0 {
        return Err(ApiError::NotFound);
    }
    core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id: user.org_id,
            actor: Actor::Human(user.id),
            action: "auth.federated_account_unlinked".into(),
            entity_type: Some("federation_link".into()),
            entity_id: Some(link_id.to_string()),
            detail: serde_json::json!({ "bundle": BUNDLE_KIND, "source": name }),
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
// Public exchange
// ---------------------------------------------------------------------------

/// Pre-auth resolution via the SECURITY DEFINER bridge: returns None
/// for unknown AND disabled sources (uniform denial — no enumeration).
async fn resolve_enabled_source(
    pool: &sqlx::PgPool,
    name: &str,
) -> ApiResult<Option<(Uuid, Uuid, String, String, String)>> {
    let row: Option<(Uuid, Uuid, String, String, String)> = sqlx::query_as(
        "SELECT source_id, org_id, issuer, audience, jwks_url \
         FROM app_resolve_edge_source($1)",
    )
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from)?;
    Ok(row)
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(untagged)]
pub enum EdgeExchangeResponse {
    Tokens(TokenPair),
    /// Org MFA policy satisfied the same way as password login: any
    /// enrolled factor must complete via POST /api/auth/mfa/verify.
    MfaRequired {
        mfa_required: bool,
        mfa_token: String,
        methods: Vec<String>,
    },
}

/// POST /api/auth/edge/{name}/exchange — swaps a valid
/// `Cf-Access-Jwt-Assertion` (header) for Marula tokens. The
/// assertion MUST verify against the source's pinned team JWKS with
/// issuer AND audience pinned; failures are a uniform typed 401.
#[utoipa::path(operation_id = "edge_exchange", post, path = "/api/auth/edge/{name}/exchange",
    tag = "auth", params(("name" = String, Path)),
    responses((status = 200, body = EdgeExchangeResponse), (status = 401)))]
pub async fn exchange(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Json<EdgeExchangeResponse>> {
    let denied = |reason: &'static str| match reason {
        denial_reasons::LINK_REQUIRED => ApiError::AuthDenied {
            reason: denial_reasons::LINK_REQUIRED,
            message: "this edge identity is not linked to an account; ask an administrator",
        },
        _ => ApiError::AuthDenied {
            reason: "edge_assertion_invalid",
            message: "edge assertion rejected",
        },
    };

    let Some(source) = resolve_enabled_source(&state.pool, &name).await? else {
        // Uniform for unknown AND disabled: no source enumeration.
        return Err(ApiError::AuthDenied {
            reason: "edge_source_unknown",
            message: "no such edge-assertion source",
        });
    };
    let (source_id, org_id, issuer, audience, jwks_url) = source;

    let assertion = headers
        .get(CF_ASSERTION_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if assertion.is_empty() {
        tracing::warn!(source = %name, "edge exchange: assertion header missing");
        return Err(denied("invalid"));
    }

    // Signature (RS256 via the pinned team JWKS), expiry, and BOTH pins
    // are enforced inside this call. Every failure mode below is the
    // same external reason; only the server log says which.
    let claims = state
        .oidc
        .verify_id_token(assertion, &jwks_url, &issuer, &audience)
        .await
        .map_err(|e| {
            tracing::warn!(source = %name, error = %e, "edge assertion rejected");
            denied("invalid")
        })?;

    let mut tx = org_tx(&state, org_id).await?;
    let user_id =
        core_db::federation::find_link_user(&mut *tx, BUNDLE_KIND, source_id, &claims.sub)
            .await?
            .ok_or_else(|| {
                audit_edge_failed(&state, org_id, denial_reasons::LINK_REQUIRED, &name);
                denied(denial_reasons::LINK_REQUIRED)
            })?;
    core_db::federation::touch_link_login(&mut *tx, BUNDLE_KIND, source_id, &claims.sub).await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    // Org MFA policy applies regardless of identity source: same gate
    // as password login, BEFORE tokens exist.
    if let Some((mfa_token, methods)) = federated_mfa_gate(&state, user_id, org_id).await? {
        audit_best_effort(
            &state,
            org_id,
            Actor::Human(user_id),
            "auth.mfa_challenge_issued",
            Some("user"),
            Some(user_id.to_string()),
            serde_json::json!({ "methods": methods, "source": "edge_assertion" }),
        )
        .await;
        return Ok(Json(EdgeExchangeResponse::MfaRequired {
            mfa_required: true,
            mfa_token,
            methods,
        }));
    }

    // Same session machinery as password login — never a parallel one.
    let tokens = issue_tokens(&state, user_id, org_id, None).await?;
    audit_best_effort(
        &state,
        org_id,
        Actor::Human(user_id),
        "auth.edge_login_success",
        Some("user"),
        Some(user_id.to_string()),
        serde_json::json!({ "source": name }),
    )
    .await;
    Ok(Json(EdgeExchangeResponse::Tokens(tokens)))
}

fn audit_edge_failed(state: &AppState, org_id: Uuid, reason: &str, source: &str) {
    let state = state.clone();
    let reason = reason.to_string();
    let source = source.to_string();
    tokio::spawn(async move {
        let result = async {
            let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
            core_db::set_org(&mut tx, org_id).await?;
            core_db::audit::insert(
                &mut *tx,
                &AuditEvent {
                    id: Uuid::now_v7(),
                    org_id,
                    actor: Actor::System,
                    action: "auth.edge_login_failed".into(),
                    entity_type: None,
                    entity_id: None,
                    detail: serde_json::json!({ "reason": reason, "source": source }),
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

/// Resolves a source by name under RLS scope (admin CRUD surfaces).
async fn find_source_by_name(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    name: &str,
) -> ApiResult<core_db::federation::EdgeSourceMeta> {
    core_db::federation::list_edge_sources(&mut *tx, org_id)
        .await?
        .into_iter()
        .find(|s| s.name == name)
        .ok_or(ApiError::NotFound)
}
