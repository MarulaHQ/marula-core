//! One ticket card, two renderers (G8/G9).
//!
//! Purpose: the same facts render in Slack and Teams, so "a shared ticket
//! looks the same in both products" is a property of the data rather than of
//! two hand-kept layouts. Core builds [`TicketCard`] from the ticket; this
//! module turns it into Slack Block Kit and into an Adaptive Card.
//!
//! The card carries exactly six facts — number, title, client, status,
//! priority, assignee — plus a link back to Marula and the actions the press
//! registry offers. There is deliberately no body, no note and no comment
//! field: a channel audience is wider than a ticket's audience, so the only
//! safe card is one that cannot hold restricted content in the first place.
//!
//! Non-purpose: no permission decision. Whether this ticket may be shared at
//! all is Core's answer (a restricted client refuses the share before this
//! module is reached); this module renders what it is given.

use serde_json::{json, Value};

/// One pressable action on a card.
///
/// `id` is the action registry's id (`take`, `status:in_progress`). Both
/// renderers carry it back verbatim, so the callback handler resolves the same
/// entry whichever product the press came from — that is what makes a new
/// action one registry entry rather than two vendor match arms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardAction {
    pub id: String,
    pub label: String,
    /// A link button (the `open` behaviour) instead of a press. Both
    /// renderers emit it as a URL action, never as a callback.
    pub url: Option<String>,
}

impl CardAction {
    /// A callback button the registry will resolve on press.
    pub fn press(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            url: None,
        }
    }

    /// A link button (`open`). Opening is not a press and is not audited.
    pub fn link(id: impl Into<String>, label: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            url: Some(url.into()),
        }
    }
}

/// The facts a ticket card renders, in both products.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketCard {
    pub number: String,
    pub title: String,
    pub client: Option<String>,
    pub status: String,
    pub priority: String,
    pub assignee: Option<String>,
    /// Deep link into Marula. Absent when the deployment has no web base URL.
    pub url: Option<String>,
    pub actions: Vec<CardAction>,
    /// One line naming who shared it, rendered small. Optional because the
    /// slash command answers with the same card and has no sharer.
    pub footer: Option<String>,
}

impl TicketCard {
    /// The one-line summary both renderers use as the fallback `text`, so a
    /// client that cannot draw blocks still shows the ticket, not a blank.
    pub fn summary(&self) -> String {
        format!("{} · {}", self.number, self.title)
    }

    /// The fact lines in display order: the same order in both products, which
    /// is what "renders the same facts" means in practice.
    fn facts(&self) -> Vec<(&'static str, String)> {
        let mut facts = Vec::with_capacity(5);
        if let Some(client) = &self.client {
            facts.push(("Client", client.clone()));
        }
        facts.push(("Status", self.status.clone()));
        facts.push(("Priority", self.priority.clone()));
        if let Some(assignee) = &self.assignee {
            facts.push(("Assignee", assignee.clone()));
        }
        facts
    }
}

/// Slack Block Kit: one section with the ticket and its facts, one actions
/// block, and the numbering in `text` so notifications read correctly.
pub fn render_slack(card: &TicketCard) -> Value {
    let mut body = format!("*{}* {}", card.number, escape_mrkdwn(&card.title));
    for (label, value) in card.facts() {
        body.push_str(&format!("\n*{label}:* {}", escape_mrkdwn(&value)));
    }
    if let Some(footer) = &card.footer {
        body.push_str(&format!("\n_{}_", escape_mrkdwn(footer)));
    }

    let mut blocks = vec![json!({
        "type": "section",
        "text": { "type": "mrkdwn", "text": body },
    })];

    if let Some(url) = &card.url {
        // A context block, not a button: the link belongs to the card itself,
        // and the action row below is reserved for pressable entries.
        blocks.push(json!({
            "type": "context",
            "elements": [{ "type": "mrkdwn", "text": format!("<{url}|Open in Marula>") }],
        }));
    }

    if !card.actions.is_empty() {
        let elements: Vec<Value> = card
            .actions
            .iter()
            .map(|action| match &action.url {
                Some(url) => json!({
                    "type": "button",
                    "text": { "type": "plain_text", "text": action.label },
                    "url": url,
                    "action_id": slack_action_id(&action.id),
                }),
                None => json!({
                    "type": "button",
                    "text": { "type": "plain_text", "text": action.label },
                    // The registry id is the value, so a press never has to
                    // carry hidden state that could drift from the button.
                    "value": action.id,
                    "action_id": slack_action_id(&action.id),
                }),
            })
            .collect();
        blocks.push(json!({ "type": "actions", "elements": elements }));
    }

    json!({ "text": card.summary(), "blocks": blocks })
}

/// Slack's `action_id` length limit is 255 bytes; the prefix makes our
/// actions recognisable in a payload we did not author.
pub fn slack_action_id(registry_id: &str) -> String {
    format!("marula.{registry_id}")
}

/// The registry id behind a Slack `action_id`, or `None` for a foreign action
/// (a card another app posted into a channel our connection shares).
pub fn registry_id_from_slack(action_id: &str) -> Option<&str> {
    action_id.strip_prefix("marula.")
}

/// Microsoft Teams: one Adaptive Card, version 1.4 — the version the Bot
/// Framework's cards render without a preview gate.
pub fn render_teams(card: &TicketCard) -> Value {
    let mut body = vec![json!({
        "type": "TextBlock",
        "text": format!("{} · {}", card.number, card.title),
        "weight": "Bolder",
        "size": "Medium",
        "wrap": true,
    })];

    let facts: Vec<Value> = card
        .facts()
        .into_iter()
        .map(|(label, value)| json!({ "title": label, "value": value }))
        .collect();
    if !facts.is_empty() {
        body.push(json!({ "type": "FactSet", "facts": facts }));
    }
    if let Some(footer) = &card.footer {
        body.push(json!({
            "type": "TextBlock",
            "text": footer,
            "isSubtle": true,
            "size": "Small",
            "wrap": true,
        }));
    }

    let mut actions: Vec<Value> = Vec::with_capacity(card.actions.len() + 1);
    if let Some(url) = &card.url {
        actions.push(json!({ "type": "Action.OpenUrl", "title": "Open in Marula", "url": url }));
    }
    for action in &card.actions {
        match &action.url {
            Some(url) => actions.push(json!({
                "type": "Action.OpenUrl",
                "title": action.label,
                "url": url,
            })),
            None => actions.push(json!({
                "type": "Action.Submit",
                "title": action.label,
                // One key, so both an `Action.Submit` and an
                // `adaptiveCard/action` invoke resolve the same registry id.
                "data": { "marulaAction": action.id },
            })),
        }
    }

    let mut content = json!({
        "type": "AdaptiveCard",
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
        "version": "1.4",
        "body": body,
    });
    if !actions.is_empty() {
        content["actions"] = Value::Array(actions);
    }

    json!({
        "type": "message",
        "attachments": [{
            "contentType": "application/vnd.microsoft.card.adaptive",
            "content": content,
        }],
    })
}

/// Escape the four characters Slack's mrkdwn treats as syntax.
///
/// A ticket title is user-authored text, so `*` and `_` in it would otherwise
/// restyle neighbouring words — the small class of bug that makes a card look
/// untrustworthy.
fn escape_mrkdwn(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if matches!(ch, '&' | '<' | '>') || ch == '*' || ch == '_' || ch == '`' {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> TicketCard {
        TicketCard {
            number: "T-1042".into(),
            title: "Printer *down* in reception".into(),
            client: Some("Acme Ltd".into()),
            status: "In progress".into(),
            priority: "P2".into(),
            assignee: Some("Dana Scott".into()),
            url: Some("https://marula.example.test/tickets/abc".into()),
            actions: vec![
                CardAction::press("take", "Take it"),
                CardAction::press("status:in_progress", "In progress"),
                CardAction::link("open", "Open in Marula", "https://x.test/t"),
            ],
            footer: Some("Shared by Dana Scott".into()),
        }
    }

    /// The acceptance test for G8: the same facts, in both products, from one
    /// builder. The assertions name the values rather than the layout, because
    /// the layout is each vendor's business.
    #[test]
    fn both_products_carry_the_same_six_facts() {
        let card = card();
        let slack = render_slack(&card).to_string();
        let teams = render_teams(&card).to_string();
        for value in [
            "T-1042",
            "Printer",
            "Acme Ltd",
            "In progress",
            "P2",
            "Dana Scott",
        ] {
            assert!(slack.contains(value), "slack card lost {value}");
            assert!(teams.contains(value), "teams card lost {value}");
        }
        assert!(slack.contains("https://marula.example.test/tickets/abc"));
        assert!(teams.contains("https://marula.example.test/tickets/abc"));
    }

    /// A press registry id round-trips through Slack's namespace and back, and
    /// a foreign action id is refused rather than guessed at.
    #[test]
    fn slack_action_ids_round_trip_and_refuse_foreign_buttons() {
        assert_eq!(slack_action_id("take"), "marula.take");
        assert_eq!(registry_id_from_slack("marula.take"), Some("take"));
        assert_eq!(registry_id_from_slack("other_app.take"), None);
    }

    /// Mrkdwn is escaped, so a title cannot restyle the card or forge a link.
    #[test]
    fn user_text_cannot_inject_mrkdwn() {
        let esc = escape_mrkdwn("a *bold* <script> _x_ `y`");
        assert!(!esc.contains("<script>"));
        assert!(esc.contains("\\*bold\\*"));
        assert!(esc.contains("\\<script\\>"));
    }

    /// A Teams press carries the registry id under one key, both for
    /// `Action.Submit` and for the `adaptiveCard/action` invoke.
    #[test]
    fn teams_actions_carry_the_registry_id() {
        let teams = render_teams(&card());
        let actions = &teams["attachments"][0]["content"]["actions"];
        let submitted = actions
            .as_array()
            .expect("actions")
            .iter()
            .find(|a| a["type"] == "Action.Submit")
            .expect("a submit action");
        assert_eq!(submitted["data"]["marulaAction"], "take");
    }
}
