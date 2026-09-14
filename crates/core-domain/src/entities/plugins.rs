//! Plugin runtime v2 + marketplace registry entities (Wave 14 lane 5).

use crate::policy::{Action, Module, Permission};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Version of the plugin-runtime interface contract. Bump on any
/// breaking change to the manifest v2 shape or the capability
/// vocabulary; catalog entries declare the version they were published
/// against and the runtime refuses mismatches (the
/// `FEDERATION_BUNDLE_INTERFACE_VERSION` posture: versioned interface,
/// deny-by-default).
pub const PLUGIN_RUNTIME_INTERFACE_VERSION: u32 = 1;

/// The CLOSED capability vocabulary a v2 manifest may declare and an
/// install may be granted. Capabilities are declarative names that map
/// onto the EXISTING permission matrix — a grant never creates a new
/// permission, it resolves to a standard [`Permission`] and every check
/// flows through `core_domain::policy` like any human actor's. There is
/// deliberately NO admin capability: `special:manage_plugins`, user
/// management, billing, and audit-log specials are human-only grants a
/// plugin can never hold (deny-by-default — the vocabulary simply has
/// no key for them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum PluginCapability {
    TicketsRead,
    TicketsWrite,
    ClientsRead,
    ClientsWrite,
    TimeRead,
    TimeWrite,
    ProjectsRead,
    ProjectsWrite,
    KbRead,
    KbWrite,
    DocumentsRead,
    DocumentsWrite,
    ReportingRead,
    /// RB2 (ADR 0004): the financial-visibility capability maps onto the
    /// existing `financial:read` permission — the same gate cost rates,
    /// contract rates, and margin panels sit behind for humans.
    FinancialRead,
}

impl PluginCapability {
    pub const ALL: [PluginCapability; 14] = [
        PluginCapability::TicketsRead,
        PluginCapability::TicketsWrite,
        PluginCapability::ClientsRead,
        PluginCapability::ClientsWrite,
        PluginCapability::TimeRead,
        PluginCapability::TimeWrite,
        PluginCapability::ProjectsRead,
        PluginCapability::ProjectsWrite,
        PluginCapability::KbRead,
        PluginCapability::KbWrite,
        PluginCapability::DocumentsRead,
        PluginCapability::DocumentsWrite,
        PluginCapability::ReportingRead,
        PluginCapability::FinancialRead,
    ];

    /// The capability key as declared in a manifest / grant list.
    pub fn as_str(self) -> &'static str {
        match self {
            PluginCapability::TicketsRead => "tickets.read",
            PluginCapability::TicketsWrite => "tickets.write",
            PluginCapability::ClientsRead => "clients.read",
            PluginCapability::ClientsWrite => "clients.write",
            PluginCapability::TimeRead => "time.read",
            PluginCapability::TimeWrite => "time.write",
            PluginCapability::ProjectsRead => "projects.read",
            PluginCapability::ProjectsWrite => "projects.write",
            PluginCapability::KbRead => "kb.read",
            PluginCapability::KbWrite => "kb.write",
            PluginCapability::DocumentsRead => "documents.read",
            PluginCapability::DocumentsWrite => "documents.write",
            PluginCapability::ReportingRead => "reporting.read",
            PluginCapability::FinancialRead => "financial.read",
        }
    }

    /// Parses a capability key. Unknown keys are REJECTED — the
    /// vocabulary is closed, so a manifest naming a capability the core
    /// does not know cannot exist (deny-by-default at the data boundary).
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.as_str() == value)
    }

    /// The existing-matrix permission this capability resolves to. THE
    /// mapping — no parallel auth system: authorization for a plugin
    /// acting under a grant is `policy::check(granted_permissions, …)`
    /// with these resolved permissions in the granted set, exactly as
    /// for a human.
    pub fn permission(self) -> Permission {
        match self {
            PluginCapability::TicketsRead => Permission::Matrix {
                module: Module::Tickets,
                action: Action::View,
            },
            PluginCapability::TicketsWrite => Permission::Matrix {
                module: Module::Tickets,
                action: Action::Edit,
            },
            PluginCapability::ClientsRead => Permission::Matrix {
                module: Module::Clients,
                action: Action::View,
            },
            PluginCapability::ClientsWrite => Permission::Matrix {
                module: Module::Clients,
                action: Action::Edit,
            },
            PluginCapability::TimeRead => Permission::Matrix {
                module: Module::Time,
                action: Action::View,
            },
            PluginCapability::TimeWrite => Permission::Matrix {
                module: Module::Time,
                action: Action::Edit,
            },
            PluginCapability::ProjectsRead => Permission::Matrix {
                module: Module::Projects,
                action: Action::View,
            },
            PluginCapability::ProjectsWrite => Permission::Matrix {
                module: Module::Projects,
                action: Action::Edit,
            },
            PluginCapability::KbRead => Permission::Matrix {
                module: Module::Kb,
                action: Action::View,
            },
            PluginCapability::KbWrite => Permission::Matrix {
                module: Module::Kb,
                action: Action::Edit,
            },
            PluginCapability::DocumentsRead => Permission::Matrix {
                module: Module::Documents,
                action: Action::View,
            },
            PluginCapability::DocumentsWrite => Permission::Matrix {
                module: Module::Documents,
                action: Action::Edit,
            },
            PluginCapability::ReportingRead => Permission::Matrix {
                module: Module::Reporting,
                action: Action::View,
            },
            PluginCapability::FinancialRead => Permission::ViewFinancials,
        }
    }
}

/// The declarative v2 plugin manifest: name/version/capabilities — a
/// DATA document describing what a plugin IS and what it may ask for.
/// It is never executable content: the runtime executes nothing from a
/// manifest, and "install" only ever adopts a catalog entry's manifest
/// snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginManifestV2 {
    pub name: String,
    pub version: String,
    /// The interface version this manifest was written against; must
    /// equal [`PLUGIN_RUNTIME_INTERFACE_VERSION`] or the runtime refuses
    /// the entry.
    pub interface_version: u32,
    #[serde(default)]
    pub description: String,
    /// The capabilities the plugin MAY be granted. Grants at install
    /// time are a subset of this list — a grant outside the manifest's
    /// declared set is rejected.
    pub capabilities: Vec<String>,
}

impl PluginManifestV2 {
    /// Validates the manifest against the closed vocabulary and the
    /// pinned interface version. Unknown capability keys and interface
    /// mismatches are rejections, never coerced.
    pub fn validate(&self) -> Result<(), crate::error::DomainError> {
        if self.name.trim().is_empty() {
            return Err(crate::error::DomainError::Validation(
                "manifest name must not be empty".into(),
            ));
        }
        if self.version.trim().is_empty() {
            return Err(crate::error::DomainError::Validation(
                "manifest version must not be empty".into(),
            ));
        }
        if self.interface_version != PLUGIN_RUNTIME_INTERFACE_VERSION {
            return Err(crate::error::DomainError::Validation(format!(
                "manifest interface_version {} is not supported (runtime speaks {})",
                self.interface_version, PLUGIN_RUNTIME_INTERFACE_VERSION
            )));
        }
        for key in &self.capabilities {
            if PluginCapability::parse(key).is_none() {
                return Err(crate::error::DomainError::Validation(format!(
                    "unknown capability {key:?} — the capability vocabulary is closed"
                )));
            }
        }
        Ok(())
    }

    /// The declared capabilities parsed into the typed vocabulary.
    /// Caller validates first.
    pub fn declared(&self) -> Vec<PluginCapability> {
        self.capabilities
            .iter()
            .filter_map(|k| PluginCapability::parse(k))
            .collect()
    }
}

/// One marketplace registry catalog entry (global, server-wide).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginCatalogEntry {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub publisher: String,
    pub latest_version: Option<String>,
    pub interface_version: u32,
    /// Registry-level revocation: NULL = live; set = revoked (denies
    /// new installs and marks existing installs revoked).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = "date-time"))]
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoked_reason: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub created_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub updated_at: DateTime<Utc>,
}

/// One pinned version of a catalog entry with its manifest snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginVersion {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub catalog_id: Uuid,
    pub version: String,
    pub manifest: PluginManifestV2,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub created_at: DateTime<Utc>,
}

/// Install lifecycle. Deny-by-default: `Disabled` until an explicit
/// admin enable; `Revoked` is terminal (registry revocation or explicit
/// revoke) — a revoked install never runs again until re-installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PluginInstallStatus {
    Disabled,
    Enabled,
    Revoked,
}

impl PluginInstallStatus {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "disabled" => Some(Self::Disabled),
            "enabled" => Some(Self::Enabled),
            "revoked" => Some(Self::Revoked),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Enabled => "enabled",
            Self::Revoked => "revoked",
        }
    }
}

/// An org's adoption of one catalog entry at one pinned version.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginInstall {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub catalog_id: Uuid,
    /// Denormalized for display (catalog slug at install time).
    pub slug: String,
    /// The pinned adopted version.
    pub version: String,
    pub status: PluginInstallStatus,
    /// Granted capability keys (subset of the pinned manifest's
    /// declared set).
    pub granted_capabilities: Vec<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub installed_by: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub installed_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub updated_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = "date-time"))]
    pub revoked_at: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub revoked_by: Option<Uuid>,
}

impl PluginInstall {
    /// The granted capabilities resolved onto the existing permission
    /// matrix. Unknown keys are dropped here (they are rejected at the
    /// write boundary; this is the defensive read path).
    pub fn granted_permissions(&self) -> Vec<Permission> {
        self.granted_capabilities
            .iter()
            .filter_map(|k| PluginCapability::parse(k))
            .map(PluginCapability::permission)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(capabilities: &[&str]) -> PluginManifestV2 {
        PluginManifestV2 {
            name: "Ticket Summarizer".into(),
            version: "1.0.0".into(),
            interface_version: PLUGIN_RUNTIME_INTERFACE_VERSION,
            description: "Summarizes tickets".into(),
            capabilities: capabilities.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn manifest_should_accept_known_capabilities_at_pinned_interface_version() {
        let m = manifest(&["tickets.read", "financial.read"]);
        assert!(m.validate().is_ok());
        assert_eq!(m.declared().len(), 2);
    }

    #[test]
    fn manifest_should_reject_unknown_capability_keys() {
        let mut m = manifest(&["tickets.read"]);
        m.capabilities.push("admin.users".into());
        let err = m.validate().expect_err("unknown capability must reject");
        assert!(err.to_string().contains("closed"));
    }

    #[test]
    fn manifest_should_reject_interface_version_mismatch() {
        let mut m = manifest(&[]);
        m.interface_version = PLUGIN_RUNTIME_INTERFACE_VERSION + 1;
        assert!(m.validate().is_err());
    }

    #[test]
    fn capabilities_should_map_onto_the_existing_permission_matrix() {
        assert_eq!(
            PluginCapability::FinancialRead.permission(),
            Permission::ViewFinancials
        );
        assert_eq!(
            PluginCapability::TicketsWrite.permission(),
            Permission::Matrix {
                module: Module::Tickets,
                action: Action::Edit
            }
        );
    }

    #[test]
    fn capability_vocabulary_should_be_closed_and_round_trip() {
        for c in PluginCapability::ALL {
            let parsed = PluginCapability::parse(c.as_str()).expect("round trip");
            assert_eq!(parsed, c);
        }
        assert_eq!(PluginCapability::parse("special:manage_plugins"), None);
        assert_eq!(PluginCapability::parse("admin.plugins"), None);
    }
}
