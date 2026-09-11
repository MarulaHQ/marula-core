//! Reciprocal Rank Fusion for hybrid retrieval (§4.5: one search API —
//! FTS + vector fused by RRF).
//!
//! Purpose: the pure ranking math behind Meridian's hybrid search. Lexical
//! (websearch FTS + trigram) and vector (pgvector cosine) legs each produce
//! a ranked candidate list; this module folds them into ONE ranked list so
//! a hit strong on either leg surfaces, and a hit strong on both floats.
//!
//! Non-purpose: no SQL, no provider calls, no permissions — core-db feeds
//! the candidate lists, core-api gates them.

use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Standard RRF damping constant (Cormack et al. 2009): the top hit of one
/// leg contributes 1/(K+1), decaying smoothly with rank. 60 is the widely
/// used default and keeps no single leg from dominating.
pub const RRF_K: f64 = 60.0;

/// One fusionable search key: `(entity_kind, entity_id)` — the same identity
/// the `search_index` / `search_embeddings` tables key on.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SearchKey {
    pub entity_kind: String,
    pub entity_id: Uuid,
}

impl SearchKey {
    pub fn new(entity_kind: impl Into<String>, entity_id: Uuid) -> Self {
        Self {
            entity_kind: entity_kind.into(),
            entity_id,
        }
    }
}

/// Fuses ranked candidate lists (each ordered best-first) into one ranked
/// list of `(key, score)` pairs. Every leg contributes `1/(K + rank + 1)`
/// per distinct key; keys found in several legs SUM their contributions.
///
/// Deterministic on every axis: duplicate keys inside one leg count once
/// (a later duplicate never double-boosts), and ties break by entity_kind
/// then entity_id ascending, so replays and cross-DB reruns order the same.
pub fn reciprocal_rank_fusion(legs: &[&[SearchKey]]) -> Vec<(SearchKey, f64)> {
    let mut scores: HashMap<SearchKey, f64> = HashMap::new();
    for leg in legs {
        let mut seen: HashSet<&SearchKey> = HashSet::new();
        for (rank, key) in leg.iter().enumerate() {
            if !seen.insert(key) {
                continue;
            }
            *scores.entry(key.clone()).or_insert(0.0) += 1.0 / (RRF_K + rank as f64 + 1.0);
        }
    }
    let mut fused: Vec<(SearchKey, f64)> = scores.into_iter().collect();
    fused.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    fused
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u64) -> SearchKey {
        SearchKey::new("ticket", Uuid::from_u128(n as u128))
    }

    #[test]
    fn top_hits_of_both_legs_fuse_to_the_top_of_the_list() {
        let lexical = [key(2), key(1), key(3)];
        let vector = [key(2), key(4)];
        let fused = reciprocal_rank_fusion(&[&lexical, &vector]);
        // 2 is rank 0 on BOTH legs: 1/61 + 1/61 beats any single-leg share
        // (1 at 1/61 + 1/62, 4 only at 1/62). 4 still survives the fusion.
        assert_eq!(fused[0].0, key(2));
        assert!(
            fused.iter().any(|(k, _)| *k == key(4)),
            "union, not intersection"
        );
        assert_eq!(fused.len(), 4, "every candidate survives the fusion");
    }

    #[test]
    fn single_leg_degrades_to_that_legs_order() {
        let lexical = [key(3), key(1), key(2)];
        let fused = reciprocal_rank_fusion(&[&lexical]);
        let keys: Vec<SearchKey> = fused.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(keys, vec![key(3), key(1), key(2)]);
        assert!(fused[0].1 > fused[1].1, "scores strictly decay with rank");
    }

    #[test]
    fn duplicate_key_within_one_leg_counts_once() {
        let leg_a = [key(1)];
        let leg_a_dup = [key(1), key(1)];
        let solo = reciprocal_rank_fusion(&[&leg_a])[0].1;
        let doubled = reciprocal_rank_fusion(&[&leg_a_dup])[0].1;
        assert_eq!(solo, doubled, "a replayed candidate must not double-boost");
    }

    #[test]
    fn contributions_sum_across_legs() {
        let a = [key(1)];
        let b = [key(1)];
        let expected = 2.0 / (RRF_K + 1.0);
        let fused = reciprocal_rank_fusion(&[&a, &b]);
        assert_eq!(fused.len(), 1);
        assert!((fused[0].1 - expected).abs() < 1e-12);
    }

    #[test]
    fn rank_decay_is_smooth_and_ordered() {
        let leg: Vec<SearchKey> = (0..10).map(|n| key(n as u64)).collect();
        let fused = reciprocal_rank_fusion(&[&leg]);
        for pair in fused.windows(2) {
            assert!(pair[0].1 > pair[1].1, "scores must strictly decay");
        }
        assert!((fused[0].1 - 1.0 / (RRF_K + 1.0)).abs() < 1e-12);
        assert!((fused[1].1 - 1.0 / (RRF_K + 2.0)).abs() < 1e-12);
    }

    #[test]
    fn equal_scores_break_deterministically_by_key() {
        // Two legs, one candidate each at the same rank ⇒ equal scores.
        let a = [key(9)];
        let b = [key(4)];
        let fused = reciprocal_rank_fusion(&[&a, &b]);
        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].1, fused[1].1);
        assert_eq!(fused[0].0, key(4), "tie breaks on entity_kind, then id");
    }

    #[test]
    fn empty_legs_yield_empty_fusion() {
        assert!(reciprocal_rank_fusion(&[]).is_empty());
        assert!(reciprocal_rank_fusion(&[&[]]).is_empty());
        let a = [key(1)];
        let fused = reciprocal_rank_fusion(&[&a, &[]]);
        assert_eq!(fused.len(), 1);
    }
}
