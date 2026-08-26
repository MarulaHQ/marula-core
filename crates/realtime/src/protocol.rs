//! WebSocket wire protocol: client requests, server responses, and the
//! pure rules that govern subscriptions and delivery.
//!
//! Purpose: define the JSON message contract for `/ws` sessions —
//! subscribe/unsubscribe, ping/pong keepalive, presence queries, and event
//! delivery — plus the authorization rules applied to each request.
//!
//! Non-purpose: no socket I/O; everything here is testable without a live
//! connection. Tenancy is enforced at delivery time by comparing the
//! envelope's `org_id` to the token claim.

use crate::auth::WsClaims;
use crate::channel::Channel;
use crate::hub::RoutedEvent;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

/// A message sent by the client over the socket.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Adds a channel to this session's subscription set.
    Subscribe { channel: String },
    /// Removes a channel from this session's subscription set.
    Unsubscribe { channel: String },
    /// Application-level keepalive.
    Ping,
    /// Asks who else is online in the caller's org.
    PresenceList,
}

/// A message sent by the server over the socket.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Sent once after successful auth on connect.
    Hello { user_id: Uuid, org_id: Uuid },
    /// Confirms a subscribe.
    Subscribed { channel: String },
    /// Confirms an unsubscribe.
    Unsubscribed { channel: String },
    /// Replies to a ping.
    Pong,
    /// Answers a presence query with the org's online users.
    Presence { users: Vec<Uuid> },
    /// A routed event; `channel` names which of the session's
    /// subscriptions matched.
    Event {
        channel: String,
        event: core_events::Envelope,
    },
    /// Rejects an invalid or forbidden request; the connection stays open.
    Error { message: String },
}

impl ServerMessage {
    /// Serializes to the text frame payload.
    pub fn to_text(&self) -> Option<String> {
        serde_json::to_string(self).ok()
    }
}

/// What the session layer must do after one client request.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientAction {
    /// Send these reply messages on the socket.
    Reply(Vec<ServerMessage>),
    /// Answer with a live presence snapshot for the caller's org (needs
    /// the registry, which lives at I/O scope).
    PresenceQuery,
}

/// Validates and applies one client request against the session state,
/// returning the action to perform. The caller's own user channel is always
/// pre-subscribed by the server; subscribing to another user's channel is
/// rejected, while board channels are accepted for any org member (org
/// tenancy is enforced when events are delivered).
pub fn apply_client_message(
    subscriptions: &mut HashSet<Channel>,
    claims: &WsClaims,
    raw: &str,
) -> ClientAction {
    let request = match serde_json::from_str::<ClientMessage>(raw) {
        Ok(request) => request,
        Err(error) => {
            return ClientAction::Reply(vec![ServerMessage::Error {
                message: error.to_string(),
            }])
        }
    };
    match request {
        ClientMessage::Subscribe { channel } => match authorize_subscribe(claims, &channel) {
            Ok(parsed) => {
                subscriptions.insert(parsed);
                ClientAction::Reply(vec![ServerMessage::Subscribed { channel }])
            }
            Err(message) => ClientAction::Reply(vec![ServerMessage::Error { message }]),
        },
        ClientMessage::Unsubscribe { channel } => {
            if let Some(parsed) = Channel::parse(&channel) {
                subscriptions.remove(&parsed);
                ClientAction::Reply(vec![ServerMessage::Unsubscribed { channel }])
            } else {
                ClientAction::Reply(vec![ServerMessage::Error {
                    message: format!("unknown channel: {channel}"),
                }])
            }
        }
        ClientMessage::Ping => ClientAction::Reply(vec![ServerMessage::Pong]),
        ClientMessage::PresenceList => ClientAction::PresenceQuery,
    }
}

/// Authorization rule for subscribe requests: own-user only for user
/// channels, own-org only for org channels; boards are org-scoped at
/// delivery time.
pub fn authorize_subscribe(claims: &WsClaims, raw: &str) -> Result<Channel, String> {
    match Channel::parse(raw) {
        Some(Channel::User(id)) if id == claims.sub => Ok(Channel::User(id)),
        Some(Channel::User(_)) => Err("forbidden: cannot subscribe to another user".to_string()),
        Some(Channel::Org(id)) if id == claims.org => Ok(Channel::Org(id)),
        Some(Channel::Org(_)) => Err("forbidden: cannot subscribe to another org".to_string()),
        Some(board @ Channel::Board(_)) => Ok(board),
        None => Err(format!("unknown channel: {raw}")),
    }
}

/// Decides what (if anything) a session should receive for one routed
/// event: same-org only, first matching subscribed channel wins so each
/// event is delivered exactly once.
pub fn route_to_message(
    routed: &RoutedEvent,
    subscriptions: &HashSet<Channel>,
    org: Uuid,
) -> Option<ServerMessage> {
    if routed.envelope.org_id != org {
        return None;
    }
    let matched = routed.channels.iter().find(|c| subscriptions.contains(c))?;
    Some(ServerMessage::Event {
        channel: matched.to_string(),
        event: routed.envelope.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::envelope;
    use serde_json::json;

    fn claims() -> WsClaims {
        WsClaims {
            sub: Uuid::now_v7(),
            org: Uuid::now_v7(),
            sid: Uuid::now_v7(),
            exp: i64::MAX / 2,
            iat: 0,
        }
    }

    #[test]
    fn client_messages_deserialize_from_tagged_json() {
        assert_eq!(
            serde_json::from_str::<ClientMessage>(r#"{"type":"ping"}"#).unwrap(),
            ClientMessage::Ping
        );
        assert_eq!(
            serde_json::from_str::<ClientMessage>(
                r#"{"type":"subscribe","channel":"board:00000000-0000-0000-0000-000000000001"}"#
            )
            .unwrap(),
            ClientMessage::Subscribe {
                channel: "board:00000000-0000-0000-0000-000000000001".into()
            }
        );
    }

    #[test]
    fn server_messages_serialize_with_type_tags() {
        let text = ServerMessage::Pong.to_text().expect("serializes");
        assert_eq!(text, r#"{"type":"pong"}"#);
    }

    #[test]
    fn subscribe_accepts_own_user_channel_and_boards() {
        let c = claims();
        let mut subs = HashSet::new();
        subs.insert(Channel::User(c.sub));
        let own = format!("user:{}", c.sub);
        let action = apply_client_message(
            &mut subs,
            &c,
            &format!(r#"{{"type":"subscribe","channel":"{own}"}}"#),
        );
        assert_eq!(
            action,
            ClientAction::Reply(vec![ServerMessage::Subscribed { channel: own }])
        );
        let board_raw = "board:00000000-0000-0000-0000-000000000009";
        let action = apply_client_message(
            &mut subs,
            &c,
            r#"{"type":"subscribe","channel":"board:00000000-0000-0000-0000-000000000009"}"#,
        );
        assert!(subs.contains(&Channel::parse(board_raw).unwrap()));
        assert_eq!(
            action,
            ClientAction::Reply(vec![ServerMessage::Subscribed {
                channel: board_raw.into()
            }])
        );
    }

    #[test]
    fn subscribe_rejects_foreign_user_channels() {
        let c = claims();
        let mut subs = HashSet::new();
        let foreign = format!("user:{}", Uuid::now_v7());
        let raw = format!(r#"{{"type":"subscribe","channel":"{foreign}"}}"#);
        let action = apply_client_message(&mut subs, &c, &raw);
        assert!(matches!(
            action,
            ClientAction::Reply(ref replies) if matches!(replies.first(), Some(ServerMessage::Error { .. }))
        ));
        assert!(!subs.contains(&Channel::parse(&foreign).unwrap()));
    }

    #[test]
    fn subscribe_accepts_own_org_and_rejects_foreign_org() {
        let c = claims();
        let mut subs = HashSet::new();
        let own = format!("org:{}", c.org);
        let action = apply_client_message(
            &mut subs,
            &c,
            &format!(r#"{{"type":"subscribe","channel":"{own}"}}"#),
        );
        assert_eq!(
            action,
            ClientAction::Reply(vec![ServerMessage::Subscribed {
                channel: own.clone()
            }])
        );
        assert!(subs.contains(&Channel::Org(c.org)));

        let foreign = format!("org:{}", Uuid::now_v7());
        let action = apply_client_message(
            &mut subs,
            &c,
            &format!(r#"{{"type":"subscribe","channel":"{foreign}"}}"#),
        );
        assert!(matches!(
            action,
            ClientAction::Reply(ref replies) if matches!(replies.first(), Some(ServerMessage::Error { .. }))
        ));
        assert!(!subs.contains(&Channel::parse(&foreign).unwrap()));
    }

    #[test]
    fn malformed_requests_reply_with_error_and_stay_open() {
        let c = claims();
        let mut subs = HashSet::new();
        let action = apply_client_message(&mut subs, &c, "not json");
        assert!(matches!(
            action,
            ClientAction::Reply(ref replies) if matches!(replies.first(), Some(ServerMessage::Error { .. }))
        ));
        let action =
            apply_client_message(&mut subs, &c, r#"{"type":"subscribe","channel":"junk"}"#);
        assert!(matches!(
            action,
            ClientAction::Reply(ref replies) if matches!(replies.first(), Some(ServerMessage::Error { .. }))
        ));
    }

    #[test]
    fn unsubscribe_removes_the_channel_and_ping_replies_pong() {
        let c = claims();
        let mut subs = HashSet::new();
        let board = Channel::Board(Uuid::now_v7());
        subs.insert(board.clone());
        let raw = format!(r#"{{"type":"unsubscribe","channel":"{board}"}}"#);
        let action = apply_client_message(&mut subs, &c, &raw);
        assert_eq!(
            action,
            ClientAction::Reply(vec![ServerMessage::Unsubscribed {
                channel: board.to_string()
            }])
        );
        assert!(!subs.contains(&board));
        assert_eq!(
            apply_client_message(&mut subs, &c, r#"{"type":"ping"}"#),
            ClientAction::Reply(vec![ServerMessage::Pong])
        );
    }

    #[test]
    fn presence_list_defers_to_the_session_layer() {
        let c = claims();
        let mut subs = HashSet::new();
        assert_eq!(
            apply_client_message(&mut subs, &c, r#"{"type":"presence_list"}"#),
            ClientAction::PresenceQuery
        );
    }

    #[test]
    fn delivery_requires_matching_org_and_subscription() {
        let c = claims();
        let board = Channel::Board(Uuid::now_v7());
        let mut e = envelope("ticket", json!({ "board_id": Uuid::now_v7() }));
        e.org_id = c.org;
        let routed = RoutedEvent {
            envelope: e.clone(),
            channels: vec![board.clone()],
        };

        let mut subs = HashSet::new();
        assert!(route_to_message(&routed, &subs, c.org).is_none());
        subs.insert(board.clone());
        let msg = route_to_message(&routed, &subs, c.org).expect("delivered");
        assert!(
            matches!(msg, ServerMessage::Event { ref channel, .. } if *channel == board.to_string())
        );

        let other_org = Uuid::now_v7();
        assert!(route_to_message(&routed, &subs, other_org).is_none());
    }

    #[test]
    fn multi_channel_events_deliver_exactly_once_on_first_match() {
        let c = claims();
        let assignee = c.sub;
        let board = Channel::Board(Uuid::now_v7());
        let mut e = envelope(
            "ticket",
            json!({ "board_id": board.to_string(), "assignee_id": assignee.to_string() }),
        );
        e.org_id = c.org;
        let routed = RoutedEvent {
            envelope: e,
            channels: vec![board.clone(), Channel::User(assignee)],
        };
        let subs = HashSet::from([board.clone(), Channel::User(assignee)]);
        let msg = route_to_message(&routed, &subs, c.org).expect("one delivery");
        let ServerMessage::Event { channel, .. } = msg else {
            panic!("expected event");
        };
        assert_eq!(channel, board.to_string());
    }
}
