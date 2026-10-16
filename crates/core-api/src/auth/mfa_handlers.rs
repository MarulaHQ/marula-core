//! MFA HTTP handlers: TOTP enroll/verify/disable, recovery codes, and the
//! WebAuthn passkey ceremonies. Login-side completion of the `mfa_pending`
//! bridge also lives here (§6: TOTP + WebAuthn MFA).
//!
//! Security invariants enforced:
//! - TOTP secrets are AES-256-GCM encrypted at rest; plaintext exists only
//!   between generation and sealing, never returned after confirm.
//! - A verified code advances a per-user replay watermark; any code at or
//!   below it is rejected even inside the drift window.
//! - Recovery codes are argon2id-hashed, single-use IN SQL, revealed once.
//! - Disable/regenerate require password + a current second factor.

use crate::auth::handlers::{audit_best_effort, issue_tokens, issue_tokens_with_refresh_ttl};
use crate::auth::hardening::{enforce_gate, record_failure, record_success, GateSubject};
use crate::auth::{
    jwt, password, totp_service,
    webauthn_service::{self, CeremonyContext},
    AuthedUser,
};
use crate::client_ip::ClientIp;
use crate::error::{ApiError, ApiResult};
use crate::routes::openapi::TokenPair;
use crate::state::AppState;
use axum::{
    extract::{Extension, State},
    http::Extensions,
    Json,
};
use core_domain::entities::{Actor, AuditEvent};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

const RECOVERY_CODE_COUNT: usize = 8;
const CHALLENGE_TTL_SECS: i64 = 300;

type Tx = sqlx::Transaction<'static, sqlx::Postgres>;

/// Maps a webauthn_rs ceremony failure onto fixed human copy. The library
/// error text (challenge/attestation internals) is logged server-side
/// only — it never crosses the wire as 422 validation copy (Wave 13
/// lane-1 follow-up).
fn webauthn_rejection(e: crate::auth::webauthn_service::WebauthnError) -> ApiError {
    tracing::debug!(error = %e, "webauthn ceremony failed; fixed copy sent to client");
    ApiError::Validation(
        "the passkey could not be verified — try again, and make sure you're using the same passkey you registered".into(),
    )
}

fn ctx_from_config(state: &AppState) -> ApiResult<CeremonyContext> {
    CeremonyContext::new(
        &state.config.webauthn_rp_id,
        &state.config.webauthn_rp_origin,
    )
    .map_err(|e| ApiError::Internal(anyhow::anyhow!("{e}")))
}

async fn org_tx(state: &AppState, org_id: Uuid) -> ApiResult<Tx> {
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
// TOTP enrollment
// ---------------------------------------------------------------------------

/// POST /api/auth/mfa/totp/enroll-begin — generates a secret (returned ONCE)
/// and stores it sealed+unconfirmed. Re-calling restarts enrollment safely.
#[utoipa::path(operation_id = "mfa_totp_enroll_begin", post, path = "/api/auth/mfa/totp/enroll-begin",
    tag = "auth", security(("bearer" = [])),
    responses((status = 200, body = TotpEnrollBeginResponse)))]
pub async fn totp_enroll_begin(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<TotpEnrollBeginResponse>> {
    let authed = crate::auth::middleware::authed::<AuthedUser>(&extensions)?;
    let user = &authed.principal.user;

    let enrollment =
        totp_service::new_enrollment(&user.email, "Marula").map_err(ApiError::Validation)?;
    let sealed = crate::crypto::seal(
        &state.config.secret_key,
        enrollment.secret_base32.as_bytes(),
    )
    .map_err(|e| ApiError::Internal(anyhow::anyhow!("{e}")))?;

    let mut tx = org_tx(&state, user.org_id).await?;
    core_db::mfa::upsert_pending_totp(&mut *tx, Uuid::now_v7(), user.org_id, user.id, &sealed)
        .await?;
    core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id: user.org_id,
            actor: Actor::Human(user.id),
            action: "auth.mfa_totp_enroll_started".into(),
            entity_type: Some("user".into()),
            entity_id: Some(user.id.to_string()),
            detail: serde_json::json!({}),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    Ok(Json(TotpEnrollBeginResponse {
        secret_base32: enrollment.secret_base32,
        otpauth_url: enrollment.otpauth_url,
    }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TotpEnrollBeginResponse {
    /// Base32 secret for manual authenticator entry. Shown exactly once.
    pub secret_base32: String,
    /// Standard otpauth:// URI (renderable as QR by clients).
    pub otpauth_url: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TotpEnrollVerifyRequest {
    pub code: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecoveryCodesResponse {
    /// Plaintext codes — displayed once, stored only as argon2id hashes.
    pub recovery_codes: Vec<String>,
}

/// POST /api/auth/mfa/totp/enroll-verify — confirms the enrollment with the
/// first valid code and issues the one-time recovery-code batch.
#[utoipa::path(operation_id = "mfa_totp_enroll_verify", post, path = "/api/auth/mfa/totp/enroll-verify",
    tag = "auth", security(("bearer" = [])),
    request_body = TotpEnrollVerifyRequest,
    responses((status = 200, body = RecoveryCodesResponse), (status = 401)))]
pub async fn totp_enroll_verify(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<TotpEnrollVerifyRequest>,
) -> ApiResult<Json<RecoveryCodesResponse>> {
    let authed = crate::auth::middleware::authed::<AuthedUser>(&extensions)?;
    let user = &authed.principal.user;

    let mut tx = org_tx(&state, user.org_id).await?;
    let row = core_db::mfa::find_totp(&mut *tx, user.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if row.confirmed {
        return Err(ApiError::Conflict("totp already enabled".into()));
    }
    let b32 = String::from_utf8(
        crate::crypto::open(&state.config.secret_key, &row.secret_enc)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("{e}")))?,
    )
    .map_err(|_| ApiError::Internal(anyhow::anyhow!("stored secret not utf8")))?;

    let matched = totp_service::verify_code(
        &b32,
        &user.email,
        &req.code,
        chrono::Utc::now().timestamp().unsigned_abs(),
    )
    .map_err(ApiError::Validation)?
    .ok_or_else(|| ApiError::Unauthorized("invalid code".into()))?;

    let confirmed = core_db::mfa::confirm_totp(&mut *tx, user.id, matched).await?;
    if confirmed == 0 {
        // Lost a race (another tab confirmed); treat idempotently.
        return Err(ApiError::Conflict("totp already enabled".into()));
    }
    let codes = generate_recovery_codes(&mut tx, user.org_id, user.id).await?;
    audit_in_tx(
        &mut tx,
        user.org_id,
        user.id,
        "auth.mfa_totp_enabled",
        serde_json::json!({}),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    Ok(Json(RecoveryCodesResponse {
        recovery_codes: codes,
    }))
}

async fn audit_in_tx(
    tx: &mut Tx,
    org_id: Uuid,
    actor_user_id: Uuid,
    action: &str,
    detail: serde_json::Value,
) -> ApiResult<()> {
    core_db::audit::insert(
        &mut **tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(actor_user_id),
            action: action.into(),
            entity_type: Some("user".into()),
            entity_id: Some(actor_user_id.to_string()),
            detail,
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    Ok(())
}

/// Generates + persists a fresh hashed batch; returns PLAINTEXT once.
async fn generate_recovery_codes(
    tx: &mut Tx,
    org_id: Uuid,
    user_id: Uuid,
) -> ApiResult<Vec<String>> {
    use rand::Rng;
    const ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
    let mut plain = Vec::with_capacity(RECOVERY_CODE_COUNT);
    let mut ids = Vec::with_capacity(RECOVERY_CODE_COUNT);
    for _ in 0..RECOVERY_CODE_COUNT {
        let raw: String = (0..10)
            .map(|_| ALPHABET[rand::rngs::OsRng.gen_range(0..ALPHABET.len())] as char)
            .collect();
        plain.push(format!("{}-{}", &raw[..5], &raw[5..]));
        ids.push(Uuid::now_v7());
    }
    // Hash the whole batch off the async runtime (Wave 49 lane R3): ten
    // argon2id hashes on a worker thread is the worst single-request block
    // in the app.
    let hashes = password::hash_many_async(plain.clone())
        .await
        .map_err(ApiError::Internal)?;
    core_db::mfa::replace_recovery_codes(&mut **tx, org_id, user_id, &hashes, &ids).await?;
    Ok(plain)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TotpDisableRequest {
    pub password: String,
    /// Current TOTP code or a recovery code.
    pub code: String,
}

/// POST /api/auth/mfa/totp/disable — requires password AND a live factor;
/// wipes TOTP enrollment and all recovery codes.
#[utoipa::path(operation_id = "mfa_totp_disable", post, path = "/api/auth/mfa/totp/disable",
    tag = "auth", security(("bearer" = [])),
    request_body = TotpDisableRequest, responses((status = 204), (status = 401)))]
pub async fn totp_disable(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<TotpDisableRequest>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = crate::auth::middleware::authed::<AuthedUser>(&extensions)?;
    let user = &authed.principal.user;

    let mut tx = org_tx(&state, user.org_id).await?;
    let creds = core_db::users::find_credentials_by_email(&mut *tx, &user.email).await?;
    if !password::verify_async(req.password.clone(), creds.password_hash.clone()).await {
        return Err(ApiError::Unauthorized("invalid credentials".into()));
    }

    verify_any_second_factor(
        &state,
        &mut tx,
        user.org_id,
        user.id,
        user.email.clone(),
        &req.code,
    )
    .await?;

    core_db::mfa::delete_totp(&mut *tx, user.id).await?;
    core_db::mfa::replace_recovery_codes(&mut *tx, user.org_id, user.id, &[], &[]).await?;
    audit_in_tx(
        &mut tx,
        user.org_id,
        user.id,
        "auth.mfa_totp_disabled",
        serde_json::json!({}),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegenerateRecoveryRequest {
    pub password: String,
    pub code: String,
}

/// POST /api/auth/mfa/recovery-codes/regenerate — invalidates all previous
/// codes, issues a new batch (revealed once).
#[utoipa::path(operation_id = "mfa_recovery_regenerate", post, path = "/api/auth/mfa/recovery-codes/regenerate",
    tag = "auth", security(("bearer" = [])),
    request_body = RegenerateRecoveryRequest,
    responses((status = 200, body = RecoveryCodesResponse), (status = 401)))]
pub async fn regenerate_recovery_codes(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<RegenerateRecoveryRequest>,
) -> ApiResult<Json<RecoveryCodesResponse>> {
    let authed = crate::auth::middleware::authed::<AuthedUser>(&extensions)?;
    let user = &authed.principal.user;

    let mut tx = org_tx(&state, user.org_id).await?;
    let creds = core_db::users::find_credentials_by_email(&mut *tx, &user.email).await?;
    if !password::verify_async(req.password.clone(), creds.password_hash.clone()).await {
        return Err(ApiError::Unauthorized("invalid credentials".into()));
    }
    verify_any_second_factor(
        &state,
        &mut tx,
        user.org_id,
        user.id,
        user.email.clone(),
        &req.code,
    )
    .await?;

    let codes = generate_recovery_codes(&mut tx, user.org_id, user.id).await?;
    audit_in_tx(
        &mut tx,
        user.org_id,
        user.id,
        "auth.mfa_recovery_regenerated",
        serde_json::json!({ "count": codes.len() }),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(RecoveryCodesResponse {
        recovery_codes: codes,
    }))
}

// ---------------------------------------------------------------------------
// Login-side completion of the mfa_pending bridge
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct MfaVerifyRequest {
    /// The `mfa_token` from POST /api/auth/login when mfa_required was set.
    pub mfa_token: String,
    /// TOTP code or recovery code.
    pub code: String,
    /// "Trust this computer": keep this browser signed in for
    /// `TRUSTED_DEVICE_TTL_SECS` instead of the default refresh window.
    ///
    /// `#[serde(default)]` so every existing client keeps working unchanged —
    /// an absent field means "do not trust", which is the conservative answer
    /// and the one a caller that never heard of the option should get.
    #[serde(default)]
    pub trust_device: bool,
}

/// POST /api/auth/mfa/verify — completes an MFA-gated login with a TOTP or
/// recovery code. Tries TOTP first (advancing the replay watermark), then
/// falls back to single-use recovery codes.
#[utoipa::path(operation_id = "mfa_verify", post, path = "/api/auth/mfa/verify", tag = "auth",
    request_body = MfaVerifyRequest,
    responses((status = 200, body = TokenPair), (status = 401)))]
pub async fn mfa_verify(
    State(state): State<AppState>,
    Extension(ClientIp(client_ip)): Extension<ClientIp>,
    Json(req): Json<MfaVerifyRequest>,
) -> ApiResult<axum::response::Response> {
    let (user_id, org_id) = jwt::verify_mfa_pending_token(&state.config, &req.mfa_token)
        .ok_or_else(|| ApiError::Unauthorized("mfa session expired".into()))?;
    // "Trust this computer" only ever LENGTHENS the refresh window. It is
    // resolved once, here, so both success paths below (TOTP and recovery code)
    // honour the same choice and neither can drift.
    let refresh_ttl_secs = if req.trust_device {
        crate::auth::handlers::TRUSTED_DEVICE_TTL_SECS
    } else {
        state.config.refresh_ttl_secs
    };

    // Durable throttles keyed on the bridge identity + IP (typed 429 —
    // the caller proved possession of a live mfa_pending bridge, so the
    // code reveals nothing further). Defensive: the 6-digit TOTP
    // keyspace must not be brute-forceable online, so this GATE is the
    // aggressive one.
    enforce_gate(
        &state,
        crate::auth::hardening::kinds::MFA_VERIFY,
        &client_ip,
        GateSubject::User(user_id),
        false,
    )
    .await?;

    let mut tx = org_tx(&state, org_id).await?;
    let user = core_db::users::get_user(&mut *tx, org_id, user_id).await?;

    match try_totp(
        &state,
        &mut tx,
        org_id,
        user_id,
        user.email.clone(),
        &req.code,
    )
    .await?
    {
        Some(step) => {
            finish_mfa_login(tx, &state, org_id, user_id, "totp", step, refresh_ttl_secs).await
        }
        None => match match_recovery_code(&mut tx, user_id, req.code.trim()).await? {
            Some(code_id) => {
                // Single-use is enforced atomically in SQL.
                let consumed = core_db::mfa::consume_recovery_code(&mut *tx, code_id).await?;
                if consumed == 0 {
                    audit_mfa_failure(&state, org_id, user_id, "replay").await;
                    return Err(ApiError::Unauthorized("invalid code".into()));
                }
                audit_in_tx(
                    &mut tx,
                    org_id,
                    user_id,
                    "auth.mfa_login_success",
                    serde_json::json!({ "method": "recovery_code" }),
                )
                .await?;
                tx.commit()
                    .await
                    .map_err(|e| ApiError::Internal(e.into()))?;
                let _ = record_success(
                    &state,
                    crate::auth::hardening::kinds::MFA_VERIFY,
                    &client_ip,
                    GateSubject::User(user_id),
                )
                .await;
                let tokens =
                    issue_tokens_with_refresh_ttl(&state, user_id, org_id, None, refresh_ttl_secs)
                        .await?;
                Ok(mfa_tokens_response(&state, tokens))
            }
            None => {
                audit_mfa_failure(&state, org_id, user_id, "bad_code").await;
                let _ = record_failure(
                    &state,
                    crate::auth::hardening::kinds::MFA_VERIFY,
                    &client_ip,
                    GateSubject::User(user_id),
                )
                .await;
                Err(ApiError::Unauthorized("invalid code".into()))
            }
        },
    }
}

async fn finish_mfa_login(
    tx: Tx,
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    method: &str,
    matched_step: i64,
    refresh_ttl_secs: i64,
) -> ApiResult<axum::response::Response> {
    let mut tx = tx;
    audit_in_tx(
        &mut tx,
        org_id,
        user_id,
        "auth.mfa_login_success",
        serde_json::json!({ "method": method, "step": matched_step }),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let tokens =
        issue_tokens_with_refresh_ttl(state, user_id, org_id, None, refresh_ttl_secs).await?;
    Ok(mfa_tokens_response(state, tokens))
}

/// Token-pair responses after an MFA finish ride the same cookie trio as
/// login (threat model §A2 — one session cookie, one csrf, same flags;
/// `headers::Mark` for the memory-only access token stays in the client).
fn mfa_tokens_response(state: &AppState, tokens: TokenPair) -> axum::response::Response {
    crate::auth::handlers::mfa_cookie_response(state, tokens)
}

async fn match_recovery_code(
    tx: &mut Tx,
    user_id: Uuid,
    presented: &str,
) -> ApiResult<Option<Uuid>> {
    let candidates = core_db::mfa::unused_recovery_hashes(&mut **tx, user_id).await?;
    for (code_id, hash) in candidates {
        // Each candidate is a full argon2 verification. Run them off the
        // async runtime (Wave 49 lane R3); the count is bounded by the
        // number of unused recovery codes.
        if password::verify_async(presented.to_string(), hash).await {
            return Ok(Some(code_id));
        }
    }
    Ok(None)
}

/// Attempts TOTP verification; on match advances the watermark (0 rows ⇒
/// replay ⇒ rejected outright). Returns the matched step when verified.
async fn try_totp(
    state: &AppState,
    tx: &mut Tx,
    org_id: Uuid,
    user_id: Uuid,
    email: String,
    code: &str,
) -> ApiResult<Option<i64>> {
    let row = match core_db::mfa::find_totp(&mut **tx, user_id).await? {
        Some(row) if row.confirmed => row,
        _ => return Ok(None),
    };
    let b32 = String::from_utf8(
        crate::crypto::open(&state.config.secret_key, &row.secret_enc)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("{e}")))?,
    )
    .map_err(|_| ApiError::Internal(anyhow::anyhow!("stored secret not utf8")))?;

    let matched = totp_service::verify_code(
        &b32,
        &email,
        code,
        chrono::Utc::now().timestamp().unsigned_abs(),
    )
    .map_err(ApiError::Validation)?;

    match matched {
        Some(step) => {
            let advanced = core_db::mfa::consume_totp_step(&mut **tx, user_id, step).await?;
            if advanced == 0 {
                // Replay within/across windows — reject even though the
                // code itself is cryptographically valid right now.
                audit_mfa_failure(state, org_id, user_id, "totp_replay").await;
                Err(ApiError::Unauthorized("invalid code".into()))
            } else {
                Ok(Some(step))
            }
        }
        None => Ok(None),
    }
}

async fn audit_mfa_failure(state: &AppState, org_id: Uuid, user_id: Uuid, reason: &str) {
    audit_best_effort(
        state,
        org_id,
        Actor::Human(user_id),
        "auth.mfa_verify_failed",
        Some("user"),
        Some(user_id.to_string()),
        serde_json::json!({ "reason": reason }),
    )
    .await;
}

/// Verifies EITHER factor for sensitive operations (disable/regenerate).
/// A recovery code used here is CONSUMED, mirroring login semantics.
async fn verify_any_second_factor(
    state: &AppState,
    tx: &mut Tx,
    org_id: Uuid,
    user_id: Uuid,
    email: String,
    code: &str,
) -> ApiResult<()> {
    if try_totp(state, tx, org_id, user_id, email, code)
        .await?
        .is_some()
    {
        return Ok(());
    }
    match match_recovery_code(tx, user_id, code).await? {
        Some(id) if core_db::mfa::consume_recovery_code(&mut **tx, id).await? > 0 => Ok(()),
        _ => Err(ApiError::Unauthorized("invalid code".into())),
    }
}

// ---------------------------------------------------------------------------
// WebAuthn / passkeys
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct WebauthnBeginResponse {
    /// CreationOptions for navigator.credentials.create()/get().
    #[schema(value_type = Object)]
    pub public_key: serde_json::Value,
    /// Opaque ceremony binding — pass back to finish.
    pub challenge_token: String,
}

/// POST /api/auth/mfa/webauthn/register/begin — starts a passkey
/// registration for the authenticated user.
#[utoipa::path(operation_id = "mfa_webauthn_register_begin", post,
    path = "/api/auth/mfa/webauthn/register/begin", tag = "auth", security(("bearer" = [])),
    responses((status = 200, body = WebauthnBeginResponse)))]
pub async fn webauthn_register_begin(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<WebauthnBeginResponse>> {
    let authed = crate::auth::middleware::authed::<AuthedUser>(&extensions)?;
    let user = &authed.principal.user;
    let ctx = ctx_from_config(&state)?;

    let mut tx = org_tx(&state, user.org_id).await?;
    let existing = core_db::mfa::list_webauthn_credentials(&mut *tx, user.id).await?;
    let exclude: Vec<String> = existing.iter().map(|c| c.cred_id.clone()).collect();

    let (challenge, reg_state) = ctx
        .start_registration(user.id, &user.email, &user.display_name, exclude)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("{e}")))?;

    let challenge_id = Uuid::now_v7();
    let expires = chrono::Utc::now() + chrono::Duration::seconds(CHALLENGE_TTL_SECS);
    core_db::mfa::insert_challenge(
        &mut *tx,
        challenge_id,
        user.org_id,
        user.id,
        "registration",
        &serde_json::to_value(&reg_state)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("{e}")))?,
        expires,
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let challenge_token = jwt::issue_webauthn_challenge_token(
        &state.config,
        user.id,
        user.org_id,
        challenge_id,
        CHALLENGE_TTL_SECS,
    )
    .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(WebauthnBeginResponse {
        public_key: serde_json::to_value(&challenge)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("{e}")))?,
        challenge_token,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct WebauthnRegisterFinishRequest {
    pub challenge_token: String,
    /// Label shown on the security settings page.
    pub label: Option<String>,
    /// The PublicKeyCredential from navigator.credentials.create().
    #[schema(value_type = Object)]
    pub credential: serde_json::Value,
}

/// POST /api/auth/mfa/webauthn/register/finish — verifies the attestation
/// and stores the passkey. The public key is stored; no secret exists.
#[utoipa::path(operation_id = "mfa_webauthn_register_finish", post,
    path = "/api/auth/mfa/webauthn/register/finish", tag = "auth", security(("bearer" = [])),
    request_body = WebauthnRegisterFinishRequest, responses((status = 204)))]
pub async fn webauthn_register_finish(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<WebauthnRegisterFinishRequest>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = crate::auth::middleware::authed::<AuthedUser>(&extensions)?;
    let user = &authed.principal.user;
    let ctx = ctx_from_config(&state)?;
    let (_, _, challenge_id) =
        jwt::verify_webauthn_challenge_token(&state.config, &req.challenge_token)
            .ok_or_else(|| ApiError::Unauthorized("invalid or expired ceremony".into()))?;

    let mut tx = org_tx(&state, user.org_id).await?;
    let state_json = core_db::mfa::take_challenge(&mut *tx, challenge_id, user.id, "registration")
        .await?
        .ok_or_else(|| ApiError::Unauthorized("invalid or expired ceremony".into()))?;
    let reg_state: webauthn_rs::prelude::PasskeyRegistration =
        serde_json::from_value(state_json)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("ceremony state corrupt: {e}")))?;
    let response: webauthn_rs::prelude::RegisterPublicKeyCredential =
        serde_json::from_value(req.credential)
            .map_err(|_| ApiError::Validation("malformed credential".into()))?;

    let passkey = ctx
        .finish_registration(&response, &reg_state)
        .map_err(webauthn_rejection)?;

    let label = req.label.unwrap_or_else(|| "Passkey".to_string());
    let cred_id_b64 = data_encoding::BASE64URL_NOPAD.encode(passkey.cred_id());
    core_db::mfa::insert_webauthn_credential(
        &mut *tx,
        Uuid::now_v7(),
        user.org_id,
        user.id,
        &cred_id_b64,
        &webauthn_service::passkey_to_json(&passkey),
        &label,
    )
    .await?;
    audit_in_tx(
        &mut tx,
        user.org_id,
        user.id,
        "auth.mfa_webauthn_credential_added",
        serde_json::json!({ "label": label }),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WebauthnCredentialInfo {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    pub label: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// GET /api/auth/mfa/webauthn/credentials — list registered passkeys.
#[utoipa::path(operation_id = "mfa_webauthn_list", get,
    path = "/api/auth/mfa/webauthn/credentials", tag = "auth", security(("bearer" = [])),
    responses((status = 200, body = [WebauthnCredentialInfo])))]
pub async fn webauthn_credentials_list(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<WebauthnCredentialInfo>>> {
    let authed = crate::auth::middleware::authed::<AuthedUser>(&extensions)?;
    let mut tx = org_tx(&state, authed.principal.user.org_id).await?;
    let creds = core_db::mfa::list_webauthn_credentials(&mut *tx, authed.principal.user.id).await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(
        creds
            .into_iter()
            .map(|c| WebauthnCredentialInfo {
                id: c.id,
                label: c.label,
                created_at: c.created_at,
                last_used_at: c.last_used_at,
            })
            .collect(),
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct WebauthnRemoveRequest {
    pub password: String,
}

/// POST /api/auth/mfa/webauthn/credentials/{id}/remove — password-gated
/// removal of one passkey.
#[utoipa::path(operation_id = "mfa_webauthn_remove", post,
    path = "/api/auth/mfa/webauthn/credentials/{id}/remove", tag = "auth",
    security(("bearer" = [])), params(("id" = uuid::Uuid, Path)),
    request_body = WebauthnRemoveRequest, responses((status = 204)))]
pub async fn webauthn_credential_remove(
    State(state): State<AppState>,
    extensions: Extensions,
    axum::extract::Path(cred_row_id): axum::extract::Path<Uuid>,
    Json(req): Json<WebauthnRemoveRequest>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = crate::auth::middleware::authed::<AuthedUser>(&extensions)?;
    let user = &authed.principal.user;
    let mut tx = org_tx(&state, user.org_id).await?;
    let creds = core_db::users::find_credentials_by_email(&mut *tx, &user.email).await?;
    if !password::verify_async(req.password.clone(), creds.password_hash.clone()).await {
        return Err(ApiError::Unauthorized("invalid credentials".into()));
    }
    let deleted = core_db::mfa::delete_webauthn_credential(&mut *tx, user.id, cred_row_id).await?;
    if deleted == 0 {
        return Err(ApiError::NotFound);
    }
    audit_in_tx(
        &mut tx,
        user.org_id,
        user.id,
        "auth.mfa_webauthn_credential_removed",
        serde_json::json!({}),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// --- login-side WebAuthn (completes an mfa_pending bridge) ------------------

/// POST /api/auth/mfa/webauthn/begin — assertion request for the pending
/// MFA login. Requires a valid mfa_pending token.
#[utoipa::path(operation_id = "mfa_webauthn_login_begin", post,
    path = "/api/auth/mfa/webauthn/begin", tag = "auth",
    request_body = MfaTokenRequest,
    responses((status = 200, body = WebauthnBeginResponse), (status = 401)))]
pub async fn webauthn_login_begin(
    State(state): State<AppState>,
    Extension(ClientIp(client_ip)): Extension<ClientIp>,
    Json(req): Json<MfaTokenRequest>,
) -> ApiResult<Json<WebauthnBeginResponse>> {
    let (user_id, org_id) = jwt::verify_mfa_pending_token(&state.config, &req.mfa_token)
        .ok_or_else(|| ApiError::Unauthorized("mfa session expired".into()))?;
    // Typed 429 (proof-of-possession already established via the bridge).
    enforce_gate(
        &state,
        crate::auth::hardening::kinds::WEBAUTHN_BEGIN,
        &client_ip,
        GateSubject::User(user_id),
        false,
    )
    .await?;
    let ctx = ctx_from_config(&state)?;

    let mut tx = org_tx(&state, org_id).await?;
    let rows = core_db::mfa::list_webauthn_credentials(&mut *tx, user_id).await?;
    let passkeys = webauthn_service::passkeys_from_json(
        &rows
            .iter()
            .map(|r| r.credential.clone())
            .collect::<Vec<_>>(),
    );
    if passkeys.is_empty() {
        return Err(ApiError::NotFound);
    }
    let (challenge, auth_state) = ctx
        .start_authentication(&passkeys)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("{e}")))?;

    let challenge_id = Uuid::now_v7();
    let expires = chrono::Utc::now() + chrono::Duration::seconds(CHALLENGE_TTL_SECS);
    core_db::mfa::insert_challenge(
        &mut *tx,
        challenge_id,
        org_id,
        user_id,
        "authentication",
        &serde_json::to_value(&auth_state)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("{e}")))?,
        expires,
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let challenge_token = jwt::issue_webauthn_challenge_token(
        &state.config,
        user_id,
        org_id,
        challenge_id,
        CHALLENGE_TTL_SECS,
    )
    .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(WebauthnBeginResponse {
        public_key: serde_json::to_value(&challenge)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("{e}")))?,
        challenge_token,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MfaTokenRequest {
    pub mfa_token: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct WebauthnLoginFinishRequest {
    pub challenge_token: String,
    /// The PublicKeyCredential from navigator.credentials.get().
    #[schema(value_type = Object)]
    pub credential: serde_json::Value,
}

/// POST /api/auth/mfa/webauthn/finish — verifies the assertion and issues
/// full tokens. Ceremony state is single-use; replay of a finish call fails.
#[utoipa::path(operation_id = "mfa_webauthn_login_finish", post,
    path = "/api/auth/mfa/webauthn/finish", tag = "auth",
    request_body = WebauthnLoginFinishRequest,
    responses((status = 200, body = TokenPair), (status = 401)))]
pub async fn webauthn_login_finish(
    State(state): State<AppState>,
    Extension(ClientIp(client_ip)): Extension<ClientIp>,
    Json(req): Json<WebauthnLoginFinishRequest>,
) -> ApiResult<axum::response::Response> {
    let (user_id, org_id, challenge_id) =
        jwt::verify_webauthn_challenge_token(&state.config, &req.challenge_token)
            .ok_or_else(|| ApiError::Unauthorized("invalid or expired ceremony".into()))?;
    enforce_gate(
        &state,
        crate::auth::hardening::kinds::WEBAUTHN_LOGIN,
        &client_ip,
        GateSubject::User(user_id),
        false,
    )
    .await?;
    let ctx = ctx_from_config(&state)?;

    let mut tx = org_tx(&state, org_id).await?;
    // Existence + activity of the user is already guaranteed by the bridge
    // that issued the pending token; no extra load needed here.
    let state_json =
        core_db::mfa::take_challenge(&mut *tx, challenge_id, user_id, "authentication")
            .await?
            .ok_or_else(|| ApiError::Unauthorized("invalid or expired ceremony".into()))?;
    let auth_state: webauthn_rs::prelude::PasskeyAuthentication =
        serde_json::from_value(state_json)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("ceremony state corrupt: {e}")))?;
    let response: webauthn_rs::prelude::PublicKeyCredential =
        serde_json::from_value(req.credential)
            .map_err(|_| ApiError::Validation("malformed assertion".into()))?;

    let result = match ctx.finish_authentication(&response, &auth_state) {
        Ok(result) => result,
        Err(e) => {
            let _ignored = record_failure(
                &state,
                crate::auth::hardening::kinds::WEBAUTHN_LOGIN,
                &client_ip,
                GateSubject::User(user_id),
            )
            .await;
            return Err(webauthn_rejection(e));
        }
    };
    let used_cred_id = data_encoding::BASE64URL_NOPAD.encode(result.cred_id());

    core_db::mfa::touch_webauthn_credential(&mut *tx, &used_cred_id).await?;
    audit_in_tx(
        &mut tx,
        org_id,
        user_id,
        "auth.mfa_login_success",
        serde_json::json!({ "method": "webauthn", "cred_id": used_cred_id }),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let _ = record_success(
        &state,
        crate::auth::hardening::kinds::WEBAUTHN_LOGIN,
        &client_ip,
        GateSubject::User(user_id),
    )
    .await;
    let tokens = issue_tokens(&state, user_id, org_id, None).await?;
    Ok(mfa_tokens_response(&state, tokens))
}
