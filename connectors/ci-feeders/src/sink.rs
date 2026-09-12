//! The Meridian-side apply seam ([`FeederSink`]) plus the shipped
//! implementations.
//!
//! The sink owns the WIRE mechanics (REST, auth, envelopes); the engine
//! owns SEMANTICS (batches, reports, none-mode counting). Shipped
//! implementations:
//!
//! * [`MeridianFeederSink`] — the public v1 REST feed-upsert over a
//!   scoped machine API key (x-api-key). Every write carries the
//!   deterministic instant-first idempotency key from the record
//!   itself, so retried syncs replay and changed content updates.
//! * [`NoneFeederSink`] — zero I/O, every call reports
//!   [`SinkError::Disabled`]. THE shipped default: an org with no
//!   Meridian connection configured operates standalone, and "would
//!   have synced" is the honest answer (standalone-first doctrine).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::model::FeederRecord;

/// Typed errors surfaced by a sink.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SinkError {
    /// None mode: no credentials configured; zero network performed.
    #[error("feeder sink disabled: no Meridian connection configured (none mode)")]
    Disabled,
    /// The Meridian API rejected the credential (401/403) — the
    /// operator rotates the scoped key.
    #[error("sink unauthorized: {0}")]
    Unauthorized(String),
    #[error("sink rate limited: {0}")]
    RateLimited(String),
    /// The API refused the write (422 validation / 409 conflict).
    #[error("sink rejected the write: {0}")]
    Rejected(String),
    #[error("sink transport failure: {0}")]
    Network(String),
}

/// What one feed-upsert write did — the additive evidence packet the
/// feed-upsert response carries (`created_item`, `resolved_existing`,
/// `auto_merged`, `queued_candidates`, `contradictions`) reduced to the
/// facts a sync report needs. `source_link_id` + `config_item_id` are
/// the CI2 link identity a conformance run asserts against.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedUpsertOutcome {
    pub config_item_id: Uuid,
    pub source_link_id: Uuid,
    pub source_system: String,
    pub external_id: String,
    pub created_item: bool,
    pub resolved_existing: bool,
    pub auto_merged: bool,
    pub queued_candidates: usize,
    pub contradictions: Vec<String>,
}

/// The feeder apply seam.
#[async_trait]
pub trait FeederSink: Send + Sync {
    /// Upserts ONE record through the feed-upsert seam, bound to one
    /// Meridian client. Never blind-inserts — that property lives in
    /// the core's feed_upsert; the sink's job is to deliver the
    /// identity keys that make resolution deterministic.
    async fn feed_upsert(
        &self,
        record: &FeederRecord,
        client_id: Uuid,
    ) -> Result<FeedUpsertOutcome, SinkError>;
}

#[derive(Debug, Deserialize)]
struct WireSourceLink {
    id: Uuid,
    source_system: String,
    external_id: String,
}

#[derive(Debug, Deserialize)]
struct WireFeedUpsertResponse {
    id: Uuid,
    #[serde(default)]
    source_link: Option<WireSourceLink>,
    #[serde(default)]
    created_item: bool,
    #[serde(default)]
    resolved_existing: bool,
    #[serde(default)]
    auto_merged: bool,
    #[serde(default)]
    queued_candidates: Vec<Value>,
    #[serde(default)]
    contradictions: Vec<String>,
}

/// Credentials + endpoint configuration for [`MeridianFeederSink`].
/// Values come from the operator's secret store just-in-time — the
/// crate never persists them. `Debug` redacts the key material.
#[derive(Clone)]
pub struct MeridianFeederConfig {
    /// Meridian core API base URL (e.g. `https://meridian.example`).
    pub base_url: String,
    /// Scoped machine API key (raw `mrd_…` material). Never logged.
    /// Feeders need `clients:write` scope (config-item writes ride the
    /// Clients matrix; reads take `clients:read`).
    pub api_key: String,
}

impl std::fmt::Debug for MeridianFeederConfigDebug<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeridianFeederConfig")
            .field("base_url", &self.0.base_url)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

/// Debug wrapper so config can carry redacted key material.
pub struct MeridianFeederConfigDebug<'a>(&'a MeridianFeederConfig);

/// The public-API sink implementation.
pub struct MeridianFeederSink {
    http: reqwest::Client,
    config: MeridianFeederConfig,
}

impl std::fmt::Debug for MeridianFeederSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        MeridianFeederConfigDebug(&self.config).fmt(f)
    }
}

impl MeridianFeederSink {
    pub fn new(config: MeridianFeederConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            config,
        }
    }
}

#[async_trait]
impl FeederSink for MeridianFeederSink {
    async fn feed_upsert(
        &self,
        record: &FeederRecord,
        client_id: Uuid,
    ) -> Result<FeedUpsertOutcome, SinkError> {
        let url = format!(
            "{}/api/v1/config-items/feed-upsert",
            self.config.base_url.trim_end_matches('/')
        );
        let response = self
            .http
            .post(url)
            .header("x-api-key", &self.config.api_key)
            .json(&record.request_body(client_id))
            .send()
            .await
            .map_err(|e| SinkError::Network(e.to_string()))?;

        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        match status {
            reqwest::StatusCode::OK => {
                let parsed: WireFeedUpsertResponse = serde_json::from_value(body).map_err(|e| {
                    SinkError::Rejected(format!("feed-upsert response unreadable: {e}"))
                })?;
                let (source_system, external_id) = parsed
                    .source_link
                    .as_ref()
                    .map(|l| (l.source_system.clone(), l.external_id.clone()))
                    .unwrap_or_else(|| (record.source_system.clone(), record.external_id.clone()));
                Ok(FeedUpsertOutcome {
                    config_item_id: parsed.id,
                    source_link_id: parsed.source_link.map(|l| l.id).unwrap_or_default(),
                    source_system,
                    external_id,
                    created_item: parsed.created_item,
                    resolved_existing: parsed.resolved_existing,
                    auto_merged: parsed.auto_merged,
                    queued_candidates: parsed.queued_candidates.len(),
                    contradictions: parsed.contradictions,
                })
            }
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                Err(SinkError::Unauthorized(body_to_message(&body)))
            }
            reqwest::StatusCode::TOO_MANY_REQUESTS => {
                Err(SinkError::RateLimited(body_to_message(&body)))
            }
            code => Err(SinkError::Rejected(format!(
                "feed-upsert returned {code}: {}",
                body_to_message(&body)
            ))),
        }
    }
}

fn body_to_message(body: &Value) -> String {
    body.get("message")
        .or_else(|| body.get("error"))
        .and_then(Value::as_str)
        .unwrap_or("unspecified")
        .to_string()
}

/// None mode: the shipped default. Every call reports [`SinkError::Disabled`];
/// the engine counts those as "would have synced" (standalone-first).
pub struct NoneFeederSink;

#[async_trait]
impl FeederSink for NoneFeederSink {
    async fn feed_upsert(
        &self,
        _record: &FeederRecord,
        _client_id: Uuid,
    ) -> Result<FeedUpsertOutcome, SinkError> {
        Err(SinkError::Disabled)
    }
}
