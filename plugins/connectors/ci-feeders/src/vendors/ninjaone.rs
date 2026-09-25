//! NinjaOne RMM device payloads (NinjaOne's public v2 API device
//! shape: `id`, `uid`, `displayName`, `systemName`, `osName`,
//! `serialNumber`, `macAddresses`, `externalIp`, `lastSeen`,
//! `deviceClass`, ...). Source system: `ninjaone` — the shipped RMM
//! precedence class (serial/OS/hostname ⇒ RMM 90).

use serde_json::Value;

use crate::identity::IdentityKeysBuilder;
use crate::model::FeederRecord;
use crate::vendors::{attributes_of, first_str, observed_fields, string_list, NormalizeError};

const SOURCE: &str = "ninjaone";

/// Maps one NinjaOne device record onto the feed contract.
pub fn map_device(payload: &Value) -> Result<FeederRecord, NormalizeError> {
    let fields = attributes_of(payload, SOURCE)?;

    let id = first_str(fields, &["id"])
        .or_else(|| {
            fields
                .get("id")
                .and_then(Value::as_i64)
                .map(|id| id.to_string())
        })
        .or_else(|| first_str(fields, &["uid"]))
        .ok_or_else(|| NormalizeError::Unrecognised {
            vendor: SOURCE,
            reason: "missing device id".into(),
        })?;
    let uid = first_str(fields, &["uid"]).or_else(|| Some(id.clone()));

    let display_name = first_str(fields, &["displayName", "systemName"])
        .unwrap_or_else(|| format!("NinjaOne device {id}"));
    let name =
        first_str(fields, &["systemName", "displayName"]).unwrap_or_else(|| display_name.clone());

    let mut identity = IdentityKeysBuilder::new();
    if let Some(guid) = &uid {
        identity = identity.source_guid(guid);
    }
    if let Some(serial) = first_str(fields, &["serialNumber"]) {
        identity = identity.serial(&serial);
    }
    let mac_addresses = string_list(fields, &["macAddresses", "macAddress"]);
    if !mac_addresses.is_empty() {
        let refs: Vec<&str> = mac_addresses.iter().map(String::as_str).collect();
        identity = identity.macs(refs);
    }
    if let Some(hostname) = first_str(fields, &["systemName", "dnsHostname"]) {
        identity = identity.hostname(&hostname);
    }
    if let Some(domain) = first_str(fields, &["domain"]) {
        identity = identity.domain(&domain);
    }

    Ok(FeederRecord {
        source_system: SOURCE.into(),
        external_id: id,
        source_guid: uid,
        payload_ref: None,
        tag: display_name,
        name,
        identity_keys: identity.build(),
        observed_fields: observed_fields(
            fields,
            &[
                "osName",
                "externalIp",
                "lastSeen",
                "deviceClass",
                "nodeClass",
            ],
        ),
    })
}
