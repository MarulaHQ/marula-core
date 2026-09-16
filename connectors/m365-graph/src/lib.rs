//! Microsoft 365 Graph device inventory connector (adapters tranche 2,
//! Wave 12 lane 9).
//!
//! Pulls two Graph surfaces onto the lane-8 feed contract:
//!
//! * **Intune** `deviceManagement/managedDevices` — the enrolled-fleet
//!   inventory (serial, OS, compliance, last sync).
//! * **Entra ID** `devices` — the directory device objects (trust
//!   type, OS, account-enabled).
//!
//! Auth is the OAuth2 **client-credentials** grant against the
//! tenant's v2.0 token endpoint with the
//! `https://graph.microsoft.com/.default` scope; the token is fetched
//! once per run and cached ([`adapters_core::auth`]). Pagination is
//! Graph's `@odata.nextLink` continuation flow — every page is
//! followed.
//!
//! Records map onto [`FeederRecord`] (source system `m365_graph`) with
//! the CI3 deterministic identity keys: source GUID (`id`,
//! `azureAdDeviceId`), serial (`serialNumber`), hostname
//! (`deviceName`). Intune serials win as the stronger technical
//! signal; directory devices carry GUID + hostname only. Observed
//! fields stay verbatim (the core's precedence rules govern).
//!
//! Same seam as every feeder: [`GraphSync::run`] pushes records
//! through [`FeederSync`] + a [`FeederSink`] — never a direct write,
//! never a blind insert. None mode: [`NoneFeederSink`] or
//! `ApplyMode::DryRun` counts honestly with zero Meridian I/O; the
//! vendor pull itself still runs so the report reflects a real
//! inventory. Live-credential smoke is out of scope — tests run
//! against contract-shaped stub servers.

use adapters_core::auth::{ClientCredentials, VendorAuth};
use adapters_core::client::{PageFlow, VendorClient, VendorError};
use adapters_core::vendor_fields::{first_str_in, observed_fields_in};
use ci_feeders::identity::IdentityKeysBuilder;
use ci_feeders::model::FeederRecord;
use ci_feeders::sync::{FeederSync, SyncConfig, SyncReport};
use serde_json::Value;
use uuid::Uuid;

const SOURCE: &str = "m365_graph";

/// M365 Graph client. `tenant` is the directory tenant id or domain;
/// the token endpoint is the tenant's v2.0 endpoint. The app
/// registration needs `DeviceManagementManagedDevices.Read.All` +
/// `Device.Read.All` (application permissions).
pub struct GraphClient {
    inner: VendorClient,
}

/// Graph base (overridable for tests and sovereign-cloud endpoints).
pub const GRAPH_V1: &str = "https://graph.microsoft.com/v1.0";

impl GraphClient {
    pub fn new(
        tenant: &str,
        client_id: &str,
        client_secret: &str,
        graph_base: impl Into<String>,
    ) -> Self {
        Self {
            inner: VendorClient::new(
                graph_base,
                VendorAuth::ClientCredentials(ClientCredentials {
                    token_url: format!(
                        "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"
                    ),
                    client_id: client_id.into(),
                    client_secret: client_secret.into(),
                    scope: "https://graph.microsoft.com/.default".into(),
                }),
            ),
        }
    }

    /// Explicit token-endpoint variant — for sovereign-cloud token
    /// endpoints and for offline conformance tests against a stub
    /// token server.
    pub fn new_with_token_url(
        token_url: impl Into<String>,
        client_id: &str,
        client_secret: &str,
        graph_base: impl Into<String>,
    ) -> Self {
        Self {
            inner: VendorClient::new(
                graph_base,
                VendorAuth::ClientCredentials(ClientCredentials {
                    token_url: token_url.into(),
                    client_id: client_id.into(),
                    client_secret: client_secret.into(),
                    scope: "https://graph.microsoft.com/.default".into(),
                }),
            ),
        }
    }

    async fn list(&self, path: &str) -> Result<Vec<Value>, VendorError> {
        self.inner
            .fetch_all(
                path,
                &PageFlow::NextLink {
                    key: "@odata.nextLink",
                    items_key: "value",
                },
            )
            .await
    }

    /// Intune enrolled devices (`deviceManagement/managedDevices`).
    pub async fn managed_devices(&self) -> Result<Vec<Value>, VendorError> {
        self.list("/deviceManagement/managedDevices").await
    }

    /// Entra ID directory devices (`devices`).
    pub async fn directory_devices(&self) -> Result<Vec<Value>, VendorError> {
        self.list("/devices").await
    }
}

/// Operator toggle: the org-level `connectors.m365_graph` setting is
/// the enable/disable surface for this connector (default ON; Wave 15
/// lane 4). A run whose org turned it off reports disabled instead of
/// half-applying.
///
/// Maps one Intune managed device onto the feed contract.
pub fn map_managed_device(payload: &Value) -> Option<FeederRecord> {
    let id = payload.get("id").and_then(Value::as_str)?;
    let device_name =
        first_str_in(payload, &["deviceName"]).unwrap_or_else(|| format!("M365 device {id}"));

    let mut identity = IdentityKeysBuilder::new();
    identity = identity.source_guid(id);
    if let Some(serial) = first_str_in(payload, &["serialNumber"]) {
        identity = identity.serial(&serial);
    }
    if let Some(hostname) = first_str_in(payload, &["deviceName"]) {
        identity = identity.hostname(&hostname);
    }

    Some(FeederRecord {
        source_system: SOURCE.into(),
        external_id: id.to_string(),
        source_guid: Some(id.to_string()),
        payload_ref: None,
        tag: device_name.clone(),
        name: device_name,
        identity_keys: identity.build(),
        observed_fields: observed_fields_in(
            payload,
            &[
                "azureAdDeviceId",
                "operatingSystem",
                "osVersion",
                "complianceState",
                "lastSyncDateTime",
                "manufacturer",
                "model",
                "deviceType",
                "deviceOwnership",
                "enrolledDateTime",
            ],
        ),
    })
}

/// Maps one Entra directory device onto the feed contract.
pub fn map_directory_device(payload: &Value) -> Option<FeederRecord> {
    let id = payload.get("id").and_then(Value::as_str)?;
    let display_name =
        first_str_in(payload, &["displayName"]).unwrap_or_else(|| format!("Entra device {id}"));

    let mut identity = IdentityKeysBuilder::new();
    // `deviceId` is the Entra object's immutable device identity; the
    // object `id` is the link key and lands in observed fields.
    if let Some(device_id) = first_str_in(payload, &["deviceId"]) {
        identity = identity.source_guid(&device_id);
    } else {
        identity = identity.source_guid(id);
    }
    if let Some(hostname) = first_str_in(payload, &["displayName"]) {
        identity = identity.hostname(&hostname);
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
                "id",
                "operatingSystem",
                "operatingSystemVersion",
                "trustType",
                "accountEnabled",
                "approximateLastSignInDateTime",
                "profileType",
            ],
        ),
    })
}

/// What one pull did. Both inventories report separately — a tenant
/// with Intune but no directory sync, or the reverse, stays visible.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GraphSyncReport {
    pub managed_devices: Option<SyncReport>,
    pub directory_devices: Option<SyncReport>,
    /// Records the mapper refused (missing ids) — reported, never
    /// dropped silently.
    pub mapping_errors: Vec<String>,
}

impl GraphSyncReport {
    pub fn is_clean(&self) -> bool {
        self.mapping_errors.is_empty()
            && self.managed_devices.as_ref().is_none_or(|r| r.is_clean())
            && self.directory_devices.as_ref().is_none_or(|r| r.is_clean())
    }
}

/// The Graph sync driver. `client_id` is the ONE Meridian client the
/// tenant's devices belong to (an M365 tenant maps 1:1 to a Meridian
/// client — org mapping is client-level here, unlike multi-org docs
/// platforms); pass `None` for a dry inventory (vendor pull + report,
/// zero Meridian I/O).
pub struct GraphSync;

impl GraphSync {
    pub async fn run(
        graph: &GraphClient,
        client_id: Option<Uuid>,
        sink: &dyn ci_feeders::sink::FeederSink,
    ) -> Result<GraphSyncReport, VendorError> {
        let mut report = GraphSyncReport::default();
        for (inventory, path, map) in [
            (
                Inventory::ManagedDevices,
                "/deviceManagement/managedDevices",
                map_managed_device as fn(&Value) -> Option<FeederRecord>,
            ),
            (
                Inventory::DirectoryDevices,
                "/devices",
                map_directory_device as fn(&Value) -> Option<FeederRecord>,
            ),
        ] {
            let items = self_path_list(graph, path).await?;
            let mut records = Vec::new();
            for item in &items {
                match map(item) {
                    Some(record) => records.push(record),
                    None => report.mapping_errors.push(format!(
                        "unmappable device record (missing id): {}",
                        item.get("id").and_then(Value::as_str).unwrap_or("?")
                    )),
                }
            }
            let sync_report = match client_id {
                Some(cid) => FeederSync::run(&SyncConfig::new(cid), &records, sink).await,
                // None client binding = dry inventory: report counts
                // every record, zero Meridian I/O.
                None => SyncReport {
                    total: records.len(),
                    would_sync: records.len(),
                    ..SyncReport::default()
                },
            };
            match inventory {
                Inventory::ManagedDevices => report.managed_devices = Some(sync_report),
                Inventory::DirectoryDevices => report.directory_devices = Some(sync_report),
            }
        }
        Ok(report)
    }
}

/// One Graph list pull (futures are opaque per call site, so the two
/// inventories share one awaited helper instead of a future array).
async fn self_path_list(graph: &GraphClient, path: &str) -> Result<Vec<Value>, VendorError> {
    graph.list(path).await
}

/// Which of the two Graph inventories a batch came from — the reports
/// must stay distinguishable (a tenant with Intune but no directory
/// sync, or the reverse, stays visible).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Inventory {
    ManagedDevices,
    DirectoryDevices,
}

#[cfg(test)]
mod tests;
