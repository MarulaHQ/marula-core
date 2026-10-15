//! Read-only Cloudflare edge posture helpers for the V2 TLS runbook
//! (`p47-edge-tls` / `docs/site/reference/reverse-proxy-tls.md`).
//!
//! Every function here is a GET. Nothing in this module ever patches a zone
//! SSL setting, mints a certificate, or flips a record's proxy mode: the
//! posture is reported so an operator can check the runbook's steps
//! instead of guessing, and any change they decide on is made explicitly
//! (in the Cloudflare dashboard, or through the DNS helpers with the
//! operator's own parameters).

use adapters_core::dns::{normalize_domain, DnsRecordKind, DnsRecordState, DnsZone};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::{strip_txt_quotes, CloudflareApi, CloudflareConfig, CloudflareError};

/// Zone SSL/TLS encryption mode as Cloudflare reports it.
///
/// Cloudflare's API value is `off`, `flexible`, `full`, or `strict`; older
/// releases spelled full-with-validation as `full_strict`, which
/// [`SslMode::parse`] maps to [`SslMode::Strict`]. The serialized form is
/// the Cloudflare value itself, so an unrecognized mode round-trips instead
/// of failing a report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, rename_all = "snake_case")]
pub enum SslMode {
    Off,
    Flexible,
    Full,
    Strict,
    Unknown(String),
}

impl SslMode {
    /// Parses a Cloudflare mode value.
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" => SslMode::Off,
            "flexible" => SslMode::Flexible,
            "full" => SslMode::Full,
            "strict" | "full_strict" => SslMode::Strict,
            other => SslMode::Unknown(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            SslMode::Off => "off",
            SslMode::Flexible => "flexible",
            SslMode::Full => "full",
            SslMode::Strict => "strict",
            SslMode::Unknown(raw) => raw.as_str(),
        }
    }

    /// The V2 Edge-mode requirement is `strict` (the dashboard's
    /// "Full (strict)").
    pub fn meets_v2_edge_requirement(&self) -> bool {
        matches!(self, SslMode::Strict)
    }
}

impl std::fmt::Display for SslMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The zone's SSL/TLS mode plus whether it satisfies the V2 Edge-mode
/// checklist, with a `finding` when it does not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoneSslPosture {
    pub zone_id: String,
    pub mode: SslMode,
    pub meets_v2_strict_requirement: bool,
    /// Operator-readable gap against the runbook requirement; `None` when
    /// the requirement is met.
    pub finding: Option<String>,
}

impl ZoneSslPosture {
    pub fn is_ready(&self) -> bool {
        self.meets_v2_strict_requirement
    }
}

/// One Origin CA certificate as `GET /certificates?zone_id=` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginCaCertificate {
    pub id: String,
    pub hostnames: Vec<String>,
    pub expires_on: Option<String>,
    pub request_type: Option<String>,
    pub revoked_at: Option<String>,
}

impl OriginCaCertificate {
    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }

    /// Whether the certificate (still valid, not revoked) covers `hostname`.
    /// Wildcards (`*.example.com`) match one label below the parent.
    pub fn covers_hostname(&self, hostname: &str) -> bool {
        if self.is_revoked() {
            return false;
        }
        let wanted = hostname.trim().to_ascii_lowercase();
        self.hostnames.iter().any(|entry| {
            let entry = entry.trim().to_ascii_lowercase();
            match entry.strip_prefix("*.") {
                Some(parent) => wanted
                    .strip_suffix(parent)
                    .and_then(|rest| rest.strip_suffix('.'))
                    .map(|label| !label.is_empty())
                    .unwrap_or(false),
                None => entry == wanted,
            }
        })
    }
}

/// Structured posture report for one custom domain against the V2
/// Cloudflare Edge-mode checklist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgePostureReport {
    pub domain: String,
    pub zone: DnsZone,
    pub ssl: ZoneSslPosture,
    /// The `A`/`AAAA`/`CNAME` records currently published for `domain`.
    pub edge_records: Vec<DnsRecordState>,
    /// Origin CA certificate inventory for the zone.
    pub origin_ca_certificates: Vec<OriginCaCertificate>,
    /// Operator-readable gaps, each phrased as the runbook step that closes it.
    pub warnings: Vec<String>,
}

impl EdgePostureReport {
    /// Whether any published record for the hostname is orange-cloud proxied.
    pub fn hostname_proxied(&self) -> bool {
        self.edge_records
            .iter()
            .any(|record| record.name.eq_ignore_ascii_case(&self.domain) && record.proxied)
    }

    /// Whether a live Origin CA certificate covers the hostname.
    pub fn has_active_origin_ca_for_hostname(&self) -> bool {
        self.origin_ca_certificates
            .iter()
            .any(|cert| cert.covers_hostname(&self.domain))
    }

    /// The whole V2 Edge-mode checklist in one boolean: SSL `strict`, an
    /// orange-cloud record for the hostname, and a covering Origin CA
    /// certificate.
    pub fn v2_edge_mode_ready(&self) -> bool {
        self.ssl.meets_v2_strict_requirement
            && self.hostname_proxied()
            && self.has_active_origin_ca_for_hostname()
    }

    /// The checklist rows as `(description, passed, detail)`.
    pub fn checklist(&self) -> Vec<(String, bool, String)> {
        let mut rows = vec![(
            "Zone SSL/TLS mode is `strict` (Full (strict))".to_string(),
            self.ssl.meets_v2_strict_requirement,
            self.ssl
                .finding
                .clone()
                .unwrap_or_else(|| format!("mode `{}` meets the requirement", self.ssl.mode)),
        )];
        let proxy_count = self
            .edge_records
            .iter()
            .filter(|record| record.proxied)
            .count();
        rows.push((
            format!(
                "`{}` is served through the Cloudflare proxy (orange cloud)",
                self.domain
            ),
            self.hostname_proxied(),
            if self.hostname_proxied() {
                format!("{proxy_count} proxied record(s) published for the hostname")
            } else {
                "no proxied A/AAAA/CNAME record here; enable the proxy or run Direct TLS mode"
                    .to_string()
            },
        ));
        let live_certs: Vec<String> = self
            .origin_ca_certificates
            .iter()
            .filter(|cert| !cert.is_revoked())
            .map(|cert| cert.id.clone())
            .collect();
        rows.push((
            format!("A live Origin CA certificate covers `{}`", self.domain),
            self.has_active_origin_ca_for_hostname(),
            if self.has_active_origin_ca_for_hostname() {
                format!(
                    "live Origin CA certificates in zone: {}",
                    live_certs.join(", ")
                )
            } else {
                "no live Origin CA certificate covers the hostname".to_string()
            },
        ));
        rows
    }
}

/// Read-only edge-posture half of the Cloudflare connector.
#[derive(Clone)]
pub struct CloudflareEdgePosture {
    api: CloudflareApi,
}

impl CloudflareEdgePosture {
    pub fn new(config: &CloudflareConfig) -> Self {
        Self {
            api: CloudflareApi::new(config),
        }
    }

    pub fn with_token(api_token: impl Into<String>) -> Self {
        Self::new(&CloudflareConfig::new(api_token))
    }

    pub fn base_url(&self) -> &str {
        self.api.base_url()
    }

    /// Discovers the zone for a custom domain (delegates to the shared zone
    /// walk used by the DNS half).
    pub async fn discover_zone(&self, domain: &str) -> Result<DnsZone, CloudflareError> {
        self.api.discover_zone(domain).await
    }

    /// `GET /zones/{zone_id}/settings/ssl` — the zone's encryption mode and
    /// whether it meets the V2 Edge requirement. Read-only.
    pub async fn inspect_zone_ssl(&self, zone_id: &str) -> Result<ZoneSslPosture, CloudflareError> {
        let result = self
            .api
            .get(
                "ssl settings read",
                &format!("/zones/{zone_id}/settings/ssl"),
                &[],
            )
            .await?;
        let raw = result
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default();
        Ok(ZoneSslPosture::for_zone(zone_id, &SslMode::parse(raw)))
    }

    /// `GET /certificates?zone_id=` — the zone's Origin CA certificate
    /// inventory. Read-only.
    pub async fn list_origin_ca_certificates(
        &self,
        zone_id: &str,
    ) -> Result<Vec<OriginCaCertificate>, CloudflareError> {
        let query = vec![("zone_id".to_string(), zone_id.to_string())];
        let result = self
            .api
            .get("origin ca list", "/certificates", &query)
            .await?;
        let certificates: Vec<OriginCaCertificate> = result
            .as_array()
            .map(|items| items.iter().map(parse_certificate).collect())
            .unwrap_or_default();
        Ok(certificates)
    }

    /// The `A`/`AAAA`/`CNAME` records published for `hostname`, used to see
    /// whether the hostname is currently orange-cloud proxied. Read-only.
    pub async fn inspect_hostname_records(
        &self,
        zone_id: &str,
        hostname: &str,
    ) -> Result<Vec<DnsRecordState>, CloudflareError> {
        let query = vec![(
            "name".to_string(),
            CloudflareApi::encode_query_value(hostname),
        )];
        let result = self
            .api
            .get(
                "dns record list",
                &format!("/zones/{zone_id}/dns_records"),
                &query,
            )
            .await?;
        let items = result.as_array().cloned().unwrap_or_default();
        items
            .iter()
            .filter(|raw| {
                raw.get("type")
                    .and_then(Value::as_str)
                    .and_then(DnsRecordKind::parse_kind)
                    .is_some_and(|kind| {
                        matches!(
                            kind,
                            DnsRecordKind::A | DnsRecordKind::Aaaa | DnsRecordKind::Cname
                        )
                    })
            })
            .map(|raw| parse_posture_record(zone_id, raw))
            .collect()
    }

    /// Builds the full posture report for `domain` against the V2
    /// Cloudflare Edge-mode checklist. Read-only.
    pub async fn inspect_edge_posture(
        &self,
        domain: &str,
    ) -> Result<EdgePostureReport, CloudflareError> {
        let domain = normalize_domain(domain)?;
        let zone = self.api.discover_zone(&domain).await?;
        let ssl = self.inspect_zone_ssl(&zone.id).await?;
        let origin_ca_certificates = self.list_origin_ca_certificates(&zone.id).await?;
        let edge_records = self.inspect_hostname_records(&zone.id, &domain).await?;
        Ok(EdgePostureReport::build(
            domain,
            zone,
            ssl,
            edge_records,
            origin_ca_certificates,
        ))
    }
}

impl ZoneSslPosture {
    /// Derives the posture for a zone, naming the runbook step that closes
    /// any gap.
    pub fn for_zone(zone_id: &str, mode: &SslMode) -> Self {
        let meeting = mode.meets_v2_edge_requirement();
        let finding = match mode {
            SslMode::Strict => None,
            SslMode::Off => {
                Some("zone encryption is `off`; the V2 Edge mode requires `strict`".to_string())
            }
            SslMode::Flexible => Some(
                "zone encryption is `flexible`, so Cloudflare fetches the origin over \
                 plaintext http and an observer on that path can replay it; set `strict` \
                 (docs/site/reference/reverse-proxy-tls.md, \
                 \"Why Edge mode breaks the obvious path\")"
                    .to_string(),
            ),
            SslMode::Full => Some(
                "zone encryption is `full`, which accepts any certificate from the \
                 origin including an expired or self-signed one; set `strict`"
                    .to_string(),
            ),
            SslMode::Unknown(raw) => Some(format!(
                "zone encryption mode `{raw}` is unrecognized; the V2 Edge mode requires `strict`"
            )),
        };
        ZoneSslPosture {
            zone_id: zone_id.to_string(),
            mode: mode.clone(),
            meets_v2_strict_requirement: meeting,
            finding,
        }
    }
}

impl EdgePostureReport {
    fn build(
        domain: String,
        zone: DnsZone,
        ssl: ZoneSslPosture,
        edge_records: Vec<DnsRecordState>,
        origin_ca_certificates: Vec<OriginCaCertificate>,
    ) -> Self {
        let mut warnings = Vec::new();
        if let Some(finding) = &ssl.finding {
            warnings.push(finding.clone());
        }
        let proxied_count = edge_records.iter().filter(|record| record.proxied).count();
        if proxied_count == 0 {
            warnings.push(format!(
                "no orange-cloud A/AAAA/CNAME record is published for `{domain}`; \
                 the runbook's origin-lockdown allowlist does not cover it"
            ));
        }
        if !origin_ca_certificates
            .iter()
            .any(|cert| cert.covers_hostname(&domain))
        {
            warnings.push(format!(
                "no live Origin CA certificate covers `{domain}`; \
                 under `strict` the edge rejects the origin"
            ));
        }
        EdgePostureReport {
            domain,
            zone,
            ssl,
            edge_records,
            origin_ca_certificates,
            warnings,
        }
    }
}

fn parse_certificate(raw: &Value) -> OriginCaCertificate {
    let text = |key: &str| {
        raw.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    OriginCaCertificate {
        id: raw
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        hostnames: raw
            .get("hostnames")
            .and_then(Value::as_array)
            .map(|hostnames| {
                hostnames
                    .iter()
                    .filter_map(|entry| entry.as_str().map(strip_txt_quotes))
                    .collect()
            })
            .unwrap_or_default(),
        expires_on: text("expires_on"),
        request_type: text("request_type"),
        revoked_at: text("revoked_at"),
    }
}

fn parse_posture_record(zone_id: &str, raw: &Value) -> Result<DnsRecordState, CloudflareError> {
    let id = raw
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let kind = raw
        .get("type")
        .and_then(Value::as_str)
        .and_then(DnsRecordKind::parse_kind)
        .ok_or_else(|| CloudflareError::Rejected {
            operation: "posture record parse".to_string(),
            detail: "record response is missing a supported `type`".to_string(),
        })?;
    Ok(DnsRecordState {
        name: raw
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase(),
        kind,
        content: raw
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        ttl: raw.get("ttl").and_then(Value::as_u64).unwrap_or(1) as u32,
        proxied: raw.get("proxied").and_then(Value::as_bool).unwrap_or(false),
        proxiable: raw
            .get("proxiable")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        id,
        zone_id: zone_id.to_string(),
        zone_name: String::new(),
    })
}
