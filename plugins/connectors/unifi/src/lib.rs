//! UniFi site/controller inventory connector (Wave 22 lane 3,
//! ecosystem/SDK review leg A).
//!
//! UniFi Network exposes its official Integrations API v1 on the
//! controller itself: base path `{controller}/proxy/network/integration/v1`,
//! API-key auth via the `X-API-KEY` header (minted in Network → Settings →
//! Control Plane → Integrations), offset/limit pagination with the
//! `{offset, limit, count, totalCount, data[]}` page envelope. The
//! connector pulls `GET /v1/sites` and every site's
//! `GET /v1/sites/{siteId}/devices`, maps each adopted device onto the
//! feed contract (source system `unifi`), and feeds it through the same
//! never-blind feed-upsert seam as every other feeder. Devices whose
//! site the operator mapped ride `POST /api/v1/config-items/{id}/interfaces`
//! (the ADR 0003 rule: NICs are attributes, not their own CIs) — the
//! main NIC carries the device MAC + IP as `is_primary`, and every
//! physical port lands as a `CiInterface` named after its port index.
//!
//! Identity: the UniFi device id is the CI2 link key. The Integrations
//! API v1 device payload carries NO serial field — the device MAC
//! (`macAddress`) is the CI3 identity signal, with the assigned device
//! `name` mapping to the hostname signal. A UniFi record and an
//! Auvik/NinjaOne record for the same physical switch converge through
//! the normalized MAC set (`aa:bb:cc:dd:ee:ff`, lower colon, order-sorted)
//! exactly per ADR 0003's MAC identity class. Any `serialNumber`/`serial`
//! key a controller version happens to expose is honored when present —
//! observed, never assumed. Firmware/state/latency fields are observed
//! tier — they churn with the controller and never outrank curated values.
//!
//! None mode: [`NoneFeederSink`] (or no site→client mapping, which is a
//! dry inventory) counts honestly with zero Marula I/O — the controller
//! pull still happens so the report reflects a real inventory. The UniFi
//! controller is LOCAL hardware: when the controller is unreachable the
//! connector reports a typed [`UnifiSyncError::Vendor`] pull failure; the
//! Marula side (feed-upsert seam, alert-source ingest) stays fully
//! functional, and every push the connector WOULD have made is reported.
//! Live-controller smoke is out of scope — tests run against
//! contract-shaped stub servers.

use adapters_core::auth::VendorAuth;
use adapters_core::client::{VendorClient, VendorError};
use adapters_core::org_map::OrgMapping;
use adapters_core::vendor_fields::{first_str_in, observed_fields_in, string_list_in};
use ci_feeders::identity::IdentityKeysBuilder;
use ci_feeders::model::FeederRecord;
use ci_feeders::sink::{FeederSink, SinkError};
use ci_feeders::sync::SyncError;
use serde_json::Value;
use uuid::Uuid;

pub mod alerts;

#[cfg(test)]
mod tests;

const SOURCE: &str = "unifi";

/// UniFi client. The controller's Integrations API lives under
/// `/proxy/network/integration/v1` on UniFi OS; the API key is minted in
/// Network → Settings → Control Plane → Integrations and sent as the
/// `X-API-KEY` header on every request.
pub struct UnifiClient {
    inner: VendorClient,
}

impl UnifiClient {
    pub fn new(controller_base: impl Into<String>, api_key: impl Into<String>) -> Self {
        let base = controller_base.into().trim_end_matches('/').to_string();
        Self {
            inner: VendorClient::new(
                format!("{base}/proxy/network/integration/v1"),
                VendorAuth::ApiKeyHeader {
                    header: "X-API-KEY".into(),
                    key: api_key.into(),
                },
            ),
        }
    }

    /// Every local site, following the controller's offset/limit
    /// pagination to exhaustion.
    pub async fn sites(&self) -> Result<Vec<Value>, VendorError> {
        self.fetch_pages("/sites").await
    }

    /// Every adopted device on one site (paginated the same way).
    pub async fn site_devices(&self, site_id: &str) -> Result<Vec<Value>, VendorError> {
        self.fetch_pages(&format!("/sites/{site_id}/devices")).await
    }

    /// One device's full detail (interfaces: ports + radios). The list
    /// endpoint returns overviews only; ports ride the detail payload.
    pub async fn device(&self, site_id: &str, device_id: &str) -> Result<Value, VendorError> {
        self.inner
            .get_json(&format!("/sites/{site_id}/devices/{device_id}"), &[])
            .await
    }

    /// The Integrations API pages with offset/limit and an envelope of
    /// `{offset, limit, count, totalCount, data[]}`; a pull follows
    /// EVERY page — a silently truncated inventory is a wrong inventory.
    async fn fetch_pages(&self, path: &str) -> Result<Vec<Value>, VendorError> {
        let page_size = 100usize;
        let mut items = Vec::new();
        let mut offset = 0usize;
        let mut guard = 0usize;
        loop {
            let query = vec![
                ("offset".to_string(), offset.to_string()),
                ("limit".to_string(), page_size.to_string()),
            ];
            let body = self.inner.get_json(path, &query).await?;
            let page = body
                .get("data")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let count = page.len();
            items.extend(page);
            let total = body.get("totalCount").and_then(Value::as_u64).unwrap_or(0) as usize;
            offset += count;
            if count == 0 || offset >= total {
                break;
            }
            guard += 1;
            // Same hard stop as the other flows: a controller that
            // ignores `offset` must not re-serve page 1 forever.
            if guard > 10_000 {
                return Err(VendorError::Rejected(format!(
                    "pagination exceeded 10,000 pages at {path}"
                )));
            }
        }
        Ok(items)
    }
}

/// Maps one UniFi device summary/detail payload onto the feed contract.
///
/// The Integrations API v1 device payload carries no serial — the
/// normalized MAC set is the cross-source identity signal, with the
/// device name as the hostname signal.
pub fn map_device(payload: &Value) -> Option<FeederRecord> {
    let id = first_str_in(payload, &["id", "deviceId"])?;
    let name = first_str_in(payload, &["name"]).unwrap_or_else(|| format!("UniFi device {id}"));

    let mut identity = IdentityKeysBuilder::new();
    identity = identity.source_guid(&id);
    let mut macs: Vec<String> = string_list_in(payload, &["macAddress", "macAddresses"]);
    if let Some(uplink_mac) = payload
        .get("uplink")
        .and_then(|u| u.get("mac"))
        .and_then(Value::as_str)
    {
        macs.push(uplink_mac.to_string());
    }
    let refs: Vec<&str> = macs.iter().map(String::as_str).collect();
    identity = identity.macs(refs);
    if let Some(hostname) = first_str_in(payload, &["name"]) {
        identity = identity.hostname(&hostname);
    }
    if let Some(serial) = first_str_in(payload, &["serialNumber", "serial"]) {
        identity = identity.serial(&serial);
    }

    Some(FeederRecord {
        source_system: SOURCE.into(),
        external_id: id.to_string(),
        source_guid: Some(id.to_string()),
        payload_ref: None,
        tag: name.clone(),
        name,
        identity_keys: identity.build(),
        observed_fields: observed_fields_in(
            payload,
            &[
                "model",
                "state",
                "firmwareVersion",
                "firmwareUpdatable",
                "ipAddress",
                "supported",
                "adoptedAt",
                "provisionedAt",
                "configurationId",
                "uplink",
            ],
        ),
    })
}

/// One Marula `CiInterface` write for a device interface.
pub struct DeviceInterface {
    pub name: String,
    pub mac: Option<String>,
    pub ips: Vec<String>,
    pub vlans: Vec<String>,
    pub is_primary: bool,
}

/// Maps a device payload's interfaces onto CiInterface writes: the main
/// NIC (device MAC + IP, `is_primary`), the uplink when the payload
/// exposes one, and every physical port (`Port {idx}`). VLAN membership
/// rides through when the payload exposes it on a port (`vlans`/`vlan`
/// keys); the Integrations v1 port overview does not publish VLANs, so
/// the honest default is an empty list.
pub fn map_interfaces(device: &Value) -> Vec<DeviceInterface> {
    let mut out = Vec::new();

    let mac = first_str_in(device, &["macAddress"]);
    let ip = first_str_in(device, &["ipAddress"]);
    if mac.is_some() || ip.is_some() {
        out.push(DeviceInterface {
            name: "main".into(),
            mac: mac.clone(),
            ips: ip.clone().into_iter().collect(),
            vlans: Vec::new(),
            is_primary: true,
        });
    }

    if let Some(uplink_mac) = device
        .get("uplink")
        .and_then(|u| u.get("mac"))
        .and_then(Value::as_str)
    {
        out.push(DeviceInterface {
            name: "uplink".into(),
            mac: Some(uplink_mac.to_string()),
            ips: Vec::new(),
            vlans: Vec::new(),
            is_primary: false,
        });
    }

    if let Some(ports) = device
        .get("interfaces")
        .and_then(|i| i.get("ports"))
        .and_then(Value::as_array)
    {
        for port in ports {
            let Some(idx) = port.get("idx").and_then(Value::as_i64) else {
                continue;
            };
            out.push(DeviceInterface {
                name: format!("Port {idx}"),
                mac: first_str_in(port, &["mac", "macAddress"]),
                ips: string_list_in(port, &["ips", "ipAddresses"]),
                vlans: string_list_in(port, &["vlans", "vlan"]),
                is_primary: false,
            });
        }
    }
    out
}

/// Why a sync run failed at the vendor boundary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnifiSyncError {
    #[error("controller pull failed: {0}")]
    Vendor(#[from] VendorError),
    /// A site without an operator mapping is skipped and REPORTED —
    /// its devices are never misattributed to a guessed Marula client.
    #[error("site `{0}` is not mapped to a Marula client")]
    UnmappedSite(String),
}

/// What a Marula interface write needs beyond the feeder sink: the
/// public `POST /api/v1/config-items/{id}/interfaces` seam over the
/// same scoped machine key. The pin carries no interfaces-read, so
/// writes ride ONE deterministic rule instead of client-side state:
/// interfaces are recorded exactly when the feed-upsert CREATED the
/// canonical item (`created_item == true`) — a replayed sync gets
/// `created_item: false` (idempotent), so interfaces are never doubled.
#[async_trait::async_trait]
pub trait InterfaceSink: Send + Sync {
    async fn create_interface(
        &self,
        config_item_id: Uuid,
        interface: &DeviceInterface,
    ) -> Result<(), String>;
}

/// The public-API interface sink (x-api-key machine credential; the
/// crate imports zero Marula internals).
pub struct MarulaInterfaceSink {
    base_url: String,
    api_key: String,
}

impl MarulaInterfaceSink {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
        }
    }
}

#[async_trait::async_trait]
impl InterfaceSink for MarulaInterfaceSink {
    async fn create_interface(
        &self,
        config_item_id: Uuid,
        interface: &DeviceInterface,
    ) -> Result<(), String> {
        let url = format!(
            "{}/api/v1/config-items/{config_item_id}/interfaces",
            self.base_url.trim_end_matches('/')
        );
        let response = reqwest::Client::new()
            .post(url)
            .header("x-api-key", &self.api_key)
            .json(&serde_json::json!({
                "name": interface.name,
                "mac": interface.mac,
                "ips": interface.ips,
                "vlans": interface.vlans,
                "is_primary": interface.is_primary,
            }))
            .send()
            .await
            .map_err(|e| format!("interface push transport failure: {e}"))?;
        if response.status().is_success() {
            return Ok(());
        }
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        Err(format!(
            "interface push returned {status}: {}",
            body.get("message")
                .and_then(Value::as_str)
                .unwrap_or("unspecified")
        ))
    }
}

/// None mode for the interface leg: reports honestly, zero Marula I/O.
pub struct NoneInterfaceSink;

#[async_trait::async_trait]
impl InterfaceSink for NoneInterfaceSink {
    async fn create_interface(
        &self,
        _config_item_id: Uuid,
        _interface: &DeviceInterface,
    ) -> Result<(), String> {
        Err("interface sink disabled: no Marula connection configured (none mode)".into())
    }
}

/// The sync report, plus the interface pushes the run performed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UnifiReport {
    /// The feeder-engine counters (same vocabulary as every connector's
    /// run report).
    pub report: ci_feeders::sync::SyncReport,
    /// CiInterface writes recorded through the interface seam.
    pub interfaces_pushed: usize,
}

impl UnifiReport {
    pub fn is_clean(&self) -> bool {
        self.report.is_clean()
    }
}

/// The UniFi sync driver. With a `mapping` (UniFi site id → Marula
/// client) each site's devices feed ITS mapped client; unmapped sites
/// are REPORTED. Without a `mapping` (or with an empty one), the run is
/// a dry inventory: the report counts every device, zero Marula I/O
/// (the controller pull still happens, so the report reflects a real
/// inventory). Interfaces ride [`InterfaceSink`] and are recorded ONLY
/// on `created_item` (see the trait docs for the determinism argument).
pub struct UnifiSync;

impl UnifiSync {
    pub async fn run(
        unifi: &UnifiClient,
        mapping: Option<&OrgMapping>,
        sink: &dyn FeederSink,
        interfaces: &dyn InterfaceSink,
    ) -> Result<UnifiReport, UnifiSyncError> {
        let sites = unifi.sites().await?;
        let Some(mapping) = mapping.filter(|m| !m.is_empty()) else {
            let mut total = 0usize;
            for site in &sites {
                let Some(id) = first_str_in(site, &["id"]) else {
                    continue;
                };
                total += unifi.site_devices(&id).await?.len();
            }
            return Ok(UnifiReport {
                report: ci_feeders::sync::SyncReport {
                    total,
                    would_sync: total,
                    ..Default::default()
                },
                interfaces_pushed: 0,
            });
        };

        let mut out = UnifiReport::default();
        for site in &sites {
            let Some(site_id) = first_str_in(site, &["id"]) else {
                continue;
            };
            let site_name = first_str_in(site, &["name"]).unwrap_or_else(|| site_id.clone());
            let Ok(client_id) = mapping.client_for(&site_id) else {
                out.report.errors.push(SyncError {
                    source_system: SOURCE.into(),
                    external_id: format!("site:{site_id}"),
                    error: UnifiSyncError::UnmappedSite(site_name).to_string(),
                });
                continue;
            };
            let devices = unifi.site_devices(&site_id).await?;
            out.report.total += devices.len();
            for device in &devices {
                let Some(record) = map_device(device) else {
                    out.report.errors.push(SyncError {
                        source_system: SOURCE.into(),
                        external_id: first_str_in(device, &["id"]).unwrap_or_else(|| "?".into()),
                        error: "unmappable device record (missing id)".into(),
                    });
                    continue;
                };
                match sink.feed_upsert(&record, client_id).await {
                    Ok(outcome) => {
                        out.report.synced += 1;
                        if outcome.created_item {
                            out.report.created += 1;
                            // Interfaces ride CREATION only — replays
                            // return created_item=false, so the NIC +
                            // port set is recorded once, deterministically.
                            for interface in map_interfaces(device) {
                                match interfaces
                                    .create_interface(outcome.config_item_id, &interface)
                                    .await
                                {
                                    Ok(()) => out.interfaces_pushed += 1,
                                    Err(error) => out.report.errors.push(SyncError {
                                        source_system: SOURCE.into(),
                                        external_id: record.external_id.clone(),
                                        error,
                                    }),
                                }
                            }
                        }
                        if outcome.resolved_existing {
                            out.report.resolved_existing += 1;
                        }
                        if outcome.auto_merged {
                            out.report.auto_merged += 1;
                        }
                        out.report.queued_candidates += outcome.queued_candidates;
                        out.report.contradictions += outcome.contradictions.len();
                    }
                    Err(SinkError::Disabled) => out.report.would_sync += 1,
                    Err(err) => out.report.errors.push(SyncError {
                        source_system: SOURCE.into(),
                        external_id: record.external_id.clone(),
                        error: err.to_string(),
                    }),
                }
            }
        }
        Ok(out)
    }
}
