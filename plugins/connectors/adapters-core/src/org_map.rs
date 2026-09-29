//! Vendor-org → Marula-client mapping: the operator's explicit,
//! deterministic decision. Records pulled from a vendor organization
//! the operator has NOT mapped are an error — the connector never
//! guesses a client attribution, and a sync with unmapped orgs
//! surfaces them in its report instead of silently dropping or
//! misattributing records.

use std::collections::BTreeMap;

use uuid::Uuid;

/// The operator's mapping from vendor organization ids (as the vendor
/// API carries them, stringified) to Marula client ids.
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
    #[error("vendor organization `{vendor_org_id}` is not mapped to a Marula client")]
    UnmappedVendorOrg { vendor_org_id: String },
}

// ---- org_map reconciliation (ADR 0003 extension, Wave 22 lane 2) ------------

/// The reconcile hint a connector reports when a pull hits an unmapped
/// vendor org: the sync still resolves NOTHING (the never-guess rule is
/// unchanged) — instead of only a report entry, the connector can push
/// this to the public `POST /api/v1/clients/org-reconcile` seam so the
/// core's deterministic label proposals queue reviewable candidates on
/// the merge workbench. The accept decision stays the operator's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("vendor organization `{vendor_org_id}` is unmapped; reconciliation proposed")]
pub struct VendorOrgReconcile {
    pub vendor_org_id: String,
    /// The vendor's display label for the org, when the pull carried one
    /// (it feeds the deterministic proposal; None means no proposal).
    pub vendor_label: Option<String>,
}

impl VendorOrgReconcile {
    /// The exact request body the public org-reconcile seam expects —
    /// public-API doctrine: no internals, typed envelope from the caller.
    pub fn request_body(&self, source_system: &str, idempotency_key: &str) -> serde_json::Value {
        serde_json::json!({
            "source_system": source_system,
            "vendor_org_id": self.vendor_org_id,
            "vendor_label": self.vendor_label,
            "idempotency_key": idempotency_key,
        })
    }
}

impl OrgMapping {
    /// Resolves the mapped client OR produces the reconcile hint for an
    /// unmapped org — the ONE deterministic alternative to the plain
    /// error path. Same never-guess posture: nothing is attributed; the
    /// hint is only a proposal the operator reviews.
    pub fn resolve_or_reconcile(
        &self,
        vendor_org_id: &str,
        vendor_label: Option<&str>,
    ) -> Result<Uuid, VendorOrgReconcile> {
        self.client_for(vendor_org_id)
            .map_err(|_| VendorOrgReconcile {
                vendor_org_id: vendor_org_id.to_string(),
                vendor_label: vendor_label
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_string),
            })
    }
}

#[cfg(test)]
mod reconcile_tests {
    use super::*;

    #[test]
    fn resolve_or_reconcile_folds_label_into_the_hint() {
        let client = Uuid::from_u128(7);
        let map = OrgMappingBuilder::new().map("org-1", client).build();
        assert_eq!(map.resolve_or_reconcile("org-1", Some("Acme")), Ok(client));

        let hint = map
            .resolve_or_reconcile("org-2", Some("  Acme Corp  "))
            .expect_err("unmapped org produces a hint, never a guess");
        assert_eq!(hint.vendor_org_id, "org-2");
        assert_eq!(hint.vendor_label.as_deref(), Some("Acme Corp"));
        let body = hint.request_body("it_glue", "orgrecon-org-2");
        assert_eq!(body["source_system"], "it_glue");
        assert_eq!(body["vendor_org_id"], "org-2");
        assert_eq!(body["vendor_label"], "Acme Corp");
        assert_eq!(body["idempotency_key"], "orgrecon-org-2");

        // A blank label stays None: no deterministic proposal exists.
        let blank = map
            .resolve_or_reconcile("org-3", Some("   "))
            .expect_err("hint");
        assert_eq!(blank.vendor_label, None);
    }
}
