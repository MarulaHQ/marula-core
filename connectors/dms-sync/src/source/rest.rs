//! Generic REST source: the escape hatch for documentation systems
//! that are neither IT Glue nor Hudu (and the transport for LIVE pulls
//! from their APIs when an org prefers that over exports).
//!
//! The operator declares, per record family: the list endpoint, which
//! JSON keys name the record's id, type, client scope, and body, and
//! how external field keys rename into the neutral vocabulary (identity
//! keys live here too — `serial`, `hostname`, `domain` — so
//! deterministic identity works for any DMS). Layouts are INFERRED from
//! the union of observed field keys per type (kinded via the operator's
//! `field_kinds` map, text by default) — good enough to seed connector
//! org layouts losslessly.
//!
//! Credentials: none. A generic REST source never extracts secrets —
//! the vault boundary starts at "nothing crosses unless the operator
//! mapped it explicitly", and a generic endpoint map has no way to
//! prove a field is a credential. Vendor sources (IT Glue / Hudu
//! parsers) carry the opt-in credential extraction instead.

use serde_json::Value;

use crate::model::{DmsSnapshot, FieldKind, LinkKind, SyncField, SyncLayout, SyncRecord};

/// One mapped external collection.
#[derive(Debug, Clone)]
pub struct RestCollection {
    /// Source-relative path, e.g. `configurations` or `api/v2/assets`.
    pub path: String,
    /// Which Meridian side these records map to.
    pub record_kind: RecordKind,
    /// JSON key of the record's external id.
    pub id_key: String,
    /// JSON key naming the record's type (layout mapping input).
    pub type_key: Option<String>,
    /// JSON key carrying the record's client/organization scope; its
    /// VALUE becomes the record's `client_ref`.
    pub client_key: Option<String>,
    /// JSON key for the record title/name.
    pub title_key: String,
    /// JSON key for a document body (documents only).
    pub body_key: Option<String>,
    /// External JSON key → neutral field key. Keys not listed carry
    /// under their external name — nothing is dropped, so inferred
    /// layouts stay lossless.
    pub field_map: std::collections::BTreeMap<String, String>,
    /// Neutral field key → docs-platform field kind for the seeded
    /// layout schema. Unlisted keys default to text.
    pub field_kinds: std::collections::BTreeMap<String, FieldKind>,
}

/// Which Meridian side a collection maps onto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    Asset,
    Document,
}

impl RecordKind {
    pub fn link_kind(self) -> LinkKind {
        match self {
            RecordKind::Asset => LinkKind::Asset,
            RecordKind::Document => LinkKind::Document,
        }
    }
}

/// A generic REST DMS source: `GET {base}/{path}` per collection.
/// Records arrive as `{"data": [...]}` (unwrapped via `unwrap_key`)
/// or a bare array.
pub struct RestSource {
    pub base_url: String,
    pub unwrap_key: Option<String>,
    pub collections: Vec<RestCollection>,
    /// HTTP client (injectable for tests).
    pub http: reqwest::Client,
}

impl RestSource {
    pub fn new(base_url: impl Into<String>, collections: Vec<RestCollection>) -> Self {
        Self {
            base_url: base_url.into(),
            unwrap_key: Some("data".into()),
            collections,
            http: reqwest::Client::new(),
        }
    }

    /// The pull, in async form — the natural call shape for a REST
    /// driver. Call from a plain (sync) context via
    /// `tokio::runtime::Handle::block_on`, or compose inside an async
    /// driver directly.
    pub async fn pull_async(&self) -> Result<DmsSnapshot, crate::model::SyncError> {
        let mut snapshot = DmsSnapshot {
            source: "generic-rest".into(),
            ..Default::default()
        };

        for collection in &self.collections {
            let url = format!(
                "{}/{}",
                self.base_url.trim_end_matches('/'),
                collection.path.trim_start_matches('/')
            );
            let response = self
                .http
                .get(&url)
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|e| crate::model::SyncError::Parse(format!("REST pull {url}: {e}")))?;
            if !response.status().is_success() {
                return Err(crate::model::SyncError::Parse(format!(
                    "REST pull {url}: status {}",
                    response.status()
                )));
            }
            let mut body: Value = response.json().await.map_err(|e| {
                crate::model::SyncError::Parse(format!("REST pull {url}: body: {e}"))
            })?;
            if let Some(key) = &self.unwrap_key {
                body = body.get(key).cloned().unwrap_or(Value::Null);
            }
            let Some(items) = body.as_array() else {
                return Err(crate::model::SyncError::Parse(format!(
                    "REST collection {} did not decode to an array",
                    collection.path
                )));
            };

            // Per-type union of observed field keys (lossless layout
            // seeds). Order of first observation.
            let mut observed: std::collections::BTreeMap<String, OrderedKeys> = Default::default();

            for item in items {
                let external_id = scalar_id(item.get(&collection.id_key));
                if external_id.is_empty() {
                    continue;
                }
                let title = item
                    .get(&collection.title_key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let type_name = collection
                    .type_key
                    .as_ref()
                    .and_then(|k| item.get(k))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| collection.path.clone());

                let mut fields = std::collections::BTreeMap::new();
                if let Some(map) = item.as_object() {
                    for (key, value) in map {
                        if key == "id"
                            || Some(key) == collection.client_key.as_ref()
                            || Some(key) == collection.type_key.as_ref()
                            || collection.body_key.as_ref() == Some(key)
                        {
                            continue;
                        }
                        let neutral = collection
                            .field_map
                            .get(key)
                            .cloned()
                            .unwrap_or_else(|| key.clone());
                        let string = match value {
                            Value::String(s) => s.clone(),
                            Value::Number(n) => n.to_string(),
                            Value::Bool(b) => b.to_string(),
                            Value::Null => continue,
                            other => other.to_string(),
                        };
                        if !string.is_empty() {
                            fields.insert(neutral, string);
                        }
                    }
                }

                let per_type = observed.entry(type_name.clone()).or_default();
                for key in fields.keys() {
                    let kind = collection
                        .field_kinds
                        .get(key)
                        .copied()
                        .unwrap_or(FieldKind::Text);
                    per_type.observe(key.clone(), kind);
                }

                snapshot.records.push(SyncRecord {
                    external_id,
                    kind: collection.record_kind.link_kind(),
                    client_ref: collection
                        .client_key
                        .as_ref()
                        .and_then(|k| item.get(k))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    title,
                    type_name: Some(type_name),
                    fields,
                    body: collection
                        .body_key
                        .as_ref()
                        .and_then(|k| item.get(k))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    tag: None,
                    credentials: Vec::new(),
                });
            }

            // Seed one layout per observed type from the union of keys.
            for (type_name, keys) in observed {
                let mut fields = Vec::new();
                for (position, key) in keys.keys.clone().into_iter().enumerate() {
                    fields.push(SyncField::new(
                        key.clone(),
                        key.clone(),
                        keys.kind_of(&key),
                        i32::try_from(position).unwrap_or(i32::MAX),
                    ));
                }
                snapshot.layouts.push(SyncLayout {
                    external_id: format!("{}:{}", collection.path, type_name),
                    name: type_name,
                    description: format!("synced from {} (generic REST)", collection.path),
                    icon: "file".into(),
                    fields,
                });
            }
        }

        Ok(snapshot)
    }
}

/// Insertion-ordered field keys + their kinds (layout seeding needs
/// stable positions; std BTreeMap would alphabetize the schema).
#[derive(Debug, Default)]
struct OrderedKeys {
    keys: Vec<String>,
    kinds: std::collections::BTreeMap<String, FieldKind>,
}

impl OrderedKeys {
    fn observe(&mut self, key: String, kind: FieldKind) {
        self.kinds.entry(key.clone()).or_insert(kind);
        if !self.keys.contains(&key) {
            self.keys.push(key);
        }
    }

    fn kind_of(&self, key: &str) -> FieldKind {
        self.kinds.get(key).copied().unwrap_or(FieldKind::Text)
    }
}

fn scalar_id(value: Option<&Value>) -> String {
    match value {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.trim().to_string(),
        _ => String::new(),
    }
}

impl crate::source::DmsSource for RestSource {
    fn name(&self) -> &str {
        "generic-rest"
    }

    /// Sync-shaped pull for plain drivers: bridges onto a tokio
    /// runtime. Outside a runtime this errors — use `pull_async`
    /// inside an async driver instead (documented in the README).
    fn pull(&self) -> Result<DmsSnapshot, crate::model::SyncError> {
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle.block_on(self.pull_async()),
            Err(_) => Err(crate::model::SyncError::Parse(
                "generic REST source must be pulled inside or against a tokio runtime \
                 (use pull_async within an async driver)"
                    .into(),
            )),
        }
    }
}
