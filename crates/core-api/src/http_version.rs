//! HTTP conditional-read helpers: weak validators built from the instant-first
//! per-entity write sequence.
//!
//! A weak validator (RFC 9110 §8.8.3) is the right choice here because the
//! wire representation of an entity may differ by negotiation — field
//! selection, ordering, and serialization change between releases and between
//! clients — while the underlying revision is what a caller actually cares
//! about. The monotonic `entity_write_seqs.seq` names that revision exactly,
//! so two responses that agree on the seq are interchangeable for
//! revalidation even if their bytes are not identical; a strong validator
//! would over-promise byte equality this endpoint never makes.

/// Builds the canonical weak validator for one entity revision:
/// `W/"<entity>:<id>:<seq>"`. The opaque tag is deliberately one string with
/// no separators of its own, so it round-trips through [`etag_matches`]
/// without escaping.
pub fn weak_etag(entity: &str, id: &str, seq: i64) -> String {
    format!("W/\"{entity}:{id}:{seq}\"")
}

/// Weak comparison of an `If-None-Match` header value against a validator
/// (RFC 9110 §8.8.3.2). Returns `true` when the caller's cached revision may
/// be reused.
///
/// Rules implemented:
/// - an absent header never matches;
/// - the wildcard `*` matches any existing representation;
/// - otherwise the header is a comma-separated list of entity-tags, and it
///   matches when ANY candidate's opaque tag equals ours;
/// - comparison is weak: a leading `W/` on either side is ignored, so
///   `W/"x"` and `"x"` compare equal.
pub fn etag_matches(header_value: Option<&str>, etag: &str) -> bool {
    let Some(header_value) = header_value else {
        return false;
    };
    let header_value = header_value.trim();
    if header_value == "*" {
        return true;
    }
    let ours = strip_weak(etag);
    header_value
        .split(',')
        .any(|candidate| strip_weak(candidate.trim()) == ours)
}

/// Drops a leading `W/` weakness marker. That prefix is the only difference
/// weak comparison is required to ignore; everything after it (including the
/// quotes) is the opaque tag and must match exactly.
fn strip_weak(tag: &str) -> &str {
    tag.strip_prefix("W/").unwrap_or(tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ETAG: &str = "W/\"ticket:11111111-1111-1111-1111-111111111111:7\"";
    const OPAQUE: &str = "\"ticket:11111111-1111-1111-1111-111111111111:7\"";

    #[test]
    fn weak_etag_has_the_documented_shape() {
        assert_eq!(
            weak_etag("ticket", "abc", 42),
            "W/\"ticket:abc:42\"",
            "entity, id and seq are joined with colons inside a weak tag"
        );
        assert_eq!(weak_etag("ticket", "abc", 0), "W/\"ticket:abc:0\"");
    }

    #[test]
    fn absent_header_never_matches() {
        assert!(!etag_matches(None, ETAG));
    }

    #[test]
    fn exact_weak_match() {
        assert!(etag_matches(Some(ETAG), ETAG));
    }

    #[test]
    fn strong_candidate_matches_against_our_weak_tag() {
        // Our validator is weak; a client echoing the opaque tag without the
        // W/ prefix still revalidates — weak comparison ignores weakness on
        // either side.
        assert!(etag_matches(Some(OPAQUE), ETAG));
    }

    #[test]
    fn weak_candidate_matches_against_our_weak_tag() {
        assert!(etag_matches(Some(ETAG), ETAG));
    }

    #[test]
    fn differing_opaque_tag_does_not_match() {
        let stale = "W/\"ticket:11111111-1111-1111-1111-111111111111:6\"";
        assert!(!etag_matches(Some(stale), ETAG));
    }

    #[test]
    fn a_list_matches_when_any_candidate_matches() {
        let list = format!("\"other:1\", {OPAQUE}, \"third:2\"");
        assert!(etag_matches(Some(list.as_str()), ETAG));
    }

    #[test]
    fn a_list_without_a_match_does_not() {
        let list = "\"other:1\", \"third:2\"";
        assert!(!etag_matches(Some(list), ETAG));
    }

    #[test]
    fn list_whitespace_is_ignored() {
        let list = format!("  \"other:1\" ,\t{ETAG}  ");
        assert!(etag_matches(Some(list.as_str()), ETAG));
    }

    #[test]
    fn wildcard_matches_any_representation() {
        assert!(etag_matches(Some("*"), ETAG));
        assert!(etag_matches(Some("  *  "), ETAG));
        assert!(etag_matches(
            Some("*"),
            weak_etag("ticket", "x", 1).as_str()
        ));
    }

    #[test]
    fn garbage_never_matches() {
        assert!(!etag_matches(Some(""), ETAG));
        assert!(!etag_matches(Some("not-a-tag"), ETAG));
        assert!(!etag_matches(Some("W/"), ETAG));
        // A `*` inside a list is not the wildcard form; it is just a
        // candidate opaque tag that does not equal ours.
        assert!(!etag_matches(Some("\"a\", *"), ETAG));
    }
}
