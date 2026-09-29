//! Deterministic identity keys (ADR 0003 CI3): "Adapters declare stable
//! identity keys per entity type (source GUID, serial/service tag, MAC
//! set, hostname+domain)".
//!
//! The normalization rules here mirror the core's published CI3 rules
//! byte-for-byte (this crate speaks only the public contract, so the
//! rules are re-stated, not imported):
//!
//! * serial / service tag — uppercase, every non-alphanumeric stripped
//!   (printed serials pick up dashes/spaces that vary between feeds);
//! * MACs — lower colon form `aa:bb:cc:dd:ee:ff`; input that does not
//!   carry 12 hex digits is NOT a MAC and is dropped rather than
//!   guessed;
//! * hostname / domain — lowercase; hostnames lose a trailing dot,
//!   domains lose leading/trailing dots.
//!
//! Because normalization happens connector-side AND the core re-applies
//! the same rules on arrival, the identity keys are deterministic under
//! every vendor formatting: `SN-8F/3K2`, `sn8f3k2` and ` sn 8f 3k2 ` all
//! resolve to `SN8F3K2Q`.

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::model::FeederRecord;

/// Uppercases and strips all non-alphanumerics.
pub fn normalize_serial(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_uppercase())
        .collect()
}

/// Normalizes a MAC to lower colon form; `None` for non-MAC input.
pub fn normalize_mac(raw: &str) -> Option<String> {
    let hex: String = raw
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if hex.len() != 12 {
        return None;
    }
    Some(
        hex.as_bytes()
            .chunks(2)
            .map(|c| std::str::from_utf8(c).unwrap_or("").to_string())
            .collect::<Vec<_>>()
            .join(":"),
    )
}

/// Lowercases, trims, strips a trailing dot from a hostname.
pub fn normalize_hostname(raw: &str) -> String {
    raw.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// Lowercases/trims a domain.
pub fn normalize_domain(raw: &str) -> String {
    raw.trim().trim_matches('.').to_ascii_lowercase()
}

/// Builds the `identity_keys` JSON object from raw vendor values.
/// Deterministic: the same logical device yields byte-identical keys
/// regardless of payload formatting, and MAC sets are sorted so key
/// order can never influence resolution.
#[derive(Debug, Clone, Default)]
pub struct IdentityKeysBuilder {
    source_guid: Option<String>,
    serial: Option<String>,
    service_tag: Option<String>,
    macs: Vec<String>,
    hostname: Option<String>,
    domain: Option<String>,
}

impl IdentityKeysBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The source system's own GUID (primary identity signal).
    pub fn source_guid(mut self, raw: &str) -> Self {
        self.source_guid = Some(raw.trim().to_string());
        self
    }

    pub fn serial(mut self, raw: &str) -> Self {
        self.serial = Some(normalize_serial(raw));
        self
    }

    pub fn service_tag(mut self, raw: &str) -> Self {
        self.service_tag = Some(normalize_serial(raw));
        self
    }

    /// Adds every parseable MAC; duplicates collapse.
    pub fn macs<'a>(mut self, raws: impl IntoIterator<Item = &'a str>) -> Self {
        for raw in raws {
            if let Some(mac) = normalize_mac(raw) {
                if !self.macs.contains(&mac) {
                    self.macs.push(mac);
                }
            }
        }
        self.macs.sort();
        self
    }

    pub fn hostname(mut self, raw: &str) -> Self {
        self.hostname = Some(normalize_hostname(raw));
        self
    }

    pub fn domain(mut self, raw: &str) -> Self {
        self.domain = Some(normalize_domain(raw));
        self
    }

    /// Renders the normalized key set; absent signals are omitted so the
    /// core sees exactly the evidence the record actually carries.
    pub fn build(self) -> Value {
        let mut out = Map::new();
        if let Some(guid) = self.source_guid.filter(|g| !g.is_empty()) {
            out.insert("source_guid".into(), Value::String(guid));
        }
        if let Some(serial) = self.serial.filter(|s| !s.is_empty()) {
            out.insert("serial".into(), Value::String(serial));
        }
        if let Some(tag) = self.service_tag.filter(|t| !t.is_empty()) {
            out.insert("service_tag".into(), Value::String(tag));
        }
        if !self.macs.is_empty() {
            out.insert(
                "macs".into(),
                Value::Array(self.macs.iter().cloned().map(Value::String).collect()),
            );
        }
        if let Some(hostname) = self.hostname.filter(|h| !h.is_empty()) {
            out.insert("hostname".into(), Value::String(hostname));
        }
        if let Some(domain) = self.domain.filter(|d| !d.is_empty()) {
            out.insert("domain".into(), Value::String(domain));
        }
        Value::Object(out)
    }
}

/// sha256 content fingerprint over a record's semantic fields — what the
/// engine treats as "the source's current content". Two snapshots of the
/// same record fingerprint equal when nothing changed; a changed
/// fingerprint means the source moved and the sync must land the update.
pub fn fingerprint(record: &FeederRecord) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"marula-ci-feeders-record-v1");
    hasher.update(record.source_system.as_bytes());
    hasher.update(b"\x00");
    hasher.update(record.external_id.as_bytes());
    hasher.update(b"\x00");
    hasher.update(record.tag.as_bytes());
    hasher.update(b"\x00");
    hasher.update(record.name.as_bytes());
    hasher.update(b"\x00");
    hasher.update(record.identity_keys.to_string().as_bytes());
    hasher.update(b"\x00");
    hasher.update(record.observed_fields.to_string().as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod determinism {
    use super::*;

    #[test]
    fn serial_normalization_strips_separators_case_insensitively() {
        assert_eq!(normalize_serial("sn-8f/3k2-q"), "SN8F3K2Q");
        assert_eq!(normalize_serial(" SN 8f 3k2 q "), "SN8F3K2Q");
        assert_eq!(normalize_serial("SN8F3K2Q"), "SN8F3K2Q");
    }

    #[test]
    fn mac_normalization_accepts_every_common_vendor_format() {
        assert_eq!(
            normalize_mac("AA-BB-CC-DD-EE-FF"),
            Some("aa:bb:cc:dd:ee:ff".into())
        );
        assert_eq!(
            normalize_mac("aabb.ccdd.eeff"),
            Some("aa:bb:cc:dd:ee:ff".into())
        );
        assert_eq!(
            normalize_mac("AABBCCDDEEFF"),
            Some("aa:bb:cc:dd:ee:ff".into())
        );
        assert_eq!(normalize_mac("not-a-mac"), None);
        assert_eq!(normalize_mac("aa:bb:cc:dd:ee"), None);
    }

    #[test]
    fn hostname_and_domain_normalization_is_case_and_dot_insensitive() {
        assert_eq!(
            normalize_hostname("ACME-MAIL-01.acme.local."),
            "acme-mail-01.acme.local"
        );
        assert_eq!(normalize_domain(".Acme.Local."), "acme.local");
    }
}
