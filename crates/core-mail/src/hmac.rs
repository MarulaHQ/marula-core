//! HMAC-SHA256 shared-secret verification for the inbound email webhook
//! and the internal worker↔core-api send pipeline.
//!
//! Purpose: sign/verify raw request bytes with a constant-time comparison,
//! so a forged or replayed-tampered webhook is rejected before any
//! processing. The same helper authenticates worker result callbacks.
//!
//! Non-purpose: no key rotation window, no timestamped anti-replay (the
//! message-id idempotency ledger covers replay); v1 keeps the scheme
//! deliberately simple — hex HMAC over the exact request body.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Hex-encoded HMAC-SHA256 of `body` under `secret`.
#[must_use]
pub fn sign(secret: &[u8], body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac accepts any key length");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Constant-time verification of a presented hex signature.
#[must_use]
pub fn verify(secret: &[u8], body: &[u8], presented: &str) -> bool {
    let expected = sign(secret, body);
    // Length mismatch short-circuits BEFORE the timing-sensitive compare;
    // lengths are not secret.
    if presented.len() != expected.len() {
        return false;
    }
    subtle::ConstantTimeEq::ct_eq(
        expected.as_bytes(),
        presented.to_ascii_lowercase().as_bytes(),
    )
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_case_insensitivity() {
        let sig = sign(b"secret", b"body");
        assert!(verify(b"secret", b"body", &sig));
        assert!(verify(b"secret", b"body", &sig.to_uppercase()));
    }

    #[test]
    fn wrong_secret_body_or_garbage_rejected() {
        let sig = sign(b"secret", b"body");
        assert!(!verify(b"other", b"body", &sig));
        assert!(!verify(b"secret", b"tampered", &sig));
        assert!(!verify(b"secret", b"body", "deadbeef"));
        assert!(!verify(b"secret", b"body", ""));
        assert!(!verify(b"secret", b"body", "zz"));
    }
}
