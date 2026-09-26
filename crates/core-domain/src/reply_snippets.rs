//! Reply snippets — the typed canned-response library for staff
//! composition (Wave 34 lane 3, `p34-canned-templates`).
//!
//! Purpose: the Stage 4 service-desk half that never shipped. A snippet is
//! a short reusable reply body a staff member inserts into the ticket
//! composer or a client-visible reply (the portal reply path). Bodies
//! carry `{{path}}` variable tokens from the SAME closed ticket
//! vocabulary the template registry uses (`templates::variables_for_kind`
//! on `TemplateKind::Ticket`) — one interpolation engine, no second
//! dialect. Per-board scoping is a field on the snippet (`board_ids`,
//! empty = every board), so the whole library is a typed value in the
//! settings domain (scope `org`, key `reply_snippet:<id>`) — no table, no
//! migration.
//!
//! Non-purpose: ticket TEMPLATES (`/api/v1/templates`) — those stay a
//! separate typed library; snippets never merge into them. Provenance
//! rides the templates fork-on-edit pattern: forking records
//! `forked_from` on the copy and the origin is untouched.

use crate::entities::templates::TemplateKind;
use crate::templates::{self, RenderError};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The settings-domain key prefix every snippet row lives under.
pub const SNIPPET_KEY_PREFIX: &str = "reply_snippet:";

/// One canned reply in the org's snippet library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplySnippet {
    pub id: Uuid,
    /// Short picker label (the search predicate matches this and body).
    pub name: String,
    /// The insertable body; `{{path}}` tokens resolve from ticket context.
    pub body: String,
    /// Boards the snippet is offered on. Empty = every board.
    #[serde(default)]
    pub board_ids: Vec<Uuid>,
    /// Fork-on-edit provenance (templates pattern): the origin snippet's
    /// id this copy was forked from, when it is a fork.
    #[serde(default)]
    pub forked_from: Option<Uuid>,
    pub updated_by: Uuid,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

pub const NAME_MAX: usize = 120;
pub const BODY_MAX: usize = 20_000;

impl ReplySnippet {
    fn validate(&self) -> Result<(), SnippetError> {
        let name = self.name.trim();
        if name.is_empty() || name.len() > NAME_MAX {
            return Err(SnippetError::InvalidName);
        }
        if self.body.trim().is_empty() || self.body.len() > BODY_MAX {
            return Err(SnippetError::InvalidBody);
        }
        templates::validate_content(TemplateKind::Ticket, None, &self.body)
            .map_err(|e| SnippetError::Variable(e))?;
        Ok(())
    }
}

/// Typed validation failures (the templates convention: out-of-vocabulary
/// placeholders are typed errors, never silent empties).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SnippetError {
    #[error("snippet name must be 1..={NAME_MAX} characters")]
    InvalidName,
    #[error("snippet body must be 1..={BODY_MAX} characters")]
    InvalidBody,
    #[error(transparent)]
    Variable(#[from] RenderError),
}

/// The ticket-variable vocabulary snippets may use, exposed for editors.
pub fn variable_vocabulary() -> &'static [&'static str] {
    templates::variables_for_kind(TemplateKind::Ticket)
}

/// Parses a stored settings value into a snippet, validating on the way
/// in (the settings domain stores JSONB; core-db never interprets).
pub fn from_settings_value(key: &str, value: &serde_json::Value) -> Option<ReplySnippet> {
    let snippet: ReplySnippet = serde_json::from_value(value.clone()).ok()?;
    if !key.starts_with(SNIPPET_KEY_PREFIX) {
        return None;
    }
    snippet.validate().ok()?;
    Some(snippet)
}

/// Validates and serializes a snippet for storage.
pub fn to_settings_value(snippet: &ReplySnippet) -> Result<serde_json::Value, SnippetError> {
    snippet.validate()?;
    Ok(serde_json::to_value(snippet).unwrap_or(serde_json::Value::Null))
}

/// The search-in-composer predicate: case-insensitive substring over name
/// and body, filtered to the board (`board_id` None = unfiltered).
pub fn matches(snippet: &ReplySnippet, board_id: Option<Uuid>, query: &str) -> bool {
    if let Some(board) = board_id {
        if !snippet.board_ids.is_empty() && !snippet.board_ids.contains(&board) {
            return false;
        }
    }
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    snippet.name.to_lowercase().contains(&q) || snippet.body.to_lowercase().contains(&q)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snippet(name: &str, body: &str) -> ReplySnippet {
        ReplySnippet {
            id: Uuid::now_v7(),
            name: name.into(),
            body: body.into(),
            board_ids: vec![],
            forked_from: None,
            updated_by: Uuid::now_v7(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn accepts_ticket_vocabulary_tokens() {
        let s = snippet("Greeting", "Hi {{contact.name}}, about {{ticket.number}}.");
        assert!(to_settings_value(&s).is_ok());
    }

    #[test]
    fn rejects_out_of_vocabulary_tokens() {
        let s = snippet("Bad", "Hello {{client.contact_first}}");
        assert!(matches!(
            to_settings_value(&s),
            Err(SnippetError::Variable(RenderError::UnknownVariable(_, _)))
        ));
    }

    #[test]
    fn predicate_filters_by_board_and_query() {
        let mut s = snippet("Wifi reset", "Restart the {{client.name}} router.");
        let board = Uuid::now_v7();
        s.board_ids = vec![board];
        assert!(matches(&s, Some(board), "wifi"));
        assert!(!matches(&s, Some(Uuid::now_v7()), "wifi"));
        // empty query still honours the board filter
        assert!(matches(&s, Some(board), ""));
        assert!(!matches(&s, Some(Uuid::now_v7()), ""));
        assert!(matches(&s, None, "ROUTER"));
        assert!(!matches(&s, None, "printer"));
    }
}
