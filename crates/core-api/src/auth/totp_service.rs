//! TOTP (RFC 6238) verification service on top of the `totp-rs` crate.
//!
//! Purpose: generate enrollment material, produce otpauth URIs, and verify
//! codes with a ±1 window while reporting WHICH timestep matched — the
//! matched step feeds the replay watermark stored on `totp_secrets`.
//! Non-purpose: persistence (core-db/mfa) and HTTP concerns.

use rand::RngCore;
use totp_rs::{Algorithm, TOTP};

pub const STEP_SECONDS: u64 = 30;
/// Accept one step of clock drift in each direction (industry standard).
pub const DRIFT_STEPS: i64 = 1;

/// A freshly generated enrollment candidate: raw secret bytes + their
/// base32 form + the standard otpauth URI for authenticator apps.
pub struct Enrollment {
    pub secret_bytes: Vec<u8>,
    pub secret_base32: String,
    pub otpauth_url: String,
}

/// Generates a new 160-bit TOTP secret (RFC 4226 recommends ≥160 bits).
pub fn new_enrollment(account_email: &str, issuer: &str) -> Result<Enrollment, String> {
    let mut bytes = vec![0u8; 20];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let b32 = data_encoding::BASE32_NOPAD.encode(&bytes);
    let totp = build_totp(&bytes, account_email, issuer)?;
    Ok(Enrollment {
        secret_bytes: bytes,
        secret_base32: b32,
        otpauth_url: totp.get_url(),
    })
}

fn build_totp(secret_bytes: &[u8], account_name: &str, issuer: &str) -> Result<TOTP, String> {
    TOTP::new(
        Algorithm::SHA1,
        6,
        u8::try_from(DRIFT_STEPS).unwrap_or(1),
        STEP_SECONDS,
        secret_bytes.to_vec(),
        Some(issuer.to_string()),
        account_name.to_string(),
    )
    .map_err(|e| e.to_string())
}

fn instance_for_verification(secret_base32: &str, account_name: &str) -> Result<TOTP, String> {
    let bytes = data_encoding::BASE32_NOPAD
        .decode(secret_base32.to_ascii_uppercase().as_bytes())
        .map_err(|_| "stored secret is not valid base32".to_string())?;
    build_totp(&bytes, account_name, "Marula")
}

/// Verifies `code` against the current time with ±[`DRIFT_STEPS`] drift.
/// Returns the matching ABSOLUTE timestep on success — callers persist it
/// as the replay watermark and reject any code at or below it.
///
/// The comparison is constant-time per candidate (`subtle` via slice eq on
/// fixed-width digit strings).
pub fn verify_code(
    secret_base32: &str,
    account_name: &str,
    code: &str,
    now_unix: u64,
) -> Result<Option<i64>, String> {
    let trimmed = code.trim();
    if trimmed.len() != 6 || !trimmed.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(None);
    }
    let totp = instance_for_verification(secret_base32, account_name)?;
    let current_step = (now_unix / STEP_SECONDS) as i64;
    for delta in -DRIFT_STEPS..=DRIFT_STEPS {
        let step = current_step + delta;
        let candidate = totp.generate((step.unsigned_abs()) * STEP_SECONDS);
        // Constant-time comparison of equal-length digit strings.
        if subtle_ct_eq(candidate.as_bytes(), trimmed.as_bytes()) {
            return Ok(Some(step));
        }
    }
    Ok(None)
}

fn subtle_ct_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.ct_eq(b).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 Appendix B test seed ("12345678901234567890" ASCII),
    /// base32-encoded.
    const RFC_SEED_B32: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
    const RFC_ACCOUNT: &str = "rfc6238@test";

    fn rfc_code_at(unix: u64) -> String {
        // RFC vectors are 8-digit SHA1; derive the 8-digit expectation the
        // same way the RFC does, then compare against our service in
        // 8-digit mode by building TOTP directly.
        let secret = data_encoding::BASE32_NOPAD
            .decode(RFC_SEED_B32.as_bytes())
            .unwrap();
        let totp = TOTP::new(
            Algorithm::SHA1,
            8,
            0,
            STEP_SECONDS,
            secret,
            None,
            "rfc6238@test".to_string(),
        )
        .unwrap();
        totp.generate(unix)
    }

    #[test]
    fn rfc6238_vectors_match_reference_implementation() {
        // RFC 6238 Appendix B (SHA-1): T → expected HOTP truncation.
        // We assert our construction reproduces the RFC's own algorithm by
        // checking the documented pairs through the same primitive the
        // crate uses for 8 digits.
        let cases: [(u64, &str); 6] = [
            (59, "94287082"),
            (1_111_111_109, "07081804"),
            (1_111_111_111, "14050471"),
            (1_234_567_890, "89005924"),
            (2_000_000_000, "69279037"),
            (20_000_000_000, "65353130"),
        ];
        for (unix, expected) in cases {
            assert_eq!(rfc_code_at(unix), expected, "T={unix}");
        }
    }

    #[test]
    fn six_digit_verification_matches_and_reports_timestep() {
        let unix: u64 = 59;
        let expected6: String = {
            let full = rfc_code_at(unix);
            // RFC 6-digit value is the last 6 of the 31-bit truncation;
            // totp-rs computes it directly, so derive via its own output.
            let secret = data_encoding::BASE32_NOPAD
                .decode(RFC_SEED_B32.as_bytes())
                .unwrap();
            let t = TOTP::new(
                Algorithm::SHA1,
                6,
                0,
                STEP_SECONDS,
                secret,
                None,
                "rfc6238@test".to_string(),
            )
            .unwrap();
            let _ = full;
            t.generate(unix)
        };
        let got = verify_code(RFC_SEED_B32, RFC_ACCOUNT, &expected6, unix)
            .unwrap()
            .expect("code must verify");
        assert_eq!(got, (unix / STEP_SECONDS) as i64);
    }

    #[test]
    fn drift_window_is_accepted() {
        let unix: u64 = 1_111_111_109;
        let secret = data_encoding::BASE32_NOPAD
            .decode(RFC_SEED_B32.as_bytes())
            .unwrap();
        let t = TOTP::new(
            Algorithm::SHA1,
            6,
            0,
            STEP_SECONDS,
            secret,
            None,
            "rfc6238@test".to_string(),
        )
        .unwrap();
        let next_window = t.generate(unix + STEP_SECONDS);
        // Verifying one window ahead of the token's own step must succeed.
        let got = verify_code(RFC_SEED_B32, RFC_ACCOUNT, &next_window, unix)
            .unwrap()
            .expect("±1 drift accepted");
        assert_eq!(got, ((unix + STEP_SECONDS) / STEP_SECONDS) as i64);
    }

    #[test]
    fn wrong_and_malformed_codes_reject() {
        assert_eq!(
            verify_code(RFC_SEED_B32, RFC_ACCOUNT, "000000", 59).unwrap(),
            None
        );
        assert_eq!(
            verify_code(RFC_SEED_B32, RFC_ACCOUNT, "12345", 59).unwrap(),
            None
        );
        assert_eq!(
            verify_code(RFC_SEED_B32, RFC_ACCOUNT, "abcdef", 59).unwrap(),
            None
        );
        assert!(verify_code("not!valid!base32!", RFC_ACCOUNT, "123456", 59).is_err());
    }

    #[test]
    fn enrollment_material_is_unique_and_urlshaped() {
        let a = new_enrollment("user@x.test", "Marula").unwrap();
        let b = new_enrollment("user@x.test", "Marula").unwrap();
        assert_ne!(a.secret_base32, b.secret_base32);
        assert!(a.otpauth_url.starts_with("otpauth://totp/"));
        assert!(a.otpauth_url.contains("issuer=Marula"));
        assert!(a.secret_bytes.len() >= 20);
    }
}
