//! Provider-agnostic inbound email model and routing decisions.
//!
//! Purpose: the canonical webhook payload (`InboundEmail` — providers'
//! native formats are adapted at the edge into this shape), the threading
//! key set (References ∪ In-Reply-To), subject-token extraction for the
//! `[TCK-123]` fallback, and the pure precedence decision of what an
//! inbound message becomes.
//!
//! Non-purpose: no persistence, no org/board address resolution (core-db
//! owns lookups; core-api orchestrates) — this module only computes from
//! values handed to it so every rule is unit/proptest-testable.

use chrono::{DateTime, Utc};
use serde::Deserialize;

/// Canonical normalized inbound message. The HMAC-authenticated webhook in
/// core-api deserializes POST bodies into exactly this struct.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct InboundEmail {
    /// RFC 5322 Message-ID including angle brackets when the provider
    /// preserves them; uniqueness is enforced per org by the ledger.
    #[serde(rename = "message_id")]
    pub rfc_message_id: String,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(rename = "from")]
    pub from_address: String,
    #[serde(default)]
    pub from_name: Option<String>,
    #[serde(default)]
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub html: Option<String>,
    #[serde(default)]
    pub received_at: Option<DateTime<Utc>>,
    /// Provider-extracted attachments (base64 bodies). Optional and
    /// defaulted so existing webhook senders keep working unchanged
    /// (the private build tracker 2.14).
    #[serde(default)]
    pub attachments: Vec<InboundAttachment>,
}

/// One provider-extracted attachment: metadata plus base64 body. Marula
/// deliberately does no MIME parsing in v1 — extraction is the edge's job.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct InboundAttachment {
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub content_type: String,
    /// Base64 (standard alphabet, padding optional-tolerant) of the bytes.
    pub content_base64: String,
}

impl InboundAttachment {
    /// Decodes the base64 body. Errors are per-attachment so one broken
    /// part cannot poison the whole message.
    pub fn decoded(&self) -> Result<Vec<u8>, base64::DecodeError> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.decode(self.content_base64.as_bytes())
    }
}

impl InboundEmail {
    /// Structural validation before any processing: the fields idempotency
    /// and routing depend on must be present.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.rfc_message_id.trim().is_empty() {
            return Err("message_id is required");
        }
        if self.from_address.trim().is_empty() {
            return Err("from is required");
        }
        if self.attachments.len() > MAX_INBOUND_ATTACHMENTS {
            return Err("too many attachments");
        }
        for a in &self.attachments {
            if a.filename.trim().is_empty() {
                return Err("attachment filename is required");
            }
            if a.content_base64.is_empty() {
                return Err("attachment body is required");
            }
        }
        Ok(())
    }

    /// Attachment count (structural convenience for pipeline logging).
    #[must_use]
    pub fn attachment_count(&self) -> usize {
        self.attachments.len()
    }

    /// Best-effort plain text body: `text` when present, else a naive
    /// tag-stripped HTML fallback. Documented v1 limitation: no full MIME
    /// or CSS handling.
    #[must_use]
    pub fn effective_text(&self) -> Option<String> {
        if let Some(t) = self.text.as_deref().filter(|t| !t.trim().is_empty()) {
            return Some(t.to_string());
        }
        self.html.as_deref().map(strip_html_naive)
    }

    /// Threading keys to match against prior messages: References first
    /// (most-recent-last preserved), then In-Reply-To, deduplicated.
    #[must_use]
    pub fn thread_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        for k in self.references.iter().chain(self.in_reply_to.iter()) {
            let k = normalize_message_key(k);
            if !k.is_empty() && !keys.contains(&k) {
                keys.push(k);
            }
        }
        keys
    }

    /// Recipient addresses in priority order (to before cc), lowercased.
    #[must_use]
    pub fn recipients_in_priority_order(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for addr in self.to.iter().chain(self.cc.iter()) {
            let addr = addr.trim().to_ascii_lowercase();
            if !addr.is_empty() && !out.contains(&addr) {
                out.push(addr);
            }
        }
        out
    }
}

/// Hard ceiling on attachments per inbound message — a structural bound,
/// not policy (per-part and total size caps live in core-api config).
pub const MAX_INBOUND_ATTACHMENTS: usize = 10;

/// Message-IDs may or may not keep their RFC angle brackets depending on
/// the provider; comparisons normalize by trimming whitespace and brackets
/// and lowercasing (Message-IDs are case-sensitive per RFC but providers
/// routinely case-fold — matching conservatively on exact-lower form).
#[must_use]
pub fn normalize_message_key(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_ascii_lowercase()
}

/// Extracts bracketed subject tokens: `Re: [TCK-12] printer` → `["TCK-12"]`.
/// Pure scan, no regex dependency; tokens are returned trimmed, deduplicated
/// in first-occurrence order. Nested/unbalanced bracket artifacts (a token
/// that still contains a bracket) are discarded defensively.
#[must_use]
pub fn extract_subject_tokens(subject: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = subject;
    while let Some(open) = rest.find('[') {
        let Some(close_rel) = rest[open..].find(']') else {
            break;
        };
        let token = rest[open + 1..open + close_rel].trim();
        if !token.is_empty()
            && !token.contains('[')
            && !token.contains(']')
            && !out.iter().any(|t: &String| t == token)
        {
            out.push(token.to_string());
        }
        rest = &rest[open + close_rel + 1..];
    }
    out
}

/// What an inbound message should become, decided purely from resolved
/// lookups (the caller performs DB resolution, this decides precedence).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundOutcome {
    /// Reply threads onto an existing ticket (References/In-Reply-To hit).
    CommentOn { ticket_id: uuid::Uuid },
    /// Subject token matched an existing ticket number.
    AttachByToken { ticket_id: uuid::Uuid },
    /// No thread and no token: create a ticket on the routed board.
    NewTicket,
}

/// Precedence: explicit threading beats subject token beats new-ticket.
/// Unroutable orgs are rejected earlier (no route ⇒ no processing).
#[must_use]
pub fn plan_inbound(
    thread_match: Option<uuid::Uuid>,
    token_match: Option<uuid::Uuid>,
) -> InboundOutcome {
    match (thread_match, token_match) {
        (Some(ticket_id), _) => InboundOutcome::CommentOn { ticket_id },
        (None, Some(ticket_id)) => InboundOutcome::AttachByToken { ticket_id },
        (None, None) => InboundOutcome::NewTicket,
    }
}

/// Naive HTML→text: drops tags, decodes the handful of entities Marula's
/// own templates emit. Good enough for v1 reply rendering; not a general
/// sanitizer.
#[must_use]
fn strip_html_naive(html: &str) -> String {
    const ENTITIES: [(&str, &str); 5] = [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
    ];
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let Some(close_rel) = rest[open..].find('>') else {
            break;
        };
        // Block-level tags become spacing so words do not fuse together.
        let tag = rest[open + 1..open + close_rel].to_ascii_lowercase();
        if tag.starts_with("br")
            || tag.starts_with("/p")
            || tag.starts_with("p")
            || tag.starts_with("div")
        {
            out.push('\n');
        }
        rest = &rest[open + close_rel + 1..];
    }
    out.push_str(rest);
    let mut text = out;
    for (entity, ch) in ENTITIES {
        text = text.replace(entity, ch);
    }
    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(subject: &str) -> InboundEmail {
        InboundEmail {
            rfc_message_id: "<m1@example.test>".into(),
            in_reply_to: None,
            references: vec![],
            subject: Some(subject.into()),
            from_address: "Client@Example.TEST".into(),
            from_name: None,
            to: vec!["Support@ACME.test".into()],
            cc: vec![],
            text: Some("hello".into()),
            html: None,
            received_at: None,
            attachments: vec![],
        }
    }

    #[test]
    fn validation_requires_message_id_and_sender() {
        assert!(sample("s").validate().is_ok());
        let bad = InboundEmail {
            rfc_message_id: "  ".into(),
            ..sample("s")
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn thread_keys_merge_references_and_reply_to_without_duplicates() {
        let mut m = sample("[TCK-1] x");
        m.references = vec!["<A@x>", "<b@x>"]
            .into_iter()
            .map(String::from)
            .collect();
        m.in_reply_to = Some("<a@x>".into());
        assert_eq!(m.thread_keys(), vec!["a@x", "b@x"]);
    }

    #[test]
    fn recipients_prioritize_to_over_cc_case_insensitively() {
        let mut m = sample("s");
        m.cc = vec!["cc@x.test".into()];
        assert_eq!(
            m.recipients_in_priority_order(),
            vec!["support@acme.test", "cc@x.test"]
        );
    }

    #[test]
    fn subject_tokens_handle_re_prefixes_and_multiple_brackets() {
        assert_eq!(
            extract_subject_tokens("Re: [TCK-12] printer"),
            vec!["TCK-12"]
        );
        assert_eq!(
            extract_subject_tokens("[a] fwd: [TCK-3] x"),
            vec!["a", "TCK-3"]
        );
        assert!(extract_subject_tokens("no tokens").is_empty());
        assert!(extract_subject_tokens("unclosed [bracket").is_empty());
        assert!(extract_subject_tokens("empty [] brackets").is_empty());
    }

    #[test]
    fn precedence_thread_beats_token_beats_new() {
        let t = uuid::Uuid::now_v7();
        let k = uuid::Uuid::now_v7();
        assert_eq!(
            plan_inbound(Some(t), Some(k)),
            InboundOutcome::CommentOn { ticket_id: t }
        );
        assert_eq!(
            plan_inbound(None, Some(k)),
            InboundOutcome::AttachByToken { ticket_id: k }
        );
        assert_eq!(plan_inbound(None, None), InboundOutcome::NewTicket);
    }

    #[test]
    fn html_fallback_strips_tags_and_decodes_entities() {
        let mut m = sample("s");
        m.text = None;
        m.html = Some("<p>Printer &amp; scanner<br>broken</p>".into());
        assert_eq!(m.effective_text(), Some("Printer & scanner\nbroken".into()));
    }

    #[test]
    fn plain_text_wins_over_html() {
        let mut m = sample("s");
        m.html = Some("<p>ignored</p>".into());
        assert_eq!(m.effective_text().as_deref(), Some("hello"));
    }
}
