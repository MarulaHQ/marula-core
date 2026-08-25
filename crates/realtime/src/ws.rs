//! The `/ws` endpoint: token auth at upgrade time and the per-connection
//! session loop.
//!
//! Purpose: accept WebSocket upgrades only with a valid Meridian access
//! token (Authorization header preferred, `?token=` query for browsers),
//! then pump client requests and routed hub events until disconnect.
//!
//! Non-purpose: no re-auth mid-session — a socket lives as long as its
//! token was valid at connect; no server-initiated snapshots (clients call
//! REST to reconcile); no binary framing beyond the JSON text protocol.

use crate::auth::{self, WsClaims};
use crate::hub::Hub;
use crate::protocol::{self, ClientAction, ServerMessage};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{RawQuery, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::Arc;

/// Shared handler state: the fan-out hub and the JWT secret.
#[derive(Clone)]
pub struct AppState {
    hub: Hub,
    jwt_secret: Arc<String>,
}

impl AppState {
    /// Bundles a hub and JWT verification secret into router state.
    pub fn new(hub: Hub, jwt_secret: String) -> Self {
        Self {
            hub,
            jwt_secret: Arc::new(jwt_secret),
        }
    }
}

/// `?token=` fallback for browser clients that cannot set headers on
/// WebSocket connects. Prefer the Authorization header: URLs end up in
/// access logs.
#[derive(Debug, Default, Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

/// Auth-gated upgrade handler; invalid tokens get a plain HTTP 401 before
/// any upgrade happens. Token extraction deliberately precedes the
/// WebSocketUpgrade extractor so unauthenticated probes see 401 rather
/// than an upgrade-specific 400.
pub async fn upgrade(
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> Response {
    let token = bearer_token(&headers).or_else(|| query.as_deref().and_then(token_from_query));
    let verified = token
        .as_deref()
        .and_then(|t| auth::verify_token(&state.jwt_secret, t));
    match verified {
        Some(claims) => ws.on_upgrade(move |socket| run_session(socket, state.hub, claims)),
        None => (StatusCode::UNAUTHORIZED, "missing or invalid token").into_response(),
    }
}

/// Reads `Authorization: Bearer <token>` if present and well-formed.
fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ")?.parse::<String>().ok()
}

/// Pulls the percent-decoded `token` parameter out of a raw query string.
fn token_from_query(raw: &str) -> Option<String> {
    let parsed = serde_urlencoded::from_str::<TokenQuery>(raw).ok()?;
    parsed.token
}

/// One connected client: registers presence, auto-subscribes its own user
/// channel, then multiplexes client frames against the event broadcast.
async fn run_session(socket: WebSocket, hub: Hub, claims: WsClaims) {
    let (mut sink, mut stream) = socket.split();
    let mut events = hub.subscribe();
    hub.presence().join(claims.org, claims.sub);
    tracing::debug!(user = %claims.sub, org = %claims.org, "ws session opened");

    let mut subscriptions = HashSet::new();
    subscriptions.insert(crate::channel::Channel::User(claims.sub));

    let hello = ServerMessage::Hello {
        user_id: claims.sub,
        org_id: claims.org,
    };
    if send(&mut sink, &hello).await.is_err() {
        hub.presence().leave(claims.org, claims.sub);
        return;
    }

    loop {
        tokio::select! {
            frame = stream.next() => match frame {
                Some(Ok(Message::Text(text))) => {
                    if !handle_client_frame(&mut sink, &hub, &claims, &mut subscriptions, &text).await {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {} // ping/pong/binary frames are transport-level here
                Some(Err(error)) => {
                    tracing::debug!(%error, "ws receive failed");
                    break;
                }
            },
            delivery = events.recv() => match delivery {
                Ok(routed) => {
                    if let Some(message) = protocol::route_to_message(&routed, &subscriptions, claims.org) {
                        if send(&mut sink, &message).await.is_err() {
                            break;
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    // Buffer overflow: this session missed events. Clients
                    // are required to reconcile via REST snapshots anyway.
                    tracing::warn!(skipped, user = %claims.sub, "session lagged; REST reconcile needed");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
        }
    }

    hub.presence().leave(claims.org, claims.sub);
    tracing::debug!(user = %claims.sub, org = %claims.org, "ws session closed");
}

/// Applies one client text frame; returns false when the session must end
/// (send failure).
async fn handle_client_frame(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    hub: &Hub,
    claims: &WsClaims,
    subscriptions: &mut HashSet<crate::channel::Channel>,
    raw: &str,
) -> bool {
    match protocol::apply_client_message(subscriptions, claims, raw) {
        ClientAction::Reply(replies) => {
            for reply in replies {
                if send(sink, &reply).await.is_err() {
                    return false;
                }
            }
            true
        }
        ClientAction::PresenceQuery => {
            let users = hub.presence().users_in_org(claims.org);
            send(sink, &ServerMessage::Presence { users }).await.is_ok()
        }
    }
}

/// Serializes and pushes one server message.
async fn send(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    message: &ServerMessage,
) -> Result<(), axum::Error> {
    match message.to_text() {
        Some(text) => sink.send(Message::Text(text.into())).await,
        None => Ok(()), // serialization cannot fail for our owned types
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn bearer_token_is_extracted_from_the_header() {
        let mut headers = HeaderMap::new();
        assert_eq!(bearer_token(&headers), None);
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer abc.def.ghi"),
        );
        assert_eq!(bearer_token(&headers).as_deref(), Some("abc.def.ghi"));
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Basic nope"),
        );
        assert_eq!(bearer_token(&headers), None);
    }

    #[test]
    fn token_is_read_from_the_raw_query_string() {
        assert_eq!(
            token_from_query("token=abc.def"),
            Some("abc.def".to_string())
        );
        assert_eq!(
            token_from_query("a=1&token=x%2By&b=2"),
            Some("x+y".to_string())
        );
        assert_eq!(token_from_query(""), None);
        assert_eq!(token_from_query("other=1"), None);
    }

    #[test]
    fn app_state_bundles_hub_and_secret() {
        let state = AppState::new(Hub::new(), "secret".to_string());
        assert_eq!(state.jwt_secret.as_str(), "secret");
    }
}
