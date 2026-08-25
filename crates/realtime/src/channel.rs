//! Subscription channels and the envelope→channel routing rules.
//!
//! Purpose: define the two channel kinds clients subscribe to — per-user
//! (`user:<uuid>`) and per-board (`board:<uuid>`) — and the pure function
//! that decides which channels each event envelope is delivered on.
//!
//! Non-purpose: no authorization decisions here beyond "a user may target
//! their own user channel"; board ACLs stay in core-api's policy module.

use core_events::Envelope;
use std::fmt;
use uuid::Uuid;

/// A routable subscription target. On the wire it appears as the string
/// form produced by `Display` (`user:<uuid>` / `board:<uuid>`); protocol
/// messages carry channels as plain strings and parse them back.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Channel {
    /// Events addressed to one user (timers, notifications).
    User(Uuid),
    /// Events scoped to one ticket board.
    Board(Uuid),
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Channel::User(id) => write!(f, "user:{id}"),
            Channel::Board(id) => write!(f, "board:{id}"),
        }
    }
}

impl Channel {
    /// Parses the wire form `user:<uuid>` / `board:<uuid>`.
    pub fn parse(raw: &str) -> Option<Self> {
        let (kind, id) = raw.split_once(':')?;
        let id = Uuid::parse_str(id).ok()?;
        match kind {
            "user" => Some(Channel::User(id)),
            "board" => Some(Channel::Board(id)),
            _ => None,
        }
    }
}

/// Reads a UUID-valued string field off a JSON payload.
fn uuid_field(payload: &serde_json::Value, key: &str) -> Option<Uuid> {
    payload
        .get(key)?
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
}

/// Computes every channel an envelope should be delivered on. The order is
/// stable (board targets before user targets) so multi-channel events are
/// deduplicated deterministically by the session layer.
///
/// Routing table (driven by the payloads core-api actually emits):
/// - `ticket.*`: `payload.board_id` → board channel; `payload.assignee_id`
///   → user channel. Events carrying neither (e.g. today's
///   `ticket.updated` payload) are not WS-routable yet; clients pick them
///   up via REST reconciliation.
/// - `timer.*`, `notification.*`: `payload.user_id` → user channel.
/// - `time_entry.*`: `payload.entry.user_id` → user channel.
/// - `ticket.comment_added`: each `payload.mentioned_user_ids` entry →
///   user channel.
/// - Anything else has no channels and never reaches WebSocket clients.
pub fn routes_for(envelope: &Envelope) -> Vec<Channel> {
    let payload = &envelope.payload;
    match envelope.entity.as_str() {
        "ticket" => {
            let mut routes = Vec::new();
            if let Some(id) = uuid_field(payload, "board_id") {
                routes.push(Channel::Board(id));
            }
            if let Some(id) = uuid_field(payload, "assignee_id") {
                routes.push(Channel::User(id));
            }
            routes
        }
        "timer" | "notification" => uuid_field(payload, "user_id")
            .map(Channel::User)
            .into_iter()
            .collect(),
        "time_entry" => {
            let entry = payload.get("entry").unwrap_or(&serde_json::Value::Null);
            uuid_field(entry, "user_id")
                .map(Channel::User)
                .into_iter()
                .collect()
        }
        "ticket_comment" => payload
            .get("mentioned_user_ids")
            .and_then(|v| v.as_array())
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| id.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                    .map(Channel::User)
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::envelope;
    use serde_json::json;

    #[test]
    fn channel_wire_format_roundtrips() {
        let id = Uuid::now_v7();
        for channel in [Channel::User(id), Channel::Board(id)] {
            let parsed = Channel::parse(&channel.to_string()).expect("parses");
            assert_eq!(parsed, channel);
        }
    }

    #[test]
    fn channel_parse_rejects_garbage() {
        assert_eq!(Channel::parse("user:not-a-uuid"), None);
        assert_eq!(
            Channel::parse("widget:00000000-0000-0000-0000-000000000000"),
            None
        );
        assert_eq!(Channel::parse("noseparator"), None);
    }

    #[test]
    fn ticket_created_routes_to_board_and_assignee() {
        let (board, assignee) = (Uuid::now_v7(), Uuid::now_v7());
        let e = envelope(
            "ticket",
            json!({ "ticket_id": Uuid::now_v7(), "board_id": board, "assignee_id": assignee }),
        );
        assert_eq!(
            routes_for(&e),
            vec![Channel::Board(board), Channel::User(assignee)]
        );
    }

    #[test]
    fn minimal_ticket_update_is_not_routable() {
        let e = envelope("ticket", json!({ "ticket_id": Uuid::now_v7() }));
        assert!(routes_for(&e).is_empty());
    }

    #[test]
    fn timer_routes_to_owning_user() {
        let user = Uuid::now_v7();
        let e = envelope(
            "timer",
            json!({ "session_id": Uuid::now_v7(), "user_id": user }),
        );
        assert_eq!(routes_for(&e), vec![Channel::User(user)]);
    }

    #[test]
    fn time_entry_routes_via_nested_entry_user() {
        let user = Uuid::now_v7();
        let e = envelope(
            "time_entry",
            json!({ "entry": { "id": Uuid::now_v7(), "user_id": user } }),
        );
        assert_eq!(routes_for(&e), vec![Channel::User(user)]);
    }

    #[test]
    fn comment_mentions_route_to_each_mentioned_user() {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let e = envelope(
            "ticket_comment",
            json!({ "mentioned_user_ids": [a.to_string(), b.to_string()] }),
        );
        assert_eq!(routes_for(&e), vec![Channel::User(a), Channel::User(b)]);
    }

    #[test]
    fn unknown_entity_has_no_routes() {
        let e = envelope("invoice", json!({ "user_id": Uuid::now_v7() }));
        assert!(routes_for(&e).is_empty());
    }

    #[test]
    fn malformed_uuid_fields_are_ignored() {
        let e = envelope("timer", json!({ "user_id": "nope" }));
        assert!(routes_for(&e).is_empty());
    }
}
