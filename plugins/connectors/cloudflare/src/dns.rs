//! Cloudflare implementation of the vendor-agnostic DNS-record-management
//! contract (Wave 47 lanes A8 / A11).
//!
//! It automates exactly the three operations the contract names:
//!
//! 1. **Zone discovery** — walk parent labels against `GET /zones?name=`.
//! 2. **`_marula-verify.<domain>` TXT lifecycle** — create/upsert and remove,
//!    always `proxied: false`, idempotently.
//! 3. **Proxy-mode-aware edge `A`/`AAAA`/`CNAME` upsert** — `ttl: 1` when the
//!    record is orange-cloud proxied, with the resulting `proxied` state
//!    surfaced in the returned record.
//!
//! Every write is an upsert: an existing record of the same name and type is
//! updated in place (never duplicated), and a no-op re-run performs no write.

use adapters_core::dns::{
    normalize_domain, verification_record_name, DnsProviderError, DnsRecordKind, DnsRecordProvider,
    DnsRecordState, DnsZone, EdgeRecordSpec, EdgeTargetKind,
};
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::api::{CloudflareApi, CloudflareConfig, CloudflareError};

/// TTL used for unproxied records when the operator does not declare one
/// (Cloudflare accepts 60..86400 for DNS-only records).
const DEFAULT_DNS_ONLY_TTL: u32 = 300;

/// TTL Cloudflare forces for proxied records (`1` == automatic).
const PROXIED_TTL: u32 = 1;

/// DNS-management half of the Cloudflare connector.
///
/// ```no_run
/// # use cloudflare::{CloudflareConfig, CloudflareDnsClient};
/// # async fn discover_shapes() -> Result<(), cloudflare::CloudflareError> {
/// let dns = CloudflareDnsClient::new(&CloudflareConfig::new("operator-api-token"));
/// let zone = dns.discover_zone_for("support.msp.example.com").await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct CloudflareDnsClient {
    api: CloudflareApi,
}

impl CloudflareDnsClient {
    pub fn new(config: &CloudflareConfig) -> Self {
        Self {
            api: CloudflareApi::new(config),
        }
    }

    /// Convenience constructor for the stock API endpoint.
    pub fn with_token(api_token: impl Into<String>) -> Self {
        Self::new(&CloudflareConfig::new(api_token))
    }

    pub fn base_url(&self) -> &str {
        self.api.base_url()
    }

    /// Discovers the authoritative zone for `domain`.
    pub async fn discover_zone_for(&self, domain: &str) -> Result<DnsZone, CloudflareError> {
        self.api.discover_zone(domain).await
    }

    /// Every DNS record in `zone_id`, parsed into the vendor-agnostic shape.
    pub async fn list_records(
        &self,
        zone_id: &str,
        zone_name: &str,
        filters: &[(String, String)],
    ) -> Result<Vec<DnsRecordState>, CloudflareError> {
        let raw = self.api.dns_records(zone_id, filters).await?;
        raw.iter()
            .map(|record| parse_record(zone_id, zone_name, record))
            .collect()
    }

    /// Every DNS record in a zone as the vendor-agnostic shape.
    pub async fn zone_records(
        &self,
        zone: &DnsZone,
    ) -> Result<Vec<DnsRecordState>, CloudflareError> {
        self.list_records(&zone.id, &zone.name, &[]).await
    }

    /// Creates or updates the unproxied `_marula-verify.<domain>` TXT record
    /// carrying `verification_token`.
    ///
    /// Idempotent: an existing record with the same name and type is updated
    /// in place, and a re-run whose content already matches performs no
    /// write at all. The TXT record is **never** orange-cloud proxied so
    /// Core's system-resolver verification works regardless of how the edge
    /// `A`/`AAAA`/`CNAME` record is proxied.
    pub async fn upsert_txt(
        &self,
        domain: &str,
        verification_token: &str,
    ) -> Result<DnsRecordState, CloudflareError> {
        let domain = normalize_domain(domain)?;
        let token = strip_quotes(verification_token.trim());
        if token.is_empty() {
            return Err(CloudflareError::InvalidRecord(
                "verification_token must not be empty".into(),
            ));
        }
        let name = verification_record_name(&domain)?;
        let zone = self.api.discover_zone(&domain).await?;

        let query = vec![
            ("type".to_string(), "TXT".to_string()),
            ("name".to_string(), CloudflareApi::encode_query_value(&name)),
        ];
        let existing = self.api.dns_records(&zone.id, &query).await?;
        if let Some(record) = existing
            .iter()
            .find(|raw| record_matches(raw, &name, "TXT"))
        {
            let current = parse_record(&zone.id, &zone.name, record)?;
            if current.content == token && !current.proxied {
                // Already exactly right: a no-op, never a duplicate row.
                return Ok(current);
            }
            let payload = txt_payload(&name, &token);
            let updated = self
                .api
                .put(
                    "verification txt upsert",
                    &CloudflareApi::record_path(&zone.id, &current.id),
                    &payload,
                )
                .await?;
            return parse_record(&zone.id, &zone.name, &updated);
        }

        let payload = txt_payload(&name, &token);
        let created = self
            .api
            .post(
                "verification txt create",
                &format!("/zones/{}/dns_records", urlencode_zone(&zone.id)),
                &payload,
            )
            .await?;
        parse_record(&zone.id, &zone.name, &created)
    }

    /// Removes the `_marula-verify.<domain>` TXT record and returns
    /// `true`, or `false` when no such record exists (idempotent).
    pub async fn remove_txt(&self, domain: &str) -> Result<bool, CloudflareError> {
        let domain = normalize_domain(domain)?;
        let name = verification_record_name(&domain)?;
        let zone = self.api.discover_zone(&domain).await?;

        let query = vec![
            ("type".to_string(), "TXT".to_string()),
            ("name".to_string(), CloudflareApi::encode_query_value(&name)),
        ];
        let existing = self.api.dns_records(&zone.id, &query).await?;
        let mut removed_any = false;
        for raw in existing
            .iter()
            .filter(|raw| record_matches(raw, &name, "TXT"))
        {
            let current = parse_record(&zone.id, &zone.name, raw)?;
            // A 410 Gone from Cloudflare means a concurrent removal: still
            // a successful idempotent outcome.
            match self
                .api
                .delete(
                    "verification txt remove",
                    &CloudflareApi::record_path(&zone.id, &current.id),
                )
                .await
            {
                Ok(_) => removed_any = true,
                Err(CloudflareError::Rejected { detail, .. })
                    if detail.contains("410") || detail.contains("already deleted") =>
                {
                    removed_any = true;
                }
                Err(err) => return Err(err),
            }
        }
        Ok(removed_any)
    }

    /// Creates or updates the operator-declared edge record for
    /// `spec.domain` (`A`, `AAAA`, or `CNAME`), honouring `spec.proxied`
    /// (orange cloud) and forcing `ttl: 1` when it is enabled.
    pub async fn upsert_edge(
        &self,
        spec: &EdgeRecordSpec,
    ) -> Result<DnsRecordState, CloudflareError> {
        let domain = normalize_domain(&spec.domain)?;
        let target = validate_target(spec.kind, &spec.target)?;
        let zone = self.api.discover_zone(&domain).await?;
        let kind = spec.kind.as_str();
        let ttl = if spec.proxied {
            PROXIED_TTL
        } else {
            spec.ttl.unwrap_or(DEFAULT_DNS_ONLY_TTL)
        };

        let query = vec![
            ("type".to_string(), kind.to_string()),
            (
                "name".to_string(),
                CloudflareApi::encode_query_value(&domain),
            ),
        ];
        let existing = self.api.dns_records(&zone.id, &query).await?;
        let payload = json!({
            "type": kind,
            "name": domain,
            "content": target,
            "ttl": ttl,
            "proxied": spec.proxied,
        });

        if let Some(record) = existing
            .iter()
            .find(|raw| record_matches(raw, &domain, kind))
        {
            let current = parse_record(&zone.id, &zone.name, record)?;
            if current.content == target && current.proxied == spec.proxied && current.ttl == ttl {
                // Same proxy mode, same target, same TTL: no write needed.
                return Ok(current);
            }
            let updated = self
                .api
                .put(
                    "edge record upsert",
                    &CloudflareApi::record_path(&zone.id, &current.id),
                    &payload,
                )
                .await?;
            return parse_record(&zone.id, &zone.name, &updated);
        }

        let created = self
            .api
            .post(
                "edge record create",
                &format!("/zones/{}/dns_records", urlencode_zone(&zone.id)),
                &payload,
            )
            .await?;
        parse_record(&zone.id, &zone.name, &created)
    }
}

#[async_trait]
impl DnsRecordProvider for CloudflareDnsClient {
    fn provider_kind(&self) -> &'static str {
        "cloudflare"
    }

    async fn discover_zone(&self, domain: &str) -> Result<DnsZone, DnsProviderError> {
        self.api.discover_zone(domain).await.map_err(Into::into)
    }

    async fn upsert_verification_txt(
        &self,
        domain: &str,
        verification_token: &str,
    ) -> Result<DnsRecordState, DnsProviderError> {
        self.upsert_txt(domain, verification_token)
            .await
            .map_err(Into::into)
    }

    async fn remove_verification_txt(&self, domain: &str) -> Result<bool, DnsProviderError> {
        self.remove_txt(domain).await.map_err(Into::into)
    }

    async fn upsert_edge_record(
        &self,
        spec: &EdgeRecordSpec,
    ) -> Result<DnsRecordState, DnsProviderError> {
        self.upsert_edge(spec).await.map_err(Into::into)
    }
}

fn urlencode_zone(zone_id: &str) -> String {
    let mut out = String::with_capacity(zone_id.len());
    for ch in zone_id.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push_str(&format!("%{:02X}", ch as u32));
        }
    }
    out
}

fn strip_quotes(raw: &str) -> String {
    crate::api::strip_txt_quotes(raw)
}

fn txt_payload(name: &str, token: &str) -> Value {
    json!({
        "type": "TXT",
        "name": name,
        "content": token,
        "ttl": DEFAULT_DNS_ONLY_TTL,
        // Always unproxied: a proxied TXT record would break the
        // system-resolver verification Core runs on the value.
        "proxied": false,
    })
}

/// Case-insensitive exact match on record name and type. Cloudflare
/// normalizes neither, so a comparison that ignores ASCII case is the
/// strongest safe equality a client can assert.
fn record_matches(raw: &Value, name: &str, kind: &str) -> bool {
    let name_matches = raw
        .get("name")
        .and_then(Value::as_str)
        .map(|v| v.eq_ignore_ascii_case(name))
        .unwrap_or(false);
    let kind_matches = raw
        .get("type")
        .and_then(Value::as_str)
        .map(|v| v.eq_ignore_ascii_case(kind))
        .unwrap_or(false);
    name_matches && kind_matches
}

/// Parses a Cloudflare DNS record into the vendor-agnostic record state.
fn parse_record(
    zone_id: &str,
    zone_name: &str,
    raw: &Value,
) -> Result<DnsRecordState, CloudflareError> {
    let detail = |what: &str| CloudflareError::Rejected {
        operation: "dns record parse".to_string(),
        detail: format!("DNS record response is missing `{what}`"),
    };
    let id = raw
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| detail("id"))?;
    let kind_raw = raw
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| detail("type"))?;
    let kind = DnsRecordKind::parse_kind(kind_raw).ok_or_else(|| CloudflareError::Rejected {
        operation: "dns record parse".to_string(),
        detail: format!("unsupported Cloudflare record type `{kind_raw}`"),
    })?;
    let name = raw
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let content_raw = raw
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let content = if kind == DnsRecordKind::Txt {
        strip_quotes(content_raw)
    } else {
        content_raw.to_string()
    };
    let proxied = raw.get("proxied").and_then(Value::as_bool).unwrap_or(false);
    Ok(DnsRecordState {
        id: id.to_string(),
        zone_id: zone_id.to_string(),
        zone_name: zone_name.to_string(),
        name,
        kind,
        content,
        ttl: raw.get("ttl").and_then(Value::as_u64).unwrap_or(1) as u32,
        proxied,
        // Cloudflare marks proxiability on the record; when the field is
        // absent assume only address/alias records carry a proxy.
        proxiable: raw
            .get("proxiable")
            .and_then(Value::as_bool)
            .unwrap_or(matches!(
                kind,
                DnsRecordKind::A | DnsRecordKind::Aaaa | DnsRecordKind::Cname
            )),
    })
}

/// Validates and normalizes the record target for the declared edge kind.
fn validate_target(kind: EdgeTargetKind, target: &str) -> Result<String, CloudflareError> {
    let value = strip_quotes(target.trim());
    if value.is_empty() {
        return Err(CloudflareError::InvalidRecord(
            "edge record target must not be empty".into(),
        ));
    }
    match kind {
        EdgeTargetKind::A => {
            if value.parse::<std::net::Ipv4Addr>().is_err() {
                return Err(CloudflareError::InvalidRecord(format!(
                    "`{value}` is not a valid IPv4 address"
                )));
            }
        }
        EdgeTargetKind::Aaaa => {
            if value.parse::<std::net::Ipv6Addr>().is_err() {
                return Err(CloudflareError::InvalidRecord(format!(
                    "`{value}` is not a valid IPv6 address"
                )));
            }
        }
        EdgeTargetKind::Cname => {
            if !is_hostname(&value) {
                return Err(CloudflareError::InvalidRecord(format!(
                    "`{value}` is not a valid CNAME target hostname"
                )));
            }
        }
    }
    Ok(value)
}

fn is_hostname(value: &str) -> bool {
    let trimmed = value.trim_end_matches('.');
    let labels: Vec<&str> = trimmed.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    labels.iter().all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    })
}
