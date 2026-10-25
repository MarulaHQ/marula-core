//! The FIRST-PARTY auth-provider contract (Wave 19 lane 5, Scope C).
//!
//! Generalizes the Wave 13 lane 7 federation pattern (edge-assertion
//! exchange + OIDC callback) into ONE resolution seam: an auth provider is
//! an admin-registered, deny-by-default, audited source that resolves a
//! credential/assertion into a principal. SESSION AND TOKEN ISSUANCE STAY
//! IN CORE — pluggable credential verification, NON-pluggable sessions.
//!
//! Doctrine (non-negotiable, decode from FEDERATION_BUNDLE_INTERFACE_VERSION):
//! * Providers are FIRST-PARTY/admin-registered, exactly like edge sources.
//!   Enabling one is an explicit, audited admin action; disabled and
//!   unknown providers are indistinguishable rejections.
//! * A provider NEVER issues sessions, tokens, refresh chains, cookies, or
//!   copies audit rows in tx — `handlers::issue_tokens`, hashed refresh
//!   rotation, audit-in-tx, and the RLS org scope are the ONE machinery.
//! * A provider is NOT part of the plugin capability vocabulary (ADR:
//!   plugin-host "no parallel auth system"); there is no `auth:*` plugin
//!   scope and there never will be one. A commercial headless IdP (Ory
//!   Kratos / Keycloak) would integrate by ASSERTING THROUGH this seam as
//!   a future provider kind — registered by an admin like an edge source —
//!   not by shipping auth logic in a plugin. (Not built in this lane;
//!   documented in docs/reference/auth-provider-contract.md.)
//! * The org MFA policy applies regardless of identity source (federated
//!   gate parity), and every lifecycle step is audited.

use crate::auth::federation::LinkNamespace;
use crate::auth::handlers::{audit_best_effort, federated_mfa_gate};
use crate::error::{ApiError, ApiResult};
use crate::routes::openapi::TokenPair;
use crate::state::AppState;
use axum::Json;
use uuid::Uuid;

/// Version of the auth-provider interface contract. Bump on any debate
/// change to [`ResolvedAssertion`]/[`ProviderLoginOutcome`]; provider
/// implementations declare the version they were built against.
pub const AUTH_PROVIDER_INTERFACE_VERSION: u32 = 1;

/// What a provider VERIFIED (its job ends here — nothing downstream).
#[derive(Debug, Clone)]
pub struct VerifiedPrincipal {
    /// The verified identity subject (IdP `sub`, proxy user id…).
    pub subject: String,
    /// The identity's email — display/audit only for non-JIT sources.
    pub email: String,
    /// Whether the SOURCE asserted (and the provider verified) email
    /// control — the only field JIT may provision from.
    pub email_verified: bool,
    /// Display name as the source asserted it (display-only).
    pub display_name: Option<String>,
    /// The AMR set the source proved (e.g. ["mfa","pwd"] for an IdP).
    pub amr: Vec<String>,
}

/// One resolution batch — link-or-refuse is the shared decision, then the
/// SAME token machinery as password login issues the session.
pub struct ProviderContext {
    /// Namespace for link rows (bundle kind + source id, or OIDC provider id).
    pub namespace: LinkNamespace,
    pub org_id: Uuid,
    /// Audit-facing source label ("edge", provider name, …).
    pub display_source: &'static str,
    /// NO JIT for sources without a verified-email authority (edge);
    /// OIDC providers use the org's explicit opt-in + verified claim.
    pub jit_allowed: bool,
}

/// The uniform login result: tokens, or the ORG MFA gate (same shape as
/// password login's `mfa_required` answer).
pub enum ProviderLoginOutcome {
    Tokens(TokenPair),
    MfaRequired {
        mfa_required: bool,
        mfa_token: String,
        methods: Vec<String>,
    },
}

impl From<ProviderLoginOutcome> for Json<crate::routes::edge::EdgeExchangeResponse> {
    fn from(outcome: ProviderLoginOutcome) -> Self {
        match outcome {
            ProviderLoginOutcome::MfaRequired {
                mfa_required,
                mfa_token,
                methods,
            } => Json(crate::routes::edge::EdgeExchangeResponse::MfaRequired {
                mfa_required,
                mfa_token,
                methods,
            }),
            ProviderLoginOutcome::Tokens(tokens) => {
                Json(crate::routes::edge::EdgeExchangeResponse::Tokens(tokens))
            }
        }
    }
}

/// The shared login path every first-party provider routes through:
/// conservative link resolution → org MFA gate → SAME session machinery.
/// Returns AuthDenied(link_required) for unlinked principals, then issues.
pub async fn resolve_and_issue(
    state: &AppState,
    ctx: &ProviderContext,
    verified: VerifiedPrincipal,
) -> ApiResult<ProviderLoginOutcome> {
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, ctx.org_id).await?;
    let user_id = match core_db::federation::find_link_user(
        &mut *tx,
        match ctx.namespace {
            LinkNamespace::Oidc(_) => crate::auth::federation::kind::OIDC,
            LinkNamespace::Bundle(kind, _) => kind,
        },
        match &ctx.namespace {
            LinkNamespace::Oidc(id) | LinkNamespace::Bundle(_, id) => *id,
        },
        &verified.subject,
    )
    .await?
    {
        Some(user_id) => user_id,
        None if ctx.jit_allowed && verified.email_verified => {
            // JIT is the ORG's explicit opt-in (deny-by-default at the
            // provider level) and rides the verified-email claim.
            return provision_jit_then_issue(state, ctx, verified, tx).await;
        }
        None => {
            audit_best_effort(
                state,
                ctx.org_id,
                core_domain::entities::Actor::System,
                "auth.federated_login_failed",
                None,
                None,
                serde_json::json!({
                    "reason": crate::auth::federation::denial_reasons::LINK_REQUIRED,
                    "source": ctx.display_source,
                }),
            )
            .await;
            return Err(ApiError::AuthDenied {
                reason: crate::auth::federation::denial_reasons::LINK_REQUIRED,
                message: "this identity is not linked to an account; ask an administrator",
            });
        }
    };
    core_db::federation::touch_link_login(
        &mut *tx,
        match ctx.namespace {
            LinkNamespace::Oidc(_) => crate::auth::federation::kind::OIDC,
            LinkNamespace::Bundle(kind, _) => kind,
        },
        ctx.namespace.source_id(),
        &verified.subject,
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    // Org MFA gate BEFORE tokens exist — identity source irrelevant.
    if let Some((mfa_token, methods)) = federated_mfa_gate(state, user_id, ctx.org_id).await? {
        audit_best_effort(
            state,
            ctx.org_id,
            core_domain::entities::Actor::Human(user_id),
            "auth.mfa_challenge_issued",
            Some("user"),
            Some(user_id.to_string()),
            serde_json::json!({ "methods": methods, "source": ctx.display_source }),
        )
        .await;
        return Ok(ProviderLoginOutcome::MfaRequired {
            mfa_required: true,
            mfa_token,
            methods,
        });
    }
    let tokens = crate::auth::handlers::issue_tokens(state, user_id, ctx.org_id, None).await?;
    audit_best_effort(
        state,
        ctx.org_id,
        core_domain::entities::Actor::Human(user_id),
        "auth.federated_login_success",
        Some("user"),
        Some(user_id.to_string()),
        serde_json::json!({ "source": ctx.display_source, "amr": verified.amr }),
    )
    .await;
    Ok(ProviderLoginOutcome::Tokens(tokens))
}

/// JIT falls through here: provision, audit, then the SAME issue path.
async fn provision_jit_then_issue(
    state: &AppState,
    ctx: &ProviderContext,
    verified: VerifiedPrincipal,
    tx: sqlx::Transaction<'static, sqlx::Postgres>,
) -> ApiResult<ProviderLoginOutcome> {
    let email = verified.email.trim().to_lowercase();
    let display_name = verified
        .display_name
        .clone()
        .unwrap_or_else(|| email.clone());
    let mut tx = tx;
    // The same bound as the OIDC callback's provisioning arm (Wave 58 lane
    // `p58-hashing`). This seam cannot answer a header, so a refusal here is
    // the typed 503 envelope through `hash_capacity_error`; the HTTP door that
    // calls it is what adds `Retry-After`.
    let permit = match state.config.auth_hash_gate().admit().await {
        Ok(permit) => permit,
        Err(refused) => return Err(crate::auth::handlers::hash_capacity_error(refused)),
    };
    let user_id =
        crate::routes::oidc::provision_jit_user(&mut tx, ctx.org_id, &email, &display_name, permit)
            .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    audit_best_effort(
        state,
        ctx.org_id,
        core_domain::entities::Actor::System,
        "auth.oidc_jit_provisioned",
        Some("user"),
        Some(user_id.to_string()),
        serde_json::json!({
            "source": ctx.display_source,
            "kind": crate::auth::federation::kind::OIDC,
        }),
    )
    .await;
    if let Some((mfa_token, methods)) = federated_mfa_gate(state, user_id, ctx.org_id).await? {
        return Ok(ProviderLoginOutcome::MfaRequired {
            mfa_required: true,
            mfa_token,
            methods,
        });
    }
    let tokens = crate::auth::handlers::issue_tokens(state, user_id, ctx.org_id, None).await?;
    audit_best_effort(
        state,
        ctx.org_id,
        core_domain::entities::Actor::Human(user_id),
        "auth.federated_login_success",
        Some("user"),
        Some(user_id.to_string()),
        serde_json::json!({ "source": ctx.display_source, "amr": verified.amr, "jit": true }),
    )
    .await;
    Ok(ProviderLoginOutcome::Tokens(tokens))
}

/// The contract DOES extend the DOMAIN event vocabulary: registration of
/// the published payload shapes lives in core-events (Wave 17 lanes did
/// the same). New event types this seam emits are AUDIT-only today — the
/// `auth.federated_*` audit actions above are the trail; nothing here
/// needs a `types::` constant yet.
pub mod contract {
    pub use super::{
        ProviderContext, ProviderLoginOutcome, VerifiedPrincipal, AUTH_PROVIDER_INTERFACE_VERSION,
    };

    /// Interface check: event types this lane's federation path emits and
    /// where they live. Audit-only by design — `contract.replay` fixtures
    /// remain untouched.
    pub mod audit_kinds {
        pub const FEDERATED_LOGIN_SUCCESS: &str = "auth.federated_login_success";
        pub const FEDERATED_LOGIN_FAILED: &str = "auth.federated_login_failed";
    }
}
