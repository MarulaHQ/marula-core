//! Vendor-org → Meridian-client mapping: the operator's explicit,
//! deterministic decision. Records pulled from a vendor organization
//! the operator has NOT mapped are an error — the connector never
//! guesses a client attribution, and a sync with unmapped orgs
//! surfaces them in its report instead of silently dropping or
//! misattributing records.

use std::collections::BTreeMap;

use uuid::Uuid;

/// The operator's mapping from vendor organization ids (as the vendor
/// API carries them, stringified) to Meridian client ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrgMapping {
    entries: BTreeMap<String, Uuid>,
}

impl OrgMapping {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn client_for(&self, vendor_org_id: &str) -> Result<Uuid, OrgMappingError> {
        self.entries
            .get(vendor_org_id)
            .copied()
            .ok_or_else(|| OrgMappingError::UnmappedVendorOrg {
                vendor_org_id: vendor_org_id.to_string(),
            })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Builder for [`OrgMapping`]; duplicate vendor orgs overwrite — the
/// operator's LAST declaration wins (deterministic on re-declare).
#[derive(Debug, Clone, Default)]
pub struct OrgMappingBuilder {
    entries: BTreeMap<String, Uuid>,
}

impl OrgMappingBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn map(mut self, vendor_org_id: impl Into<String>, client_id: Uuid) -> Self {
        self.entries.insert(vendor_org_id.into(), client_id);
        self
    }

    pub fn build(self) -> OrgMapping {
        OrgMapping {
            entries: self.entries,
        }
    }
}

/// Why an org→client resolution failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OrgMappingError {
    /// The vendor org is not in the operator's mapping. The sync must
    /// surface this — never guess a client.
    #[error("vendor organization `{vendor_org_id}` is not mapped to a Meridian client")]
    UnmappedVendorOrg { vendor_org_id: String },
}
