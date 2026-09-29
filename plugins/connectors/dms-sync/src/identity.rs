//! Deterministic identity keys (ADR 0003): "Adapters declare stable
//! identity keys per entity type (source GUID, serial/service tag, MAC
//! set, hostname+domain)". Low-cost deterministic rules resolve the
//! common case; ambiguous matches become merge candidates with visible
//! evidence — never a silent guess.
//!
//! The key ORDER is fixed: the source GUID is the primary key (it is
//! the source's own identity claim), serial second, hostname+domain
//! third. `derive` collects every key a record carries; the engine
//! tries them in order against its ledger and the sink's records, and
//! reports anything ambiguous rather than picking a winner.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::model::SyncRecord;

/// The deterministic key kinds this connector declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum IdentityKeyKind {
    /// The source system's own GUID — primary identity.
    Guid,
    /// Serial number / service tag.
    Serial,
    /// hostname + dns domain (lowercased, joined by '.').
    HostnameDomain,
}

impl IdentityKeyKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            IdentityKeyKind::Guid => "guid",
            IdentityKeyKind::Serial => "serial",
            IdentityKeyKind::HostnameDomain => "hostname_domain",
        }
    }
}

/// One deterministic identity key: `(kind, normalized value)`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct IdentityKey {
    pub kind: IdentityKeyKind,
    /// Normalized (trimmed; hostname+domain lowercased) value.
    pub value: String,
}

/// The identity of one external record: its primary key plus any
/// alternates. The FIRST key is the one used for provenance
/// (`source_ref`), the rest are deterministic fallbacks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Identity {
    pub primary: IdentityKey,
    pub alternates: Vec<IdentityKey>,
}

impl Identity {
    /// All keys, primary first.
    pub fn keys(&self) -> impl Iterator<Item = &IdentityKey> {
        std::iter::once(&self.primary).chain(self.alternates.iter())
    }

    /// Stable connector-side handle for ledgers and idempotency keys:
    /// `<kind>:<value>` of the primary key.
    pub fn handle(&self) -> String {
        format!("{}:{}", self.primary.kind.as_str(), self.primary.value)
    }

    /// sha256 content fingerprint over a record's semantic fields —
    /// what the engine treats as "the source's current content". Two
    /// snapshots of the same record fingerprint equal when nothing
    /// changed; a changed fingerprint means the source moved.
    pub fn fingerprint(record: &SyncRecord) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"marula-dms-sync-record-v1");
        hasher.update(record.external_id.as_bytes());
        hasher.update(b"\x00");
        hasher.update(record.title.as_bytes());
        hasher.update(b"\x00");
        hasher.update(record.body.as_bytes());
        hasher.update(b"\x00");
        for (key, value) in &record.fields {
            hasher.update(key.as_bytes());
            hasher.update(b"\x01");
            hasher.update(value.as_bytes());
            hasher.update(b"\x02");
        }
        hex::encode(hasher.finalize())
    }
}

/// Serial-number field names seen across the vendor DMSes; the first
/// present, non-empty value wins.
const SERIAL_FIELDS: &[&str] = &["serial", "serial_number", "service_tag"];

/// Hostname field names + the domain field to pair them with.
const HOSTNAME_FIELDS: &[&str] = &["hostname", "host_name", "name_fqdn"];
const DOMAIN_FIELDS: &[&str] = &["domain", "dns_domain", "organization_domain"];

/// Derive a record's identity. Always yields the source GUID key;
/// serial and hostname+domain are appended when the record carries
/// them (field names cover IT Glue configurations, Hudu assets, and
/// the generic mapping vocabulary).
pub fn derive(
    source_name: &str,
    external_id: &str,
    fields: &std::collections::BTreeMap<String, String>,
) -> Identity {
    let mut alternates = Vec::new();

    for key in SERIAL_FIELDS {
        if let Some(value) = fields.get(*key).map(|v| v.trim()).filter(|v| !v.is_empty()) {
            alternates.push(IdentityKey {
                kind: IdentityKeyKind::Serial,
                value: value.to_string(),
            });
            break;
        }
    }
    for host_key in HOSTNAME_FIELDS {
        let Some(hostname) = fields
            .get(*host_key)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        else {
            continue;
        };
        // A hostname that already looks like an FQDN IS the key; a bare
        // host joins with the record's domain field when present.
        let mut fqdn = hostname.to_lowercase();
        if !hostname.contains('.') {
            for domain_key in DOMAIN_FIELDS {
                if let Some(domain) = fields
                    .get(*domain_key)
                    .map(|v| v.trim())
                    .filter(|v| !v.is_empty())
                {
                    fqdn = format!("{fqdn}.{}", domain.to_lowercase());
                    break;
                }
            }
        }
        alternates.push(IdentityKey {
            kind: IdentityKeyKind::HostnameDomain,
            value: fqdn,
        });
        break;
    }

    let primary = IdentityKey {
        kind: IdentityKeyKind::Guid,
        value: format!("{source_name}:{external_id}"),
    };
    Identity {
        primary,
        alternates,
    }
}
