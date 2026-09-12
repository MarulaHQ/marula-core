//! IT Glue configuration payloads (IT Glue's JSON:API configuration
//! shape: `id`, `attributes.name`, `hostname`, `primary_ip`,
//! `mac_address`, `serial_number`, `operating_system_notes`, ... —
//! either the raw attributes object or the full JSON:API resource).
//! Source system: `it_glue` — the shipped docs precedence class
//! (default 30; the core's docs-source fields never outrank RMM/NMS
//! technical signals).

use serde_json::Value;

use crate::identity::IdentityKeysBuilder;
use crate::model::FeederRecord;
use crate::vendors::{attributes_of, first_str, observed_fields, string_list, NormalizeError};

const SOURCE: &str = "it_glue";

/// Maps one IT Glue configuration record onto the feed contract.
pub fn map_configuration(payload: &Value) -> Result<FeederRecord, NormalizeError> {
    let fields = attributes_of(payload, SOURCE)?;

    let id = first_str(fields, &["id"])
        .or_else(|| {
            payload
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .ok_or_else(|| NormalizeError::Unrecognised {
            vendor: SOURCE,
            reason: "missing configuration id".into(),
        })?;

    let name = first_str(fields, &["name"]).unwrap_or_else(|| format!("IT Glue config {id}"));

    let mut identity = IdentityKeysBuilder::new();
    // IT Glue's configuration id IS the stable GUID of the record.
    identity = identity.source_guid(&id);
    if let Some(serial) = first_str(fields, &["serial_number", "serialNumber"]) {
        identity = identity.serial(&serial);
    }
    if let Some(tag) = first_str(fields, &["service_tag", "serviceTag"]) {
        identity = identity.service_tag(&tag);
    }
    let mac_addresses = string_list(fields, &["mac_address", "macAddress"]);
    if !mac_addresses.is_empty() {
        let refs: Vec<&str> = mac_addresses.iter().map(String::as_str).collect();
        identity = identity.macs(refs);
    }
    if let Some(hostname) = first_str(fields, &["hostname"]) {
        identity = identity.hostname(&hostname);
    }

    Ok(FeederRecord {
        source_system: SOURCE.into(),
        external_id: id,
        source_guid: first_str(fields, &["id"]).or_else(|| {
            payload
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
        }),
        payload_ref: None,
        tag: name.clone(),
        name,
        identity_keys: identity.build(),
        observed_fields: observed_fields(
            fields,
            &[
                "primary_ip",
                "operating_system_notes",
                "operating_system",
                "notes",
                "location_id",
                "contact_id",
            ],
        ),
    })
}
