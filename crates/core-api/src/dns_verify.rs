//! Generic DNS verification for per-org custom domains (TASKLIST Wave 47
//! lane A8, `p47-org-custom-domains`).
//!
//! Core's custom-domain verification is vendor-AGNOSTIC by construction: it
//! resolves `_marula-verify.<domain>` `TXT` through the SYSTEM resolver and
//! compares the issued token. `TXT` records are universal and are never
//! edge-proxied, so the same check works whether the tenant's DNS is
//! Cloudflare, Route 53, Gandi, Azure DNS, a self-hosted BIND, or anything
//! else — with zero provider integration. The DNS-record-management seam
//! (`adapters_core::dns::DnsRecordProvider`) only AUTOMATES creating those
//! records; it is never required, and this module never consults it.
//!
//! Two lookups, deliberately different in strength:
//!
//!   * The **TXT check is the authority.** [`verify_domain`] resolves
//!     `_marula-verify.<domain>` and hands the observed strings to
//!     `adapters_core::dns::NoneDnsRecordProvider::verify_txt_records`, the
//!     single token matcher. A match is what sets `custom_domain_verified_at`.
//!   * The **A/CNAME check is plausibility only.** When the owner's DNS sits
//!     behind Cloudflare (or any CDN) with the proxy on, resolving the domain
//!     returns the edge's IPs rather than the origin's, so the check asks only
//!     "does this hostname resolve to SOMETHING?" — never origin-IP equality.
//!     The TXT token stays the authority; a resolved-but-proxied domain is
//!     perfectly valid.
//!
//! The resolver is abstracted behind [`TxtResolver`] so verification is
//! unit-testable without the network, and the process-wide instance
//! ([`txt_resolver`]) defaults to a system-configured
//! [`SystemTxtResolver`]. [`install_txt_resolver`] is the composition/test
//! hook that swaps in a deterministic resolver; it is idempotent (the first
//! install wins, later calls are refused).

use adapters_core::dns::{verification_record_name, NoneDnsRecordProvider};
use async_trait::async_trait;
use std::sync::{Arc, OnceLock};

/// A DNS lookup failure. Transport failures and an unbuildable resolver are
/// distinguished so a caller can tell "the record is absent" (an empty
/// answer, NOT an error) from "we could not ask".
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum DnsLookupError {
    #[error("no system resolver is available: {0}")]
    Unavailable(String),
    #[error("DNS lookup failed: {0}")]
    Failed(String),
}

/// Minimal resolver surface the verification path needs: `TXT` at a name,
/// and "does this hostname resolve at all" for the plausibility leg.
#[async_trait]
pub trait TxtResolver: Send + Sync {
    /// The `TXT` strings at `name` (one entry per record's joined
    /// character-strings). An empty vector is a definitive "no record",
    /// distinct from an error.
    async fn lookup_txt(&self, name: &str) -> Result<Vec<String>, DnsLookupError>;

    /// Whether `domain` resolves to at least one address (following CNAME
    /// chains). Edge-proxied records resolve to the CDN's IPs and count —
    /// this is plausibility, not an origin match.
    async fn resolves_to_address(&self, domain: &str) -> Result<bool, DnsLookupError>;
}

/// The system-resolver implementation: a hickory resolver configured from the
/// host (`/etc/resolv.conf` on Unix, the registry on Windows). When that
/// configuration is missing or malformed the struct is still constructed but
/// every lookup reports [`DnsLookupError::Unavailable`] — a boot never fails
/// over DNS, and the verification endpoint says so in a typed error instead.
pub struct SystemTxtResolver {
    resolver: Option<hickory_resolver::TokioResolver>,
}

impl SystemTxtResolver {
    pub fn new() -> Self {
        match hickory_resolver::TokioResolver::builder_tokio() {
            Ok(builder) => match builder.build() {
                Ok(resolver) => Self {
                    resolver: Some(resolver),
                },
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "system DNS resolver failed to build; custom-domain verification \
                         will report the resolver as unavailable"
                    );
                    Self { resolver: None }
                }
            },
            Err(error) => {
                tracing::warn!(
                    %error,
                    "no system DNS configuration found; custom-domain verification will \
                     report the resolver as unavailable"
                );
                Self { resolver: None }
            }
        }
    }
}

impl Default for SystemTxtResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TxtResolver for SystemTxtResolver {
    async fn lookup_txt(&self, name: &str) -> Result<Vec<String>, DnsLookupError> {
        let resolver = self.resolver.as_ref().ok_or_else(|| {
            DnsLookupError::Unavailable("no /etc/resolv.conf-equivalent configuration".into())
        })?;
        let lookup = resolver
            .txt_lookup(name)
            .await
            .map_err(|e| DnsLookupError::Failed(e.to_string()))?;
        let mut out = Vec::new();
        for record in lookup.answers() {
            if let hickory_resolver::proto::rr::RData::TXT(txt) = &record.data {
                let mut joined = String::new();
                for chunk in txt.txt_data.iter() {
                    // TXT character-strings are bytes; interpolate with the
                    // same lossy conversion the seam's matcher assumes.
                    joined.push_str(&String::from_utf8_lossy(chunk));
                }
                out.push(joined);
            }
        }
        Ok(out)
    }

    async fn resolves_to_address(&self, domain: &str) -> Result<bool, DnsLookupError> {
        let resolver = self.resolver.as_ref().ok_or_else(|| {
            DnsLookupError::Unavailable("no /etc/resolv.conf-equivalent configuration".into())
        })?;
        let lookup = resolver
            .lookup_ip(domain)
            .await
            .map_err(|e| DnsLookupError::Failed(e.to_string()))?;
        Ok(lookup.iter().next().is_some())
    }
}

static TXT_RESOLVER: OnceLock<Arc<dyn TxtResolver>> = OnceLock::new();

/// The process-wide resolver, built from the system configuration on first
/// use unless one was installed.
pub fn txt_resolver() -> Arc<dyn TxtResolver> {
    TXT_RESOLVER
        .get_or_init(|| Arc::new(SystemTxtResolver::new()))
        .clone()
}

/// Installs a resolver for the process (composition/testing hook). Returns
/// `false` when one is already installed — the first wins, so a stray later
/// call cannot silently redirect verification.
pub fn install_txt_resolver(resolver: Arc<dyn TxtResolver>) -> bool {
    TXT_RESOLVER.set(resolver).is_ok()
}

/// The raw evidence gathered for one verification attempt. Kept as data so
/// the API can surface exactly what was observed, not just a boolean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationReport {
    /// The `_marula-verify.<domain>` name that was queried.
    pub record_name: String,
    /// The authority: an observed TXT string equalled the issued token.
    pub matched: bool,
    /// TXT strings observed at that name (empty = no record).
    pub txt_observed: Vec<String>,
    /// Set when the TXT lookup itself failed (no resolver, transport error).
    pub txt_error: Option<String>,
    /// Whether the domain resolves to at least one address (plausibility).
    pub edge_resolves: bool,
    /// Set when the address lookup failed.
    pub edge_error: Option<String>,
}

/// Runs the TXT authority check and the address plausibility check against
/// `resolver`. The token comparison is the seam's single matcher
/// (`NoneDnsRecordProvider::verify_txt_records`), so verification is decided
/// in exactly one place. A lookup failure is captured IN the report (not
/// returned as an error) so the API can distinguish "not published yet" from
/// "could not ask", while a malformed domain name — which cannot even be
/// queried — is a hard error.
pub async fn verify_domain(
    resolver: &dyn TxtResolver,
    domain: &str,
    expected_token: &str,
) -> Result<VerificationReport, DnsLookupError> {
    let record_name =
        verification_record_name(domain).map_err(|e| DnsLookupError::Failed(e.to_string()))?;
    let (txt_observed, txt_error) = match resolver.lookup_txt(&record_name).await {
        Ok(values) => (values, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    let (edge_resolves, edge_error) = match resolver.resolves_to_address(domain).await {
        Ok(resolves) => (resolves, None),
        Err(error) => (false, Some(error.to_string())),
    };
    let matched = NoneDnsRecordProvider::verify_txt_records(&txt_observed, expected_token);
    Ok(VerificationReport {
        record_name,
        matched,
        txt_observed,
        txt_error,
        edge_resolves,
        edge_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A resolver with canned per-name answers, so verification is exercised
    /// without the network. `answers` maps queried names to TXT strings;
    /// `resolvable` names resolve to an address.
    struct FakeResolver {
        txt: HashMap<String, Vec<String>>,
        addressable: Vec<String>,
    }

    #[async_trait]
    impl TxtResolver for FakeResolver {
        async fn lookup_txt(&self, name: &str) -> Result<Vec<String>, DnsLookupError> {
            Ok(self.txt.get(name).cloned().unwrap_or_default())
        }
        async fn resolves_to_address(&self, domain: &str) -> Result<bool, DnsLookupError> {
            Ok(self.addressable.iter().any(|d| d == domain))
        }
    }

    #[tokio::test]
    async fn matching_token_with_proxied_edge_is_verified() {
        // The seam matcher accepts both a presentation-quoted and a raw TXT.
        let resolver = FakeResolver {
            txt: HashMap::from([(
                "_marula-verify.support.example.com".to_string(),
                vec!["\"marula-verify-tok\"".to_string()],
            )]),
            // A proxied edge resolves to the CDN's IPs, not the origin's;
            // plausibility only needs "resolves".
            addressable: vec!["support.example.com".to_string()],
        };
        let report = verify_domain(&resolver, "support.example.com", "marula-verify-tok")
            .await
            .expect("report");
        assert_eq!(report.record_name, "_marula-verify.support.example.com");
        assert!(report.matched);
        assert!(report.edge_resolves);
        assert!(report.txt_error.is_none() && report.edge_error.is_none());
    }

    #[tokio::test]
    async fn a_wrong_token_is_not_verified_even_when_the_edge_resolves() {
        let resolver = FakeResolver {
            txt: HashMap::from([(
                "_marula-verify.support.example.com".to_string(),
                vec!["marula-verify-other".to_string()],
            )]),
            addressable: vec!["support.example.com".to_string()],
        };
        let report = verify_domain(&resolver, "support.example.com", "marula-verify-tok")
            .await
            .expect("report");
        assert!(!report.matched);
        // The edge leg is independent and still reports plausibility.
        assert!(report.edge_resolves);
    }

    #[tokio::test]
    async fn an_absent_txt_record_is_not_an_error() {
        let resolver = FakeResolver {
            txt: HashMap::new(),
            addressable: Vec::new(),
        };
        let report = verify_domain(&resolver, "nope.example.com", "marula-verify-tok")
            .await
            .expect("report");
        assert!(report.txt_observed.is_empty());
        assert!(!report.matched);
        assert!(!report.edge_resolves);
        assert!(
            report.txt_error.is_none(),
            "an empty answer is a definitive 'no record', not a lookup failure"
        );
    }
}
