//! Property tests for the provider-agnostic inbound logic and backoff
//! math (core-mail). Arbitrary inputs must never break the invariants the
//! pipeline relies on: token extraction is bracket-balanced-safe, thread
//! keys are normalized + deduplicated, threading precedence holds for any
//! lookup result, and backoff is monotone and capped.

use core_mail::inbound::{
    extract_subject_tokens, normalize_message_key, plan_inbound, InboundEmail, InboundOutcome,
};
use core_mail::outbound::{backoff_delay, excerpt};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Extraction never panics on arbitrary subjects and only returns
    /// non-empty, bracket-free tokens.
    #[test]
    fn token_extraction_is_total_and_clean(
        subject in "\\PC{0,80}",
    ) {
        for token in extract_subject_tokens(&subject) {
            prop_assert!(!token.is_empty());
            prop_assert!(!token.contains('['));
            prop_assert!(!token.contains(']'));
        }
    }

    /// Tokens extracted from a well-formed subject round-trip: re-wrapping
    /// them in brackets and re-extracting yields the same set.
    #[test]
    fn token_extraction_roundtrips(
        tokens in proptest::collection::vec("[a-zA-Z0-9-]{1,12}", 0..4),
        noise in r"[^\[\]]{0,20}",
    ) {
        // Extraction preserves first-occurrence order and deduplicates.
        let mut distinct: Vec<String> = Vec::new();
        for t in &tokens {
            if !distinct.contains(t) {
                distinct.push(t.clone());
            }
        }
        let subject = tokens
            .iter()
            .map(|t| format!("[{t}] "))
            .collect::<String>()
            + &noise;
        assert_eq!(extract_subject_tokens(&subject), distinct);
    }

    /// Thread keys: normalization strips brackets/whitespace/case; output
    /// is deduplicated regardless of input duplication.
    #[test]
    fn thread_keys_normalize_and_dedupe(
        raw_ids in proptest::collection::vec("[<>a-zA-Z0-9@.\\- ]{0,30}", 0..8),
    ) {
        let msg = InboundEmail {
            rfc_message_id: "seed@x".into(),
            in_reply_to: Some(raw_ids.first().cloned().unwrap_or_default()),
            references: raw_ids.clone(),
            subject: None,
            from_address: "a@b.c".into(),
            from_name: None,
            to: vec![],
            cc: vec![],
            text: None,
            html: None,
            received_at: None,
        };
        let keys = msg.thread_keys();
        let normalized_all: Vec<String> =
            raw_ids.iter().map(|r| normalize_message_key(r)).filter(|k| !k.is_empty()).collect();
        // Every expected key survives (order preserved, dedup applied).
        let mut expected: Vec<String> = Vec::new();
        for k in normalized_all {
            if !expected.contains(&k) {
                expected.push(k);
            }
        }
        prop_assert_eq!(keys, expected);
    }

    /// Threading precedence is total: explicit thread beats token beats
    /// new-ticket, for ANY pair of lookup results.
    #[test]
    fn precedence_holds_for_arbitrary_lookups(
        a in proptest::option::of(proptest::num::u128::ANY),
        b in proptest::option::of(proptest::num::u128::ANY),
    ) {
        let uuid = |n: u128| uuid::Uuid::from_u128(n);
        match (a.map(uuid), b.map(uuid)) {
            (Some(t), _) => assert!(matches!(
                plan_inbound(a.map(uuid), b.map(uuid)),
                InboundOutcome::CommentOn { ticket_id } if ticket_id == t
            )),
            (None, Some(k)) => assert_eq!(
                plan_inbound(None, b.map(uuid)),
                InboundOutcome::AttachByToken { ticket_id: k }
            ),
            (None, None) => {
                assert_eq!(plan_inbound(None, None), InboundOutcome::NewTicket)
            }
        }
    }

    /// Backoff never decreases with more attempts and never exceeds 1h.
    #[test]
    fn backoff_is_monotone_and_capped(
        a in 0_i64..64,
        step in 1_i64..8,
    ) {
        let lo = backoff_delay(a);
        let hi = backoff_delay(a + step);
        prop_assert!(lo <= hi, "backoff decreased: {lo:?} > {hi:?}");
        prop_assert!(backoff_delay(i64::from(1000)) <= chrono::Duration::seconds(3600));
    }

    /// Excerpting never panics, respects the char budget (+ellipsis), and
    /// preserves short inputs verbatim.
    #[test]
    fn excerpt_respects_budget(
        body in "\\PC{0,400}",
        max in 1_usize..120,
    ) {
        let out = excerpt(&body, max);
        if body.trim().chars().count() <= max {
            prop_assert_eq!(out, body.trim());
        } else {
            prop_assert!(out.chars().count() <= max + 1);
            prop_assert!(out.ends_with('…'));
        }
    }
}
