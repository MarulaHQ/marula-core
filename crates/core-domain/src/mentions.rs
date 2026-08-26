//! @mention parsing and org-member resolution.
//!
//! Purpose: the pure text layer of ticket-comment mentions — tokenize
//! `@handle` occurrences out of free-form note bodies and resolve them
//! against the org member directory (active members only), ignoring the
//! author's own handle and anything that does not match a visible member.
//!
//! Non-purpose: no persistence, no notification creation (core-api owns the
//! transactional write), and no board/client-level visibility rules — the
//! caller passes an already-visibility-filtered member directory; when
//! board RBAC lands it narrows the directory, not this module.

/// One `@handle` occurrence in a note body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    /// Handle text between `@` and the token end, original casing.
    pub handle: String,
    /// Byte offset of the `@` in the source body.
    pub start: usize,
    /// Byte offset one past the last handle byte.
    pub end: usize,
}

impl Mention {
    /// Lowercased handle for case-insensitive directory matching.
    pub fn lowercase(&self) -> String {
        self.handle.to_lowercase()
    }
}

/// A mentionable character inside a handle.
fn is_handle_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

/// True when the character before the `@` permits a mention start.
/// Word characters (and a second `@`) mean we are inside an email address
/// or another token — not a mention.
fn is_mention_boundary(prev: Option<char>) -> bool {
    match prev {
        None => true,
        Some(c) => !(c.is_alphanumeric() || c == '@' || c == '_'),
    }
}

/// Parses every distinct `@handle` occurrence in `body`, in first-seen
/// order.
///
/// Edge cases handled:
/// - punctuation adjacency (`@bob,` / `(@carol)` / `@dan!`) — trailing
///   punctuation is not part of the handle;
/// - email addresses (`mail me at foo@bar.com`) — `@` preceded by a word
///   character never starts a mention;
/// - doubles and empties (`@@x`, `@`, `@.`) are skipped;
/// - internal dots allowed (handles like `a.b`), trailing dots trimmed;
/// - duplicates fold case-insensitively, first occurrence wins.
pub fn parse(body: &str) -> Vec<Mention> {
    let mut out: Vec<Mention> = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0_usize;
    while i < bytes.len() {
        if bytes[i] != b'@' {
            i += 1;
            continue;
        }
        let prev = body[..i].chars().next_back();
        if !is_mention_boundary(prev) {
            i += 1;
            continue;
        }
        // Collect the longest run of handle characters after the `@`.
        let content_start = i + 1;
        let mut content_end = content_start;
        for (offset, c) in body[content_start..].char_indices() {
            if is_handle_char(c) {
                // `.char_indices()` yields byte offsets into the slice.
                content_end = content_start + offset + c.len_utf8();
            } else {
                break;
            }
        }
        // Trailing dots belong to sentence punctuation, not the handle.
        let mut handle_end = content_end;
        while handle_end > content_start && bytes[handle_end - 1] == b'.' {
            handle_end -= 1;
        }
        let start = i;
        i = content_end.max(i + 1);
        if handle_end == content_start {
            continue;
        }
        let handle = body[content_start..handle_end].to_string();
        if handle.len() > 100 {
            continue;
        }
        let lowered = handle.to_lowercase();
        if out.iter().any(|m| m.lowercase() == lowered) {
            continue;
        }
        out.push(Mention {
            handle,
            start,
            end: handle_end,
        });
    }
    out
}

/// A mentionable org member: their id plus every handle they answer to.
/// Built by the caller from the visibility-respected member list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberHandles {
    pub user_id: uuid::Uuid,
    pub handles: Vec<String>,
}

impl MemberHandles {
    /// Directory entry from a user's identity: the email local-part and
    /// the slug of the display name (`Alice Chen` → `alice-chen`).
    pub fn from_identity(user_id: uuid::Uuid, email: &str, display_name: &str) -> Self {
        let local = email.split('@').next().unwrap_or_default().to_lowercase();
        let slug = display_name
            .trim()
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-");
        let mut handles = Vec::new();
        if !local.is_empty() {
            handles.push(local);
        }
        if !slug.is_empty() && !handles.contains(&slug) {
            handles.push(slug);
        }
        Self { user_id, handles }
    }
}

/// Resolves parsed mentions to distinct mentioned-user ids, preserving
/// first-mention order.
///
/// Resolution rules: matching is case-insensitive across each member's
/// whole handle set; unknown handles are ignored; members whose handle set
/// is empty cannot be mentioned; self-mentions (the author) are ignored.
pub fn resolve(
    mentions: &[Mention],
    members: &[MemberHandles],
    author: uuid::Uuid,
) -> Vec<uuid::Uuid> {
    let mut resolved: Vec<uuid::Uuid> = Vec::new();
    for mention in mentions {
        let needle = mention.lowercase();
        let hit = members
            .iter()
            .find(|member| member.user_id != author && member.handles.iter().any(|h| h == &needle));
        if let Some(member) = hit {
            if !resolved.contains(&member.user_id) {
                resolved.push(member.user_id);
            }
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use uuid::Uuid;

    fn handles(hs: &[&str]) -> MemberHandles {
        MemberHandles {
            user_id: Uuid::now_v7(),
            handles: hs.iter().map(|h| h.to_string()).collect(),
        }
    }

    #[test]
    fn parses_simple_and_punctuated_mentions() {
        let body = "ping @alice and @bob, cc (@carol) — thanks @dan!";
        let parsed = parse(body);
        let found: Vec<String> = parsed.iter().map(|m| m.handle.clone()).collect();
        assert_eq!(found, vec!["alice", "bob", "carol", "dan"]);
    }

    #[test]
    fn emails_are_not_mentions() {
        assert!(parse("contact foo@bar.com about it").is_empty());
        assert!(parse("x@y.z").is_empty());
    }

    #[test]
    fn empty_and_double_at_are_skipped() {
        assert!(parse("@ @ @@x @.").is_empty());
    }

    #[test]
    fn duplicates_fold_case_first_wins() {
        let parsed = parse("@alice and @ALICE and @Alice");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].handle, "alice");
    }

    #[test]
    fn internal_dots_survive_trailing_dots_trim() {
        let parsed = parse("hey @a.b.");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].handle, "a.b");
    }

    #[test]
    fn offsets_point_at_the_handle() {
        let body = "hi @bob!";
        let parsed = parse(body);
        assert_eq!(parsed.len(), 1);
        assert_eq!(&body[parsed[0].start..parsed[0].end], "@bob");
    }

    #[test]
    fn unicode_word_before_at_blocks_the_mention() {
        assert!(parse("café@bar.com").is_empty());
    }

    #[test]
    fn resolves_across_email_and_display_name_handles() {
        let alice = Uuid::now_v7();
        let bob = Uuid::now_v7();
        let author = Uuid::now_v7();
        let members = vec![
            MemberHandles::from_identity(alice, "alice@acme.test", "Alice Chen"),
            MemberHandles::from_identity(bob, "bob.jones@acme.test", "Bob Jones"),
        ];
        let mentions = parse("@alice-chen please triage; cc @bob.jones and @ghost");
        let resolved = resolve(&mentions, &members, author);
        assert_eq!(resolved, vec![alice, bob]);
    }

    #[test]
    fn self_mention_is_ignored() {
        let author = Uuid::now_v7();
        let members = vec![MemberHandles::from_identity(author, "me@acme.test", "Me")];
        let mentions = parse("note to @me");
        assert!(resolve(&mentions, &members, author).is_empty());
    }

    #[test]
    fn unknown_handles_resolve_to_nothing() {
        let members = vec![handles(&["alice"])];
        assert!(resolve(&parse("@nobody"), &members, Uuid::now_v7()).is_empty());
    }

    proptest! {
        /// The parser never panics on arbitrary input and only emits
        /// well-formed, bounded, deduplicated handles.
        #[test]
        fn parse_is_total_and_wellformed(body in ".*") {
            let parsed = parse(&body);
            let mut seen = std::collections::HashSet::new();
            for m in &parsed {
                prop_assert!(!m.handle.is_empty());
                prop_assert!(m.handle.len() <= 100);
                prop_assert!(m.handle.chars().all(is_handle_char));
                prop_assert!(!m.handle.ends_with('.'));
                prop_assert!(seen.insert(m.lowercase()));
                prop_assert_eq!(&body[m.start..m.end], format!("@{}", m.handle));
            }
        }

        /// Emails embedded anywhere never leak their domain as a handle.
        #[test]
        fn email_domains_never_become_handles(prefix in "[a-z]{0,8}", domain in "[a-z]{2,8}") {
            let body = format!("{prefix}name@{domain}.com");
            for m in parse(&body) {
                prop_assert!(m.handle.as_str() != domain, "email domain leaked as handle");
            }
        }
    }
}
