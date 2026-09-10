//! Hudu pull adapter: parses Hudu's export JSON into a [`DmsSnapshot`].
//!
//! Hudu's export carries the collections this adapter reads:
//!
//! * `companies` — client scope (matched by name, like IT Glue orgs).
//! * `asset_layouts` — typed layouts with field schemas (`fields`:
//!   `{label, field_type, required, options?}`); these map onto
//!   `document_layouts` with the docs-platform field vocabulary.
//! * `assets` — records of those layouts. ADR 0003's lifecycle rule
//!   decides the Meridian side per record: identity-bearing records
//!   (serial or hostname fields → their own identity, fails or manages
//!   independently) sync as ASSETS; everything else syncs as typed
//!   documents. Websites (their own export collection) are documents.
//! * `passwords` — vault candidates, extracted ONLY when the source
//!   was constructed with [`HuduExport::include_credentials`] (source
//!   layer of the two-layer vault boundary).
//!
//! Hudu asset fields arrive either as `[{label, value}]` arrays or as
//! `{slug: value}` maps depending on export vintage — both normalize.
//! Field values that are references (`{id, ...}` objects) are RELATIONS,
//! not field data; they never masquerade as record fields.

use serde_json::Value;

use crate::model::{
    Credential, FieldKind, LinkEndpoint, LinkKind, Relation, SyncField, SyncLayout, SyncRecord,
};

/// The parsed pieces of a Hudu export, keyed by collection.
#[derive(Debug, Clone, Default)]
pub struct HuduExportFiles {
    pub companies: Option<Value>,
    pub asset_layouts: Option<Value>,
    pub assets: Option<Value>,
    pub websites: Option<Value>,
    /// Hudu's password vault export.
    pub passwords: Option<Value>,
}

pub struct HuduExport {
    files: HuduExportFiles,
    include_credentials: bool,
}

impl HuduExport {
    pub fn new(files: HuduExportFiles) -> Self {
        Self {
            files,
            include_credentials: false,
        }
    }

    /// Opt in to password extraction (see module doc; the engine's
    /// `map_credentials` gate still applies downstream).
    pub fn include_credentials(mut self) -> Self {
        self.include_credentials = true;
        self
    }
}

impl crate::source::DmsSource for HuduExport {
    fn name(&self) -> &str {
        "hudu"
    }

    fn pull(&self) -> Result<crate::model::DmsSnapshot, crate::model::SyncError> {
        parse(&self.files, self.include_credentials)
    }
}

fn scalar_id(value: Option<&Value>) -> String {
    match value {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.trim().to_string(),
        _ => String::new(),
    }
}

fn map_field_type(field_type: &str) -> FieldKind {
    match field_type.to_lowercase().as_str() {
        "number" | "currency" => FieldKind::Number,
        "date" => FieldKind::Date,
        "checkbox" | "bool" => FieldKind::Bool,
        "select" => FieldKind::Select,
        "website" | "url" => FieldKind::Url,
        "email" => FieldKind::Email,
        // text, rich_content, asset_select, and unknowns degrade to
        // text — stringified, never dropped.
        _ => FieldKind::Text,
    }
}

fn slug_key(label: &str) -> String {
    label
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

/// A Hudu layout field definition (export shape).
fn layout_fields(layout: &Value) -> Vec<SyncField> {
    let mut fields = Vec::new();
    let Some(defs) = layout.get("fields").and_then(Value::as_array) else {
        return fields;
    };
    for def in defs {
        let label = def.get("label").and_then(Value::as_str).unwrap_or_default();
        if label.trim().is_empty() {
            continue;
        }
        let key = def
            .get("slug")
            .and_then(Value::as_str)
            .map(slug_key)
            .filter(|k| !k.is_empty())
            .unwrap_or_else(|| slug_key(label));
        let mut field = SyncField::new(
            key,
            label,
            map_field_type(
                def.get("field_type")
                    .and_then(Value::as_str)
                    .unwrap_or("text"),
            ),
            i32::try_from(fields.len()).unwrap_or(i32::MAX),
        );
        if def
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            field = field.required();
        }
        if let Some(options) = def.get("options").and_then(Value::as_array) {
            field.options = options
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
        }
        fields.push(field);
    }
    fields
}

/// Asset field values: accept `[{label, value}]` (export shape) and
/// `{key: value}` maps (API shape) alike. Reference-shaped values are
/// relations, not fields.
fn record_fields(fields_value: Option<&Value>) -> std::collections::BTreeMap<String, String> {
    let mut fields = std::collections::BTreeMap::new();
    match fields_value {
        Some(Value::Array(items)) => {
            for item in items {
                let Some(label) = item.get("label").and_then(Value::as_str) else {
                    continue;
                };
                let value = match item.get("value") {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Number(n)) => n.to_string(),
                    Some(Value::Bool(b)) => b.to_string(),
                    _ => continue,
                };
                fields.insert(slug_key(label), value);
            }
        }
        Some(Value::Object(map)) => {
            for (key, value) in map {
                let string = match value {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => continue,
                };
                fields.insert(slug_key(key), string);
            }
        }
        _ => {}
    }
    fields
}

/// ADR 0003 lifecycle rule: a record with its own identity (serial or
/// hostname or MAC) syncs as an asset; everything else is a typed
/// document.
fn identity_bearing(fields: &std::collections::BTreeMap<String, String>) -> bool {
    fields.keys().any(|k| {
        matches!(
            k.as_str(),
            "serial"
                | "serial_number"
                | "service_tag"
                | "hostname"
                | "host_name"
                | "mac"
                | "mac_address"
        )
    })
}

fn parse(
    files: &HuduExportFiles,
    include_credentials: bool,
) -> Result<crate::model::DmsSnapshot, crate::model::SyncError> {
    let mut snapshot = crate::model::DmsSnapshot {
        source: "hudu".into(),
        ..Default::default()
    };

    // Layouts → SyncLayout + an id → type-name map for asset records.
    let mut layout_name_by_id: std::collections::BTreeMap<String, String> = Default::default();
    if let Some(layouts) = files.asset_layouts.as_ref().and_then(Value::as_array) {
        for layout in layouts {
            let external_id = scalar_id(layout.get("id"));
            let name = layout
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            snapshot.layouts.push(SyncLayout {
                external_id: external_id.clone(),
                name: name.clone(),
                description: layout
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                icon: layout
                    .get("icon")
                    .and_then(Value::as_str)
                    .unwrap_or("file")
                    .to_string(),
                fields: layout_fields(layout),
            });
            if !external_id.is_empty() && !name.is_empty() {
                layout_name_by_id.insert(external_id, name);
            }
        }
    }

    // Companies: client scope map (company_id → name).
    let mut company_names: std::collections::BTreeMap<String, String> = Default::default();
    if let Some(companies) = files.companies.as_ref().and_then(Value::as_array) {
        for company in companies {
            let id = scalar_id(company.get("id"));
            let name = company
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if !id.is_empty() && !name.is_empty() {
                company_names.insert(id, name);
            }
        }
    }
    snapshot.client_refs = company_names.clone();

    // Assets → records (asset vs document per the lifecycle rule).
    if let Some(assets) = files.assets.as_ref().and_then(Value::as_array) {
        for asset in assets {
            let external_id = scalar_id(asset.get("id"));
            if external_id.is_empty() {
                continue;
            }
            let layout_id = scalar_id(asset.get("asset_layout_id"));
            let type_name = layout_name_by_id.get(&layout_id).cloned();
            let fields = record_fields(asset.get("fields"));
            let kind = if identity_bearing(&fields) {
                LinkKind::Asset
            } else {
                LinkKind::Document
            };
            snapshot.records.push(SyncRecord {
                external_id,
                kind,
                client_ref: Some(scalar_id(asset.get("company_id"))).filter(|s| !s.is_empty()),
                title: asset
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                type_name,
                fields,
                body: asset
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                tag: None,
                credentials: Vec::new(),
            });
        }
    }

    // Websites → typed documents with a url field.
    if let Some(websites) = files.websites.as_ref().and_then(Value::as_array) {
        for website in websites {
            let external_id = scalar_id(website.get("id"));
            if external_id.is_empty() {
                continue;
            }
            let mut fields = std::collections::BTreeMap::new();
            if let Some(url) = website
                .get("url")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
            {
                fields.insert("url".into(), url.to_string());
            }
            snapshot.records.push(SyncRecord {
                external_id,
                kind: LinkKind::Document,
                client_ref: Some(scalar_id(website.get("company_id"))).filter(|s| !s.is_empty()),
                title: website
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                type_name: Some("Website".into()),
                fields,
                body: String::new(),
                tag: None,
                credentials: Vec::new(),
            });
        }
    }

    // Passwords → vault candidates, only when explicitly opted in.
    if include_credentials {
        if let Some(passwords) = files.passwords.as_ref().and_then(Value::as_array) {
            for password in passwords {
                let external_id = scalar_id(password.get("id"));
                if external_id.is_empty() {
                    continue;
                }
                snapshot.credentials.push(Credential {
                    external_id,
                    client_ref: Some(scalar_id(password.get("company_id")))
                        .filter(|s| !s.is_empty()),
                    title: password
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    username: password
                        .get("username")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    url: password
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    secret: password
                        .get("password")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    notes: password
                        .get("otp")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    expires_at: password
                        .get("expires_at")
                        .and_then(Value::as_str)
                        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
                        .map(|d| d.with_timezone(&chrono::Utc)),
                });
            }
        }
    }

    // Relations: asset_select-style reference values
    // ({value: {id, type}} or {value: [{id, type}]}) inside asset
    // fields; the reference target syncs as a typed document end
    // unless the exporter told us otherwise (websites/asset ids).
    if let Some(assets) = files.assets.as_ref().and_then(Value::as_array) {
        for asset in assets {
            let from_id = scalar_id(asset.get("id"));
            if from_id.is_empty() {
                continue;
            }
            let from_kind = if identity_bearing(&record_fields(asset.get("fields"))) {
                LinkKind::Asset
            } else {
                LinkKind::Document
            };
            let Some(items) = asset.get("fields").and_then(Value::as_array) else {
                continue;
            };
            for field in items {
                let Some(value) = field.get("value") else {
                    continue;
                };
                let targets: Vec<Value> = match value {
                    Value::Array(items) => items.clone(),
                    Value::Object(_) => vec![value.clone()],
                    _ => continue,
                };
                for target in targets {
                    let target_id = scalar_id(target.get("id"));
                    if target_id.is_empty() {
                        continue;
                    }
                    let kind = match target
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                    {
                        "asset" | "configurations" => LinkKind::Asset,
                        // Hudu references don't label kinds; the
                        // record-level lifecycle rule decides.
                        _ => LinkKind::Document,
                    };
                    snapshot.relations.push(Relation {
                        from: LinkEndpoint {
                            external_id: from_id.clone(),
                            kind: from_kind,
                        },
                        to: LinkEndpoint {
                            external_id: target_id,
                            kind,
                        },
                    });
                }
            }
        }
    }

    Ok(snapshot)
}
