//! Webhook OUT delivery primitives (TASKLIST 2.15): the signed-payload
//! scheme receivers verify, the anti-replay window, and the retry backoff
//! schedule.
//!
//! Purpose: pure, framework-free rules for HMAC-signed webhook delivery.
//! The signature covers `"{unix_timestamp}.{raw_body}"` under the
//! subscription secret and travels as `t=<ts>,v1=<hex>` in
//! `X-Meridian-Signature` plus a redundant `X-Meridian-Timestamp` header —
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
pub const SIGNATURE_HEADER: &str = "X-Meridian-Signature";
/// Redundant plain timestamp header (mirrors the `t=` component).
pub const TIMESTAMP_HEADER: &str = "X-Meridian-Timestamp";

/// Delivery attempts before a job dead-letters (first try + 5 retries).
pub const MAX_DELIVERY_ATTEMPTS: i32 = 6;

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

/// Retry backoff for delivery attempt n (the attempt that FAILED):
/// 30s, 60s, 120s … capped at one hour. Pure so property tests can pin
/// monotonicity and the cap regardless of host clocks.
#[must_use]
pub fn backoff_secs(failed_attempt: i32) -> i64 {
    let shift = failed_attempt.clamp(1, 8) as u32 - 1;
    30_i64.saturating_mul(1 << shift).min(3_600)
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

    #[test]
    fn backoff_schedule_is_pinned() {
        assert_eq!(backoff_secs(1), 30);
        assert_eq!(backoff_secs(2), 60);
        assert_eq!(backoff_secs(3), 120);
        assert_eq!(backoff_secs(8), 3600);
        assert_eq!(backoff_secs(99), 3600);
    }
}
