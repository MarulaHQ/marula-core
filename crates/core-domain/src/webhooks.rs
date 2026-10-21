//! Webhook OUT delivery primitives (the private build tracker 2.15): the signed-payload
//! scheme receivers verify, the anti-replay window, and the retry backoff
//! schedule.
//!
//! Purpose: pure, framework-free rules for HMAC-signed webhook delivery.
//! The signature covers `"{unix_timestamp}.{raw_body}"` under the
//! subscription secret and travels as `t=<ts>,v1=<hex>` in
//! `X-Marula-Signature` plus a redundant `X-Marula-Timestamp` header —
//! the timestamp binding is what beats replay of an intercepted body.
//! Receivers MUST reject timestamps outside `verify`'s tolerance window.
//!
//! Non-purpose: no HTTP client, no storage, no scheduling — the worker
//! owns attempts; this module only answers "what should be sent" and
//! "is a presented delivery authentic".

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// Signature header: `t=<unix_secs>,v1=<hex hmac over "{t}.{body}">`.
pub const SIGNATURE_HEADER: &str = "X-Marula-Signature";
/// Redundant plain timestamp header (mirrors the `t=` component).
pub const TIMESTAMP_HEADER: &str = "X-Marula-Timestamp";

/// Delivery attempts before a job dead-letters under the shipped `balanced`
/// preset. Derived from the webhook
/// [`RetryPolicy`](crate::retry::RetryPolicy) rather than written twice: the
/// live budget comes from the resolved matrix
/// (`retry::policy(Subsystem::Webhook)`), which an operator moves with
/// `MARULA_ROBUSTNESS` or `MARULA_RETRY_WEBHOOK_ATTEMPTS`. This constant is
/// the fallback those resolutions use.
pub const MAX_DELIVERY_ATTEMPTS: i32 = crate::retry::RobustnessPreset::Balanced
    .policy(crate::retry::Subsystem::Webhook)
    .budget
    .limit_i32(6);

/// Default receiver-side tolerance for the signed timestamp. Generous
/// enough for queue delay plus clock skew; tight enough to make captured
/// payloads worthless after ~ten minutes.
pub const REPLAY_TOLERANCE_SECS: i64 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyError {
    /// Header missing or not `t=<int>,v1=<hex>`.
    MalformedHeader,
    /// Signature valid but older than the replay window.
    TimestampOutsideWindow,
    /// Timestamp in window but MAC does not match body/secret.
    SignatureMismatch,
}

/// Computes the `t=<ts>,v1=<hex>` signature value over `"{ts}.{body}"`.
#[must_use]
pub fn signature_value(secret: &[u8], body: &[u8], unix_ts: i64) -> String {
    let mac = hmac_hex(secret, &timestamp_prefixed(unix_ts, body));
    format!("t={unix_ts},v1={mac}")
}

fn hmac_hex(secret: &[u8], msg: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac accepts any key length");
    mac.update(msg);
    hex::encode(mac.finalize().into_bytes())
}

fn timestamp_prefixed(unix_ts: i64, body: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(24 + body.len());
    msg.extend_from_slice(unix_ts.to_string().as_bytes());
    msg.push(b'.');
    msg.extend_from_slice(body);
    msg
}

/// Receiver-side verification: parses both headers, checks the replay
/// window against `now_unix`, then compares the MAC in constant time.
pub fn verify(
    secret: &[u8],
    body: &[u8],
    signature_header: &str,
    now_unix: i64,
    tolerance_secs: i64,
) -> Result<(), VerifyError> {
    let Some((raw_ts, raw_mac)) = parse_signature(signature_header) else {
        return Err(VerifyError::MalformedHeader);
    };
    if (now_unix - raw_ts).abs() > tolerance_secs {
        return Err(VerifyError::TimestampOutsideWindow);
    }
    let expected = hmac_hex(secret, &timestamp_prefixed(raw_ts, body));
    // Length gate before the timing-sensitive compare; lengths are public.
    if expected.len() != raw_mac.len()
        || !bool::from(
            expected
                .as_bytes()
                .ct_eq(raw_mac.to_ascii_lowercase().as_bytes()),
        )
    {
        return Err(VerifyError::SignatureMismatch);
    }
    Ok(())
}

/// Splits `t=<ts>,v1=<mac>` into `(ts, mac)`.
fn parse_signature(header: &str) -> Option<(i64, &str)> {
    let (t_part, v_part) = header.trim().split_once(',')?;
    let ts = t_part.strip_prefix("t=")?.parse().ok()?;
    let mac = v_part.strip_prefix("v1=")?;
    if mac.is_empty() || !mac.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((ts, mac))
}

/// Retry backoff for delivery attempt n (the attempt that FAILED), in seconds
/// and already jittered: the schedule is the webhook
/// [`RetryPolicy`](crate::retry::RetryPolicy)'s, not a constant here.
///
/// The numbers used to live in this function (30s doubling with the shift
/// clamped at 8) and in `MAX_DELIVERY_ATTEMPTS` beside it; both moved to
/// `crate::retry` so the webhook budget, the queue's and email's cannot drift
/// apart again.
#[must_use]
pub fn backoff_secs(failed_attempt: i32) -> i64 {
    let attempts = u32::try_from(failed_attempt.max(1)).unwrap_or(1);
    crate::retry::policy(crate::retry::Subsystem::Webhook)
        .delay_secs(attempts, crate::retry::sample())
}

/// [`backoff_secs`] with the jitter sample supplied, so a test can pin the
/// bounds without depending on the sample stream.
#[must_use]
pub fn backoff_secs_with(failed_attempt: i32, sample: f64) -> i64 {
    let attempts = u32::try_from(failed_attempt.max(1)).unwrap_or(1);
    crate::retry::policy(crate::retry::Subsystem::Webhook).delay_secs(attempts, sample)
}

/// How many consecutive retryable delivery failures pause a destination.
///
/// It is the webhook policy's own attempt budget, not a second number: one
/// event that exhausts every attempt without a single success has produced
/// exactly this many failures, and an operator reading "the endpoint returned
/// 503 six times in a row" is reading the same evidence either way. A policy
/// widened for a flaky receiver therefore also delays the pause, which is the
/// behaviour an operator who widened it asked for.
///
/// The floor of 2 keeps a one-attempt budget from pausing on a single try:
/// "repeated failures" is the point, and one is not repeated. A destination
/// that accepts nothing still pauses on its second failure.
#[must_use]
pub fn pause_after_failures() -> i32 {
    crate::retry::policy(crate::retry::Subsystem::Webhook)
        .max_attempts_i32(MAX_DELIVERY_ATTEMPTS)
        .max(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"whsec_test";
    const BODY: &[u8] = b"{\"x\":1}";

    /// Independently computed (Python hashlib) reference vector for the
    /// canonical scheme — guards against accidental format drift.
    #[test]
    fn matches_independent_reference_vector() {
        assert_eq!(
            signature_value(SECRET, BODY, 1_700_000_000),
            "t=1700000000,v1=808d84eb394c8f099c0aecdf6c762197e3251fccecf4dd6fad647063de565e81"
        );
    }

    #[test]
    fn verify_accepts_fresh_valid_delivery() {
        let sig = signature_value(SECRET, BODY, 1_700_000_000);
        assert_eq!(verify(SECRET, BODY, &sig, 1_700_000_100, 600), Ok(()));
    }

    #[test]
    fn tampered_body_is_signature_mismatch() {
        let sig = signature_value(SECRET, BODY, 1_700_000_000);
        assert_eq!(
            verify(SECRET, b"{\"x\":2}", &sig, 1_700_000_000, 600),
            Err(VerifyError::SignatureMismatch)
        );
    }

    #[test]
    fn stale_timestamp_outside_window_is_replay_rejected() {
        let sig = signature_value(SECRET, BODY, 1_700_000_000);
        assert_eq!(
            verify(SECRET, BODY, &sig, 1_700_001_000, 600),
            Err(VerifyError::TimestampOutsideWindow)
        );
    }

    #[test]
    fn wrong_secret_and_garbage_headers_fail() {
        let sig = signature_value(b"other", BODY, 1_700_000_000);
        assert_eq!(
            verify(SECRET, BODY, &sig, 1_700_000_000, 600),
            Err(VerifyError::SignatureMismatch)
        );
        for garbage in ["", "deadbeef", "t=abc,v1=ff", "t=1,v1=", "t=1"] {
            assert_eq!(
                verify(SECRET, BODY, garbage, 1_700_000_000, 600),
                Err(VerifyError::MalformedHeader),
                "garbage {garbage:?} must be malformed"
            );
        }
    }

    #[test]
    fn uppercase_hex_signatures_verify() {
        let sig = signature_value(SECRET, BODY, 1_700_000_000);
        let (t_part, v_part) = sig.split_once(',').expect("shape");
        let mac = v_part.strip_prefix("v1=").expect("mac component");
        // Receivers may normalize the MAC's case; prefixes stay literal.
        let upper_mac = format!("{t_part},v1={}", mac.to_uppercase());
        assert_eq!(verify(SECRET, BODY, &upper_mac, 1_700_000_000, 600), Ok(()));
    }

    /// The schedule is the policy's now, so this pins the shape (doubling,
    /// capped, jittered within bounds) rather than five exact seconds values
    /// that would go stale the moment the preset changes.
    #[test]
    fn backoff_schedule_is_pinned_to_the_policy() {
        use crate::retry::{policy, Subsystem};
        let webhook = policy(Subsystem::Webhook);
        // No jitter: the computed curve.
        assert_eq!(backoff_secs_with(1, 1.0), 30);
        assert_eq!(backoff_secs_with(2, 1.0), 60);
        assert_eq!(backoff_secs_with(3, 1.0), 120);
        assert_eq!(
            backoff_secs_with(99, 1.0),
            i64::try_from(webhook.cap.as_secs()).unwrap()
        );
        // Full jitter spans [0, computed] and floors at one second.
        assert_eq!(backoff_secs_with(1, 0.0), 1);
        for failed in 1..10 {
            let upper = backoff_secs_with(failed, 1.0);
            let lower = backoff_secs_with(failed, 0.0);
            assert!(lower >= 1 && lower <= upper, "attempt {failed}");
            let sampled = backoff_secs(failed);
            assert!(
                sampled >= 1 && sampled <= upper,
                "attempt {failed}: {sampled}s outside 1..={upper}s"
            );
        }
        // A nonsensical attempt count waits the first delay, not the last.
        assert!(backoff_secs_with(-4, 1.0) <= 30);
        assert!(backoff_secs_with(0, 1.0) <= 30);
    }

    /// The pause threshold is the policy's attempt budget, not a second
    /// constant: widen the budget and the pause waits longer, which is what an
    /// operator who widened it for a flaky receiver asked for.
    #[test]
    fn the_pause_threshold_is_the_webhook_attempt_budget() {
        use crate::retry::{policy, Subsystem};
        let budget = policy(Subsystem::Webhook).max_attempts_i32(MAX_DELIVERY_ATTEMPTS);
        assert_eq!(pause_after_failures(), budget.max(2));
        // Two failures is the floor, so a single wasted attempt never pauses.
        assert!(pause_after_failures() >= 2);
    }
}
