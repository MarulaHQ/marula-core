//! Vendor-agnostic DNS-record-management contract (Wave 47 lanes A8 / A11).
//!
//! Core's custom-domain verification path is generic DNS: the core resolves
//! `_marula-verify.<domain>` `TXT` through the system resolver and compares the
//! issued verification token. Because `TXT` records are never edge-proxied,
//! that verification path works identically with Cloudflare, Route 53, Gandi,
//! Azure DNS, Google Cloud DNS, or a self-hosted nameserver with zero provider
//! integration.
//!
//! When an operator registers a DNS provider plugin (such as the in-repo
//! Apache-2.0 `cloudflare` reference connector or a commercial marketplace pack),
//! [`DnsRecordProvider`] automates three operations on the tenant's zone:
//!
//! 1. **Zone discovery** ([`DnsRecordProvider::discover_zone`]) — walk parent
//!    domain labels from the target hostname down to the registrable apex until
//!    the authoritative zone is found.
//! 2. **Verification TXT lifecycle** ([`DnsRecordProvider::upsert_verification_txt`],
//!    [`DnsRecordProvider::remove_verification_txt`]) — idempotently create/update
//!    and remove `_marula-verify.<domain>` `TXT` (`proxied: false` always).
//! 3. **Proxy-mode-aware edge record upsert** ([`DnsRecordProvider::upsert_edge_record`]) —
//!    upsert the platform edge `A`, `AAAA`, or `CNAME` record and report whether
//!    the record is CDN-proxied (`proxied: true`, orange cloud) or DNS-only
//!    (`proxied: false`).
//!
//! **None mode:** [`NoneDnsRecordProvider`] generates exact manual DNS paste
//! instructions ([`ManualDnsInstructions`]) and provides the deterministic TXT
//! token matcher ([`NoneDnsRecordProvider::verify_txt_records`]) so custom
//! domains never depend on a provider plugin.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Prefix prepended to the custom domain for DNS TXT ownership verification.
pub const VERIFICATION_TXT_PREFIX: &str = "_marula-verify";

/// DNS record type managed through the custom-domain seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DnsRecordKind {
    A,
    #[serde(rename = "AAAA")]
    Aaaa,
    #[serde(rename = "CNAME")]
    Cname,
    #[serde(rename = "TXT")]
    Txt,
}

impl DnsRecordKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            DnsRecordKind::A => "A",
            DnsRecordKind::Aaaa => "AAAA",
            DnsRecordKind::Cname => "CNAME",
            DnsRecordKind::Txt => "TXT",
        }
    }

    pub fn parse_kind(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_uppercase().as_str() {
            "A" => Some(DnsRecordKind::A),
            "AAAA" => Some(DnsRecordKind::Aaaa),
            "CNAME" => Some(DnsRecordKind::Cname),
            "TXT" => Some(DnsRecordKind::Txt),
            _ => None,
        }
    }
}

impl std::fmt::Display for DnsRecordKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Address/alias record kinds valid for pointing a custom domain at the
/// platform edge (`A`, `AAAA`, or `CNAME`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum EdgeTargetKind {
    A,
    #[serde(rename = "AAAA")]
    Aaaa,
    #[serde(rename = "CNAME")]
    Cname,
}

impl EdgeTargetKind {
    pub const fn as_record_kind(self) -> DnsRecordKind {
        match self {
            EdgeTargetKind::A => DnsRecordKind::A,
            EdgeTargetKind::Aaaa => DnsRecordKind::Aaaa,
            EdgeTargetKind::Cname => DnsRecordKind::Cname,
        }
    }

    pub const fn as_str(self) -> &'static str {
        self.as_record_kind().as_str()
    }
}

impl From<EdgeTargetKind> for DnsRecordKind {
    fn from(value: EdgeTargetKind) -> Self {
        value.as_record_kind()
    }
}

/// Operator-declared platform edge record specification for a custom domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeRecordSpec {
    /// Custom domain hostname (e.g. `support.msp.example.com`).
    pub domain: String,
    /// Record type (`A`, `AAAA`, or `CNAME`).
    pub kind: EdgeTargetKind,
    /// Target IPv4 address, IPv6 address, or canonical edge hostname.
    pub target: String,
    /// Whether CDN edge proxying (Cloudflare orange cloud) should be enabled.
    pub proxied: bool,
    /// Optional explicit TTL in seconds. When `proxied` is `true`, Cloudflare
    /// forces `ttl = 1` (automatic).
    pub ttl: Option<u32>,
}

impl EdgeRecordSpec {
    pub fn cname(domain: impl Into<String>, target: impl Into<String>, proxied: bool) -> Self {
        Self {
            domain: domain.into(),
            kind: EdgeTargetKind::Cname,
            target: target.into(),
            proxied,
            ttl: None,
        }
    }

    pub fn a(domain: impl Into<String>, ipv4: impl Into<String>, proxied: bool) -> Self {
        Self {
            domain: domain.into(),
            kind: EdgeTargetKind::A,
            target: ipv4.into(),
            proxied,
            ttl: None,
        }
    }

    pub fn aaaa(domain: impl Into<String>, ipv6: impl Into<String>, proxied: bool) -> Self {
        Self {
            domain: domain.into(),
            kind: EdgeTargetKind::Aaaa,
            target: ipv6.into(),
            proxied,
            ttl: None,
        }
    }

    pub fn with_ttl(mut self, ttl: u32) -> Self {
        self.ttl = Some(ttl);
        self
    }

    /// Effective TTL sent to the DNS provider: `1` (automatic) when `proxied`
    /// is enabled, otherwise the explicit TTL or `300` seconds.
    pub fn effective_ttl(&self) -> u32 {
        if self.proxied {
            1
        } else {
            self.ttl.unwrap_or(300)
        }
    }
}

/// Authoritative DNS zone metadata resolved for a custom domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsZone {
    pub id: String,
    pub name: String,
    pub status: String,
}

/// Normalized DNS record state returned after a create, update, or lookup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsRecordState {
    pub id: String,
    pub zone_id: String,
    pub zone_name: String,
    pub name: String,
    pub kind: DnsRecordKind,
    pub content: String,
    /// TTL in seconds (`1` means automatic on Cloudflare).
    pub ttl: u32,
    /// True when the record is orange-cloud proxied through the CDN edge.
    /// Always `false` for `_marula-verify.<domain>` `TXT` records.
    pub proxied: bool,
    /// Whether the provider permits edge proxying on this record type.
    pub proxiable: bool,
}

/// One record entry inside [`ManualDnsInstructions`] for operators configuring
/// DNS by hand (`NoneMode`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualDnsRecord {
    pub name: String,
    pub kind: DnsRecordKind,
    pub value: String,
    pub ttl: u32,
    pub notes: String,
}

/// Complete manual DNS setup instructions when no DNS provider plugin is
/// configured (`NoneMode`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualDnsInstructions {
    pub domain: String,
    pub verification_txt: ManualDnsRecord,
    pub edge_record: Option<ManualDnsRecord>,
    pub verification_note: String,
}

/// Typed errors returned by [`DnsRecordProvider`] implementations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DnsProviderError {
    #[error("invalid domain name `{0}`")]
    InvalidDomain(String),
    #[error("invalid DNS record parameter: {0}")]
    InvalidRecord(String),
    #[error("no authoritative zone found for `{domain}` (tried: {})", tried.join(", "))]
    ZoneNotFound { domain: String, tried: Vec<String> },
    #[error("provider token unauthorized or missing required scope for {operation}: {detail}")]
    UnauthorizedScope { operation: String, detail: String },
    #[error("provider rate limited request: {0}")]
    RateLimited(String),
    #[error("provider rejected {operation}: {detail}")]
    Rejected { operation: String, detail: String },
    #[error("provider transport failure: {0}")]
    Transport(String),
}

/// Vendor-agnostic contract for custom-domain DNS automation (A8 / A11 seam).
///
/// Implementations automate zone discovery, `_marula-verify.<domain>` `TXT`
/// record creation/removal, and proxy-mode-aware edge `A`/`AAAA`/`CNAME`
/// upserts. Core never requires a provider to be registered: [`NoneDnsRecordProvider`]
/// is the complete manual fallback.
#[async_trait]
pub trait DnsRecordProvider: Send + Sync {
    /// Stable provider identifier (e.g. `"cloudflare"`, `"none"`).
    fn provider_kind(&self) -> &'static str;

    /// Discover the authoritative zone for `domain` by walking parent labels.
    async fn discover_zone(&self, domain: &str) -> Result<DnsZone, DnsProviderError>;

    /// Create or update the unproxied `_marula-verify.<domain>` `TXT` record
    /// carrying `verification_token`. Must be idempotent on repeated calls.
    async fn upsert_verification_txt(
        &self,
        domain: &str,
        verification_token: &str,
    ) -> Result<DnsRecordState, DnsProviderError>;

    /// Remove the `_marula-verify.<domain>` `TXT` record if present.
    /// Returns `Ok(true)` when a record was deleted and `Ok(false)` when
    /// already absent (idempotent).
    async fn remove_verification_txt(&self, domain: &str) -> Result<bool, DnsProviderError>;

    /// Create or update the platform edge `A`, `AAAA`, or `CNAME` record for
    /// `spec.domain`, preserving `spec.proxied` and reporting the resulting
    /// proxy posture in [`DnsRecordState::proxied`].
    async fn upsert_edge_record(
        &self,
        spec: &EdgeRecordSpec,
    ) -> Result<DnsRecordState, DnsProviderError>;
}

/// Normalizes and validates a DNS hostname (lowercased, trailing dot stripped,
/// at least two labels, valid label characters).
pub fn normalize_domain(raw: &str) -> Result<String, DnsProviderError> {
    let trimmed = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    let record_prefix = format!("{VERIFICATION_TXT_PREFIX}.");
    let stripped = trimmed
        .strip_prefix(record_prefix.as_str())
        .unwrap_or(trimmed.as_str());

    if stripped.is_empty() || stripped.len() > 253 {
        return Err(DnsProviderError::InvalidDomain(raw.to_string()));
    }

    let labels: Vec<&str> = stripped.split('.').collect();
    if labels.len() < 2 {
        return Err(DnsProviderError::InvalidDomain(raw.to_string()));
    }

    for label in &labels {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(DnsProviderError::InvalidDomain(raw.to_string()));
        }
    }

    Ok(stripped.to_string())
}

/// Builds the canonical `_marula-verify.<domain>` `TXT` record name for `domain`.
pub fn verification_record_name(domain: &str) -> Result<String, DnsProviderError> {
    let normalized = normalize_domain(domain)?;
    Ok(format!("{VERIFICATION_TXT_PREFIX}.{normalized}"))
}

/// Returns candidate zone names from most specific (`support.msp.example.com`)
/// down to the two-label registrable apex (`example.com`). Never includes a
/// single-label TLD (`com`).
pub fn candidate_zone_names(domain: &str) -> Result<Vec<String>, DnsProviderError> {
    let normalized = normalize_domain(domain)?;
    let labels: Vec<&str> = normalized.split('.').collect();
    // Walk from the hostname down to the two-label registrable apex: a bare
    // TLD (`com`) is never a zone, so it is never a candidate. One candidate
    // per label past the last, which for `support.msp.example.com` is the
    // hostname, `msp.example.com`, then `example.com`.
    Ok((0..labels.len().saturating_sub(1))
        .map(|start| labels[start..].join("."))
        .collect())
}

/// Manual / fallback DNS provider (`NoneMode`).
///
/// When no DNS provider plugin is configured, operators copy the records from
/// [`NoneDnsRecordProvider::manual_instructions`] into any DNS host (Cloudflare,
/// Route 53, Gandi, Azure DNS, BIND, etc.) and Core validates ownership by
/// resolving `_marula-verify.<domain>` `TXT` through the system resolver.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoneDnsRecordProvider;

impl NoneDnsRecordProvider {
    pub const fn new() -> Self {
        Self
    }

    /// Generates human- and UI-ready manual DNS instructions for a custom
    /// domain and optional edge target (`A`, `AAAA`, or `CNAME`).
    pub fn manual_instructions(
        domain: &str,
        verification_token: &str,
        edge: Option<&EdgeRecordSpec>,
    ) -> Result<ManualDnsInstructions, DnsProviderError> {
        let normalized_domain = normalize_domain(domain)?;
        let token = verification_token.trim();
        if token.is_empty() {
            return Err(DnsProviderError::InvalidRecord(
                "verification_token must not be empty".into(),
            ));
        }
        let txt_name = verification_record_name(&normalized_domain)?;
        let verification_txt = ManualDnsRecord {
            name: txt_name.clone(),
            kind: DnsRecordKind::Txt,
            value: token.to_string(),
            ttl: 300,
            notes: "DNS-only (unproxied). TXT records are never edge-proxied; Core \
                    verifies ownership by resolving this record via the system resolver."
                .into(),
        };

        let edge_record = match edge {
            Some(spec) => {
                let target = spec.target.trim();
                if target.is_empty() {
                    return Err(DnsProviderError::InvalidRecord(
                        "edge target must not be empty".into(),
                    ));
                }
                let proxy_note = if spec.proxied {
                    "Orange-cloud proxy enabled, which pairs with SSL/TLS Full (strict). \
                     The custom-domain verifier tolerates edge proxy IPs on this record \
                     because the TXT token is the ownership authority."
                } else {
                    "DNS-only (unproxied / Direct TLS mode)."
                };
                Some(ManualDnsRecord {
                    name: normalized_domain.clone(),
                    kind: spec.kind.into(),
                    value: target.to_string(),
                    ttl: spec.effective_ttl(),
                    notes: proxy_note.into(),
                })
            }
            None => None,
        };

        Ok(ManualDnsInstructions {
            domain: normalized_domain,
            verification_txt,
            edge_record,
            verification_note: format!(
                "Publish {txt_name} TXT with the exact token value. Core resolves \
                 {txt_name} over standard DNS, whether your DNS is a provider you \
                 configured or a host you edit by hand."
            ),
        })
    }

    /// Evaluates whether the TXT values returned by a DNS resolver for
    /// `_marula-verify.<domain>` satisfy `expected_token`.
    ///
    /// Handles both raw resolver strings and presentation-quoted TXT strings
    /// (`"\"marula-verify-...\""`).
    pub fn verify_txt_records<S: AsRef<str>>(
        observed_txt_values: &[S],
        expected_token: &str,
    ) -> bool {
        let expected = expected_token.trim();
        if expected.is_empty() {
            return false;
        }
        observed_txt_values.iter().any(|raw| {
            let trimmed = raw.as_ref().trim();
            let unquoted = trimmed
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(trimmed);
            unquoted == expected
        })
    }
}

#[async_trait]
impl DnsRecordProvider for NoneDnsRecordProvider {
    fn provider_kind(&self) -> &'static str {
        "none"
    }

    async fn discover_zone(&self, domain: &str) -> Result<DnsZone, DnsProviderError> {
        let candidates = candidate_zone_names(domain)?;
        let apex = candidates
            .last()
            .cloned()
            .ok_or_else(|| DnsProviderError::InvalidDomain(domain.to_string()))?;
        Ok(DnsZone {
            id: format!("none-zone-{apex}"),
            name: apex,
            status: "manual".into(),
        })
    }

    async fn upsert_verification_txt(
        &self,
        domain: &str,
        verification_token: &str,
    ) -> Result<DnsRecordState, DnsProviderError> {
        let token = verification_token.trim();
        if token.is_empty() {
            return Err(DnsProviderError::InvalidRecord(
                "verification_token must not be empty".into(),
            ));
        }
        let zone = self.discover_zone(domain).await?;
        let name = verification_record_name(domain)?;
        Ok(DnsRecordState {
            id: format!("none-txt-{name}"),
            zone_id: zone.id,
            zone_name: zone.name,
            name,
            kind: DnsRecordKind::Txt,
            content: token.to_string(),
            ttl: 300,
            proxied: false,
            proxiable: false,
        })
    }

    async fn remove_verification_txt(&self, domain: &str) -> Result<bool, DnsProviderError> {
        let _ = verification_record_name(domain)?;
        Ok(false)
    }

    async fn upsert_edge_record(
        &self,
        spec: &EdgeRecordSpec,
    ) -> Result<DnsRecordState, DnsProviderError> {
        let domain = normalize_domain(&spec.domain)?;
        let target = spec.target.trim();
        if target.is_empty() {
            return Err(DnsProviderError::InvalidRecord(
                "edge target must not be empty".into(),
            ));
        }
        let zone = self.discover_zone(&domain).await?;
        Ok(DnsRecordState {
            id: format!("none-{}-{domain}", spec.kind.as_str().to_ascii_lowercase()),
            zone_id: zone.id,
            zone_name: zone.name,
            name: domain,
            kind: spec.kind.into(),
            content: target.to_string(),
            ttl: spec.effective_ttl(),
            proxied: spec.proxied,
            proxiable: true,
        })
    }
}
