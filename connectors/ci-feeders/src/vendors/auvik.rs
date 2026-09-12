//! Auvik NMS device payloads (Auvik's public API device shape: `id`,
//! `name`, `deviceType`, `serial`, `macAddress`/`macAddresses`,
//! `ipAddress`/`ipAddresses`, `lastSeen`, `vendorName`, `osName`,
//! `domain`, ...). Source system: `auvix` — the wire name the core's
//! shipped NMS precedence class carries (ip/last_seen/MAC ⇒ NMS 90).

use serde_json::Value;

use crate::identity::IdentityKeysBuilder;
use crate::model::FeederRecord;
use crate::vendors::{attributes_of, first_str, observed_fields, string_list, NormalizeError};

const SOURCE: &str = "auvix";

/// Maps one Auvik device record onto the feed contract.
pub fn map_device(payload: &Value) -> Result<FeederRecord, NormalizeError> {
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
            reason: "missing device id".into(),
        })?;

    let name =
        first_str(fields, &["name", "systemName"]).unwrap_or_else(|| format!("Auvik device {id}"));

    let mut identity = IdentityKeysBuilder::new();
    // Auvik's device id IS the stable GUID of the record.
    identity = identity.source_guid(&id);
    if let Some(serial) = first_str(fields, &["serial", "serialNumber"]) {
        identity = identity.serial(&serial);
    }
    let mac_addresses = string_list(fields, &["macAddresses", "macAddress"]);
    if !mac_addresses.is_empty() {
        let refs: Vec<&str> = mac_addresses.iter().map(String::as_str).collect();
        identity = identity.macs(refs);
    }
    if let Some(hostname) = first_str(fields, &["systemName", "name"]) {
        identity = identity.hostname(&hostname);
    }
    if let Some(domain) = first_str(fields, &["domain"]) {
        identity = identity.domain(&domain);
    }

    Ok(FeederRecord {
        source_system: SOURCE.into(),
        external_id: id.clone(),
        source_guid: Some(id),
        payload_ref: None,
        tag: name.clone(),
        name,
        identity_keys: identity.build(),
        observed_fields: observed_fields(
            fields,
            &[
                "ipAddresses",
                "ipAddress",
                "lastSeen",
                "deviceType",
                "vendorName",
                "osName",
            ],
        ),
    })
}
