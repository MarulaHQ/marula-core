//! core-events — the integration contract (§4.2).
//!
//! Purpose: the versioned event envelope and the outbox→Redis relay. Every
//! consumer of Meridian's change feed (websockets, webhooks, plugins, search
//! indexing) reads this one stream.
//!
//! Non-purpose: no business logic, no HTTP, no storage writes other than
//! marking outbox rows published.

use core_db::outbox::OutboxRow;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The wire envelope. `payload_schema_ref` pins the payload contract so
/// consumers can version-gate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Envelope {
    pub id: Uuid,
    #[serde(rename = "type")]
    pub event_type: String,
    pub org_id: Uuid,
    pub entity: String,
    pub entity_id: Option<String>,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    pub payload_schema_ref: String,
    pub payload: serde_json::Value,
}

impl Envelope {
    pub fn from_outbox(row: OutboxRow) -> Self {
        Self {
            id: row.id,
            event_type: row.event_type,
            org_id: row.org_id,
            entity: row.entity,
            entity_id: row.entity_id,
            occurred_at: row.occurred_at,
            payload_schema_ref: row.payload_schema_ref,
            payload: row.payload,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_else(|_| serde_json::json!({}))
    }
}

/// The single Redis stream every event flows through.
pub const EVENT_STREAM: &str = "meridian:events";

#[derive(Debug, thiserror::Error)]
pub enum EventError {
    #[error("redis: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("db: {0}")]
    Db(#[from] core_db::DbError),
    #[error("sql: {0}")]
    Sql(#[from] sqlx::Error),
}

pub struct Publisher {
    conn: redis::aio::MultiplexedConnection,
}

impl Publisher {
    pub async fn connect(redis_url: &str) -> Result<Self, EventError> {
        let client = redis::Client::open(redis_url)?;
        let conn = client.get_multiplexed_async_connection().await?;
        Ok(Self { conn })
    }

    /// Appends an envelope to the stream. Max-len trims to keep stream size
    /// bounded; consumers reconcile via REST snapshots, not full replay.
    pub async fn publish(&mut self, envelope: &Envelope) -> Result<(), EventError> {
        let json = envelope.to_json();
        let _: String = self
            .conn
            .xadd(
                EVENT_STREAM,
                "*",
                &[
                    ("id", envelope.id.to_string().as_str()),
                    ("type", envelope.event_type.as_str()),
                    ("data", json.to_string().as_str()),
                ],
            )
            .await?;
        Ok(())
    }
}

/// Relay loop: drains unpublished outbox rows and publishes them to the
/// stream inside one transaction per batch (fetch → publish → mark).
pub async fn relay_once(
    pool: &sqlx::PgPool,
    publisher: &mut Publisher,
    batch: i64,
) -> Result<usize, EventError> {
    let mut tx = pool.begin().await?;
    let rows = core_db::outbox::fetch_unpublished(&mut *tx, batch).await?;
    if rows.is_empty() {
        return Ok(0);
    }
    let mut ids = Vec::with_capacity(rows.len());
    for row in rows {
        let envelope = Envelope::from_outbox(row);
        publisher.publish(&envelope).await?;
        ids.push(envelope.id);
    }
    core_db::outbox::mark_published(&mut *tx, &ids).await?;
    tx.commit().await?;
    Ok(ids.len())
}

/// Long-running relay task with backoff on error.
pub async fn run_relay(
    pool: sqlx::PgPool,
    mut publisher: Publisher,
    batch: i64,
) -> Result<(), EventError> {
    let mut backoff_ms = 100_u64;
    loop {
        match relay_once(&pool, &mut publisher, batch).await {
            Ok(0) => {
                backoff_ms = 100;
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
            Ok(_) => {
                backoff_ms = 100;
            }
            Err(e) => {
                tracing::error!(error = %e, "event relay failed");
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                backoff_ms = (backoff_ms * 2).min(10_000);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_serializes_with_type_field() {
        let e = Envelope {
            id: Uuid::now_v7(),
            event_type: "ticket.created".into(),
            org_id: Uuid::now_v7(),
            entity: "ticket".into(),
            entity_id: Some(Uuid::now_v7().to_string()),
            occurred_at: chrono::Utc::now(),
            payload_schema_ref: "core-domain/v1".into(),
            payload: serde_json::json!({"title": "x"}),
        };
        let v = e.to_json();
        assert_eq!(v["type"], "ticket.created");
        let round: Envelope = serde_json::from_value(v).unwrap();
        assert_eq!(round, e);
    }
}
