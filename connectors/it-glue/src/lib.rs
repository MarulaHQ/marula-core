//! IT Glue LIVE REST transport (adapters tranche 2, Wave 12 lane 9).
//!
//! Lane 8's `ci-feeders` crate maps IT Glue configuration payloads onto
//! the feed contract; this crate supplies what was deliberately left to
//! the adapters tranche — the LIVE transport:
//!
//! * **Auth:** IT Glue's static key under the `x-api-key` header
//!   ([`VendorAuth::ApiKeyHeader`]).
//! * **Pagination:** the native JSON:API flow — `page[number]` request
//!   parameters, `links.next` as the continuation signal. Every page is
//!   followed; a truncated pull is a wrong inventory.
//! * **Org→client mapping:** IT Glue organizes everything under
//!   `organizations`; the operator's explicit vendor-org → Meridian-
//!   client map ([`OrgMapping`]) drives attribution, and an unmapped
//!   org is an ERROR surfaced in the sync report — never a guess.
//!
//! The pull surface:
//!
//! * [`ItGlueClient::organizations`] — lists organizations (ids +
//!   names) so the operator can build the mapping honestly.
//! * [`ItGlueClient::configurations_for_org`] — one org's
//!   configurations as [`FeederRecord`]s through
//!   `ci_feeders::vendors::it_glue::map_configuration` (identity keys
//!   and fingerprints stay lane-8 code — one normalizer, one truth).
//! * [`ItGlueSync::run`] — orgs × configurations through the
//!   [`FeederSync`] engine + [`FeederSink`], with a per-org report.
//!
//! None mode: construct the sync with [`NoneFeederSink`] (or run with
//! `ApplyMode::DryRun`) — zero Meridian I/O, records counted honestly
//! as "would have synced" (standalone-first doctrine). The vendor-side
//! pull itself still runs so the report reflects a real inventory.

use adapters_core::client::{PageFlow, VendorClient, VendorError};
use adapters_core::org_map::OrgMapping;
use adapters_core::VendorAuth;
use ci_feeders::model::FeederRecord;
use ci_feeders::sink::{FeederSink, SinkError};
use ci_feeders::sync::{FeederSync, SyncConfig, SyncReport};
use ci_feeders::vendors::NormalizeError;
use serde_json::Value;

/// IT Glue live client. `base_url` defaults to the production API
/// (`https://api.itglue.com`); EU accounts use
/// `https://api.eu.itglue.com`.
pub struct ItGlueClient {
    inner: VendorClient,
}

impl ItGlueClient {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            inner: VendorClient::new(
                base_url,
                VendorAuth::ApiKeyHeader {
                    header: "x-api-key".into(),
                    key: api_key.into(),
                },
            ),
        }
    }

    async fn list(&self, path: &str) -> Result<Vec<Value>, VendorError> {
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
    }

    /// All organizations, as `(id, name)` pairs in vendor order — the
    /// inventory an operator builds the org→client mapping from.
    pub async fn organizations(&self) -> Result<Vec<(String, String)>, VendorError> {
        let items = self.list("/organizations").await?;
        Ok(items
            .iter()
            .filter_map(|org| {
                let id = org.get("id").and_then(Value::as_str).map(str::to_string)?;
                let name = org
                    .pointer("/attributes/name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                Some((id, name))
            })
            .collect())
    }

    /// One organization's configurations, mapped onto the feed
    /// contract. Malformed records are REPORTED (`errors`), never
    /// dropped silently — one bad row never aborts the org.
    pub async fn configurations_for_org(
        &self,
        org_id: &str,
    ) -> Result<(Vec<FeederRecord>, Vec<String>), VendorError> {
        let items = self
            .list(&format!("/configurations?filter[organization_id]={org_id}"))
            .await?;
        let mut records = Vec::new();
        let mut errors = Vec::new();
        for item in &items {
            match ci_feeders::vendors::map_it_glue_configuration(item) {
                Ok(record) => records.push(record),
                Err(NormalizeError::Unrecognised { reason, .. }) => {
                    errors.push(format!(
                        "configuration {}: {reason}",
                        item.get("id").and_then(Value::as_str).unwrap_or("?")
                    ));
                }
            }
        }
        Ok((records, errors))
    }
}

/// Why an org→client attribution failed mid-sync.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ItGlueSyncError {
    /// The operator's mapping names no Meridian client for this vendor
    /// org; its records are skipped and reported (never misattributed).
    #[error("vendor organization `{0}` is not mapped to a Meridian client")]
    UnmappedOrg(String),
    #[error("vendor pull failed: {0}")]
    Vendor(#[from] VendorError),
}

/// What one org contributed to a run — the report unit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OrgSyncOutcome {
    pub org_id: String,
    /// Feeder records submitted for this org.
    pub total: usize,
    /// Per-record errors from the mapping or the sink.
    pub errors: Vec<String>,
    /// The engine's own counters (empty when the org was unmapped).
    pub report: Option<SyncReport>,
}

/// The IT Glue sync: organizations × configurations through the
/// feeder engine. Stateless; construct per run.
pub struct ItGlueSync;

impl ItGlueSync {
    /// Pulls the mapped orgs' configurations and feeds them. `config`
    /// carries the Meridian client binding per org via `mapping` —
    /// each org feeds ITS OWN mapped client (the engine's one-client
    /// run config is applied per org here).
    pub async fn run(
        client: &ItGlueClient,
        mapping: &OrgMapping,
        org_ids: &[String],
        sink: &dyn FeederSink,
    ) -> Result<Vec<OrgSyncOutcome>, ItGlueSyncError> {
        let mut outcomes = Vec::new();
        for org_id in org_ids {
            let Ok(client_id) = mapping.client_for(org_id) else {
                outcomes.push(OrgSyncOutcome {
                    org_id: org_id.clone(),
                    errors: vec![ItGlueSyncError::UnmappedOrg(org_id.clone()).to_string()],
                    ..OrgSyncOutcome::default()
                });
                continue;
            };
            let (records, map_errors) = client.configurations_for_org(org_id).await?;
            let report = FeederSync::run(&SyncConfig::new(client_id), &records, sink).await;
            let mut errors: Vec<String> = map_errors;
            for err in &report.errors {
                errors.push(format!("{}: {}", err.external_id, err.error));
            }
            outcomes.push(OrgSyncOutcome {
                org_id: org_id.clone(),
                total: records.len(),
                errors,
                report: Some(report),
            });
        }
        Ok(outcomes)
    }
}

/// Convenience re-export so operators wire one sink vocabulary.
pub use ci_feeders::sink::{MeridianFeederConfig, MeridianFeederSink, NoneFeederSink};

/// Re-exported for sync drivers that need the sink error type.
pub type SinkSurface = SinkError;

#[cfg(test)]
mod tests;
