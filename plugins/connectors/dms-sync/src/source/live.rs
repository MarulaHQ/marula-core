//! LIVE REST pulls from the IT Glue / Hudu APIs (the TASKLIST DMS
//! follow-up): the export-based parsers in [`super::it_glue`] /
//! [`super::hudu`] stay zero-network; these drivers pull the SAME
//! snapshot vocabulary from the vendor APIs with their native
//! pagination/auth flows:
//!
//! * **IT Glue** — `x-api-key` header, JSON:API `page[number]` +
//!   `links.next` pagination; organizations, configurations, and
//!   flexible assets.
//! * **Hudu** — bearer API key, `page`/`per_page` pagination with
//!   short-page termination; companies, asset layouts, and assets.
//!
//! Same rules as every live driver: every page is followed (a silently
//! truncated snapshot is a wrong snapshot), credential material is
//! never logged, and records keep the vendor's own field names —
//! the sync engine + layout inference handle the rest (same as the
//! export parsers).

use std::collections::BTreeMap;

use serde_json::Value;

use adapters_core::client::{PageFlow, VendorClient, VendorError};
use adapters_core::VendorAuth;

use crate::model::{
    DmsSnapshot, FieldKind, LinkKind, SyncError, SyncField, SyncLayout, SyncRecord,
};

/// One pulled collection → snapshot section.
struct Pulled {
    items: Vec<Value>,
    /// Type name for the layout seed (records carry it per item).
    type_name: String,
    kind: LinkKind,
    id_key: &'static str,
    title_key: &'static str,
    client_key: Option<&'static str>,
    body_key: Option<&'static str>,
}

impl Pulled {
    fn into_snapshot_section(self, snapshot: &mut DmsSnapshot) {
        // Insertion-ordered field keys + kinds (stable positions for
        // the seeded layout schema).
        let mut observed: BTreeMap<String, OrderedKeys> = Default::default();

        for item in &self.items {
            let attributes = match item.get("attributes").and_then(Value::as_object) {
                Some(attributes) => attributes,
                None => match item.as_object() {
                    Some(object) => object,
                    None => continue,
                },
            };
            let external_id = match item
                .get(self.id_key)
                .or_else(|| attributes.get(self.id_key))
            {
                Some(Value::String(s)) => s.trim().to_string(),
                Some(Value::Number(n)) => n.to_string(),
                _ => continue, // no id → unrecordable; skipped, not guessed
            };
            let title = item
                .get(self.title_key)
                .or_else(|| attributes.get(self.title_key))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let client_ref = self.client_key.and_then(|key| {
                item.get(key)
                    .or_else(|| attributes.get(key))
                    .and_then(|v| match v {
                        Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
                        Value::Number(n) => Some(n.to_string()),
                        Value::Object(o) => o.get("id").and_then(Value::as_str).map(str::to_string),
                        _ => None,
                    })
            });
            let body = self
                .body_key
                .and_then(|key| attributes.get(key).or_else(|| item.get(key)))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();

            let mut fields = BTreeMap::new();
            for (key, value) in attributes {
                if key == "id"
                    || Some(key.as_str()) == Some(self.title_key)
                    || Some(key.as_str()) == self.client_key
                    || Some(key.as_str()) == self.body_key
                {
                    continue;
                }
                let string = match value {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    Value::Null => continue,
                    other => other.to_string(),
                };
                if !string.is_empty() {
                    fields.insert(key.clone(), string);
                }
            }
            let per_type = observed.entry(self.type_name.clone()).or_default();
            for key in fields.keys() {
                per_type.observe(key.clone(), FieldKind::Text);
            }

            snapshot.records.push(SyncRecord {
                external_id,
                kind: self.kind,
                client_ref,
                title,
                type_name: Some(self.type_name.clone()),
                fields,
                body,
                tag: None,
                credentials: Vec::new(),
            });
        }

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
                external_id: format!("live:{}", type_name),
                name: type_name,
                description: "seeded from a live API pull".into(),
                icon: "file".into(),
                fields,
            });
        }
    }
}

/// Insertion-ordered field keys + kinds (layout seeding needs stable
/// positions; std BTreeMap would alphabetize the schema).
#[derive(Debug, Default)]
struct OrderedKeys {
    keys: Vec<String>,
    kinds: BTreeMap<String, FieldKind>,
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

fn vendor_err(err: VendorError) -> SyncError {
    SyncError::Parse(format!("live DMS pull: {err}"))
}

/// IT Glue LIVE source: `x-api-key` + JSON:API pagination, pulling
/// organizations (client refs), configurations (typed assets), and
/// flexible assets (assets + documents by layout name).
pub struct ItGlueLive {
    inner: VendorClient,
    /// Only these flexible-asset layouts are pulled (`None` = all).
    pub asset_layouts: Option<Vec<String>>,
}

impl ItGlueLive {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            inner: VendorClient::new(
                base_url,
                VendorAuth::ApiKeyHeader {
                    header: "x-api-key".into(),
                    key: api_key.into(),
                },
            ),
            asset_layouts: None,
        }
    }

    async fn list(&self, path: &str) -> Result<Vec<Value>, SyncError> {
        self.inner
            .fetch_all(
                path,
                &PageFlow::JsonApi {
                    items_key: "data",
                    page_param: "page[number]",
                    page_size: 50,
                },
            )
            .await
            .map_err(vendor_err)
    }

    /// The live pull (async form; `DmsSource::pull` bridges via the
    /// tokio runtime handle, same as `RestSource`).
    pub async fn pull_async(&self) -> Result<DmsSnapshot, SyncError> {
        let mut snapshot = DmsSnapshot {
            source: "it-glue".into(),
            ..Default::default()
        };

        let orgs = self.list("/organizations").await?;
        for (id, name) in orgs.iter().filter_map(|org| {
            let id = org.get("id").and_then(Value::as_str)?;
            let name = org
                .pointer("/attributes/name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Some((id.to_string(), name.to_string()))
        }) {
            snapshot.client_refs.insert(id, name);
        }

        Pulled {
            items: self.list("/configurations").await?,
            type_name: "Configuration".into(),
            kind: LinkKind::Asset,
            id_key: "id",
            title_key: "name",
            client_key: Some("organization_id"),
            body_key: None,
        }
        .into_snapshot_section(&mut snapshot);

        let layouts = match &self.asset_layouts {
            Some(names) => names.clone(),
            None => self
                .list("/flexible_asset_types")
                .await?
                .iter()
                .filter_map(|t| {
                    t.pointer("/attributes/name")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect(),
        };
        for layout in layouts {
            let items = self
                .list(&format!(
                    "/flexible_assets?filter[flexible_asset_type_name]={layout}"
                ))
                .await?;
            Pulled {
                items,
                type_name: layout,
                kind: LinkKind::Asset,
                id_key: "id",
                title_key: "name",
                client_key: Some("organization_id"),
                body_key: Some("notes"),
            }
            .into_snapshot_section(&mut snapshot);
        }

        Ok(snapshot)
    }
}

impl crate::source::DmsSource for ItGlueLive {
    fn name(&self) -> &str {
        "it-glue"
    }

    fn pull(&self) -> Result<DmsSnapshot, SyncError> {
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle.block_on(self.pull_async()),
            Err(_) => Err(SyncError::Parse(
                "live DMS sources must be pulled inside or against a tokio runtime \
                 (use pull_async within an async driver)"
                    .into(),
            )),
        }
    }
}

/// Hudu LIVE source: bearer API key + page/per_page pagination with
/// short-page termination; companies (client refs), asset layouts, and
/// assets.
pub struct HuduLive {
    inner: VendorClient,
    page_size: usize,
}

impl HuduLive {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            inner: VendorClient::new(base_url, VendorAuth::Bearer(api_key.into())),
            page_size: 50,
        }
    }

    async fn list(&self, path: &str, items_key: &'static str) -> Result<Vec<Value>, SyncError> {
        self.inner
            .fetch_all(
                path,
                &PageFlow::PageNumber {
                    items_key,
                    page_param: "page",
                    size_param: Some("per_page"),
                    page_size: self.page_size,
                },
            )
            .await
            .map_err(vendor_err)
    }

    pub async fn pull_async(&self) -> Result<DmsSnapshot, SyncError> {
        let mut snapshot = DmsSnapshot {
            source: "hudu".into(),
            ..Default::default()
        };

        let companies = self.list("/api/v1/companies", "companies").await?;
        for (id, name) in companies.iter().filter_map(|c| {
            let id = c.get("id")?.to_string();
            let name = c
                .pointer("/attributes/name")
                .or_else(|| c.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            Some((id, name.to_string()))
        }) {
            snapshot.client_refs.insert(id, name);
        }

        Pulled {
            items: self.list("/api/v1/assets", "assets").await?,
            type_name: "Asset".into(),
            kind: LinkKind::Asset,
            id_key: "id",
            title_key: "name",
            client_key: Some("company_id"),
            body_key: Some("description"),
        }
        .into_snapshot_section(&mut snapshot);

        Ok(snapshot)
    }
}

impl crate::source::DmsSource for HuduLive {
    fn name(&self) -> &str {
        "hudu"
    }

    fn pull(&self) -> Result<DmsSnapshot, SyncError> {
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle.block_on(self.pull_async()),
            Err(_) => Err(SyncError::Parse(
                "live DMS sources must be pulled inside or against a tokio runtime \
                 (use pull_async within an async driver)"
                    .into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::extract::Query;
    use axum::http::HeaderMap;
    use axum::routing::get;
    use axum::{Json, Router};
    use serde_json::json;

    use super::*;

    async fn spawn(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn it_glue_live_pulls_all_pages_with_orgs_configs_and_assets() {
        let app = Router::new()
            .route(
                "/organizations",
                get(|| async {
                    Json(json!({
                        "data": [{"type": "organizations", "id": "77", "attributes": {"name": "Acme"}}],
                        "links": {}
                    }))
                }),
            )
            .route(
                "/configurations",
                get(|| async {
                    Json(json!({
                        "data": [{
                            "type": "configurations", "id": "c1",
                            "attributes": {
                                "name": "srv-a", "hostname": "srv-a.acme",
                                "serial_number": "SN-1",
                                "organization_id": {"id": "77", "name": "Acme"}
                            }
                        }],
                        "links": {}
                    }))
                }),
            )
            .route(
                "/flexible_asset_types",
                get(|| async {
                    Json(json!({
                        "data": [{"type": "flexible_asset_types", "id": "t1", "attributes": {"name": "Passwords"}}],
                        "links": {}
                    }))
                }),
            )
            .route(
                "/flexible_assets",
                get(|Query(query): Query<HashMap<String, String>>| async move {
                    // All pages must carry the layout filter.
                    assert!(query
                        .get("filter[flexible_asset_type_name]")
                        .is_some_and(|f| f == "Passwords"));
                    Json(json!({
                        "data": [{
                            "type": "flexible_assets", "id": "f1",
                            "attributes": {
                                "name": "root", "notes": "secret body",
                                "organization_id": {"id": "77", "name": "Acme"}
                            }
                        }],
                        "links": {}
                    }))
                }),
            );
        let base = spawn(app).await;

        let live = ItGlueLive::new(&base, "key");
        let snapshot = live.pull_async().await.unwrap();

        assert_eq!(snapshot.source, "it-glue");
        assert_eq!(
            snapshot.client_refs.get("77").map(String::as_str),
            Some("Acme")
        );
        let configs: Vec<_> = snapshot
            .records
            .iter()
            .filter(|r| r.type_name.as_deref() == Some("Configuration"))
            .collect();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].external_id, "c1");
        assert_eq!(configs[0].fields["serial_number"], "SN-1");
        assert_eq!(configs[0].client_ref.as_deref(), Some("77"));
        let assets: Vec<_> = snapshot
            .records
            .iter()
            .filter(|r| r.type_name.as_deref() == Some("Passwords"))
            .collect();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].body, "secret body");
        // Layout seeded from the union of observed keys per type.
        assert!(snapshot
            .layouts
            .iter()
            .any(|l| l.name == "Configuration" && !l.fields.is_empty()));
    }

    #[tokio::test]
    async fn hudu_live_pulls_with_bearer_and_paged_assets() {
        let page: Arc<std::sync::atomic::AtomicUsize> =
            Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let page_state = page.clone();
        let app = Router::new()
            .route(
                "/api/v1/companies",
                get(|headers: HeaderMap| async move {
                    assert_eq!(
                        headers.get("authorization").unwrap(),
                        "Bearer hudu-key"
                    );
                    Json(json!({
                        "companies": [{"id": 5, "name": "Beta LLC", "attributes": {"name": "Beta LLC"}}]
                    }))
                }),
            )
            .route(
                "/api/v1/assets",
                get(move |headers: HeaderMap| async move {
                    assert_eq!(
                        headers.get("authorization").unwrap(),
                        "Bearer hudu-key"
                    );
                    let n = page_state.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if n == 0 {
                        Json(json!({
                            "assets": (0..50).map(|i| json!({
                                "id": i, "name": format!("asset-{i}"),
                                "company_id": 5,
                                "description": "desc"
                            })).collect::<Vec<_>>()
                        }))
                    } else {
                        Json(json!({
                            "assets": [{"id": 999, "name": "asset-999", "company_id": 5}]
                        }))
                    }
                }),
            );
        let base = spawn(app).await;

        let live = HuduLive::new(&base, "hudu-key");
        let snapshot = live.pull_async().await.unwrap();

        assert_eq!(snapshot.source, "hudu");
        assert_eq!(
            snapshot.client_refs.get("5").map(String::as_str),
            Some("Beta LLC")
        );
        assert_eq!(snapshot.records.len(), 51); // full page + short page
        assert_eq!(snapshot.records[50].external_id, "999");
        assert_eq!(snapshot.records[0].client_ref.as_deref(), Some("5"));
        assert_eq!(page.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
