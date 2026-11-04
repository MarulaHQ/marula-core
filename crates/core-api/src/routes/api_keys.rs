//! API-key management (the private build tracker 2.15 / §6): scoped, rotatable, last-used
//! tracked — with show-once creation.
//!
//! Purpose: admin CRUD over org API keys. Raw key material (`mrl_<hex>`)
//! exists only in the creation response; storage keeps hex(HMAC-SHA256(
//! secret_key, raw)) plus a non-secret display prefix. Scopes use plugin
//! capability names ("tickets:read") and are enforced by the x-api-key
//! auth path in `crate::auth::middleware`. Revocation is immediate and
//! idempotent.
//!
//! Non-purpose: no per-request authorization decisions here (the
//! middleware owns scope enforcement), no user-facing tokens (JWTs own
//! interactive sessions).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{extract::State, http::Extensions, Json};
use chrono::Utc;
use core_domain::entities::{Actor, AuditEvent};
use core_domain::policy::Permission;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use utoipa::ToSchema;
use uuid::Uuid;

// Capability scopes a machine key may hold live in `crate::api_scopes`, which
// also publishes them to the OpenAPI document. This module validates against
// that list; it does not keep a second copy.

// --- hashing ------------------------------------------------------------------

/// hex(HMAC-SHA256(server secret_key, raw_key)). Keyed by the at-rest
/// secret so a leaked DB alone cannot verify stolen keys offline.
pub fn hash_raw_key(state: &AppState, raw: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(&state.config.secret_key.0)
        .expect("hmac accepts any key length");
    mac.update(raw.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn validate_scopes(scopes: &[String]) -> ApiResult<()> {
    if scopes.is_empty() {
        return Err(ApiError::Validation(
            "at least one scope is required".into(),
        ));
    }
    for s in scopes {
        // Only the published machine vocabulary is mintable (crate::api_scopes).
        // The MCP read catalog is deliberately NOT a source here: those
        // capabilities authorize MCP tools through the org's grant registry,
        // and the credential that carries a caller's session is a JWT. A
        // catalog name on an API key authenticated with no permission behind
        // it, which is what F95 recorded.
        if crate::api_scopes::get(s).is_none() {
            let known: Vec<&str> = crate::api_scopes::names().collect();
            return Err(ApiError::Validation(format!(
                "unknown scope {s:?}; known: {}",
                known.join(", ")
            )));
        }
    }
    Ok(())
}

// --- wire shapes ---------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyView {
    pub id: Uuid,
    pub name: String,
    /// Non-secret display prefix, e.g. "mrl_ab12cd34".
    pub prefix: String,
    pub scopes: Vec<String>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<core_db::webhooks::ApiKeyRow> for ApiKeyView {
    fn from(r: core_db::webhooks::ApiKeyRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            prefix: r.prefix,
            scopes: r.scopes,
            last_used_at: r.last_used_at,
            revoked_at: r.revoked_at,
            created_at: r.created_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyList {
    pub keys: Vec<ApiKeyView>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateApiKeyRequest {
    pub name: String,
    /// Capability scopes from [`crate::api_scopes`]. Must be empty when the
    /// key is bound to a service account: the account's own keys decide.
    pub scopes: Vec<String>,
    /// Bind the key to a service account, making it that agent.
    #[serde(default)]
    pub service_account_id: Option<Uuid>,
}

/// The ONLY response carrying raw key material. Lost keys are rotated by
/// creating a new key and revoking this one — never recovered.
#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyCreatedResponse {
    pub key: ApiKeyView,
    /// Raw bearer material: `mrl_<32 hex>`.
    pub raw_key: String,
}

// --- handlers ---------------------------------------------------------------------

/// GET /api/api-keys — listing without secrets.
#[utoipa::path(operation_id = "api_keys_list", get, path = "/api/api-keys", tag = "integrations", security(("bearer" = [])),
    responses((status = 200, body = ApiKeyList)))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<ApiKeyList>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManageApiKeys)?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, authed.principal.user.org_id).await?;
    let rows = core_db::webhooks::list_api_keys(&mut *tx, authed.principal.user.org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(ApiKeyList {
        keys: rows.into_iter().map(ApiKeyView::from).collect(),
    }))
}

/// POST /api/api-keys — creates a key and returns the raw secret ONCE.
#[utoipa::path(operation_id = "api_keys_create", post, path = "/api/api-keys", tag = "integrations", security(("bearer" = [])),
    request_body = CreateApiKeyRequest,
    responses((status = 201, body = ApiKeyCreatedResponse)))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateApiKeyRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<ApiKeyCreatedResponse>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManageApiKeys)?;
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let name = req.name.trim();
    if name.is_empty() || name.len() > 120 {
        return Err(ApiError::Validation("name must be 1..=120 chars".into()));
    }
    let service_account_id = match req.service_account_id {
        Some(account_id) => {
            let account = core_db::service_accounts::find_active_service_account_by_id(
                &state.pool,
                account_id,
            )
            .await?
            .ok_or_else(|| ApiError::Validation("service account is not active".into()))?;
            if account.org_id != org_id {
                return Err(ApiError::Validation(
                    "service account belongs to another organization".into(),
                ));
            }
            if !req.scopes.is_empty() {
                return Err(ApiError::Validation(
                    "a service-account key takes the account's permissions; omit scopes".into(),
                ));
            }
            Some(account_id)
        }
        None => {
            validate_scopes(&req.scopes)?;
            None
        }
    };

    // 128 bits of entropy is plenty for an offline-verifiable token.
    let raw_key = format!("mrl_{}", uuid::Uuid::now_v7().simple());
    // Display prefix: first 12 chars ("mrl_" + 8 hex), never the tail.
    let prefix = raw_key[..12].to_string();

    let id = Uuid::now_v7();
    let mut tx = org_tx::begin(&state, org_id, user_id).await?;
    let row = core_db::webhooks::insert_api_key(
        &mut *tx.tx,
        id,
        org_id,
        name,
        &hash_raw_key(&state, &raw_key),
        &prefix,
        &req.scopes,
        user_id,
        service_account_id,
    )
    .await?;
    core_db::audit::insert(
        &mut *tx.tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(user_id),
            action: "apikey.created".into(),
            entity_type: Some("api_key".into()),
            entity_id: Some(id.to_string()),
            detail: serde_json::json!({ "name": name, "scopes": req.scopes }),
            ip: None,
            occurred_at: Utc::now(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(ApiKeyCreatedResponse {
            key: row.into(),
            raw_key,
        }),
    ))
}

/// DELETE /api/api-keys/{id} — immediate revocation (rotation story:
/// create replacement → revoke old).
#[utoipa::path(operation_id = "api_keys_revoke", delete, path = "/api/api-keys/{id}", tag = "integrations", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 204)))]
pub async fn revoke(
    State(state): State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManageApiKeys)?;
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;

    let mut tx = org_tx::begin(&state, org_id, user_id).await?;
    core_db::webhooks::revoke_api_key(&mut *tx.tx, org_id, id).await?;
    core_db::audit::insert(
        &mut *tx.tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(user_id),
            action: "apikey.revoked".into(),
            entity_type: Some("api_key".into()),
            entity_id: Some(id.to_string()),
            detail: serde_json::json!({}),
            ip: None,
            occurred_at: Utc::now(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::validate_scopes;

    /// The published vocabulary is the mintable vocabulary. A scope the
    /// document names but this endpoint refuses would be a contract that
    /// cannot be exercised; the reverse is F95.
    #[test]
    fn every_published_scope_is_mintable() {
        for scope in crate::api_scopes::names() {
            assert!(
                validate_scopes(&[scope.to_string()]).is_ok(),
                "{scope} is published but this endpoint refuses it"
            );
        }
    }

    /// A catalog capability authorizes an MCP tool through the grant
    /// registry, not through a key. Minting one used to succeed and produce a
    /// key that authenticated with no permission behind it.
    #[test]
    fn a_catalog_capability_is_not_mintable() {
        let error = validate_scopes(&["invoices:read".to_string()])
            .expect_err("invoices:read is a grant, not a key scope");
        let message = error.to_string();
        assert!(
            message.contains("unknown scope"),
            "the refusal must name the vocabulary, got: {message}"
        );
    }

    #[test]
    fn an_empty_scope_list_is_refused() {
        assert!(validate_scopes(&[]).is_err());
    }
}
