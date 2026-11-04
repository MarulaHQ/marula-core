//! Auth throttling/lockout request-side helpers (Wave 19 lane 5).
//!
//! Durable per-IP AND per-actor gates sit in front of every throttled
//! auth surface: POST /api/auth/login, /api/auth/mfa/verify,
//! /api/auth/mfa/webauthn/begin + /finish, and /api/auth/refresh — plus the
//! client portal's own sign-ins, `/api/portal/login` and the magic-link
//! pair, which run on kinds of their own so the two populations never share
//! a lock (see `core_db::hardening::gate_kind`).
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
//! (threat model §A1), and `/api/portal/login` follows it for the same
//! reason — an anonymous caller chooses the address on both. Every other
//! throttled surface surfaces the typed 429 envelope
//! (`auth_rate_limited`, IF3) — their request shape already
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
//!
//! The refresh gate is keyed TWICE (ADR 0029). The IP key is the outer
//! bound: it is checked before any credential work, so a caller who never
//! presents a valid token still pays it. The session key is the inner
//! arm: once `users::resolve_refresh` has produced a session, the request
//! is checked again against that session, and a failure is charged to the
//! session alone — the shared IP counter is left to the session-less
//! failures, which is what stops one stale cookie from throttling every
//! user behind one egress. Both keys are cleared by a successful
//! rotation.

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
    /// The INNER arm of the two-key refresh gate (ADR 0029): a session
    /// that `resolve_refresh` has already returned. A session id the
    /// caller merely presented does not name one — keying a failure on
    /// an unvalidated id would let a caller charge a bucket that is not
    /// their own.
    Session(Uuid),
    /// No actor is known yet. Refresh runs on the IP half alone only
    /// until a session resolves; every other surface uses this for its
    /// pre-auth leg.
    IpOnly,
}

/// Salted sha256 of the IP — raw IPs are PII and the surface stores
/// hashes (refresh-token parity).
fn ip_fingerprint(jwt_secret: &str, ip: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"ip:");
    digest.update(jwt_secret.as_bytes());
    digest.update(ip.as_bytes());
    format!("ip:{}", hex::encode(digest.finalize()))
}

fn email_fingerprint(email: &str) -> String {
    format!("email:{}", email.trim().to_lowercase())
}

/// The inner arm's key. A session id is a UUIDv7, not a secret of its own,
/// but the value never leaves the server: the key is a bucket name, and the
/// row it names carries counts only.
fn session_fingerprint(session: Uuid) -> String {
    format!("session:{session}")
}

fn lock_keys(jwt_secret: &str, ip: &str, subject: &GateSubject<'_>) -> Vec<String> {
    let mut keys = vec![ip_fingerprint(jwt_secret, ip)];
    match subject {
        GateSubject::Email(email) => keys.push(email_fingerprint(email)),
        GateSubject::User(id) => keys.push(format!("user:{id}")),
        GateSubject::Session(id) => keys.push(session_fingerprint(*id)),
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
    let keys = lock_keys(&state.config.jwt_secret, client_ip, &subject);
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
    let keys = lock_keys(&state.config.jwt_secret, client_ip, &subject);
    let subject_key = match subject {
        GateSubject::Email(email) => Some(email_fingerprint(email)),
        GateSubject::User(id) => Some(format!("user:{id}")),
        GateSubject::Session(id) => Some(session_fingerprint(id)),
        GateSubject::IpOnly => None,
    };
    let remaining = gate_fail(
        &state.pool,
        kind,
        &keys,
        None,
        subject_key.as_deref(),
        Some(&ip_fingerprint(&state.config.jwt_secret, client_ip)),
    )
    .await
    .map_err(ApiError::from)?;
    tracing::warn!(%kind, remaining_secs = remaining, "auth gate failure recorded");
    Ok(())
}

/// Records a refresh failure on the resolved session's INNER arm only
/// (ADR 0029).
///
/// The two keys of the refresh gate cover different failures, so they do
/// not collect the same one. A session that keeps presenting a stale
/// secret must not charge the shared IP counter: that counter is what an
/// office NAT, a CI runner, or a long E2E sweep all sit behind, and the
/// incident the ADR rules on is exactly one stale cookie arming an hour
/// for everyone. Failures that never resolve a session — the caller has
/// no valid token at all — still pay the IP key through
/// [`record_failure`], which is the half an attacker cannot avoid.
///
/// The durable attempt row carries BOTH keys for attribution
/// (`subject_key` and `ip_key`), and a later success clears both
/// (`record_success` takes `GateSubject::Session`, whose key set is the IP
/// key and the session key).
pub async fn record_session_failure(
    state: &AppState,
    kind: &str,
    client_ip: &str,
    session: Uuid,
) -> ApiResult<()> {
    let subject_key = session_fingerprint(session);
    let charged = [subject_key.clone()];
    let remaining = gate_fail(
        &state.pool,
        kind,
        &charged,
        None,
        Some(&subject_key),
        Some(&ip_fingerprint(&state.config.jwt_secret, client_ip)),
    )
    .await
    .map_err(ApiError::from)?;
    tracing::warn!(%kind, remaining_secs = remaining, "auth session gate failure recorded");
    Ok(())
}

/// Clears the counters after a successful authentication.
pub async fn record_success(
    state: &AppState,
    kind: &str,
    client_ip: &str,
    subject: GateSubject<'_>,
) -> ApiResult<()> {
    let keys = lock_keys(&state.config.jwt_secret, client_ip, &subject);
    gate_clear(&state.pool, kind, &keys)
        .await
        .map_err(ApiError::from)
}

#[cfg(test)]
mod tests {
    //! The two-key refresh gate (ADR 0029) is about WHICH bucket a request
    //! lands in, and that is decided by key derivation alone — no database
    //! needed. The counter arithmetic (the 30s first rung, the geometric
    //! climb, the clear-on-success) is pinned by the durable-gate tests in
    //! `tests/auth_identity`, which need a real Postgres.

    use super::*;

    const SECRET: &str = "test-jwt-secret";
    const IP_A: &str = "203.0.113.10";
    const IP_B: &str = "203.0.113.20";

    fn keys(ip: &str, subject: GateSubject<'_>) -> Vec<String> {
        lock_keys(SECRET, ip, &subject)
    }

    fn session_a() -> Uuid {
        Uuid::from_u128(1)
    }

    fn session_b() -> Uuid {
        Uuid::from_u128(2)
    }

    #[test]
    fn the_inner_arm_is_keyed_by_session_alone() {
        assert_eq!(
            session_fingerprint(session_a()),
            format!("session:{}", session_a())
        );
        assert_ne!(
            session_fingerprint(session_a()),
            session_fingerprint(session_b())
        );
        // The outer half of a session check is still present — the IP bound
        // runs on every request, including the one that resolves.
        assert_eq!(
            keys(IP_A, GateSubject::Session(session_a())),
            vec![
                ip_fingerprint(SECRET, IP_A),
                session_fingerprint(session_a())
            ]
        );
    }

    #[test]
    fn a_different_session_on_the_same_ip_does_not_inherit_the_inner_arm() {
        let one = keys(IP_A, GateSubject::Session(session_a()));
        let two = keys(IP_A, GateSubject::Session(session_b()));
        let ip_key = ip_fingerprint(SECRET, IP_A);

        // Exactly one key in common: the shared outer IP bound. An armed
        // `session_a` row is invisible to `session_b`, which is the whole
        // point — one laptop's stale cookie must not throttle the office.
        let shared: Vec<&String> = one.iter().filter(|key| two.contains(key)).collect();
        assert_eq!(shared, vec![&ip_key]);
        assert!(!two.contains(&session_fingerprint(session_a())));
    }

    #[test]
    fn the_same_session_on_a_different_ip_still_meets_the_inner_arm() {
        let here = keys(IP_A, GateSubject::Session(session_a()));
        let elsewhere = keys(IP_B, GateSubject::Session(session_a()));
        let session_key = session_fingerprint(session_a());

        // The outer keys differ (a new egress is a fresh IP bucket), but the
        // session key is in both sets, so changing address cannot drop the
        // session's own lock.
        let shared: Vec<&String> = here.iter().filter(|key| elsewhere.contains(key)).collect();
        assert_eq!(shared, vec![&session_key]);
        assert_ne!(ip_fingerprint(SECRET, IP_A), ip_fingerprint(SECRET, IP_B));
    }

    #[test]
    fn a_session_failure_charges_the_inner_arm_and_not_the_shared_ip_bucket() {
        let charged = [session_fingerprint(session_a())];

        // The office-wide counter is not among them: this is the incident
        // ADR 0029 removes, and it is a property of the charged key set, not
        // of the counter arithmetic.
        assert!(!charged.contains(&ip_fingerprint(SECRET, IP_A)));
        // Another session presenting the same stale cookie shape is charged
        // nothing by this one.
        assert!(!charged.contains(&session_fingerprint(session_b())));
    }

    #[test]
    fn a_successful_rotation_clears_both_arms() {
        let cleared = keys(IP_A, GateSubject::Session(session_a()));
        let charged_by_a_session = [session_fingerprint(session_a())];

        // `record_success` clears every key of the subject's set; the inner
        // arm a failure charged must be inside it, or the ladder ratchets.
        assert!(charged_by_a_session.iter().all(|key| cleared.contains(key)));
        assert!(cleared.contains(&ip_fingerprint(SECRET, IP_A)));
    }

    #[test]
    fn a_session_less_failure_keeps_the_ip_only_shape() {
        // No session resolved: the caller has no valid token, which is the
        // case an attacker controls, so the IP key is the whole set.
        assert_eq!(
            keys(IP_A, GateSubject::IpOnly),
            vec![ip_fingerprint(SECRET, IP_A)]
        );
    }

    #[test]
    fn the_ip_key_is_salted_and_not_a_readable_address() {
        let key = ip_fingerprint(SECRET, IP_A);
        assert!(key.starts_with("ip:"));
        assert!(!key.contains(IP_A));
        // A different salt is a different bucket: two deployments never share
        // per-IP counters.
        assert_ne!(key, ip_fingerprint("another-secret", IP_A));
    }
}
