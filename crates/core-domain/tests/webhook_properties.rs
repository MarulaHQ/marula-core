//! Property tests for webhook delivery primitives: the backoff schedule
//! must stay monotonic, positive and capped for ANY attempt number, and
//! signatures must verify only for exactly (secret, body, timestamp)
//! triples — fuzzed across arbitrary inputs.

use core_domain::webhooks::{
    backoff_secs, signature_value, verify, VerifyError, MAX_DELIVERY_ATTEMPTS,
    REPLAY_TOLERANCE_SECS,
};
use proptest::prelude::*;

proptest! {
    #[test]
    fn backoff_is_positive_monotonic_and_capped(a in -50_i32..500, b in -50_i32..500) {
        let (a, b) = (a.max(1), b.max(1));
        let fa = backoff_secs(a);
        let fb = backoff_secs(b);
        prop_assert!(fa > 0 && fb > 0, "backoff must be positive");
        prop_assert!(fa <= 3600 && fb <= 3600, "backoff must stay under the cap");
        if a <= b {
            prop_assert!(fa <= fb, "schedule must be non-decreasing");
        }
        // The last allowed attempt still schedules a bounded retry.
        prop_assert!(backoff_secs(MAX_DELIVERY_ATTEMPTS) <= 3600);
    }

    #[test]
    fn signatures_verify_only_for_exact_triples(
        secret in proptest::collection::vec(any::<u8>(), 1..64),
        body in proptest::collection::vec(any::<u8>(), 0..512),
        ts in 0_i64..2_000_000_000,
        skew in -3_600_i64..3_600,
    ) {
        let sig = signature_value(&secret, &body, ts);
        // Fresh within window ⇒ valid.
        prop_assert_eq!(verify(&secret, &body, &sig, ts + skew.clamp(-600, 600), REPLAY_TOLERANCE_SECS), Ok(()));
        // Any single-byte body tamper breaks it.
        if !body.is_empty() {
            let mut tampered = body.clone();
            let idx = (ts as usize) % tampered.len();
            tampered[idx] ^= 0x5A;
            prop_assert_ne!(verify(&secret, &tampered, &sig, ts, REPLAY_TOLERANCE_SECS), Ok(()));
        }
        // Outside the window is a replay rejection regardless of MAC.
        prop_assert_eq!(
            verify(&secret, &body, &sig, ts + REPLAY_TOLERANCE_SECS + 1, REPLAY_TOLERANCE_SECS),
            Err(VerifyError::TimestampOutsideWindow)
        );
    }
}
