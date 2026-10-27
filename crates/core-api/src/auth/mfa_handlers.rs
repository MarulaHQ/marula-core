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
//! - A destructive factor change (replacing a confirmed TOTP, removing a
//!   passkey) requires a FRESH step-up on this sign-in lineage: a factor
//!   verification recorded in `mfa_step_ups` within `STEP_UP_TTL_SECS`.
//!   A live TOTP code cannot authorize replacing the TOTP secret it belongs
//!   to, and a passkey cannot authorize removing itself.
//! - An administrator can reset another user's factors (`manage_users`),
//!   which clears them, revokes that user's sessions, and marks the account
//!   as owing a factor (`users.mfa_reenroll_required`) until one is enrolled.

use crate::auth::handlers::{audit_best_effort, issue_tokens, issue_tokens_with_refresh_ttl};
use crate::auth::hardening::{enforce_gate, record_failure, record_success, GateSubject};
use crate::auth::middleware::require_permission;
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
    extract::{Extension, Path, State},
    http::Extensions,
    response::IntoResponse,
    Json,
};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::policy::{BuiltinRole, Permission};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

const RECOVERY_CODE_COUNT: usize = 8;
const CHALLENGE_TTL_SECS: i64 = 300;

/// How long a step-up stays usable.
///
/// Five minutes: long enough to walk from "prove it" to the change, short
/// enough that a machine left signed in is not a standing authorization to
/// swap the account's second factor. The window is written into each
/// `mfa_step_ups` row at insert time, so the record explains itself later.
const STEP_UP_TTL_SECS: i64 = 300;

/// The call that establishes a step-up. Named in every typed refusal so a
/// client never has to guess which door to knock on.
const STEP_UP_ENDPOINT: &str = "/api/auth/mfa/step-up";

const ENROLL_ENDPOINT: &str = "/api/auth/mfa/totp/enroll-begin";

type Tx = sqlx::Transaction<'static, sqlx::Postgres>;

/// Builds the typed 428 a destructive factor change answers with when it has
/// no fresh step-up. `reason` is the machine code; `message` says what to do.
fn step_up_refusal(
    reason: &'static str,
    message: impl Into<String>,
    endpoint: &'static str,
) -> ApiError {
    ApiError::MfaStepUpRequired {
        reason,
        message: message.into(),
        endpoint,
        ttl_secs: STEP_UP_TTL_SECS,
    }
}

fn allowed_factor_label(allowed: &[&str]) -> String {
    let mut parts: Vec<&str> = Vec::with_capacity(allowed.len());
    for method in allowed {
        parts.push(match *method {
            "totp" => "a TOTP code",
            "recovery_code" => "a recovery code",
            "webauthn" => "a passkey sign-in",
            other => other,
        });
    }
    match parts.len() {
        0 => "a live factor".to_string(),
        1 => parts[0].to_string(),
        _ => {
            let last = parts.pop().unwrap_or("a live factor");
            format!("{} or {last}", parts.join(", "))
        }
    }
}

/// What proved a factor, carried into the audit row that records the act it
/// authorized (DECISIONS record 1 rule 3: name both halves).
struct SecondFactor {
    method: &'static str,
    factor_ref: Option<String>,
}

/// Requires a step-up recorded for THIS caller's refresh chain that has not
/// expired, proved by one of `allowed`.
///
/// The chain, not the session row, is the unit: a refresh rotation inserts a
/// new session row on the same chain, and a proof taken two minutes ago must
/// survive it. A new login starts a new chain, so no other session can lend
/// its proof to this one.
async fn require_fresh_step_up(
    tx: &mut Tx,
    authed: &AuthedUser,
    allowed: &[&str],
    purpose: &str,
) -> ApiResult<core_db::mfa::StepUpRow> {
    let user = &authed.principal.user;
    let Some(session_id) = authed.session_id else {
        return Err(step_up_refusal(
            "step_up_required",
            format!(
                "{purpose} needs a signed-in browser session — a machine credential cannot \
                 prove a human factor"
            ),
            STEP_UP_ENDPOINT,
        ));
    };
    match core_db::mfa::latest_step_up(&mut **tx, session_id, user.id).await? {
        None => Err(step_up_refusal(
            "step_up_required",
            format!("prove a live factor with POST {STEP_UP_ENDPOINT} before {purpose}"),
            STEP_UP_ENDPOINT,
        )),
        Some(row) if !row.fresh => Err(step_up_refusal(
            "step_up_stale",
            format!(
                "your last factor verification has expired; prove one again with \
                 POST {STEP_UP_ENDPOINT} before {purpose}"
            ),
            STEP_UP_ENDPOINT,
        )),
        Some(row) if !allowed.contains(&row.method.as_str()) => Err(step_up_refusal(
            "step_up_factor_not_allowed",
            format!(
                "{purpose} cannot be authorized by the factor you proved ({}); \
                 prove {} with POST {STEP_UP_ENDPOINT}",
                row.method,
                allowed_factor_label(allowed)
            ),
            STEP_UP_ENDPOINT,
        )),
        Some(row) => Ok(row),
    }
}

/// Refuses destructive factor changes while an admin reset has left the
/// account owing a factor. Enrolment itself is the way out and does not call
/// this.
async fn refuse_when_reenrolling(tx: &mut Tx, user_id: Uuid, purpose: &str) -> ApiResult<()> {
    if core_db::mfa::user_mfa_reenroll_required(&mut **tx, user_id).await? {
        return Err(step_up_refusal(
            "mfa_reenrollment_required",
            format!(
                "an administrator reset this account's factors; enrol a new one with \
                 POST {ENROLL_ENDPOINT} before {purpose}"
            ),
            ENROLL_ENDPOINT,
        ));
    }
    Ok(())
}

/// Stamps a step-up inside a transaction the caller already opened, for the
/// session this request authenticated with. Used by the destructive handlers
/// that just proved a factor, so the next change in the same visit does not
/// re-prompt.
async fn stamp_step_up(tx: &mut Tx, authed: &AuthedUser, factor: &SecondFactor) -> ApiResult<()> {
    let Some(session_id) = authed.session_id else {
        return Ok(());
    };
    let user = &authed.principal.user;
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(STEP_UP_TTL_SECS);
    core_db::mfa::record_step_up(
        &mut **tx,
        core_db::mfa::StepUpRecord {
            id: Uuid::now_v7(),
            org_id: user.org_id,
            user_id: user.id,
            session_id,
            method: factor.method,
            factor_ref: factor.factor_ref.as_deref(),
            expires_at,
        },
    )
    .await?;
    Ok(())
}

/// Records a step-up for a session this request just established (the MFA
/// login paths), in its own transaction.
///
/// Best-effort by contract: a login must not fail because the proof could not
/// be stamped — the caller falls back to proving a factor again.
async fn record_step_up_for_session(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    session_id: Uuid,
    factor: &SecondFactor,
) -> ApiResult<()> {
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(STEP_UP_TTL_SECS);
    let mut tx = org_tx(state, org_id).await?;
    core_db::mfa::record_step_up(
        &mut *tx,
        core_db::mfa::StepUpRecord {
            id: Uuid::now_v7(),
            org_id,
            user_id,
            session_id,
            method: factor.method,
            factor_ref: factor.factor_ref.as_deref(),
            expires_at,
        },
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(())
}

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
// Factor proofs: what has to be hashed, and the slot that bounds it
// ---------------------------------------------------------------------------
//
// A factor change proves two things — the account password and a live second
// factor — and both are argon2id work nobody has bounded for a signed-in
// caller. The shape below splits the request into three parts so the CPU work
// is not paid for with a pooled connection: read what the hashes need, close
// the transaction, hash under the process's one admission slot, then reopen a
// transaction for the effects the proof authorizes.

/// What a factor proof needs from the database, read through ONE short
/// transaction that is closed before any hash runs.
struct ProofMaterial {
    password_hash: String,
    /// The user's TOTP row, confirmed or pending. Encrypted: decryption is
    /// local work and needs no connection.
    totp: Option<core_db::mfa::TotpSecretRow>,
    /// Unused recovery codes as `(row id, argon2id hash)` — a candidate per
    /// unused code, one verify each.
    recovery: Vec<(Uuid, String)>,
}

/// Closes a read-only transaction, handing its connection back to the pool
/// before the caller does anything expensive with the rows it returned.
async fn end_read(tx: Tx) -> ApiResult<()> {
    tx.rollback()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(())
}

/// Reads everything a password-plus-factor proof hashes, and gives the
/// connection back before the first verify.
///
/// `purpose` is threaded into the re-enrolment refusal, so the typed answer a
/// reset account gets still names what it was trying to do.
async fn read_proof_material(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    email: &str,
    purpose: &str,
) -> ApiResult<ProofMaterial> {
    let mut tx = org_tx(state, org_id).await?;
    refuse_when_reenrolling(&mut tx, user_id, purpose).await?;
    let creds = core_db::users::find_credentials_by_email(&mut *tx, email).await?;
    let totp = core_db::mfa::find_totp(&mut *tx, user_id).await?;
    let recovery = core_db::mfa::unused_recovery_hashes(&mut *tx, user_id).await?;
    end_read(tx).await?;
    Ok(ProofMaterial {
        password_hash: creds.password_hash,
        totp,
        recovery,
    })
}

/// The password-only form of [`read_proof_material`], for the doors that prove
/// a factor some other way (a passkey removal proves a stored step-up).
async fn read_password_hash(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    email: &str,
    purpose: &str,
) -> ApiResult<String> {
    let mut tx = org_tx(state, org_id).await?;
    refuse_when_reenrolling(&mut tx, user_id, purpose).await?;
    let creds = core_db::users::find_credentials_by_email(&mut *tx, email).await?;
    end_read(tx).await?;
    Ok(creds.password_hash)
}

/// Which factor proved possession, established with no connection held and
/// spent later, inside the transaction that owns the change it authorizes.
enum FactorProof {
    Totp { step: i64 },
    RecoveryCode { id: Uuid },
}

/// Runs every hash a request owes — the password, then each unused recovery
/// code until one matches — and reports the factor that proved possession.
///
/// The admission slot is a PARAMETER rather than something this function
/// takes itself: the caller acquires it at the point of need, so a slow read
/// earlier in the handler never holds one hostage, and making it a parameter
/// is what stops a later edit from reaching argon2 without a slot (Wave 58
/// lane `p58-hashing`). This function owns the permit, so every return path
/// — including both `?` errors — hands the slot back before the caller's next
/// database call.
///
/// One slot covers the whole section. A request owes at most one password
/// verify plus one verify per unused recovery code, and those run in sequence:
/// the slot bounds how many requests hash at once, not how many hashes a
/// single honest request needs to try.
async fn prove_factor(
    state: &AppState,
    material: &ProofMaterial,
    password: &str,
    email: &str,
    code: &str,
    permit: tokio::sync::SemaphorePermit<'static>,
) -> ApiResult<FactorProof> {
    let proof =
        if !password::verify_async(password.to_string(), material.password_hash.clone()).await {
            Err(ApiError::Unauthorized("invalid credentials".into()))
        } else {
            // TOTP is an HMAC over a sealed secret, not argon2: it rides the slot
            // this request already holds and costs nothing worth bounding.
            match matching_totp_step(state, material.totp.as_ref(), email, code)? {
                Some(step) => Ok(FactorProof::Totp { step }),
                None => match match_recovery_hash(&material.recovery, code).await {
                    Some(id) => Ok(FactorProof::RecoveryCode { id }),
                    None => Err(ApiError::Unauthorized("invalid code".into())),
                },
            }
        };
    // The slot is for the hashes and nothing else: hand it back before the
    // caller opens the transaction that spends this proof.
    drop(permit);
    proof
}

/// The TOTP half of a proof: unseal the secret, check the code in the drift
/// window, and report the matched timestep. Local work only.
fn matching_totp_step(
    state: &AppState,
    row: Option<&core_db::mfa::TotpSecretRow>,
    email: &str,
    code: &str,
) -> ApiResult<Option<i64>> {
    let Some(row) = row.filter(|row| row.confirmed) else {
        return Ok(None);
    };
    let b32 = String::from_utf8(
        crate::crypto::open(&state.config.secret_key, &row.secret_enc)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("{e}")))?,
    )
    .map_err(|_| ApiError::Internal(anyhow::anyhow!("stored secret not utf8")))?;
    totp_service::verify_code(
        &b32,
        email,
        code,
        chrono::Utc::now().timestamp().unsigned_abs(),
    )
    .map_err(ApiError::Validation)
}

/// The recovery-code half of a proof: one argon2id verify per unused code,
/// stopping at the first match. Bounded by the batch size and run against
/// hashes read out of the transaction, so no connection is held here.
async fn match_recovery_hash(candidates: &[(Uuid, String)], presented: &str) -> Option<Uuid> {
    for (code_id, hash) in candidates {
        if password::verify_async(presented.to_string(), hash.clone()).await {
            return Some(*code_id);
        }
    }
    None
}

/// Spends the proof inside the transaction that owns the change it authorized:
/// the TOTP watermark advance, or the recovery-code row.
///
/// Zero rows updated means someone already spent it — a replay — and the
/// answer is the same uniform 401 a wrong code gets.
async fn consume_factor_proof(
    state: &AppState,
    tx: &mut Tx,
    org_id: Uuid,
    user_id: Uuid,
    proof: &FactorProof,
) -> ApiResult<SecondFactor> {
    match *proof {
        FactorProof::Totp { step } => {
            if core_db::mfa::consume_totp_step(&mut **tx, user_id, step).await? == 0 {
                audit_mfa_failure(state, org_id, user_id, "totp_replay").await;
                return Err(ApiError::Unauthorized("invalid code".into()));
            }
            Ok(SecondFactor {
                method: "totp",
                factor_ref: None,
            })
        }
        FactorProof::RecoveryCode { id } => {
            if core_db::mfa::consume_recovery_code(&mut **tx, id).await? == 0 {
                audit_mfa_failure(state, org_id, user_id, "replay").await;
                return Err(ApiError::Unauthorized("invalid code".into()));
            }
            Ok(SecondFactor {
                method: "recovery_code",
                factor_ref: Some(id.to_string()),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// TOTP enrollment
// ---------------------------------------------------------------------------

/// POST /api/auth/mfa/totp/enroll-begin — generates a secret (returned ONCE)
/// and stores it sealed+unconfirmed.
///
/// A FIRST enrolment (no confirmed factor) simply restarts the pending window.
/// Replacing a CONFIRMED factor is a destructive act and needs a fresh step-up
/// proved by a method OTHER than TOTP: a live code cannot authorize replacing
/// the secret it belongs to, so the caller uses a recovery code or a passkey.
/// The account keeps its recovery codes through the swap, which is what stops
/// it holding zero working factors while the new secret is pending.
///
/// After an admin reset (`users.mfa_reenroll_required`) enrolment is the
/// sanctioned way out and the audited reset is its authorization: there is no
/// factor left to step up with, which is exactly why the reset travels in this
/// lane.
#[utoipa::path(operation_id = "mfa_totp_enroll_begin", post, path = "/api/auth/mfa/totp/enroll-begin",
    tag = "auth", security(("bearer" = [])),
    responses((status = 200, body = TotpEnrollBeginResponse)))]
pub async fn totp_enroll_begin(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<TotpEnrollBeginResponse>> {
    let authed = crate::auth::middleware::authed::<AuthedUser>(&extensions)?;
    let user = &authed.principal.user;

    let mut tx = org_tx(&state, user.org_id).await?;
    let existing = core_db::mfa::find_totp(&mut *tx, user.id).await?;
    let confirmed = existing.as_ref().is_some_and(|row| row.confirmed);
    let reenrolling = core_db::mfa::user_mfa_reenroll_required(&mut *tx, user.id).await?;

    // The account's factor SET is the gate, not the method being moved: a
    // confirmed PASSKEY is a working factor too, so a session on a
    // passkey-protected account must prove one of the account's factors
    // before it may enroll a new TOTP secret it controls (p57). The TOTP
    // code itself cannot authorize anything here — replacing a confirmed
    // row must be proved by a DIFFERENT method (a live code cannot vouch
    // for the secret it belongs to). After an admin reset the enrolment IS
    // the sanctioned way out and no proof exists to ask for.
    let caps = core_db::mfa::mfa_capabilities(&mut *tx, user.id).await?;
    let holds_confirmed_factor = caps.totp_confirmed || caps.webauthn_credentials > 0;
    let authorized_by = if holds_confirmed_factor && (confirmed || !reenrolling) {
        Some(
            require_fresh_step_up(
                &mut tx,
                &authed,
                &["recovery_code", "webauthn"],
                "replacing a confirmed factor under a new TOTP enrollment",
            )
            .await?,
        )
    } else {
        None
    };

    let enrollment =
        totp_service::new_enrollment(&user.email, "Marula").map_err(ApiError::Validation)?;
    let sealed = crate::crypto::seal(
        &state.config.secret_key,
        enrollment.secret_base32.as_bytes(),
    )
    .map_err(|e| ApiError::Internal(anyhow::anyhow!("{e}")))?;

    if confirmed {
        core_db::mfa::replace_confirmed_totp_with_pending(
            &mut *tx,
            Uuid::now_v7(),
            user.org_id,
            user.id,
            &sealed,
        )
        .await?;
    } else {
        let written = core_db::mfa::upsert_pending_totp(
            &mut *tx,
            Uuid::now_v7(),
            user.org_id,
            user.id,
            &sealed,
        )
        .await?;
        if written == 0 {
            // A concurrent request confirmed a factor between the read above
            // and this write; the SQL refuses to downgrade it, and the honest
            // answer is the gate the caller skipped.
            return Err(step_up_refusal(
                "step_up_required",
                format!(
                    "a TOTP factor was confirmed while this request started; prove a live \
                     factor with POST {STEP_UP_ENDPOINT} before replacing it"
                ),
                STEP_UP_ENDPOINT,
            ));
        }
    }

    core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id: user.org_id,
            actor: Actor::Human(user.id),
            action: "auth.mfa_totp_enroll_started".into(),
            entity_type: Some("user".into()),
            entity_id: Some(user.id.to_string()),
            detail: serde_json::json!({
                "replaces_confirmed_factor": confirmed,
                "authorized_by": authorized_by.as_ref().map(|row| row.method.clone()),
                "reenrollment": reenrolling,
            }),
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
    let reenrolling = core_db::mfa::user_mfa_reenroll_required(&mut *tx, user.id).await?;
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
    // A factor is live again, so the account no longer owes one.
    if reenrolling {
        core_db::mfa::clear_mfa_reenroll_required(&mut *tx, user.id).await?;
    }
    let codes = generate_recovery_codes(&mut tx, user.org_id, user.id).await?;
    audit_in_tx(
        &mut tx,
        user.org_id,
        user.id,
        "auth.mfa_totp_enabled",
        serde_json::json!({ "reenrollment_completed": reenrolling }),
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

/// Mints the plaintext batch and the row ids it will be stored under. No
/// hashing, no database: the caller decides when and under which slot the
/// batch gets hashed.
fn mint_recovery_codes() -> (Vec<String>, Vec<Uuid>) {
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
    (plain, ids)
}

/// Hashes a freshly minted batch with the caller's admission slot.
///
/// Recovery codes are argon2id secrets like any other, so the batch owes the
/// same slot the password does. Passing the permit in is deliberate: the
/// batch cannot be hashed without one.
async fn hash_recovery_batch(
    plain: &[String],
    permit: tokio::sync::SemaphorePermit<'static>,
) -> ApiResult<Vec<String>> {
    let hashes = password::hash_many_async(plain.to_vec())
        .await
        .map_err(ApiError::Internal)?;
    drop(permit);
    Ok(hashes)
}

/// Generates + persists a fresh hashed batch; returns PLAINTEXT once.
async fn generate_recovery_codes(
    tx: &mut Tx,
    org_id: Uuid,
    user_id: Uuid,
) -> ApiResult<Vec<String>> {
    let (plain, ids) = mint_recovery_codes();
    // Hash the whole batch off the async runtime (Wave 49 lane R3): eight
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
///
/// This is the explicit "turn MFA off" action the last-factor rule points at.
/// A successful call also stamps a step-up, because the caller just proved a
/// factor on this session; the next change within the window does not
/// re-prompt.
#[utoipa::path(operation_id = "mfa_totp_disable", post, path = "/api/auth/mfa/totp/disable",
    tag = "auth", security(("bearer" = [])),
    request_body = TotpDisableRequest, responses((status = 204), (status = 401)))]
pub async fn totp_disable(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<TotpDisableRequest>,
) -> ApiResult<axum::response::Response> {
    let authed = crate::auth::middleware::authed::<AuthedUser>(&extensions)?;
    let user = &authed.principal.user;

    // Read what the proof hashes, then give the connection back: the password
    // verify and any recovery-code verify below run with no transaction open.
    let material =
        read_proof_material(&state, user.org_id, user.id, &user.email, "turning MFA off").await?;
    // The same admission bound the login doors take (Wave 59 lane
    // `p59-mfa-hash`): the slot is taken at the point of need, released when
    // the proof returns, and a caller that cannot get one inside the queue wait
    // gets the typed 503 with `Retry-After` instead of spending the hash.
    let permit = match state.config.auth_hash_gate().admit().await {
        Ok(permit) => permit,
        Err(refused) => return Ok(crate::auth::handlers::hash_slots_exhausted(refused)),
    };
    let proof = prove_factor(
        &state,
        &material,
        &req.password,
        &user.email,
        &req.code,
        permit,
    )
    .await?;

    let mut tx = org_tx(&state, user.org_id).await?;
    let proved = consume_factor_proof(&state, &mut tx, user.org_id, user.id, &proof).await?;
    stamp_step_up(&mut tx, &authed, &proved).await?;

    core_db::mfa::delete_totp(&mut *tx, user.id).await?;
    core_db::mfa::replace_recovery_codes(&mut *tx, user.org_id, user.id, &[], &[]).await?;
    audit_in_tx(
        &mut tx,
        user.org_id,
        user.id,
        "auth.mfa_totp_disabled",
        serde_json::json!({ "authorized_by": proved.method }),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(axum::http::StatusCode::NO_CONTENT.into_response())
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
) -> ApiResult<axum::response::Response> {
    let authed = crate::auth::middleware::authed::<AuthedUser>(&extensions)?;
    let user = &authed.principal.user;

    let material = read_proof_material(
        &state,
        user.org_id,
        user.id,
        &user.email,
        "regenerating recovery codes",
    )
    .await?;
    let permit = match state.config.auth_hash_gate().admit().await {
        Ok(permit) => permit,
        Err(refused) => return Ok(crate::auth::handlers::hash_slots_exhausted(refused)),
    };
    let proof = prove_factor(
        &state,
        &material,
        &req.password,
        &user.email,
        &req.code,
        permit,
    )
    .await?;

    // The replacement batch is hashed before the transaction that stores it
    // opens, under its own slot: eight argon2id hashes is the largest cost any
    // request in this module pays, and none of it needs a connection.
    let (plain, ids) = mint_recovery_codes();
    let permit = match state.config.auth_hash_gate().admit().await {
        Ok(permit) => permit,
        Err(refused) => return Ok(crate::auth::handlers::hash_slots_exhausted(refused)),
    };
    let hashes = hash_recovery_batch(&plain, permit).await?;

    let mut tx = org_tx(&state, user.org_id).await?;
    let proved = consume_factor_proof(&state, &mut tx, user.org_id, user.id, &proof).await?;
    stamp_step_up(&mut tx, &authed, &proved).await?;

    core_db::mfa::replace_recovery_codes(&mut *tx, user.org_id, user.id, &hashes, &ids).await?;
    audit_in_tx(
        &mut tx,
        user.org_id,
        user.id,
        "auth.mfa_recovery_regenerated",
        serde_json::json!({ "count": plain.len(), "authorized_by": proved.method }),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(RecoveryCodesResponse {
        recovery_codes: plain,
    })
    .into_response())
}

// ---------------------------------------------------------------------------
// Step-up
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct MfaStepUpRequest {
    pub password: String,
    /// Current TOTP code or a recovery code. A recovery code is CONSUMED,
    /// mirroring login semantics.
    pub code: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MfaStepUpResponse {
    /// Which factor proved possession (`totp` or `recovery_code`). A passkey
    /// step-up is the passkey login itself, recorded on the session it
    /// establishes.
    pub method: String,
    /// Seconds the proof stays usable, counted from `verified_at`.
    pub ttl_secs: i64,
    pub verified_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// POST /api/auth/mfa/step-up — proves password plus a live factor and records
/// it against this sign-in lineage.
///
/// This is the door the destructive factor changes point at. It is deliberately
/// separate from `mfa/verify`: that one completes a login, this one raises what
/// an already-signed-in session may do, and the record it leaves expires.
///
/// A passkey user does not call this — their step-up is the passkey login,
/// which stamps the same record on the session it creates.
#[utoipa::path(operation_id = "mfa_step_up", post, path = "/api/auth/mfa/step-up",
    tag = "auth", security(("bearer" = [])),
    request_body = MfaStepUpRequest,
    responses((status = 200, body = MfaStepUpResponse), (status = 401), (status = 428)))]
pub async fn step_up(
    State(state): State<AppState>,
    Extension(ClientIp(client_ip)): Extension<ClientIp>,
    extensions: Extensions,
    Json(req): Json<MfaStepUpRequest>,
) -> ApiResult<axum::response::Response> {
    let authed = crate::auth::middleware::authed::<AuthedUser>(&extensions)?;
    let user = &authed.principal.user;
    let Some(session_id) = authed.session_id else {
        return Err(ApiError::Unauthorized(
            "a step-up needs a signed-in browser session".into(),
        ));
    };

    // Same durable throttle as the login bridge: being signed in must not turn
    // the 6-digit keyspace into a free guessing game.
    enforce_gate(
        &state,
        crate::auth::hardening::kinds::MFA_VERIFY,
        &client_ip,
        GateSubject::User(user.id),
        false,
    )
    .await?;

    // After an admin reset there is no factor to prove; the typed answer names
    // enrolment instead of asking for a code that cannot exist. Read here, so
    // a reset account never pays for a hash.
    let material =
        read_proof_material(&state, user.org_id, user.id, &user.email, "stepping up").await?;
    let permit = match state.config.auth_hash_gate().admit().await {
        Ok(permit) => permit,
        Err(refused) => return Ok(crate::auth::handlers::hash_slots_exhausted(refused)),
    };
    let proof = match prove_factor(
        &state,
        &material,
        &req.password,
        &user.email,
        &req.code,
        permit,
    )
    .await
    {
        Ok(proof) => proof,
        Err(error) => {
            let _ = record_failure(
                &state,
                crate::auth::hardening::kinds::MFA_VERIFY,
                &client_ip,
                GateSubject::User(user.id),
            )
            .await;
            return Err(error);
        }
    };

    let mut tx = org_tx(&state, user.org_id).await?;
    let proved = match consume_factor_proof(&state, &mut tx, user.org_id, user.id, &proof).await {
        Ok(proved) => proved,
        Err(error) => {
            let _ = record_failure(
                &state,
                crate::auth::hardening::kinds::MFA_VERIFY,
                &client_ip,
                GateSubject::User(user.id),
            )
            .await;
            return Err(error);
        }
    };

    let verified_at = chrono::Utc::now();
    let expires_at = verified_at + chrono::Duration::seconds(STEP_UP_TTL_SECS);
    let written = core_db::mfa::record_step_up(
        &mut *tx,
        core_db::mfa::StepUpRecord {
            id: Uuid::now_v7(),
            org_id: user.org_id,
            user_id: user.id,
            session_id,
            method: proved.method,
            factor_ref: proved.factor_ref.as_deref(),
            expires_at,
        },
    )
    .await?;
    if written == 0 {
        return Err(ApiError::Unauthorized(
            "this session is no longer active".into(),
        ));
    }
    audit_in_tx(
        &mut tx,
        user.org_id,
        user.id,
        "auth.mfa_step_up",
        serde_json::json!({ "method": proved.method, "ttl_secs": STEP_UP_TTL_SECS }),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let _ = record_success(
        &state,
        crate::auth::hardening::kinds::MFA_VERIFY,
        &client_ip,
        GateSubject::User(user.id),
    )
    .await;

    Ok(Json(MfaStepUpResponse {
        method: proved.method.to_string(),
        ttl_secs: STEP_UP_TTL_SECS,
        verified_at,
        expires_at,
    })
    .into_response())
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
                let factor = SecondFactor {
                    method: "recovery_code",
                    factor_ref: Some(code_id.to_string()),
                };
                if let Err(error) =
                    record_step_up_for_session(&state, org_id, user_id, tokens.session_id, &factor)
                        .await
                {
                    tracing::warn!(%error, "mfa login: step-up stamp failed; the session must prove a factor again to change one");
                }
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
    method: &'static str,
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
    // The session that just proved a factor starts life with a fresh step-up:
    // "this sign-in verified a factor moments ago" is exactly the fact the
    // destructive factor changes ask for.
    let factor = SecondFactor {
        method,
        factor_ref: None,
    };
    if let Err(error) =
        record_step_up_for_session(state, org_id, user_id, tokens.session_id, &factor).await
    {
        tracing::warn!(%error, "mfa login: step-up stamp failed; the session must prove a factor again to change one");
    }
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

    // Registering a passkey is an ADD to the account's factor set, and a
    // passkey the caller controls is a future login credential: an account
    // that already holds a confirmed factor must prove one on this sign-in
    // lineage first (p57), exactly as replacing its TOTP does. After an
    // admin reset enrolment is the sanctioned exit and needs no proof.
    let caps = core_db::mfa::mfa_capabilities(&mut *tx, user.id).await?;
    let reenrolling = core_db::mfa::user_mfa_reenroll_required(&mut *tx, user.id).await?;
    if (caps.totp_confirmed || caps.webauthn_credentials > 0) && !reenrolling {
        require_fresh_step_up(
            &mut tx,
            &authed,
            &["totp", "recovery_code", "webauthn"],
            "registering a passkey",
        )
        .await?;
    }

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
    // A passkey is a working factor, so an account that owed one no longer
    // does. This is the other half of the admin reset's exit path.
    let reenrolling = core_db::mfa::user_mfa_reenroll_required(&mut *tx, user.id).await?;
    if reenrolling {
        core_db::mfa::clear_mfa_reenroll_required(&mut *tx, user.id).await?;
    }
    audit_in_tx(
        &mut tx,
        user.org_id,
        user.id,
        "auth.mfa_webauthn_credential_added",
        serde_json::json!({ "label": label, "reenrollment_completed": reenrolling }),
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

/// POST /api/auth/mfa/webauthn/credentials/{id}/remove — removes one passkey.
///
/// The password is not the authorization. A phished password must not be able
/// to downgrade a passkey-only account to password-only sign-in (DECISIONS
/// record 2), so removal needs a FRESH step-up on this sign-in lineage, proved
/// by a factor that is not the credential being removed. The last factor of
/// any kind cannot leave this way: that is the explicit "turn MFA off" action
/// (`POST /api/auth/mfa/totp/disable`), and the refusal says so.
#[utoipa::path(operation_id = "mfa_webauthn_remove", post,
    path = "/api/auth/mfa/webauthn/credentials/{id}/remove", tag = "auth",
    security(("bearer" = [])), params(("id" = uuid::Uuid, Path)),
    request_body = WebauthnRemoveRequest, responses((status = 204)))]
pub async fn webauthn_credential_remove(
    State(state): State<AppState>,
    extensions: Extensions,
    axum::extract::Path(cred_row_id): axum::extract::Path<Uuid>,
    Json(req): Json<WebauthnRemoveRequest>,
) -> ApiResult<axum::response::Response> {
    let authed = crate::auth::middleware::authed::<AuthedUser>(&extensions)?;
    let user = &authed.principal.user;

    // This door proves the password plus a step-up recorded earlier, so the
    // only hash it owes is the password verify — and that runs with the
    // connection already given back.
    let password_hash = read_password_hash(
        &state,
        user.org_id,
        user.id,
        &user.email,
        "removing a passkey",
    )
    .await?;
    let permit = match state.config.auth_hash_gate().admit().await {
        Ok(permit) => permit,
        Err(refused) => return Ok(crate::auth::handlers::hash_slots_exhausted(refused)),
    };
    let matched = password::verify_async(req.password.clone(), password_hash).await;
    drop(permit);
    if !matched {
        return Err(ApiError::Unauthorized("invalid credentials".into()));
    }

    let mut tx = org_tx(&state, user.org_id).await?;
    let step = require_fresh_step_up(
        &mut tx,
        &authed,
        &["totp", "recovery_code", "webauthn"],
        "removing a passkey",
    )
    .await?;

    // The credential being removed must not be the one that authorized the
    // removal — otherwise "prove a factor" proves the very thing about to
    // disappear, and a stolen key could walk itself out while leaving the
    // account on a password.
    let registered = core_db::mfa::list_webauthn_credentials(&mut *tx, user.id).await?;
    let target = registered
        .iter()
        .find(|cred| cred.id == cred_row_id)
        .ok_or(ApiError::NotFound)?;
    if step.method == "webauthn" && step.factor_ref.as_deref() == Some(target.cred_id.as_str()) {
        return Err(step_up_refusal(
            "step_up_factor_not_allowed",
            format!(
                "a passkey cannot authorize removing itself; prove a different factor with \
                 POST {STEP_UP_ENDPOINT}"
            ),
            STEP_UP_ENDPOINT,
        ));
    }

    // Removing the last working factor would hand the account back as a
    // password-only login. That is a deliberate product action, not a
    // side effect of tidying passkeys.
    let remaining = {
        let caps = core_db::mfa::mfa_capabilities(&mut *tx, user.id).await?;
        (caps.totp_confirmed as i64) + (caps.webauthn_credentials - 1) + caps.unused_recovery_codes
    };
    if remaining <= 0 {
        return Err(ApiError::Conflict(
            "this is the account's last second factor; removing it would leave password-only \
             sign-in — turn MFA off deliberately with POST /api/auth/mfa/totp/disable, or ask \
             an administrator for a factor reset if you hold no other factor"
                .into(),
        ));
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
        serde_json::json!({
            "label": target.label,
            "authorized_by": step.method,
            "remaining_factors": remaining,
        }),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(axum::http::StatusCode::NO_CONTENT.into_response())
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
        serde_json::json!({ "method": "webauthn", "cred_id": &used_cred_id }),
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
    let factor = SecondFactor {
        method: "webauthn",
        factor_ref: Some(used_cred_id),
    };
    if let Err(error) =
        record_step_up_for_session(&state, org_id, user_id, tokens.session_id, &factor).await
    {
        tracing::warn!(%error, "mfa login: step-up stamp failed; the session must prove a factor again to change one");
    }
    Ok(mfa_tokens_response(&state, tokens))
}

// ---------------------------------------------------------------------------
// Admin-initiated factor reset
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct MfaResetResponse {
    pub user_id: Uuid,
    /// TOTP enrolments removed (0 or 1 — the schema holds one per user).
    pub totp_removed: i64,
    pub passkeys_removed: i64,
    pub recovery_codes_removed: i64,
    /// Live sessions revoked, so a token minted before the reset cannot keep
    /// acting on the account.
    pub sessions_revoked: i64,
    /// Always true. The account owes a factor until one is enrolled, and the
    /// MFA surface refuses every other factor change until then.
    pub reenroll_required: bool,
}

/// POST /api/auth/users/{id}/mfa-reset — clears another user's factors.
///
/// This exists because the strict rule needs an exit: a user who lost their only
/// factor cannot step up, and without a reset path the step-up requirement
/// locks them out of their own account changes (the ruling of 2026-10-25 puts
/// the two in one lane for exactly that reason).
///
/// What makes it safe rather than a takeover tool:
/// - `manage_users` is required, and an administrator cannot reset their own
///   factors — a stolen admin session must not be able to turn its owner's MFA
///   off with one call.
/// - An Owner's or Admin's factors can only be reset by an Owner, so a lesser
///   administrator cannot strip a peer's second factor.
/// - The account is left owing a factor (`users.mfa_reenroll_required`), not
///   merely factor-free: enrolment is the only factor change it may make until
///   one is confirmed, and the flag is audited with the counts.
/// - Every live session is revoked, so the user re-authenticates with their
///   password and learns the factor is gone.
/// - The audit row names the administrator, the target, what was removed and
///   how many sessions died.
#[utoipa::path(operation_id = "mfa_admin_factor_reset", post,
    path = "/api/auth/users/{id}/mfa-reset", tag = "auth",
    security(("bearer" = [])), params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = MfaResetResponse), (status = 403), (status = 404)))]
pub async fn admin_reset_user_factors(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(target_user_id): Path<Uuid>,
) -> ApiResult<Json<MfaResetResponse>> {
    let authed = crate::auth::middleware::authed::<AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManageUsers)?;
    let admin = &authed.principal.user;

    if admin.id == target_user_id {
        return Err(ApiError::Forbidden(
            "an administrator cannot reset their own factors; use the self-service path, or \
             ask another administrator"
                .into(),
        ));
    }

    let mut tx = org_tx(&state, admin.org_id).await?;
    // RLS scopes this to the caller's org, so a foreign id is NotFound — the
    // same answer as an id that never existed.
    let target = core_db::users::get_user(&mut *tx, admin.org_id, target_user_id).await?;

    let target_roles =
        core_db::users::effective_roles_for_user(&mut *tx, target.id, admin.org_id).await?;
    let target_is_privileged = target_roles.iter().any(|role| {
        matches!(
            role.builtin,
            Some(BuiltinRole::Owner) | Some(BuiltinRole::Admin)
        )
    });
    let caller_is_owner = authed
        .principal
        .roles
        .iter()
        .any(|role| matches!(role.builtin, Some(BuiltinRole::Owner)));
    if target_is_privileged && !caller_is_owner {
        return Err(ApiError::Forbidden(
            "only an owner can reset an owner's or administrator's factors".into(),
        ));
    }

    let counts = core_db::mfa::reset_user_factors(&mut *tx, target.id).await?;
    let revoked = core_db::users::revoke_all_user_sessions(&mut *tx, target.id).await?;
    // A count of live sessions cannot exceed i64 in any real deployment; the
    // saturating conversion keeps the DTO type honest without a panic path.
    let sessions_revoked = i64::try_from(revoked).unwrap_or(i64::MAX);

    core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id: admin.org_id,
            actor: Actor::Human(admin.id),
            action: "auth.mfa_factors_reset".into(),
            entity_type: Some("user".into()),
            entity_id: Some(target.id.to_string()),
            detail: serde_json::json!({
                "totp_removed": counts.totp_removed,
                "passkeys_removed": counts.passkeys_removed,
                "recovery_codes_removed": counts.recovery_codes_removed,
                "sessions_revoked": sessions_revoked,
                "reenroll_required": true,
            }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    Ok(Json(MfaResetResponse {
        user_id: target.id,
        totp_removed: counts.totp_removed,
        passkeys_removed: counts.passkeys_removed,
        recovery_codes_removed: counts.recovery_codes_removed,
        sessions_revoked,
        reenroll_required: true,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    /// The refusal is the client's instruction manual, so it is part of the
    /// contract: a 428, a machine reason, the endpoint that fixes it, and the
    /// window it buys. Asserting the envelope here keeps a later refactor from
    /// quietly demoting it to a generic 403.
    #[tokio::test]
    async fn a_missing_step_up_is_a_typed_428_that_says_what_to_do() {
        let response = step_up_refusal(
            "step_up_stale",
            format!("prove a factor again with POST {STEP_UP_ENDPOINT}"),
            STEP_UP_ENDPOINT,
        )
        .into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::PRECONDITION_REQUIRED
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let envelope: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(envelope["error"]["reason"], "step_up_stale");
        assert_eq!(envelope["error"]["endpoint"], STEP_UP_ENDPOINT);
        assert_eq!(envelope["error"]["ttl_secs"], STEP_UP_TTL_SECS);
        assert!(envelope["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("prove a factor again")));
    }

    /// The "factor not allowed" answer has to name an alternative the caller
    /// can actually produce, in words, or it reads as a dead end.
    #[test]
    fn the_allowed_factors_read_as_a_sentence() {
        assert_eq!(
            allowed_factor_label(&["recovery_code", "webauthn"]),
            "a recovery code or a passkey sign-in"
        );
        assert_eq!(allowed_factor_label(&["totp"]), "a TOTP code");
        assert_eq!(allowed_factor_label(&[]), "a live factor");
    }
}
