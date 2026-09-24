//! Push side (parity): Meridian documentation rendered back OUT in the
//! same formats the pull side consumes — so an org that keeps its DMS
//! gets full data movement in BOTH directions.
//!
//! Three pieces:
//!
//! * [`MeridianSnapshot`] — the neutral shape a Meridian-side reader
//!   produces from the public REST surface (`GET /api/assets`,
//!   `/api/documents`, `/api/documents/layouts`, `/api/clients`, and
//!   the per-record link listings).
//! * Renderers — [`render_it_glue_export`] and [`render_hudu_export`]
//!   produce export JSON in the SAME file shapes the IT Glue / Hudu
//!   parsers read (round-trip tested in `tests`).
//! * [`RestPushTarget`] — a generic REST DMS writer: POST/PUT the
//!   neutral records to an operator-declared endpoint map (the escape
//!   hatch for DMSes with no export format).
//!
//! Non-purpose: no IT Glue / Hudu write-API clients (their REST write
//! surfaces are vendor-specific; the renderers are the portable
//! contract), no attachment pushes (v1), and no credential rendering
//! — vault secrets never leave Meridian's audited path, so the push
//! side carries password METADATA only unless the caller supplies the
//! secret explicitly.

use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::model::{FieldKind, LinkKind, SyncField, SyncRecord};

/// One Meridian-side link (resolved form).
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotLink {
    pub from_kind: LinkKind,
    pub from_id: Uuid,
    pub to_kind: LinkKind,
    pub to_id: Uuid,
}

/// The neutral Meridian snapshot the renderers consume.
#[derive(Debug, Clone, Default)]
pub struct MeridianSnapshot {
    pub source: String,
    /// (layout id → layout)
    pub layouts: Vec<SnapshotLayout>,
    /// (client id → client name)
    pub clients: Vec<(Uuid, String)>,
    pub assets: Vec<SnapshotAsset>,
    pub documents: Vec<SnapshotDocument>,
    pub links: Vec<SnapshotLink>,
    /// Password METADATA (title/username/url) — never secrets.
    pub credential_metadata: Vec<SnapshotCredentialMeta>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotLayout {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub is_system: bool,
    pub fields: Vec<SyncField>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotAsset {
    pub id: Uuid,
    pub client_id: Option<Uuid>,
    pub tag: String,
    pub name: String,
    pub kind: String,
    pub status: String,
    pub layout_id: Option<Uuid>,
    /// Schema-validated custom fields (the docs-platform shape).
    pub fields: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotDocument {
    pub id: Uuid,
    pub client_id: Option<Uuid>,
    pub title: String,
    pub layout_id: Option<Uuid>,
    pub fields: Map<String, Value>,
    /// Current version body.
    pub body: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotCredentialMeta {
    pub id: Uuid,
    pub client_id: Option<Uuid>,
    pub title: String,
    pub username: String,
    pub url: String,
}

/// Render a Meridian snapshot as IT Glue export files (JSON:API). The
/// returned map is keyed by export file name, matching the parser's
/// [`crate::source::it_glue::ItGlueExportFiles`] slots:
/// organizations / flexible_asset_types / flexible_assets /
/// configurations.
pub fn render_it_glue_export(snapshot: &MeridianSnapshot) -> BTreeMapExport {
    let mut organizations: Vec<Value> = Vec::new();
    for (id, name) in &snapshot.clients {
        organizations.push(json!({
            "id": id.to_string(),
            "type": "organizations",
            "attributes": { "name": name },
        }));
    }

    // Layouts (connector org layouts carry the field schemas; system
    // layouts render too so the round trip is complete).
    let mut flexible_asset_types: Vec<Value> = Vec::new();
    let mut layout_slug_by_id: std::collections::BTreeMap<Uuid, String> = Default::default();
    for (position, layout) in snapshot.layouts.iter().enumerate() {
        layout_slug_by_id.insert(layout.id, layout.slug.clone());
        let traits: Vec<Value> = layout
            .fields
            .iter()
            .map(|f| {
                json!({
                    "id": f.key,
                    "type": "flexible_asset_kind",
                    "attributes": {
                        "key": f.key,
                        "name": f.label,
                        "kind": it_glue_kind(f.kind),
                        "required": f.required,
                    },
                })
            })
            .collect();
        flexible_asset_types.push(json!({
            "id": layout.id.to_string(),
            "type": "flexible_asset_types",
            "attributes": {
                "name": layout.name,
                "icon": layout.slug,
                "description": format!("meridian layout {}", layout.slug),
                "traits": { "data": traits },
            },
            "_position": position,
        }));
    }

    // Documents → flexible assets.
    let mut flexible_assets: Vec<Value> = Vec::new();
    for document in &snapshot.documents {
        let type_id = document
            .layout_id
            .map(|id| json!({ "data": { "id": id.to_string(), "type": "flexible_asset_types" } }))
            .unwrap_or(Value::Null);
        let mut traits = Map::new();
        for (key, value) in &document.fields {
            traits.insert(key.clone(), value.clone());
        }
        let organization = document
            .client_id
            .map(|id| json!({ "data": { "id": id.to_string(), "type": "organizations" } }))
            .unwrap_or(Value::Null);
        flexible_assets.push(json!({
            "id": document.id.to_string(),
            "type": "flexible_assets",
            "attributes": {
                "name": document.title,
                "flexible_asset_type_id": type_id,
                "organization_id": organization,
                "traits": traits,
            },
        }));
    }

    // Assets → configurations (identity-bearing fields survive: serial,
    // hostname, primary ip, os, mac).
    let mut configurations: Vec<Value> = Vec::new();
    for asset in &snapshot.assets {
        let type_name = asset
            .layout_id
            .and_then(|id| layout_slug_by_id.get(&id))
            .map(String::as_str)
            .unwrap_or("Configuration");
        let configuration_type = json!({
            "data": {
                "id": format!("{type_name}"),
                "type": "configuration_types",
                "attributes": { "name": type_name },
            }
        });
        let mut attributes = Map::new();
        attributes.insert("name".into(), json!(asset.name));
        attributes.insert(
            "hostname".into(),
            asset.fields.get("hostname").cloned().unwrap_or(Value::Null),
        );
        attributes.insert(
            "primary_ip".into(),
            asset
                .fields
                .get("primary_ip")
                .cloned()
                .unwrap_or(Value::Null),
        );
        attributes.insert(
            "mac_address".into(),
            asset
                .fields
                .get("mac_address")
                .cloned()
                .unwrap_or(Value::Null),
        );
        attributes.insert(
            "serial_number".into(),
            asset.fields.get("serial").cloned().unwrap_or(Value::Null),
        );
        attributes.insert(
            "operating_system".into(),
            asset.fields.get("os").cloned().unwrap_or(Value::Null),
        );
        if let Some(client_id) = asset.client_id {
            attributes.insert(
                "organization_id".into(),
                json!({ "data": { "id": client_id.to_string(), "type": "organizations" } }),
            );
        }
        attributes.insert("configuration_type_id".into(), configuration_type);
        configurations.push(json!({
            "id": asset.id.to_string(),
            "type": "configurations",
            "attributes": attributes,
        }));
    }

    BTreeMapExport {
        files: [
            ("organizations", json!({ "data": organizations })),
            (
                "flexible_asset_types",
                json!({ "data": flexible_asset_types }),
            ),
            ("flexible_assets", json!({ "data": flexible_assets })),
            ("configurations", json!({ "data": configurations })),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect(),
    }
}

/// The rendered export files, keyed by export file name.
#[derive(Debug, Clone)]
pub struct BTreeMapExport {
    pub files: std::collections::BTreeMap<String, Value>,
}

impl BTreeMapExport {
    /// Convenience: one named file's JSON.
    pub fn file(&self, name: &str) -> Option<&Value> {
        self.files.get(name)
    }
}

fn it_glue_kind(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Number => "Number",
        FieldKind::Date => "Date",
        FieldKind::Bool => "Checkbox",
        FieldKind::Select => "Select",
        FieldKind::Url => "URL",
        FieldKind::Email => "Email",
        FieldKind::Text => "Text",
    }
}

/// Render a Meridian snapshot as a Hudu export JSON document (the
/// collection keys match the parser's
/// [`crate::source::hudu::HuduExportFiles`]).
pub fn render_hudu_export(snapshot: &MeridianSnapshot) -> Value {
    let companies: Vec<Value> = snapshot
        .clients
        .iter()
        .map(|(id, name)| json!({ "id": id.to_string(), "name": name }))
        .collect();

    let asset_layouts: Vec<Value> = snapshot
        .layouts
        .iter()
        .map(|layout| {
            let fields: Vec<Value> = layout
                .fields
                .iter()
                .map(|f| {
                    let mut field = json!({
                        "label": f.label,
                        "slug": f.key,
                        "field_type": hudu_field_type(f.kind),
                        "required": f.required,
                    });
                    if !f.options.is_empty() {
                        field["options"] = json!(f.options);
                    }
                    field
                })
                .collect();
            json!({
                "id": layout.id.to_string(),
                "name": layout.name,
                "slug": layout.slug,
                "icon": layout.slug,
                "fields": fields,
            })
        })
        .collect();

    let mut assets: Vec<Value> = Vec::new();
    let mut websites: Vec<Value> = Vec::new();

    // Meridian ASSETS → Hudu asset records (their fields ride the
    // {label, value} shape).
    for asset in &snapshot.assets {
        let fields: Vec<Value> = asset
            .fields
            .iter()
            .map(|(key, value)| json!({ "label": key, "slug": key, "value": value }))
            .collect();
        assets.push(json!({
            "id": asset.id.to_string(),
            "name": asset.name,
            "asset_layout_id": asset.layout_id.map(|id| id.to_string()),
            "company_id": asset.client_id.map(|id| id.to_string()),
            "fields": fields,
        }));
    }

    for document in &snapshot.documents {
        let mut fields: Vec<Value> = Vec::new();
        for (key, value) in &document.fields {
            fields.push(json!({ "label": key, "slug": key, "value": value }));
        }
        let mut asset = json!({
            "id": document.id.to_string(),
            "name": document.title,
            "asset_layout_id": document.layout_id.map(|id| id.to_string()),
            "company_id": document.client_id.map(|id| id.to_string()),
            "fields": fields,
            "description": document.body,
        });
        // A document with only a url field renders as a website record.
        if document.fields.len() == 1 {
            if let Some(url) = document.fields.get("url").and_then(Value::as_str) {
                asset["url"] = json!(url);
                websites.push(asset);
                continue;
            }
        }
        assets.push(asset);
    }

    let passwords: Vec<Value> = snapshot
        .credential_metadata
        .iter()
        .map(|meta| {
            // METADATA ONLY: the push side never renders secrets —
            // vault material crosses Meridian's audited path alone.
            json!({
                "id": meta.id.to_string(),
                "name": meta.title,
                "company_id": meta.client_id.map(|id| id.to_string()),
                "username": meta.username,
                "url": meta.url,
            })
        })
        .collect();

    json!({
        "companies": companies,
        "asset_layouts": asset_layouts,
        "assets": assets,
        "websites": websites,
        "passwords": passwords,
    })
}

fn hudu_field_type(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Number => "number",
        FieldKind::Date => "date",
        FieldKind::Bool => "checkbox",
        FieldKind::Select => "select",
        FieldKind::Url => "website",
        FieldKind::Email => "email",
        FieldKind::Text => "text",
    }
}

/// A generic REST DMS writer: POSTs each record to
/// `{base}/{resource}`. The operator declares the resource paths; the
/// writer only carries the neutral record shapes (the smallest honest
/// seam for DMSes with no export format).
pub struct RestPushTarget {
    pub base_url: String,
    /// Record-resource paths: "layouts" | "assets" | "documents" | "links".
    pub paths: RestPushPaths,
    pub http: reqwest::Client,
}

#[derive(Debug, Clone)]
pub struct RestPushPaths {
    pub layouts: String,
    pub assets: String,
    pub documents: String,
}

impl RestPushTarget {
    pub fn new(base_url: impl Into<String>, paths: RestPushPaths) -> Self {
        Self {
            base_url: base_url.into(),
            paths,
            http: reqwest::Client::new(),
        }
    }

    /// Push one batch of records; returns the number of accepted
    /// records (2xx).
    pub async fn push_snapshot(
        &self,
        snapshot: &MeridianSnapshot,
    ) -> Result<usize, crate::model::SyncError> {
        let mut accepted = 0;
        for layout in &snapshot.layouts {
            accepted += self
                .post(
                    &self.paths.layouts,
                    &json!({
                        "slug": layout.slug,
                        "name": layout.name,
                        "is_system": layout.is_system,
                        "fields": layout.fields,
                    }),
                )
                .await?;
        }
        for asset in &snapshot.assets {
            accepted += self
                .post(
                    &self.paths.assets,
                    &json!({
                        "id": asset.id,
                        "client_id": asset.client_id,
                        "tag": asset.tag,
                        "name": asset.name,
                        "kind": asset.kind,
                        "status": asset.status,
                        "layout_id": asset.layout_id,
                        "fields": asset.fields,
                    }),
                )
                .await?;
        }
        for document in &snapshot.documents {
            accepted += self
                .post(
                    &self.paths.documents,
                    &json!({
                        "id": document.id,
                        "client_id": document.client_id,
                        "title": document.title,
                        "layout_id": document.layout_id,
                        "fields": document.fields,
                        "body": document.body,
                    }),
                )
                .await?;
        }
        Ok(accepted)
    }

    async fn post(&self, resource: &str, body: &Value) -> Result<usize, crate::model::SyncError> {
        let url = format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            resource.trim_start_matches('/')
        );
        let response = self
            .http
            .post(&url)
            .header("Accept", "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| crate::model::SyncError::Sink(format!("REST push {url}: {e}")))?;
        if response.status().is_success() {
            Ok(1)
        } else {
            Err(crate::model::SyncError::Sink(format!(
                "REST push {url}: status {}",
                response.status()
            )))
        }
    }
}

/// Neutral-record constructor helpers used by the Meridian-side reader
/// implementations (kept here so renderer fixtures and reader
/// implementations share one construction path).
pub fn snapshot_record_from_fields(
    external_id: String,
    kind: LinkKind,
    title: String,
    fields: std::collections::BTreeMap<String, String>,
) -> SyncRecord {
    SyncRecord {
        external_id,
        kind,
        client_ref: None,
        title,
        type_name: None,
        fields,
        body: String::new(),
        tag: None,
        credentials: Vec::new(),
    }
}
