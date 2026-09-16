//! core-kb — knowledge base domain rules (native KB v1, TASKLIST Phase 3).
//!
//! Purpose: the ONE home for KB behavior that is pure product policy, so
//! core-api routes and (later) portal v2 + the D5 Markdown/HTML importer
//! all enforce identical rules:
//!
//! * [`validate_title`] / [`validate_body`] — size + shape gates before
//!   anything reaches Postgres;
//! * [`slugify`] — the deterministic per-org slug the UNIQUE (org_id,
//!   slug) gate persists;
//! * [`client_viewable`] — the effective client-facing rule (published
//!   AND article flag AND category flag) portal v2 will honor;
//! * [`ensure_content_editable`] — archived articles are read-only until
//!   unarchived.
//!
//! Non-purpose: no SQL (core-db), no HTTP (core-api), no FTS execution —
//! the crate owns rules, not storage or transport.

pub mod content;
pub mod import;
pub mod render;
pub mod rich;

use core_domain::entities::{KbArticle, KbArticleStatus, KbCategory};

/// Upper bound for article titles; long enough for real how-tos, short
/// enough for nav rendering and FTS title weighting to stay meaningful.
pub const MAX_TITLE_CHARS: usize = 300;
/// Upper bound for article bodies (512 KiB) so pathological pastes cannot
/// bloat rows, the FTS index, or the request path.
pub const MAX_BODY_BYTES: usize = 512 * 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone)]
pub enum KbError {
    #[error("title is required")]
    EmptyTitle,
    #[error("title exceeds {MAX_TITLE_CHARS} characters")]
    TitleTooLong,
    #[error("body exceeds the 512 KiB limit")]
    BodyTooLarge,
    #[error("article is archived; unarchive it before editing content")]
    ArchivedLocked,
}

/// Validates an article title (non-empty, within the length bound). The
/// caller trims; trailing whitespace alone is not a title.
pub fn validate_title(title: &str) -> Result<(), KbError> {
    if title.trim().is_empty() {
        return Err(KbError::EmptyTitle);
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(KbError::TitleTooLong);
    }
    Ok(())
}

/// Validates an article body (byte bound; empty bodies are fine — stub
/// articles are a legitimate starting point).
pub fn validate_body(body: &str) -> Result<(), KbError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(KbError::BodyTooLarge);
    }
    Ok(())
}

/// Derives the stable per-org slug from a title (or an explicit slug the
/// caller prefers). Lowercase ASCII word characters, single hyphens,
/// trimmed at both ends; non-ASCII input collapses to `article`. The
/// UNIQUE (org_id, slug) gate remains the collision authority — this
/// function only guarantees a well-formed candidate.
pub fn slugify(input: &str) -> String {
    let mut slug = String::with_capacity(input.len());
    let mut last_hyphen = true; // swallow leading separators
    for ch in input.chars() {
        let mapped = ch.to_ascii_lowercase();
        if mapped.is_ascii_lowercase() || mapped.is_ascii_digit() {
            slug.push(mapped);
            last_hyphen = false;
        } else if !last_hyphen {
            slug.push('-');
            last_hyphen = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("article");
    }
    slug
}

/// The effective client-facing rule: a portal client sees an article only
/// when it is PUBLISHED, the article flag is set, AND its category flag is
/// set. An article without a category is never client-visible (flags feed
/// portal v2; uncategorized drafts of one stay staff-only).
pub fn client_viewable(article: &KbArticle, category: Option<&KbCategory>) -> bool {
    if article.status != KbArticleStatus::Published || !article.client_visible {
        return false;
    }
    match category {
        Some(c) => c.client_visible,
        // Article flag set but the category is gone: treat as invisible
        // rather than leaking content through a deleted gate.
        None => false,
    }
}

/// Guard for content mutations: archived articles are read-only until
/// unarchived (the archive action itself and status-only changes stay
/// legal — the caller passes the fields it intends to touch).
pub fn ensure_content_editable(current: KbArticleStatus) -> Result<(), KbError> {
    if current == KbArticleStatus::Archived {
        return Err(KbError::ArchivedLocked);
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn test_article(status: KbArticleStatus, client_visible: bool) -> KbArticle {
    use core_domain::ids::{KbArticleId, OrgId};
    KbArticle {
        id: KbArticleId::nil(),
        org_id: OrgId::nil(),
        category_id: None,
        title: "Test article".into(),
        slug: "test-article".into(),
        body: String::new(),
        content_jsonb: None,
        status,
        client_visible,
        created_by: None,
        updated_by: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

#[cfg(test)]
pub(crate) fn test_category(client_visible: bool) -> KbCategory {
    use core_domain::ids::{KbCategoryId, OrgId};
    KbCategory {
        id: KbCategoryId::nil(),
        org_id: OrgId::nil(),
        name: "Test category".into(),
        slug: "test-category".into(),
        description: String::new(),
        position: 0,
        client_visible,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_validation_bounds() {
        assert_eq!(validate_title("   "), Err(KbError::EmptyTitle));
        assert_eq!(validate_title(""), Err(KbError::EmptyTitle));
        assert!(validate_title("Reset a lost password").is_ok());
        let long = "x".repeat(MAX_TITLE_CHARS + 1);
        assert_eq!(validate_title(&long), Err(KbError::TitleTooLong));
        let exact = "x".repeat(MAX_TITLE_CHARS);
        assert!(validate_title(&exact).is_ok());
    }

    #[test]
    fn body_validation_is_byte_bound() {
        assert!(validate_body("").is_ok());
        let big = vec![b'a'; MAX_BODY_BYTES + 1];
        assert_eq!(
            validate_body(std::str::from_utf8(&big).unwrap()),
            Err(KbError::BodyTooLarge)
        );
    }

    #[test]
    fn slugify_is_deterministic_and_clean() {
        assert_eq!(slugify("Reset a Lost Password!"), "reset-a-lost-password");
        assert_eq!(slugify("  --VPN  setup (v2)  "), "vpn-setup-v2");
        assert_eq!(slugify("???"), "article");
        assert_eq!(slugify("überwachung"), "berwachung");
        // Already-sluggy input round-trips unchanged.
        assert_eq!(slugify("printer-hop-2"), "printer-hop-2");
    }

    #[test]
    fn client_view_needs_all_three_gates() {
        let category_visible = crate::test_category(true);
        let category_hidden = crate::test_category(false);
        let mut article = crate::test_article(KbArticleStatus::Published, true);

        assert!(client_viewable(&article, Some(&category_visible)));
        // Category gate closed.
        assert!(!client_viewable(&article, Some(&category_hidden)));
        // Article gate closed.
        article.client_visible = false;
        assert!(!client_viewable(&article, Some(&category_visible)));
        // Status gate closed.
        article.client_visible = true;
        article.status = KbArticleStatus::Draft;
        assert!(!client_viewable(&article, Some(&category_visible)));
        // No category at all: never client-visible.
        article.status = KbArticleStatus::Published;
        assert!(!client_viewable(&article, None));
    }

    #[test]
    fn archived_articles_refuse_content_edits() {
        assert_eq!(
            ensure_content_editable(KbArticleStatus::Archived),
            Err(KbError::ArchivedLocked)
        );
        assert!(ensure_content_editable(KbArticleStatus::Draft).is_ok());
        assert!(ensure_content_editable(KbArticleStatus::Published).is_ok());
    }
}
