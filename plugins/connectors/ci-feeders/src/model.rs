//! The vendor-neutral feeder record: everything one feed-upsert write
//! needs, mapped out of a vendor payload by a [`crate::vendors`]
//! normalizer. This is the crate's whole vocabulary — the wire shape the
//! engine and sink agree on.

use serde_json::Value;
use uuid::Uuid;

/// One configuration item arriving from one source system.
///
/// `identity_keys` and `observed_fields` stay JSON on purpose: the feed
/// contract (and the core's CI3/CI5 normalizers) is key-addressed, and
/// vendor payloads differ too much for a rigid struct. Determinism is
/// guaranteed by [`crate::identity`], not by the container.
#[derive(Debug, Clone, PartialEq)]
pub struct FeederRecord {
    /// The Meridian source system id: `ninjaone` | `it_glue` | `auvix`.
    /// Lowercase, stable — this is the CI2 link key's first component.
    pub source_system: String,
    /// The record's id IN the source system (stringified when the vendor
    /// ships numbers). The CI2 link key's second component.
    pub external_id: String,
    /// The vendor's own GUID when it carries one (an identity signal the
    /// core scores at 1.0).
    pub source_guid: Option<String>,
    /// Opaque reference to the raw payload in the feeder's own store
    /// (a URL, a bucket path, an export filename). Never parsed by the
    /// core.
    pub payload_ref: Option<String>,
    /// Display defaults for a freshly created canonical item; on an
    /// existing item the core's observed/curated rules govern.
    pub tag: String,
    pub name: String,
    /// CI3 deterministic identity signals (see [`crate::identity`]).
    pub identity_keys: Value,
    /// CI5 observed tier: feed-written fields that churn with the source.
    pub observed_fields: Value,
}

impl FeederRecord {
    /// The deterministic instant-first idempotency key for this record's
    /// CURRENT content. Same content ⇒ same key ⇒ the core replays the
    /// stored result (idempotent upsert); changed content ⇒ fresh key ⇒
    /// the record lands as its own source's update, never a 409 and
    /// never a twin.
    pub fn idempotency_key(&self) -> String {
        format!(
            "ci-feeder-{}-{}-{}",
            self.source_system,
            self.external_id,
            &crate::identity::fingerprint(self)[..16]
        )
    }

    /// The record this sync targets, under one Meridian client.
    pub(crate) fn request_body(&self, client_id: Uuid) -> Value {
        serde_json::json!({
            "source_system": self.source_system,
            "external_id": self.external_id,
            "source_guid": self.source_guid,
            "payload_ref": self.payload_ref,
            "client_id": client_id,
            "tag": self.tag,
            "name": self.name,
            "identity_keys": self.identity_keys,
            "observed_fields": self.observed_fields,
            "idempotency_key": self.idempotency_key(),
        })
    }
}
