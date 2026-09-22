//! ImmyBot device/patch-state inventory connector (adapters tranche 2,
//! Wave 12 lane 9).
//!
//! ImmyBot is the patching/deployment automation system: its device
//! inventory carries the patch-state signals Meridian's CI records
//! want (last patch scan, maintenance state, online state, OS). The
//! connector pulls `/api/v1/devices` with the native page/size
//! pagination (`page` + `itemsPerPage`, terminating at the first
//! short page) under a bearer-token API key, maps every device onto
//! the lane-8 feed contract (source system `immybot`), and feeds it
//! through [`FeederSync`] + a [`FeederSink`] — the same never-blind
//! seam as every other feeder.
//!
//! Identity: ImmyBot's device id is the CI2 link key; the WMI/DMI
//! serial (`serialNumber`) and hostname (`hostname` / `computerName`)
//! carry the CI3 identity keys, and the device MAC (`MAC` /
//! `macAddress` / `macAddresses`) joins the normalized MAC set — so an
//! ImmyBot record and a NinjaOne record for the same physical machine
//! converge on one canonical item with one link per source (the MAC
//! normalization is byte-identical across connectors because every
//! mapper rides `adapters_core::vendor_fields` +
//! `ci_feeders::identity`). Patch-state fields are observed tier —
//! they churn with the source and never outrank curated values.
//!
//! Operator toggle: the org-level `connectors.immybot` setting is the
//! enable/disable surface for this connector (default ON; core-enforced
//! per ADR the feed-upsert seam — Wave 15 lane 4). A run whose org has
//! the toggle off should be skipped, not half-applied.
//!
//! None mode: [`NoneFeederSink`] (or `ApplyMode::DryRun`) counts
//! honestly with zero Meridian I/O. Live-credential smoke is out of
//! scope — tests run against contract-shaped stub servers.

use adapters_core::client::{PageFlow, VendorClient, VendorError};
use adapters_core::org_map::OrgMapping;
use adapters_core::vendor_fields::{first_str_in, observed_fields_in, string_list_in};
use adapters_core::VendorAuth;
use ci_feeders::identity::IdentityKeysBuilder;
use ci_feeders::model::FeederRecord;
use ci_feeders::sync::{FeederSync, SyncConfig, SyncReport};
use serde_json::Value;

const SOURCE: &str = "immybot";

/// ImmyBot client. ImmyBot exposes its API under the tenant's own
/// deployment URL (`https://{tenant}.immybot.com`); API keys are
/// minted in the ImmyBot portal and sent as bearer tokens.
pub struct ImmybotClient {
    inner: VendorClient,
}

impl ImmybotClient {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            inner: VendorClient::new(base_url, VendorAuth::Bearer(api_key.into())),
        }
    }

    /// The device inventory (all pages, vendor order). Patch-state and
    /// maintenance fields ride the device payload verbatim.
    pub async fn devices(&self) -> Result<Vec<Value>, VendorError> {
        self.inner
            .fetch_all(
                "/api/v1/devices",
                &PageFlow::PageNumber {
                    items_key: "",
                    page_param: "page",
                    size_param: Some("itemsPerPage"),
                    page_size: 100,
                },
            )
            .await
    }
}

/// Maps one ImmyBot device onto the feed contract.
pub fn map_device(payload: &Value) -> Option<FeederRecord> {
    let id = payload.get("id").and_then(|v| match v {
        Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })?;
    let display_name = first_str_in(payload, &["computerName", "hostname", "name"])
        .unwrap_or_else(|| format!("ImmyBot device {id}"));

    let mut identity = IdentityKeysBuilder::new();
    identity = identity.source_guid(&id);
    if let Some(serial) = first_str_in(payload, &["serialNumber", "serial"]) {
        identity = identity.serial(&serial);
    }
    if let Some(hostname) = first_str_in(payload, &["hostname", "computerName"]) {
        identity = identity.hostname(&hostname);
    }
    // MAC identity joins the key set (ADR 0003 MAC class): ImmyBot's
    // device payload carries the WMI/DMI NIC address under `MAC` /
    // `macAddress` (plural spellings accepted too). Without it a
    // serial-less machine present in BOTH NinjaOne and ImmyBot would
    // not converge on MAC — the same identity keys every other feeder
    // carries for the same physical device.
    let macs = string_list_in(payload, &["MAC", "macAddress", "macAddresses"]);
    if !macs.is_empty() {
        let refs: Vec<&str> = macs.iter().map(String::as_str).collect();
        identity = identity.macs(refs);
    }

    Some(FeederRecord {
        source_system: SOURCE.into(),
        external_id: id.to_string(),
        source_guid: Some(id.to_string()),
        payload_ref: None,
        tag: display_name.clone(),
        name: display_name,
        identity_keys: identity.build(),
        observed_fields: observed_fields_in(
            payload,
            &[
                "osName",
                "osVersion",
                "lastSeen",
                "lastPatchScan",
                "lastPatchRun",
                "isOnline",
                "maintenanceMode",
                "isMaintenanceScheduleActive",
                "manufacturer",
                "model",
                "patchScansPaused",
                "needsReboot",
            ],
        ),
    })
}

/// Why a sync run failed at the vendor boundary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImmySyncError {
    #[error("vendor pull failed: {0}")]
    Vendor(#[from] VendorError),
    /// ImmyBot tenants can be scoped per client group; a device whose
    /// group is not in the operator's mapping is skipped and REPORTED,
    /// never misattributed to a guessed Meridian client.
    #[error("device group `{0}` is not mapped to a Meridian client")]
    UnmappedGroup(String),
}

#[cfg(test)]
mod tests;

/// The ImmyBot sync driver. With a `mapping` (ImmyBot client-group id
/// → Meridian client), each device feeds ITS mapped client; devices in
/// unmapped groups are reported. Without a `mapping` (or with an empty
/// one), the run is a dry inventory: the report counts every device,
/// zero Meridian I/O.
pub struct ImmybotSync;

impl ImmybotSync {
    pub async fn run(
        immy: &ImmybotClient,
        mapping: Option<&OrgMapping>,
        sink: &dyn ci_feeders::sink::FeederSink,
    ) -> Result<SyncReport, ImmySyncError> {
        let devices = immy.devices().await?;
        let Some(mapping) = mapping.filter(|m| !m.is_empty()) else {
            // Dry inventory: count every device, zero Meridian I/O.
            return Ok(SyncReport {
                total: devices.len(),
                would_sync: devices.len(),
                ..SyncReport::default()
            });
        };

        let mut unmappable: Vec<String> = Vec::new();
        // Group devices by their ImmyBot client group (one Meridian
        // client per group — one engine run per group).
        let mut by_group: Vec<(String, Vec<FeederRecord>)> = Vec::new();
        for device in &devices {
            let Some(record) = map_device(device) else {
                unmappable.push(
                    device
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_string(),
                );
                continue;
            };
            let group = device
                .get("clientId")
                .and_then(|v| match v {
                    Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            match by_group.iter_mut().find(|(g, _)| *g == group) {
                Some((_, records)) => records.push(record),
                None => by_group.push((group, vec![record])),
            }
        }

        let mut report = SyncReport {
            total: devices.len(),
            ..SyncReport::default()
        };
        for (group, records) in &by_group {
            let Ok(client_id) = mapping.client_for(group) else {
                report.errors.push(ci_feeders::sync::SyncError {
                    source_system: SOURCE.into(),
                    external_id: format!("group:{group}"),
                    error: ImmySyncError::UnmappedGroup(group.clone()).to_string(),
                });
                continue;
            };
            let group_report = FeederSync::run(&SyncConfig::new(client_id), records, sink).await;
            report.total += 0;
            report.synced += group_report.synced;
            report.would_sync += group_report.would_sync;
            report.created += group_report.created;
            report.resolved_existing += group_report.resolved_existing;
            report.auto_merged += group_report.auto_merged;
            report.queued_candidates += group_report.queued_candidates;
            report.contradictions += group_report.contradictions;
            report.errors.extend(group_report.errors);
        }
        for id in unmappable {
            report.errors.push(ci_feeders::sync::SyncError {
                source_system: SOURCE.into(),
                external_id: id,
                error: "unmappable device record (missing id)".into(),
            });
        }
        Ok(report)
    }
}
