//! Auth throttling/lockout request-side helpers (Wave 19 lane 5).
//!
//! Durable per-IP AND per-actor gates sit in front of every throttled
//! auth surface: POST /api/auth/login, /api/auth/mfa/verify,
//! /api/auth/mfa/webauthn/begin + /finish, and /api/auth/refresh.
//! State lives in Postgres (migration 0088) — restarts never reset a
//! lock, and the posture is deny-by-default: no key, no admission.
//!
//! Enumeration discipline: LOGIN keeps its uniform 401 "invalid
//! credentials" answer for BOTH wrong-password and locked-out callers
//! (threat model §A1); every other throttled surface surfaces the typed
//! 429 envelope (`auth_rate_limited`, IF3) — their request shape already
//! proves possession of something (a valid mfa_pending bridge, a live
//! session id), so the code cannot fingerprint lockout state of an
//! account the caller does not already control.

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::http::HeaderMap;
pub use core_db::hardening::gate_kind as kinds;
use core_db::hardening::{gate_check, gate_clear, gate_fail};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Identifies the per-actor half of the gate where one exists.
pub enum GateSubject<'a> {
    Email(&'a str),
    User(Uuid),
    /// Refresh validates before any user identity exists: IP only.
    IpOnly,
}

/// Best-effort client IP: the first X-Forwarded-For hop when a proxy set
/// one, else a shared "unknown" bucket. Deliberately NOT trusting this
/// as a fresh identity — the per-IP keys are noise to an attacker, while
/// per-actor keys (email/user) stay authoritative; IP rotation cannot
/// bypass an account lock because the per-actor key locks too.
pub fn client_ip(headers: &HeaderMap) -> String {
    let forwarded = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    forwarded.unwrap_or_else(|| "unknown".into())
}

/// Salted sha256 of the IP — raw IPs are PII and the surface stores
/// hashes (refresh-token parity).
fn ip_fingerprint(state: &AppState, ip: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"ip:");
    digest.update(state.config.jwt_secret.as_bytes());
    digest.update(ip.as_bytes());
    format!("ip:{}", hex::encode(digest.finalize()))
}

fn email_fingerprint(email: &str) -> String {
    format!("email:{}", email.trim().to_lowercase())
}

fn lock_keys(state: &AppState, ip: &str, subject: &GateSubject<'_>) -> Vec<String> {
    let mut keys = vec![ip_fingerprint(state, ip)];
    match subject {
        GateSubject::Email(email) => keys.push(email_fingerprint(email)),
        GateSubject::User(id) => keys.push(format!("user:{id}")),
        GateSubject::IpOnly => {}
    }
    keys
}

/// Deny-by-default pre-check: ANY engaged lock on this surface's keys
/// rejects the request BEFORE any credential logic runs.
///
/// `uniform_401` is the login enumeration rule: the throttled answer is
/// envelope-shape identical to a wrong-password answer (no typed 429, no
/// reason-code divergence — typed throttles on login would let an
/// attacker probe the lockout state of arbitrary accounts).
pub async fn enforce_gate(
    state: &AppState,
    kind: &str,
    headers: &HeaderMap,
    subject: GateSubject<'_>,
    uniform_401: bool,
) -> ApiResult<()> {
    let ip = client_ip(headers);
    let keys = lock_keys(state, &ip, &subject);
    let remaining = gate_check(&state.pool, kind, &keys)
        .await
        .map_err(ApiError::from)?;
    if remaining > 0 {
        tracing::warn!(%kind, remaining_secs = remaining, "auth gate engaged; denying");
        if uniform_401 {
            return Err(ApiError::Unauthorized("invalid credentials".into()));
        }
        return Err(ApiError::AuthThrottled {
            retry_after_secs: remaining,
        });
    }
    Ok(())
}

/// Records a failure on every key of the surface (fire-and-forget is NOT
/// used — the gate must be durable, so failures block on the DB write).
pub async fn record_failure(
    state: &AppState,
    kind: &str,
    headers: &HeaderMap,
    subject: GateSubject<'_>,
) -> ApiResult<()> {
    let ip = client_ip(headers);
    let keys = lock_keys(state, &ip, &subject);
    let subject_key = match subject {
        GateSubject::Email(email) => Some(email_fingerprint(email)),
        GateSubject::User(id) => Some(format!("user:{id}")),
        GateSubject::IpOnly => None,
    };
    let remaining = gate_fail(
        &state.pool,
        kind,
        &keys,
        None,
        subject_key.as_deref(),
        Some(&ip_fingerprint(state, &ip)),
    )
    .await
    .map_err(ApiError::from)?;
    tracing::warn!(%kind, remaining_secs = remaining, "auth gate failure recorded");
    Ok(())
}

/// Clears the counters after a successful authentication.
pub async fn record_success(
    state: &AppState,
    kind: &str,
    headers: &HeaderMap,
    subject: GateSubject<'_>,
) -> ApiResult<()> {
    let ip = client_ip(headers);
    let keys = lock_keys(state, &ip, &subject);
    gate_clear(&state.pool, kind, &keys)
        .await
        .map_err(ApiError::from)
}
