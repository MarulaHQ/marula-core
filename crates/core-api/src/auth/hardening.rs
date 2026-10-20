//! Auth throttling/lockout request-side helpers (Wave 19 lane 5).
//!
//! Durable per-IP AND per-actor gates sit in front of every throttled
//! auth surface: POST /api/auth/login, /api/auth/mfa/verify,
//! /api/auth/mfa/webauthn/begin + /finish, and /api/auth/refresh.
//! State lives in Postgres (migration 0088) — restarts never reset a
//! lock, and the posture is deny-by-default: no key, no admission. What
//! DOES reset a curve is a success on the same surface: every throttled
//! leg that can succeed calls `record_success`, so the exponential ladder
//! is a reaction to a run of failures, never a permanent penalty for one
//! (the refresh leg was the exception until Wave 47 lane A26 — see
//! `handlers::refresh`).
//!
//! Enumeration discipline: LOGIN keeps its uniform 401 "invalid
//! credentials" answer for BOTH wrong-password and locked-out callers
//! (threat model §A1); every other throttled surface surfaces the typed
//! 429 envelope (`auth_rate_limited`, IF3) — their request shape already
//! proves possession of something (a valid mfa_pending bridge, a live
//! session id), so the code cannot fingerprint lockout state of an
//! account the caller does not already control.
//!
//! Per-IP keys (Wave 47 lane A10): the IP half of the key is the REAL
//! client IP, resolved once per request by the `client_ip` middleware
//! from the direct peer and the trusted-proxy chain
//! (`crate::client_ip`). Gates take the resolved value as a parameter —
//! none of them reads a header, so a spoofed `X-Forwarded-For` from an
//! untrusted peer cannot choose its own bucket. The value stays NOISE:
//! per-actor keys are what lock an account, so this lane buys honest
//! attribution for throttles and audits, not new enforcement.

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
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
///
/// `client_ip` is the already-resolved real client IP published by the
/// `client_ip` middleware (peer + trusted-proxy chain). A gate never
/// reads a proxy header itself.
pub async fn enforce_gate(
    state: &AppState,
    kind: &str,
    client_ip: &str,
    subject: GateSubject<'_>,
    uniform_401: bool,
) -> ApiResult<()> {
    let keys = lock_keys(state, client_ip, &subject);
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
    client_ip: &str,
    subject: GateSubject<'_>,
) -> ApiResult<()> {
    let keys = lock_keys(state, client_ip, &subject);
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
        Some(&ip_fingerprint(state, client_ip)),
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
    client_ip: &str,
    subject: GateSubject<'_>,
) -> ApiResult<()> {
    let keys = lock_keys(state, client_ip, &subject);
    gate_clear(&state.pool, kind, &keys)
        .await
        .map_err(ApiError::from)
}
