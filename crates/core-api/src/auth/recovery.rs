//! Auth recovery flows (Wave 19 lane 5, Scope B): password reset,
//! email verification, and magic-link sign-in.
//!
//! Shared doctrine:
//! * Every REQUEST surface answers UNIFORM success (`{"ok": true}`)
//!   whether or not the mailbox exists — no enumeration (the audit row
//!   records the truth server-side only).
//! * All tokens are single-use, sha256-HASHED at rest, expiring, and
//!   consumed atomically by the `app_consume_one_use_token` bridge
//!   (migration 0088) — two concurrent redemptions give exactly one
//!   winner.
//! * Every one of these surfaces rides the durable rate gates; the
//!   request paths throttle aggressively so one mailbox cannot be
//!   mail-bombed.
//! * Mail issuance flows through the core-mail outbox (`Templated`
//!   kind); the plain `none` dev transport keeps mail queued (the
//!   documented dev posture — nothing silently dropped).
//! * Magic-link sign-in issues sessions through `handlers::issue_tokens`
//!   — the SAME machinery as password login. PORTAL IS NOT INCLUDED
//!   (portal_users is a different principal family with its own
//!   middleware; the threat model states this explicitly).

use crate::auth::hardening::{enforce_gate, GateSubject};
use crate::auth::{cookies, handlers, jwt, password};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{extract::State, http::HeaderMap, Json};
use core_db::hardening as tokens;
use core_domain::entities::{Actor, AuditEvent};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

const RESET_TTL_SECS: i64 = 30 * 60;
const VERIFY_TTL_SECS: i64 = 24 * 60 * 60;
const MAGIC_TTL_SECS: i64 = 15 * 60;

#[derive(Debug, Deserialize, ToSchema)]
pub struct PasswordResetRequest {
    pub email: String,
}

/// The uniform shape every request surface returns.
#[derive(Debug, Serialize, ToSchema)]
pub struct UniformOk {
    pub ok: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PasswordResetConfirm {
    pub token: String,
    pub new_password: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct EmailVerifyRequest {
    pub email: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TokenRequest {
    pub token: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct VerifyStatus {
    pub email_verified: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MagicLinkConfirm {
    pub token: String,
}

fn link(config: &crate::config::Config, page: &str, raw: &str) -> String {
    let base = config.web_base_url.trim_end_matches('/');
    format!("{base}/{page}?token={raw}")
}

/// Issues a hashed single-use token, invalidates previous links of the
/// same kind, audits, and enqueues the mail. The raw value exists ONLY
/// inside the mail link — never persisted, never returned in a response.
struct MailTokenSpec {
    kind: &'static str,
    ttl_secs: i64,
    page: &'static str,
    audit_action: &'static str,
}

async fn issue_mail_token(
    state: &AppState,
    user_id: Uuid,
    org_id: Uuid,
    spec: MailTokenSpec,
    render: impl FnOnce(String) -> (String, String),
) -> ApiResult<()> {
    let MailTokenSpec {
        kind,
        ttl_secs,
        page,
        audit_action,
    } = spec;
    let raw = jwt::new_refresh_token();
    let token_hash = jwt::hash_refresh_token(&raw);
    let expires = chrono::Utc::now() + chrono::Duration::seconds(ttl_secs);

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, org_id).await?;
    tokens::invalidate_one_use_tokens(&mut *tx, user_id, kind).await?;
    tokens::put_one_use_token(
        &mut *tx,
        tokens::PutToken {
            id: Uuid::now_v7(),
            org_id,
            user_id,
            kind,
            token_hash: &token_hash,
            expires_at: expires,
        },
    )
    .await?;
    core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::System,
            action: audit_action.into(),
            entity_type: Some("user".into()),
            entity_id: Some(user_id.to_string()),
            detail: serde_json::json!({ "kind": kind }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    // The email address rides the same narrow resolution the requester
    // flows through; the write tx carries the org scope for the mail
    // ledger row the same way inbound/outbound senders do.
    let mut mail_tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut mail_tx, org_id).await?;
    let address: Option<String> = sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&mut *mail_tx)
        .await
        .map_err(ApiError::from)?
        .flatten();
    if let Some(address) = address {
        let (subject, body) = render(link(&state.config, page, &raw));
        crate::email::enqueue_outbound(
            &mut mail_tx,
            crate::email::OutboundEnqueue {
                mail: &state.mail,
                org_id,
                to_addresses: vec![address],
                kind: &core_mail::outbound::OutboundKind::Templated { subject, body },
                in_reply_to: None,
                references: vec![],
                ticket_id: None,
            },
        )
        .await?;
        mail_tx
            .commit()
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
    } else {
        mail_tx.rollback().await.ok();
    }
    Ok(())
}

fn uniform_ok() -> ApiResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// Password reset
// ---------------------------------------------------------------------------

/// POST /api/auth/password-reset/request — issues a single-use token +
/// mail when the account exists; ALWAYS the uniform answer.
#[utoipa::path(operation_id = "auth_password_reset_request", post,
    path = "/api/auth/password-reset/request", tag = "auth",
    request_body = PasswordResetRequest, responses((status = 200, body = UniformOk),
    (status = 429, description = "flood throttle")))]
pub async fn password_reset_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PasswordResetRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    enforce_gate(
        &state,
        tokens::gate_kind::PASSWORD_RESET,
        &headers,
        GateSubject::Email(&req.email),
        false,
    )
    .await?;
    let resolved = tokens::resolve_auth_email(&state.pool, &req.email, false)
        .await
        .map_err(ApiError::from)?;
    if let Some((user_id, org_id, _)) = resolved {
        issue_mail_token(
            &state,
            user_id,
            org_id,
            MailTokenSpec {
                kind: tokens::gate_kind::PASSWORD_RESET,
                ttl_secs: RESET_TTL_SECS,
                page: "password-reset",
                audit_action: "auth.password_reset_requested",
            },
            |reset_link| {
                (
                    "Reset your Marula password".to_string(),
                    format!(
                        "A password reset was requested for your account.\n\n\
                     Open this single-use link (valid {RESET_TTL_SECS} minutes):\n{reset_link}\n\n\
                     If it wasn't you, ignore this mail."
                    ),
                )
            },
        )
        .await?;
    }
    uniform_ok()
}

/// POST /api/auth/password-reset/confirm — consumes the token, sets the
/// new password, and revokes ALL of the user's live sessions (the
/// reset's security payload: a stolen session cannot outlive it).
#[utoipa::path(operation_id = "auth_password_reset_confirm", post,
    path = "/api/auth/password-reset/confirm", tag = "auth",
    request_body = PasswordResetConfirm,
    responses((status = 200, body = UniformOk), (status = 401, description = "invalid or expired token"),
    (status = 422, description = "validation error"), (status = 429)))]
pub async fn password_reset_confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PasswordResetConfirm>,
) -> ApiResult<Json<serde_json::Value>> {
    enforce_gate(
        &state,
        tokens::gate_kind::PASSWORD_RESET,
        &headers,
        GateSubject::IpOnly,
        false,
    )
    .await?;
    handlers::validate_password(&req.new_password)?;
    let presented = jwt::hash_refresh_token(req.token.trim());
    let resolution =
        tokens::consume_one_use_token(&state.pool, tokens::gate_kind::PASSWORD_RESET, &presented)
            .await
            .map_err(ApiError::from)?
            .ok_or_else(|| ApiError::Unauthorized("invalid or expired reset link".into()))?;

    let new_hash = password::hash(&req.new_password)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("argon2: {e}")))?;
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, resolution.org_id).await?;
    core_db::users::set_password_hash(&mut *tx, resolution.user_id, &new_hash).await?;
    let revoked = core_db::users::revoke_all_user_sessions(&mut *tx, resolution.user_id).await?;
    core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id: resolution.org_id,
            actor: Actor::Human(resolution.user_id),
            action: "auth.password_reset_completed".into(),
            entity_type: Some("user".into()),
            entity_id: Some(resolution.user_id.to_string()),
            detail: serde_json::json!({ "sessions_revoked": revoked }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    uniform_ok()
}

// ---------------------------------------------------------------------------
// Email verification
// ---------------------------------------------------------------------------

/// POST /api/auth/email-verification/request — resend (public, THROTTLED;
/// admin override rides POST /api/org/users/{id}/email-verified).
#[utoipa::path(operation_id = "auth_email_verification_request", post,
    path = "/api/auth/email-verification/request", tag = "auth",
    request_body = EmailVerifyRequest, responses((status = 200, body = UniformOk), (status = 429)))]
pub async fn email_verification_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<EmailVerifyRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    enforce_gate(
        &state,
        tokens::gate_kind::EMAIL_VERIFY,
        &headers,
        GateSubject::Email(&req.email),
        false,
    )
    .await?;
    let resolved = tokens::resolve_auth_email(&state.pool, &req.email, false)
        .await
        .map_err(ApiError::from)?;
    if let Some((user_id, org_id, verified)) = resolved {
        if !verified {
            issue_mail_token(
                &state,
                user_id,
                org_id,
                MailTokenSpec {
                    kind: tokens::gate_kind::EMAIL_VERIFY,
                    ttl_secs: VERIFY_TTL_SECS,
                    page: "verify-email",
                    audit_action: "auth.email_verification_requested",
                },
                |verify_link| {
                    (
                        "Verify your Marula email".to_string(),
                        format!(
                            "Confirm your Marula account email.\n\n\
                         Open this single-use link (valid {VERIFY_TTL_SECS}):\n{verify_link}"
                        ),
                    )
                },
            )
            .await?;
        }
    }
    uniform_ok()
}

/// POST /api/auth/email-verification/confirm — marks the email verified
/// (single-use, audited).
#[utoipa::path(operation_id = "auth_email_verification_confirm", post,
    path = "/api/auth/email-verification/confirm", tag = "auth",
    request_body = TokenRequest, responses((status = 200, body = VerifyStatus),
    (status = 401, description = "invalid or expired token"), (status = 429)))]
pub async fn email_verification_confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<TokenRequest>,
) -> ApiResult<Json<VerifyStatus>> {
    enforce_gate(
        &state,
        tokens::gate_kind::EMAIL_VERIFY,
        &headers,
        GateSubject::IpOnly,
        false,
    )
    .await?;
    let presented = jwt::hash_refresh_token(req.token.trim());
    let resolution =
        tokens::consume_one_use_token(&state.pool, tokens::gate_kind::EMAIL_VERIFY, &presented)
            .await
            .map_err(ApiError::from)?
            .ok_or_else(|| ApiError::Unauthorized("invalid or expired verification link".into()))?;
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, resolution.org_id).await?;
    core_db::users::mark_email_verified(&mut *tx, resolution.user_id).await?;
    core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id: resolution.org_id,
            actor: Actor::Human(resolution.user_id),
            action: "auth.email_verified".into(),
            entity_type: Some("user".into()),
            entity_id: Some(resolution.user_id.to_string()),
            detail: serde_json::json!({ "via": "verification_link" }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(VerifyStatus {
        email_verified: true,
    }))
}

// ---------------------------------------------------------------------------
// Magic-link sign-in
// ---------------------------------------------------------------------------

/// POST /api/auth/magic-link/request — deny-by-default per org
/// (`orgs.magic_link_sign_in`, 0088): unknown emails AND disabled orgs
/// collapse into the SAME uniform answer.
#[utoipa::path(operation_id = "auth_magic_link_request", post,
    path = "/api/auth/magic-link/request", tag = "auth",
    request_body = EmailVerifyRequest, responses((status = 200, body = UniformOk), (status = 429)))]
pub async fn magic_link_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<EmailVerifyRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    enforce_gate(
        &state,
        tokens::gate_kind::MAGIC_LINK,
        &headers,
        GateSubject::Email(&req.email),
        false,
    )
    .await?;
    // magic_link=true folds the org opt-in INTO the SQL bridge: a
    // disabled org and an unknown mailbox are the same None.
    let resolved = tokens::resolve_auth_email(&state.pool, &req.email, true)
        .await
        .map_err(ApiError::from)?;
    if let Some((user_id, org_id, _)) = resolved {
        issue_mail_token(
            &state,
            user_id,
            org_id,
            MailTokenSpec {
                kind: tokens::gate_kind::MAGIC_LINK,
                ttl_secs: MAGIC_TTL_SECS,
                page: "magic-link",
                audit_action: "auth.magic_link_requested",
            },
            |signin_link| (
                "Your Marula sign-in link".to_string(),
                format!(
                    "Open this single-use link to sign in (valid {MAGIC_TTL_SECS}):\n{signin_link}\n\n\
                     If it wasn't you, ignore this mail."
                ),
            ),
        )
        .await?;
    }
    uniform_ok()
}

/// POST /api/auth/magic-link/confirm — consumes the link and issues a
/// full session through the SAME machinery as password login. STAFF
/// family only (the portal is NOT included — see the threat model).
#[utoipa::path(operation_id = "auth_magic_link_confirm", post,
    path = "/api/auth/magic-link/confirm", tag = "auth",
    request_body = MagicLinkConfirm, responses((status = 200, body = crate::routes::openapi::TokenPair),
    (status = 401, description = "invalid or expired token"), (status = 429)))]
pub async fn magic_link_confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<MagicLinkConfirm>,
) -> ApiResult<axum::response::Response> {
    if !cookies::double_submit_ok(&headers) {
        return Err(ApiError::Forbidden("csrf check failed".into()));
    }
    enforce_gate(
        &state,
        tokens::gate_kind::MAGIC_LINK,
        &headers,
        GateSubject::IpOnly,
        false,
    )
    .await?;
    let presented = jwt::hash_refresh_token(req.token.trim());
    let resolution =
        tokens::consume_one_use_token(&state.pool, tokens::gate_kind::MAGIC_LINK, &presented)
            .await
            .map_err(ApiError::from)?
            .ok_or_else(|| ApiError::Unauthorized("invalid or expired sign-in link".into()))?;
    let (tokens, cookie_head) =
        handlers::issue_tokens_with_cookies(&state, resolution.user_id, resolution.org_id, None)
            .await?;
    handlers::audit_best_effort(
        &state,
        resolution.org_id,
        Actor::Human(resolution.user_id),
        "auth.magic_link_login",
        Some("user"),
        Some(resolution.user_id.to_string()),
        serde_json::json!({ "amr": "magic_link" }),
    )
    .await;
    Ok(handlers::auth_json_response(tokens, cookie_head))
}
