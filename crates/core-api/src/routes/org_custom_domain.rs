//! Per-org custom domain management + generic DNS verification (the private build tracker
//! Wave 47 lane A8, `p47-org-custom-domains`, migration 0135).
//!
//! Four endpoints under `/api/v1/orgs/{org_id}/custom-domain`:
//!
//!   * `POST   .../custom-domain`         — set the domain, issue a TXT
//!     challenge, return `{ domain, verification_token, instructions }`.
//!   * `GET    .../custom-domain`         — current state + verification
//!     status + whether a DNS-provider plugin is installed.
//!   * `POST   .../custom-domain/verify`  — run the generic DNS verification.
//!   * `DELETE .../custom-domain`         — clear the domain.
//!
//! VENDOR-AGNOSTIC BY CONSTRUCTION. The verification is generic DNS: Core
//! resolves `_marula-verify.<domain>` `TXT` through the system resolver and
//! compares the token ([`crate::dns_verify`]). `TXT` is universal and never
//! edge-proxied, so this works with any DNS host with zero provider
//! integration; the A/CNAME leg only checks plausibility (an edge-proxied
//! record resolves to the CDN's IPs, not the origin's). The DNS-record
//! management seam (`adapters_core::dns::DnsRecordProvider`) is a PLUGIN
//! contract — a registered DNS-provider plugin may automate record creation,
//! but no endpoint here requires one, and the manual paste path is always
//! offered.
//!
//! Core does NOT terminate TLS or route traffic: it stores the verified
//! domain; the operator's own edge reads it back through the GET.
//!
//! AUTHORIZATION (the private build tracker A8 (G)):
//!   * READ (GET): any human member of the org reads its own state; a service
//!     account (machine key) additionally needs `org_custom_domain:read`.
//!   * MANAGE (POST/verify/DELETE): a human owner/admin holds the authority
//!     through the Admin matrix (`admin:edit`, exactly like org settings); a
//!     service account needs `org_custom_domain:manage` — the permission the
//!     `org_custom_domain.manage` plugin capability resolves onto.
//!   * A path org that is not the caller's own org is a 404, identical to
//!     "no such org", so a cross-org probe learns nothing.
//!   * The database access is RLS-scoped from the AUTHENTICATED PRINCIPAL's
//!     org (never the path value), so a future authorization slip reads and
//!     writes zero rows rather than another org's.

use crate::auth::middleware::{authed, require_permission};
use crate::auth::AuthedUser;
use crate::dns_verify::{self, VerificationReport};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use adapters_core::dns::{
    normalize_domain, ManualDnsInstructions, ManualDnsRecord, NoneDnsRecordProvider,
};
use axum::{
    extract::{Path, State},
    http::{Extensions, StatusCode},
    Json,
};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use core_domain::policy::{check_any, Action, Module, Permission};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// The token prefix Core issues on the TXT challenge. Pairs with the seam's
/// `_marula-verify` record prefix; the token is the exact string compared.
const TOKEN_PREFIX: &str = "marula-verify-";

fn admin_edit() -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action: Action::Edit,
    }
}

/// Read gate: a human member reads their own org; a machine key additionally
/// holds `org_custom_domain:read`. A non-own org is a 404.
fn authorize_read(authed: &AuthedUser, org_id: Uuid) -> ApiResult<()> {
    if authed.via_api_key {
        require_permission(&authed.principal, Permission::ViewOrgCustomDomain)?;
    }
    if authed.principal.user.org_id != org_id {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

/// Manage gate: a human holds the Admin `edit` matrix (owner/admin) or the
/// `org_custom_domain:manage` grant; a machine key must hold the grant. A
/// non-own org is a 404.
fn authorize_manage(authed: &AuthedUser, org_id: Uuid) -> ApiResult<()> {
    if authed.via_api_key {
        require_permission(&authed.principal, Permission::ManageOrgCustomDomain)?;
    } else {
        check_any(
            &authed.principal.permissions,
            &[admin_edit(), Permission::ManageOrgCustomDomain],
        )
        .expect()
        .map_err(ApiError::Forbidden)?;
    }
    if authed.principal.user.org_id != org_id {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

/// Issues a fresh verification token. Random (OsRng) and prefixed so it is
/// recognisable in a DNS record; the seam matcher compares it verbatim.
fn issue_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("{TOKEN_PREFIX}{}", hex::encode(bytes))
}

fn audit(org_id: Uuid, actor_id: Uuid, action: &str, detail: serde_json::Value) -> AuditEvent {
    AuditEvent {
        id: Uuid::now_v7(),
        org_id,
        actor: Actor::Human(actor_id),
        action: action.to_string(),
        entity_type: Some("org".to_string()),
        entity_id: Some(org_id.to_string()),
        detail,
        ip: None,
        occurred_at: chrono::Utc::now(),
    }
}

// --- wire shapes -------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetCustomDomainRequest {
    /// The custom domain to claim (e.g. `support.msp.example.com`). Stored
    /// normalized (lowercased, trailing dot stripped).
    #[schema(example = "support.msp.example.com")]
    pub domain: String,
}

/// One DNS record an operator pastes by hand. Mirrors the solver's
/// `ManualDnsRecord` as a utoipa schema (the seam crate carries no OpenAPI
/// dependency).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DnsRecordInstruction {
    /// The record NAME (e.g. `_marula-verify.support.msp.example.com`).
    pub name: String,
    /// The record TYPE: `TXT`, `A`, `AAAA`, or `CNAME`.
    #[schema(example = "TXT")]
    pub kind: String,
    /// The record VALUE to paste.
    pub value: String,
    pub ttl: u32,
    pub notes: String,
}

/// Manual DNS setup instructions (the seam's none-mode), ready to render.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CustomDomainInstructions {
    pub domain: String,
    pub verification_txt: DnsRecordInstruction,
    pub edge_record: Option<DnsRecordInstruction>,
    pub verification_note: String,
}

impl From<ManualDnsRecord> for DnsRecordInstruction {
    fn from(r: ManualDnsRecord) -> Self {
        Self {
            name: r.name,
            kind: r.kind.as_str().to_string(),
            value: r.value,
            ttl: r.ttl,
            notes: r.notes,
        }
    }
}

impl From<ManualDnsInstructions> for CustomDomainInstructions {
    fn from(i: ManualDnsInstructions) -> Self {
        Self {
            domain: i.domain,
            verification_txt: i.verification_txt.into(),
            edge_record: i.edge_record.map(Into::into),
            verification_note: i.verification_note,
        }
    }
}

/// POST response: exactly the three fields TASKLIST A8 (B) names.
#[derive(Debug, Serialize, ToSchema)]
pub struct SetCustomDomainResponse {
    pub domain: String,
    pub verification_token: String,
    pub instructions: CustomDomainInstructions,
}

/// GET response: current state + verification status + whether a DNS-provider
/// plugin is installed (A8 (F)).
#[derive(Debug, Serialize, ToSchema)]
pub struct CustomDomainView {
    pub org_id: Uuid,
    /// The stored domain, or `null` when none is set.
    pub domain: Option<String>,
    /// When Core last verified the TXT token, or `null`.
    #[schema(value_type = Option<String>, format = "date-time")]
    pub verified_at: Option<chrono::DateTime<chrono::Utc>>,
    /// `none` (no domain) | `pending` (set, unverified) | `verified`.
    #[schema(example = "pending")]
    pub status: String,
    /// The TXT challenge for the current domain (`null` when none is set).
    pub verification_token: Option<String>,
    /// Manual paste instructions for the current domain (`null` when none).
    pub instructions: Option<CustomDomainInstructions>,
    /// Whether an enabled plugin install in this org holds the
    /// `org_custom_domain.manage` capability — a DNS-provider plugin that can
    /// automate the records. Always informational: the manual path needs none.
    pub provider_plugin_installed: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TxtCheck {
    /// Whether an observed TXT string equalled the issued token (the
    /// authority for verification).
    pub matched: bool,
    /// Every TXT string observed at the record name.
    pub observed: Vec<String>,
    /// Set when the lookup itself failed (no resolver, transport error).
    pub error: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EdgeCheck {
    /// Whether the domain resolves to at least one address. Plausibility
    /// only: an edge-proxied record resolves to the CDN's IPs.
    pub resolves: bool,
    /// Set when the address lookup failed.
    pub error: Option<String>,
}

/// POST `/verify` response: the raw evidence plus the resulting status.
#[derive(Debug, Serialize, ToSchema)]
pub struct VerifyCustomDomainResponse {
    pub org_id: Uuid,
    pub domain: String,
    pub verified: bool,
    /// `pending` or `verified` (never `none` — verify requires a domain).
    pub status: String,
    /// The `_marula-verify.<domain>` name that was queried.
    pub record_name: String,
    pub txt: TxtCheck,
    pub edge: EdgeCheck,
}

fn instructions_for(domain: &str, token: &str) -> ApiResult<CustomDomainInstructions> {
    NoneDnsRecordProvider::manual_instructions(domain, token, None)
        .map(CustomDomainInstructions::from)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("manual instructions: {e}")))
}

fn build_view(
    org_id: Uuid,
    row: core_db::org_custom_domain::OrgCustomDomain,
    provider_plugin_installed: bool,
) -> ApiResult<CustomDomainView> {
    let instructions = match (&row.domain, &row.verification_token) {
        (Some(domain), Some(token)) => Some(instructions_for(domain, token)?),
        _ => None,
    };
    Ok(CustomDomainView {
        org_id,
        status: row.status().to_string(),
        domain: row.domain,
        verified_at: row.verified_at,
        verification_token: row.verification_token,
        instructions,
        provider_plugin_installed,
    })
}

// --- handlers ----------------------------------------------------------------

/// `POST /api/v1/orgs/{org_id}/custom-domain` — claim a domain.
#[utoipa::path(operation_id = "org_custom_domain_set", post,
    path = "/api/orgs/{org_id}/custom-domain", tag = "org",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "the org; must be the caller's own org")),
    request_body = SetCustomDomainRequest,
    responses((status = 200, body = SetCustomDomainResponse),
              (status = 400, description = "invalid domain"),
              (status = 401, description = "not authenticated"),
              (status = 403, description = "not an admin / missing org_custom_domain:manage"),
              (status = 404, description = "no such org for this caller"),
              (status = 409, description = "domain already claimed by another workspace")))]
pub async fn set_custom_domain(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(org_id): Path<Uuid>,
    Json(req): Json<SetCustomDomainRequest>,
) -> ApiResult<Json<SetCustomDomainResponse>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    authorize_manage(&authed, org_id)?;
    let domain = normalize_domain(&req.domain)
        .map_err(|e| ApiError::Validation(format!("invalid custom domain: {e}")))?;
    let token = issue_token();
    let instructions = instructions_for(&domain, &token)?;

    let principal_org = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let domain_for_tx = domain.clone();
    let token_for_tx = token.clone();
    crate::org_tx::with_org_tx(&state, principal_org, user_id, |mut ctx| async move {
        core_db::org_custom_domain::set_domain(
            &mut *ctx.tx,
            principal_org,
            &domain_for_tx,
            &token_for_tx,
        )
        .await?;
        ctx.emit(
            types::ORG_CUSTOM_DOMAIN_CHANGED,
            "org",
            Some(principal_org.to_string().as_str()),
            &serde_json::json!({ "domain": domain_for_tx.as_str(), "action": "set" }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &audit(
                principal_org,
                user_id,
                "org.custom_domain_set",
                serde_json::json!({ "domain": domain_for_tx.as_str() }),
            ),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;

    Ok(Json(SetCustomDomainResponse {
        domain,
        verification_token: token,
        instructions,
    }))
}

/// `GET /api/v1/orgs/{org_id}/custom-domain` — current state + verification
/// status + provider-plugin availability.
#[utoipa::path(operation_id = "org_custom_domain_get", get,
    path = "/api/orgs/{org_id}/custom-domain", tag = "org",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "the org; must be the caller's own org")),
    responses((status = 200, body = CustomDomainView),
              (status = 401, description = "not authenticated"),
              (status = 403, description = "service account without org_custom_domain:read"),
              (status = 404, description = "no such org for this caller")))]
pub async fn get_custom_domain(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(org_id): Path<Uuid>,
) -> ApiResult<Json<CustomDomainView>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    authorize_read(&authed, org_id)?;
    let principal_org = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, principal_org, user_id)
        .await
        .map_err(ApiError::from)?;
    let row = core_db::org_custom_domain::get(&mut *tx, org_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let provider = core_db::org_custom_domain::dns_provider_installed(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(build_view(org_id, row, provider)?))
}

/// `POST /api/v1/orgs/{org_id}/custom-domain/verify` — run the generic DNS
/// verification (TASKLIST A8 (C)). The TXT token is the authority; the
/// A/CNAME leg is plausibility only.
#[utoipa::path(operation_id = "org_custom_domain_verify", post,
    path = "/api/orgs/{org_id}/custom-domain/verify", tag = "org",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "the org; must be the caller's own org")),
    responses((status = 200, body = VerifyCustomDomainResponse),
              (status = 401, description = "not authenticated"),
              (status = 403, description = "not an admin / missing org_custom_domain:manage"),
              (status = 404, description = "no such org for this caller"),
              (status = 409, description = "no custom domain is set")))]
pub async fn verify_custom_domain(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(org_id): Path<Uuid>,
) -> ApiResult<Json<VerifyCustomDomainResponse>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    authorize_manage(&authed, org_id)?;
    let principal_org = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    // Read the current domain + token in a short scoped transaction, then
    // release it before the (potentially slow) network lookup.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, principal_org, user_id)
        .await
        .map_err(ApiError::from)?;
    let row = core_db::org_custom_domain::get(&mut *tx, org_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    tx.commit().await.map_err(ApiError::from)?;

    let (domain, token) = match (row.domain, row.verification_token) {
        (Some(domain), Some(token)) => (domain, token),
        _ => {
            return Err(ApiError::Conflict(
                "no custom domain is set; POST one before verifying".into(),
            ))
        }
    };

    let resolver = dns_verify::txt_resolver();
    let report: VerificationReport = dns_verify::verify_domain(resolver.as_ref(), &domain, &token)
        .await
        .map_err(|e| ApiError::Validation(format!("cannot verify {domain}: {e}")))?;
    let verified = report.matched;

    if verified {
        let user_id = authed.principal.user.id;
        let domain_for_tx = domain.clone();
        crate::org_tx::with_org_tx(&state, principal_org, user_id, |mut ctx| async move {
            core_db::org_custom_domain::mark_verified(&mut *ctx.tx, principal_org).await?;
            ctx.emit(
                types::ORG_CUSTOM_DOMAIN_CHANGED,
                "org",
                Some(principal_org.to_string().as_str()),
                &serde_json::json!({ "domain": domain_for_tx.as_str(), "action": "verified" }),
            )
            .await?;
            core_db::audit::insert(
                &mut *ctx.tx,
                &audit(
                    principal_org,
                    user_id,
                    "org.custom_domain_verified",
                    serde_json::json!({ "domain": domain_for_tx.as_str() }),
                ),
            )
            .await?;
            Ok((ctx, ()))
        })
        .await?;
    }

    Ok(Json(VerifyCustomDomainResponse {
        org_id,
        domain,
        verified,
        status: if verified { "verified" } else { "pending" }.to_string(),
        record_name: report.record_name,
        txt: TxtCheck {
            matched: verified,
            observed: report.txt_observed,
            error: report.txt_error,
        },
        edge: EdgeCheck {
            resolves: report.edge_resolves,
            error: report.edge_error,
        },
    }))
}

/// `DELETE /api/v1/orgs/{org_id}/custom-domain` — clear the domain.
#[utoipa::path(operation_id = "org_custom_domain_delete", delete,
    path = "/api/orgs/{org_id}/custom-domain", tag = "org",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "the org; must be the caller's own org")),
    responses((status = 204, description = "domain cleared"),
              (status = 401, description = "not authenticated"),
              (status = 403, description = "not an admin / missing org_custom_domain:manage"),
              (status = 404, description = "no such org for this caller")))]
pub async fn delete_custom_domain(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(org_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let authed = authed::<AuthedUser>(&extensions)?;
    authorize_manage(&authed, org_id)?;
    let principal_org = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    crate::org_tx::with_org_tx(&state, principal_org, user_id, |mut ctx| async move {
        core_db::org_custom_domain::clear(&mut *ctx.tx, principal_org).await?;
        ctx.emit(
            types::ORG_CUSTOM_DOMAIN_CHANGED,
            "org",
            Some(principal_org.to_string().as_str()),
            &serde_json::json!({ "action": "cleared" }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &audit(
                principal_org,
                user_id,
                "org.custom_domain_cleared",
                serde_json::json!({}),
            ),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
