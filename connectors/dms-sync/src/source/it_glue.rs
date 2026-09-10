//! IT Glue pull adapter: parses IT Glue's JSON:API export files into a
//! [`DmsSnapshot`]. The export set ("Export all") carries one JSON file
//! per collection; this parser accepts the files individually so an
//! operator can hand it exactly what the export zip contains:
//!
//! * `organizations` — client scope for everything else (identity:
//!   name; the engine resolves org names to Meridian clients).
//! * `flexible_asset_types` — typed document definitions (traits →
//!   layout fields).
//! * `flexible_assets` — typed documents (the bulk of IT Glue data).
//!   Flexible assets whose type is password-shaped surface ONLY when
//!   the source was explicitly constructed with
//!   [`ItGlueExport::include_credentials`] (vault boundary: extraction
//!   is opt-in at the source, forwarding is separately gated by the
//!   engine's `map_credentials`).
//! * `configurations` — the asset records (identity keys: serial,
//!   hostname+domain per the IT Glue configuration attributes).
//!
//! Non-purpose: no IT Glue REST pagination/chunking logic here (live
//! pulls ride [`crate::source::rest::RestSource`] with the operator's
//! endpoint map), no attachment/download handling (binary attachments
//! are out of scope for v1 — reported as unmapped if referenced), and
//! no lossy coercion: trait values that are not scalars stringify and
//! stay visible in the record fields so nothing disappears.

use serde_json::Value;

use crate::model::{Credential, LinkKind, Relation, SyncField, SyncLayout, SyncRecord};

/// The parsed pieces of an IT Glue export, keyed by export file name.
#[derive(Debug, Clone, Default)]
pub struct ItGlueExportFiles {
    /// organizations.json
    pub organizations: Option<Value>,
    /// flexible_asset_types.json
    pub flexible_asset_types: Option<Value>,
    /// flexible_assets.json
    pub flexible_assets: Option<Value>,
    /// configurations.json
    pub configurations: Option<Value>,
}

/// The IT Glue export source. Construct with the export files; call
/// [`ItGlueExport::include_credentials`] to surface password flexible
/// assets as [`Credential`]s (default: never).
pub struct ItGlueExport {
    files: ItGlueExportFiles,
    include_credentials: bool,
}

impl ItGlueExport {
    pub fn new(files: ItGlueExportFiles) -> Self {
        Self {
            files,
            include_credentials: false,
        }
    }

    /// Opt in to extracting password flexible assets as
    /// [`Credential`]s. The engine's `map_credentials` gate still
    /// applies before anything reaches Meridian's vault.
    pub fn include_credentials(mut self) -> Self {
        self.include_credentials = true;
        self
    }
}

impl crate::source::DmsSource for ItGlueExport {
    fn name(&self) -> &str {
        "it-glue"
    }

    fn pull(&self) -> Result<crate::model::DmsSnapshot, crate::model::SyncError> {
        parse(&self.files, self.include_credentials)
    }
}

/// JSON:API helpers: a resource array + its attributes.
fn resources<'a>(
    part: &'a Value,
    resource_type: &str,
) -> Result<Vec<&'a Value>, crate::model::SyncError> {
    let data = part.get("data").ok_or_else(|| {
        crate::model::SyncError::Parse(format!(
            "export part carries no data array: {resource_type}"
        ))
    })?;
    let Some(array) = data.as_array() else {
        return Err(crate::model::SyncError::Parse(format!(
            "export part data is not an array: {resource_type}"
        )));
    };
    // Tolerate both bare arrays and wrapped single resources.
    Ok(array
        .iter()
        .filter(|r| r.get("type").and_then(Value::as_str).unwrap_or_default() == resource_type)
        .collect())
}

fn attributes(resource: &Value) -> &Value {
    resource.get("attributes").unwrap_or(&Value::Null)
}

fn attr_id<'a>(attributes: &'a Value, key: &str) -> Option<&'a str> {
    attributes
        .get(key)
        .and_then(|v| v.get("data"))
        .and_then(|d| d.get("id"))
        .and_then(Value::as_str)
}

fn map_trait_kind(kind: &str) -> crate::model::FieldKind {
    match kind.to_lowercase().as_str() {
        "number" => crate::model::FieldKind::Number,
        "date" => crate::model::FieldKind::Date,
        "checkbox" => crate::model::FieldKind::Bool,
        "select" => crate::model::FieldKind::Select,
        "url" | "website" => crate::model::FieldKind::Url,
        "email" => crate::model::FieldKind::Email,
        // Text, Textbox, WYSIWYG, Tag, and anything unknown degrade to
        // text — stringified, never dropped.
        _ => crate::model::FieldKind::Text,
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

fn is_password_asset_type(type_name: &str) -> bool {
    let normalized = type_name.to_lowercase();
    normalized.contains("password")
}

/// Trait values → string fields. Non-scalar values stringify (JSON)
/// so the operator still sees them; the layout field's `kind` text
/// validates fine.
fn trait_values(traits: &Value) -> std::collections::BTreeMap<String, String> {
    let mut fields = std::collections::BTreeMap::new();
    if let Some(map) = traits.as_object() {
        for (key, value) in map {
            let string = match value {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Null => continue,
                other => other.to_string(),
            };
            fields.insert(key.clone(), string);
        }
    }
    fields
}

fn parse(
    files: &ItGlueExportFiles,
    include_credentials: bool,
) -> Result<crate::model::DmsSnapshot, crate::model::SyncError> {
    let mut snapshot = crate::model::DmsSnapshot {
        source: "it-glue".into(),
        ..Default::default()
    };

    // Organizations → client scope (id → display name; records carry
    // the id as their client_ref, the engine resolves names).
    let mut org_names: std::collections::BTreeMap<String, String> = Default::default();
    if let Some(part) = &files.organizations {
        for resource in resources(part, "organizations")? {
            let attrs = attributes(resource);
            let id = resource
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let name = attrs
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !id.is_empty() && !name.is_empty() {
                org_names.insert(id.to_string(), name.to_string());
            }
        }
    }
    snapshot.client_refs = org_names.clone();

    // Flexible asset types → layouts.
    if let Some(part) = &files.flexible_asset_types {
        for resource in resources(part, "flexible_asset_types")? {
            let attrs = attributes(resource);
            let external_id = resource
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let name = attrs
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let mut fields = Vec::new();
            if let Some(traits) = attrs
                .get("traits")
                .and_then(|t| t.get("data"))
                .and_then(Value::as_array)
            {
                for trait_resource in traits {
                    let trait_attrs = attributes(trait_resource);
                    let key = trait_attrs
                        .get("key")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| {
                            slug_key(
                                trait_attrs
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default(),
                            )
                        });
                    if key.is_empty() {
                        continue;
                    }
                    let label = trait_attrs
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(&key)
                        .to_string();
                    let kind = trait_attrs
                        .get("kind")
                        .and_then(Value::as_str)
                        .map(map_trait_kind)
                        .unwrap_or(crate::model::FieldKind::Text);
                    let mut field = SyncField::new(
                        key,
                        label,
                        kind,
                        i32::try_from(fields.len()).unwrap_or(i32::MAX),
                    );
                    if trait_attrs
                        .get("required")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        field = field.required();
                    }
                    fields.push(field);
                }
            }
            let description = attrs
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            snapshot.layouts.push(SyncLayout {
                external_id,
                name,
                description,
                icon: attrs
                    .get("icon")
                    .and_then(Value::as_str)
                    .unwrap_or("file")
                    .to_string(),
                fields,
            });
        }
    }

    // Flexible assets → documents (or credentials when opted in).
    if let Some(part) = &files.flexible_assets {
        // Resolve type names from the parsed layouts (exports carry
        // the type reference; the display name decides vault handling).
        let type_name_by_id: std::collections::BTreeMap<String, String> = snapshot
            .layouts
            .iter()
            .map(|l| (l.external_id.clone(), l.name.clone()))
            .collect();
        for resource in resources(part, "flexible_assets")? {
            let attrs = attributes(resource);
            let external_id = resource
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let type_id = attr_id(attrs, "flexible_asset_type_id")
                .unwrap_or_default()
                .to_string();
            let type_name = attrs
                .get("flexible_asset_type_name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| type_name_by_id.get(&type_id).cloned().unwrap_or_default());
            let title = attrs
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let client_ref = attr_id(attrs, "organization_id").map(str::to_string);
            let fields = trait_values(attrs.get("traits").unwrap_or(&Value::Null));

            if is_password_asset_type(&type_name) {
                // Vault boundary: password-typed flexible assets are
                // credentials, not documents. Without the explicit
                // source opt-in they are skipped ENTIRELY — never
                // leaked into Meridian as document content.
                if !include_credentials {
                    continue;
                }
                let secret = fields
                    .iter()
                    .find(|(k, _)| k.contains("password"))
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default();
                snapshot.credentials.push(Credential {
                    external_id,
                    client_ref: client_ref.clone(),
                    title,
                    username: fields.get("username").cloned().unwrap_or_default(),
                    url: fields.get("url").cloned().unwrap_or_default(),
                    secret,
                    notes: fields.get("notes").cloned(),
                    expires_at: fields
                        .get("expiry")
                        .or_else(|| fields.get("expires_at"))
                        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
                        .map(|d| d.with_timezone(&chrono::Utc)),
                });
                continue;
            }

            snapshot.records.push(SyncRecord {
                external_id,
                kind: LinkKind::Document,
                client_ref,
                title,
                type_name: (!type_name.is_empty()).then_some(type_name),
                fields,
                body: String::new(),
                tag: None,
                credentials: Vec::new(),
            });
        }
    }

    // Configurations → asset records (identity-bearing by design).
    if let Some(configurations) = files.configurations.as_ref() {
        for resource in resources(configurations, "configurations")? {
            let attrs = attributes(resource);
            let external_id = resource
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let name = attrs
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let mut fields = std::collections::BTreeMap::new();
            let copy = |fields: &mut std::collections::BTreeMap<String, String>,
                        attrs: &Value,
                        attr: &str,
                        key: &str| {
                if let Some(v) = attrs
                    .get(attr)
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                {
                    fields.insert(key.to_string(), v.to_string());
                }
            };
            copy(&mut fields, attrs, "hostname", "hostname");
            copy(&mut fields, attrs, "primary_ip", "primary_ip");
            copy(&mut fields, attrs, "mac_address", "mac_address");
            copy(&mut fields, attrs, "serial_number", "serial");
            copy(&mut fields, attrs, "operating_system", "os");
            copy(&mut fields, attrs, "configuration_status", "status");
            let notes = attrs
                .get("notes")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            // The type name rides the attribute when the export carries
            // it, else the referenced configuration_type resource's own
            // attributes, else the neutral fallback.
            let type_name = attrs
                .get("configuration_type_name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    attrs
                        .get("configuration_type_id")
                        .and_then(|t| t.get("data"))
                        .and_then(|d| d.get("attributes"))
                        .and_then(|a| a.get("name"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "Configuration".into());
            snapshot.records.push(SyncRecord {
                external_id,
                kind: LinkKind::Asset,
                client_ref: attr_id(attrs, "organization_id").map(str::to_string),
                title: name,
                type_name: Some(type_name),
                fields,
                body: notes,
                tag: None,
                credentials: Vec::new(),
            });
        }
    }

    // Relationships: trait values that reference other resources.
    if let Some(part) = &files.flexible_assets {
        for resource in resources(part, "flexible_assets")? {
            let Some(traits) = attributes(resource)
                .get("traits")
                .and_then(Value::as_object)
            else {
                continue;
            };
            let from_id = resource
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            for value in traits.values() {
                let targets = match value {
                    Value::Array(items) => items.clone(),
                    Value::Object(_) => vec![value.clone()],
                    _ => continue,
                };
                for target in targets {
                    let Some(target_type) = target.get("type").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(target_id) = target.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    let kind = match target_type {
                        "configurations" => LinkKind::Asset,
                        "flexible_assets" | "documents" => LinkKind::Document,
                        _ => continue,
                    };
                    snapshot.relations.push(Relation {
                        from: crate::model::LinkEndpoint {
                            external_id: from_id.clone(),
                            kind: LinkKind::Document,
                        },
                        to: crate::model::LinkEndpoint {
                            external_id: target_id.to_string(),
                            kind,
                        },
                    });
                }
            }
        }
    }

    Ok(snapshot)
}
