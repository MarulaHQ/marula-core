//! The fan-out hub: one in-process broadcast of routed envelopes plus the
//! presence registry.
//!
//! Purpose: decouple the single Redis XREAD task from every WebSocket
//! session. The reader computes each envelope's channels once; sessions
//! filter the broadcast against their own subscriptions.
//!
//! Non-purpose: no delivery guarantees — a slow session that falls a full
//! channel buffer behind gets a lag notice and reconciles via REST, and
//! events published while the hub is down are never replayed.

use crate::channel::{self, Channel};
use crate::presence::PresenceMap;
use core_events::Envelope;
use std::sync::Arc;
use tokio::sync::broadcast;

/// How many routed events a session can absorb before it is declared
/// lagged. Generous for UI traffic; REST snapshots remain the fallback.
const EVENT_BUFFER: usize = 1024;

/// An envelope paired with its precomputed delivery channels.
#[derive(Debug, Clone)]
pub struct RoutedEvent {
    /// The versioned event envelope from the stream.
    pub envelope: Envelope,
    /// Channels computed by `channel::routes_for`, stable order.
    pub channels: Vec<Channel>,
}

/// Cloneable handle to the shared fan-out state.
#[derive(Clone)]
pub struct Hub {
    tx: broadcast::Sender<Arc<RoutedEvent>>,
    presence: Arc<PresenceMap>,
}

impl Hub {
    /// Creates a hub with an empty presence map.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(EVENT_BUFFER);
        Self {
            tx,
            presence: Arc::new(PresenceMap::new()),
        }
    }

    /// Computes routes and broadcasts to all sessions. Envelopes with no
    /// routable channel are dropped silently (clients reconcile via REST).
    pub fn publish(&self, envelope: Envelope) {
        let channels = channel::routes_for(&envelope);
        if channels.is_empty() {
            tracing::trace!(event_id = %envelope.id, "no routable channels");
            return;
        }
        let routed = Arc::new(RoutedEvent { envelope, channels });
        // A send error only means no sessions are connected; that is fine.
        let _ = self.tx.send(routed);
    }

    /// Subscribes to the broadcast stream of routed events.
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<RoutedEvent>> {
        self.tx.subscribe()
    }

    /// Read-only access to the presence registry.
    pub fn presence(&self) -> &PresenceMap {
        &self.presence
    }
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::envelope;
    use serde_json::json;

    #[tokio::test]
    async fn publishes_routable_envelopes_to_subscribers() {
        let hub = Hub::new();
        let mut rx = hub.subscribe();
        let user = uuid::Uuid::now_v7();
        let mut e = envelope("timer", json!({ "user_id": user }));
        e.org_id = uuid::Uuid::now_v7();
        hub.publish(e.clone());

        let routed = rx.recv().await.expect("event delivered");
        assert_eq!(routed.envelope.id, e.id);
        assert_eq!(routed.channels, vec![Channel::User(user)]);
    }

    #[tokio::test]
    async fn drops_envelopes_without_routes() {
        let hub = Hub::new();
        let mut rx = hub.subscribe();
        hub.publish(envelope("invoice", json!({})));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn exposes_presence_registry() {
        let hub = Hub::new();
        let org = uuid::Uuid::now_v7();
        let user = uuid::Uuid::now_v7();
        hub.presence().join(org, user);
        assert_eq!(hub.presence().users_in_org(org), vec![user]);
    }
}
