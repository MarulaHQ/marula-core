//! Redis stream reader: XREADs `core_events::EVENT_STREAM` and feeds the
//! hub.
//!
//! Purpose: the only Redis consumer in this service. One dedicated task
//! holds one connection, follows the stream with blocking XREAD, parses
//! envelopes out of each entry's `data` field, and hands them to the hub.
//!
//! Non-purpose: no replay or checkpoint storage — the cursor lives only in
//! memory, so a restarted hub resumes at the stream tail (`$`) and clients
//! reconcile anything they missed via REST snapshots.

use crate::hub::Hub;
use core_events::{Envelope, EVENT_STREAM};
use redis::streams::{StreamId, StreamReadOptions, StreamReadReply};
use redis::AsyncCommands;
use tokio::sync::watch;

/// Blocking-XREAD wait per poll; also bounds shutdown latency.
const BLOCK_MS: usize = 2_000;
/// Max entries pulled per wake-up.
const COUNT: usize = 500;
/// Cap on reconnect backoff after Redis errors.
const MAX_BACKOFF_MS: u64 = 5_000;

/// Runs until `shutdown` flips: reads the stream forever, reconnecting with
/// backoff on Redis errors. Malformed entries are logged and skipped rather
/// than stalling the stream.
pub async fn run_stream_reader(redis_url: String, hub: Hub, mut shutdown: watch::Receiver<bool>) {
    let client = match redis::Client::open(redis_url.as_str()) {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(%error, "invalid redis url; stream reader exiting");
            return;
        }
    };
    let mut backoff_ms = 100_u64;
    loop {
        if *shutdown.borrow() {
            return;
        }
        match client.get_multiplexed_async_connection().await {
            Ok(mut conn) => {
                backoff_ms = 100;
                let cursor = follow_stream(&mut conn, &hub, &mut shutdown).await;
                if cursor.is_none() {
                    return; // shutdown requested mid-poll
                }
                tracing::warn!("stream connection ended; reconnecting");
            }
            Err(error) => {
                tracing::error!(%error, "redis connect failed");
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
        backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
    }
}

/// Polls until an error occurs or shutdown fires. Returns `None` when
/// shutdown was requested, `Some(())` when the connection broke.
async fn follow_stream(
    conn: &mut redis::aio::MultiplexedConnection,
    hub: &Hub,
    shutdown: &mut watch::Receiver<bool>,
) -> Option<()> {
    // `$` starts at the live tail: no replay after restarts, by design.
    let mut cursor = "$".to_string();
    let options = StreamReadOptions::default().block(BLOCK_MS).count(COUNT);
    let keys = [EVENT_STREAM];
    loop {
        if *shutdown.borrow() {
            return None;
        }
        let ids = [cursor.as_str()];
        tokio::select! {
            _ = shutdown.changed() => return None,
            result = conn.xread_options::<_, _, StreamReadReply>(&keys, &ids, &options) => {
                match result {
                    Ok(reply) => {
                        for key in reply.keys {
                            for entry in key.ids {
                                cursor = entry.id.clone();
                                dispatch_entry(hub, &entry);
                            }
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "xread failed");
                        return Some(());
                    }
                }
            }
        }
    }
}

/// Parses one stream entry's `data` payload and pushes it into the hub.
fn dispatch_entry(hub: &Hub, entry: &StreamId) {
    match entry.get::<String>("data") {
        Some(raw) => match serde_json::from_str::<Envelope>(&raw) {
            Ok(envelope) => hub.publish(envelope),
            Err(error) => tracing::warn!(entry_id = %entry.id, %error, "unparseable event"),
        },
        None => tracing::warn!(entry_id = %entry.id, "stream entry missing data field"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::envelope;
    use serde_json::json;
    use std::collections::HashMap;

    fn stream_entry(data: &str) -> StreamId {
        let mut map = HashMap::new();
        map.insert(
            "data".to_string(),
            redis::Value::BulkString(data.as_bytes().to_vec()),
        );
        StreamId {
            id: "1-1".to_string(),
            map,
        }
    }

    #[tokio::test]
    async fn dispatches_wellformed_entries_into_the_hub() {
        let hub = Hub::new();
        let mut rx = hub.subscribe();
        let user = uuid::Uuid::now_v7();
        let raw = serde_json::to_string(&envelope("timer", json!({ "user_id": user })))
            .expect("serializes");
        dispatch_entry(&hub, &stream_entry(&raw));
        let routed = rx.recv().await.expect("delivered");
        assert_eq!(routed.channels.len(), 1);
    }

    #[tokio::test]
    async fn skips_malformed_entries_without_blocking_the_hub() {
        let hub = Hub::new();
        let mut rx = hub.subscribe();
        dispatch_entry(&hub, &stream_entry("not-json"));
        assert!(rx.try_recv().is_err());
        dispatch_entry(&hub, &stream_entry("{}"));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn entries_without_data_field_are_skipped() {
        let empty = StreamId {
            id: "1-2".to_string(),
            map: HashMap::new(),
        };
        let hub = Hub::new();
        dispatch_entry(&hub, &empty); // must not panic
    }
}
