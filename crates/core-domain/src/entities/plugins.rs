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
    /// ADR 0006 leg (d) (Wave 19 lane 2): the CIPHERTEXT-ONLY vault-ingress
    /// scope. A plugin holding this capability may submit ENCRYPTED
    /// payloads (client-side E2E, key in URL fragment, wrapped to the org
    /// key) into a scoped vault folder — nothing else. The core never
    /// accepts PLAINTEXT secrets from plugin surfaces, so the capability
    /// resolves onto the same `secrets:create` matrix gate a human writer
    /// passes and every actual ingress payload is sealed bytes on arrival.
    VaultIngress,
}

impl PluginCapability {
    pub const ALL: [PluginCapability; 15] = [
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
        PluginCapability::VaultIngress,
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
            // ADR 0006 names this scope `vault:ingress` — the wire key
            // carries the ADR's literal spelling (the colon token is part
            // of the closed vocabulary; uniqueness and round-tripping are
            // unaffected by the separator choice).
            PluginCapability::VaultIngress => "vault:ingress",
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
            // Ciphertext-only ingress is still a WRITE onto the secrets
            // module: the capability resolves onto the same matrix
            // create-gate a human secret writer passes. A grant never
            // invents authority — ingress plugins are authors of sealed
            // records, nothing more.
            PluginCapability::VaultIngress => Permission::Matrix {
                module: Module::Secrets,
                action: Action::Create,
            },
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
    /// ADR 0009 §4 — the optional JSON-Schema the manifest declares for
    /// its per-install settings. The Plugins surface renders config FROM
    /// this schema (typed-registry doctrine: no hand-authored
    /// per-plugin pages); a manifest with no schema gets
    /// consent/enable/disable only. Additive to the v2 shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings_schema: Option<serde_json::Value>,
    /// ADR 0009 §6 — the artifact GOVERNANCE profile the manifest
    /// declares: WASM executables and HTTP connectors ship in one
    /// lifecycle track but distinct tabs. Absent ⇒ `wasm` (the
    /// declarative manifest/permission baseline every v1 manifest was
    /// published as); connectors must DECLARE the kind. Additive to the
    /// v2 shape; unknown keys reject at deserialization (closed
    /// vocabulary).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_kind: Option<PluginRuntimeKind>,
    /// ADR 0009 §6 full semantics (Wave 19 lane 2) — the connector
    /// governance profile: webhook endpoints + outbound scopes as MODELED
    /// manifest fields, never free text. Valid and OPTIONAL ONLY on
    /// `http_connector` manifests (a connector may legitimately declare no
    /// webhooks — the default posture an absent profile already encodes);
    /// forbidden on `wasm` manifests. Declared endpoints are the
    /// connector's egress-allowlist seed (the same posture the WASM
    /// resource profile carries); declared scopes render on the consent
    /// screen verbatim so an admin consents to exactly the SaaS scopes the
    /// connector asks for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_profile: Option<ConnectorProfile>,
}

/// The plugin-side artifact vocabulary for the roster tabs (WASM vs HTTP
/// connector). Skills are NOT in this closed set — the skill half lives
/// on `PluginArtifactKind::Skill` in the distribution seam (0079). Parse
/// is strict: a manifest naming an ungovernable kind cannot exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimeKind {
    Wasm,
    HttpConnector,
}

impl PluginRuntimeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Wasm => "wasm",
            Self::HttpConnector => "http_connector",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "wasm" => Some(Self::Wasm),
            "http_connector" => Some(Self::HttpConnector),
            _ => None,
        }
    }
}

/// ADR 0009 §6 (Wave 19 lane 2): the modeled connector governance
/// profile — WHERE the connector speaks (webhook endpoints) and what it
/// wears outward (SaaS scopes). Structured, validated DECLARATIONS, not
/// prose: the consent screen renders them verbatim and connectors inherit
/// the platform's egress posture from the declared endpoint list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ConnectorProfile {
    /// The webhook URLs traffic from the platform sends toward. https-only
    /// absolute URLs, host required, no credentials in the URL, max 8.
    #[serde(default)]
    pub webhook_endpoints: Vec<String>,
    /// The outbound scopes the connector asks the upstream SaaS for
    /// (e.g. `graph:mail.read`); opaque strings to the core — they exist
    /// for consent-time visibility and install-record provenance, max 32.
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// Capacities of the modeled connector profile (deny oversized
/// declarations at the data boundary).
pub const MAX_CONNECTOR_ENDPOINTS: usize = 8;
pub const MAX_CONNECTOR_SCOPES: usize = 32;
const MAX_ENDPOINT_URL_BYTES: usize = 2048;
const MAX_SCOPE_BYTES: usize = 64;

impl ConnectorProfile {
    /// Validates the closed profile: https-only endpoints (host present,
    /// no userinfo, no path query beyond normal URL bounds), scope charset
    /// `[A-Za-z0-9_.:-]+`, everything within the declared caps. Unknown
    /// junk cannot serialize past this (deny-by-default at the boundary).
    pub fn validate(&self) -> Result<(), crate::error::DomainError> {
        if self.webhook_endpoints.len() > MAX_CONNECTOR_ENDPOINTS {
            return Err(crate::error::DomainError::Validation(format!(
                "connector profile declares {} webhook endpoints (max {MAX_CONNECTOR_ENDPOINTS})",
                self.webhook_endpoints.len()
            )));
        }
        for url in &self.webhook_endpoints {
            Self::validate_endpoint(url)?;
        }
        if self.scopes.len() > MAX_CONNECTOR_SCOPES {
            return Err(crate::error::DomainError::Validation(format!(
                "connector profile declares {} scopes (max {MAX_CONNECTOR_SCOPES})",
                self.scopes.len()
            )));
        }
        for scope in &self.scopes {
            if scope.is_empty()
                || scope.len() > MAX_SCOPE_BYTES
                || !scope
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
            {
                return Err(crate::error::DomainError::Validation(format!(
                    "connector scope {scope:?} violates the scope charset/size rules"
                )));
            }
        }
        Ok(())
    }

    fn validate_endpoint(url: &str) -> Result<(), crate::error::DomainError> {
        if url.len() > MAX_ENDPOINT_URL_BYTES {
            return Err(crate::error::DomainError::Validation(
                "connector webhook endpoint exceeds the 2048-byte URL cap".into(),
            ));
        }
        let parsed = url::Url::parse(url).map_err(|_| {
            crate::error::DomainError::Validation(format!(
                "connector webhook endpoint {url:?} is not an absolute URL"
            ))
        })?;
        if parsed.scheme() != "https" {
            return Err(crate::error::DomainError::Validation(format!(
                "connector webhook endpoint {url:?} must be https"
            )));
        }
        if !parsed.username().is_empty() {
            return Err(crate::error::DomainError::Validation(
                "connector webhook endpoints must not carry credentials in the URL".into(),
            ));
        }
        if parsed.host_str().is_none() {
            return Err(crate::error::DomainError::Validation(format!(
                "connector webhook endpoint {url:?} is missing a host"
            )));
        }
        Ok(())
    }
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
        if let Some(schema) = &self.settings_schema {
            SettingsSchema::new(schema.clone())?;
        }
        // ADR 0009 §6 full semantics: the connector profile is
        // kind-coherent — modeled/validated for http_connector manifests
        // (optional: a connector that declares no webhooks keeps the
        // absent-profile default posture), forbidden on wasm manifests (a
        // WASM executable has no webhook endpoints to govern — declaring
        // the field refuses rather than being ignored).
        match self.runtime_kind() {
            PluginRuntimeKind::HttpConnector => {
                if let Some(profile) = &self.connector_profile {
                    profile.validate()?;
                }
            }
            PluginRuntimeKind::Wasm => {
                if self.connector_profile.is_some() {
                    return Err(crate::error::DomainError::Validation(
                        "wasm manifests must not declare a connector_profile".into(),
                    ));
                }
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

    /// The governance profile this manifest declares (the roster-tab
    /// discriminator). Absent ⇒ `Wasm` — the v1 manifest/permission
    /// baseline every shipped entry was published as; only connectors
    /// need to declare.
    pub fn runtime_kind(&self) -> PluginRuntimeKind {
        self.artifact_kind.unwrap_or(PluginRuntimeKind::Wasm)
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

// --- ADR 0009 distribution: sources, staged imports, provenance, settings ----

/// The closed source vocabulary — every distribution adapter records the
/// same provenance chain, typed by the kind of source it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PluginSourceKind {
    /// The public marketplace registry: static JSON catalog + git tag,
    /// code-signed/digest-pinned artifacts.
    MarketplaceRegistry,
    /// A GitHub repo pull at a PINNED tag/release + commit sha — never
    /// HEAD; unpinned pulls are refused.
    GithubRepo,
    /// A local zip import through sandboxed validation.
    LocalZip,
    /// A free/open community skill marketplace or repo API (skills).
    SkillCommunity,
    /// A manual member upload (skills or, where allowed, zips).
    ManualUpload,
}

impl PluginSourceKind {
    pub const ALL: [PluginSourceKind; 5] = [
        PluginSourceKind::MarketplaceRegistry,
        PluginSourceKind::GithubRepo,
        PluginSourceKind::LocalZip,
        PluginSourceKind::SkillCommunity,
        PluginSourceKind::ManualUpload,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            PluginSourceKind::MarketplaceRegistry => "marketplace_registry",
            PluginSourceKind::GithubRepo => "github_repo",
            PluginSourceKind::LocalZip => "local_zip",
            PluginSourceKind::SkillCommunity => "skill_community",
            PluginSourceKind::ManualUpload => "manual_upload",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.as_str() == value)
    }
}

/// What kind of artifact a staged row carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PluginArtifactKind {
    Plugin,
    Skill,
    /// Wave 24 theming system: a declarative theme pack — token overrides
    /// validated against the theme-token schema, zero code, zero
    /// capabilities (restyle-only by construction).
    Theme,
}

impl PluginArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PluginArtifactKind::Plugin => "plugin",
            PluginArtifactKind::Skill => "skill",
            PluginArtifactKind::Theme => "theme",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "plugin" => Some(Self::Plugin),
            "skill" => Some(Self::Skill),
            "theme" => Some(Self::Theme),
            _ => None,
        }
    }
}

/// The staged state machine: `pending` → `approved` | `refused`
/// (approval mints the adoption; both terminal states are auditable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PluginStagedStatus {
    Pending,
    Approved,
    Refused,
}

impl PluginStagedStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            PluginStagedStatus::Pending => "pending",
            PluginStagedStatus::Approved => "approved",
            PluginStagedStatus::Refused => "refused",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "approved" => Some(Self::Approved),
            "refused" => Some(Self::Refused),
            _ => None,
        }
    }
}

/// A staged member import (ADR 0009 §3): the row BEFORE install.
/// Validated on arrival, never executed, visible only to the uploader
/// and admins; the owner/admin consent diff on approval is the path
/// that mints the catalog adoption (plugins) or the skill row (skills).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginStagedItem {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: Uuid,
    pub artifact_kind: PluginArtifactKind,
    pub source_kind: PluginSourceKind,
    pub source_url: String,
    /// Registry git tag / GitHub `tag@sha`; NULL for local uploads.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub fetched_ref: Option<String>,
    /// sha256 of the verified artifact (hex).
    pub artifact_digest: String,
    /// The full declarative manifest (plugin v2 shape incl.
    /// settings_schema, or the skill manifest).
    pub manifest: serde_json::Value,
    pub declared_capabilities: Vec<String>,
    pub status: PluginStagedStatus,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub uploaded_by: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub uploaded_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub review_note: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub reviewed_by: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = "date-time"))]
    pub reviewed_at: Option<DateTime<Utc>>,
    /// The adoption (or skill) minted on approval, for trace.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub adopted_install_id: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub adopted_skill_id: Option<Uuid>,
    /// Wave 24: the theme-pack row minted on approval of a `theme`
    /// staged item.
    pub adopted_theme_pack_id: Option<Uuid>,
}

/// One link of the append-only provenance chain
/// `{source_kind, source_url, fetched_ref, artifact_digest, updater}`
/// shown on the install record. Appended by the route layer on every
/// lifecycle touch; uninstall never deletes it (ADR 0009 §5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginProvenanceEntry {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub install_id: Uuid,
    pub source_kind: PluginSourceKind,
    pub source_url: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub fetched_ref: Option<String>,
    pub artifact_digest: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub uploader: Option<Uuid>,
    /// `installed` | `updated` | `re_consented` | `enabled` | `disabled`.
    pub action: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<serde_json::Value>))]
    pub detail: Option<serde_json::Value>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub recorded_at: DateTime<Utc>,
}

impl PluginProvenanceEntry {
    fn valid_action(action: &str) -> bool {
        matches!(
            action,
            "staged" | "installed" | "updated" | "re_consented" | "enabled" | "disabled"
        )
    }

    pub fn validate(&self) -> Result<(), crate::error::DomainError> {
        if !Self::valid_action(&self.action) {
            return Err(crate::error::DomainError::Validation(format!(
                "unknown provenance action {:?}",
                self.action
            )));
        }
        if !self.artifact_digest.is_empty() && !is_sha256_hex(&self.artifact_digest) {
            return Err(crate::error::DomainError::Validation(
                "artifact_digest must be a sha256 hex digest".into(),
            ));
        }
        Ok(())
    }
}

/// True iff `value` is a 64-char lowercase sha256 hex string.
pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// The declared SETTINGS schema a manifest may carry (ADR 0009 §4):
/// the typed-registry doctrine per install — the Plugins surface
/// renders config FROM this schema at the UI close-out; this type owns
/// the write-boundary VALIDATION of a config value against it, so every
/// stored config is schema-checked with the same deterministic rules.
#[derive(Debug, Clone)]
pub struct SettingsSchema {
    pub raw: serde_json::Value,
}

impl SettingsSchema {
    /// Builds a schema handle, validating the declaration itself is a
    /// supported subset: a top-level object whose `properties` map holds
    /// `{"type": "string"|"number"|"integer"|"boolean"}` leaf nodes
    /// (plus optional `enum`/`minimum`/`maximum`/`default` and string
    /// `minLength`/`maxLength`). Unsupported constructs REJECT the
    /// manifest rather than guessing.
    pub fn new(raw: serde_json::Value) -> Result<Self, crate::error::DomainError> {
        let err = |m: String| Err(crate::error::DomainError::Validation(m));
        let Some(obj) = raw.as_object() else {
            return err("settings_schema must be a JSON object".into());
        };
        if obj.get("type").and_then(|t| t.as_str()) != Some("object") {
            return err("settings_schema must declare \"type\": \"object\"".into());
        }
        let props = match obj.get("properties") {
            None | Some(serde_json::Value::Null) => serde_json::Map::new(),
            Some(p) => match p.as_object() {
                Some(p) => p.clone(),
                None => {
                    return err("settings_schema.properties must be an object".into());
                }
            },
        };
        for (key, prop) in props {
            let Some(p) = prop.as_object() else {
                return err(format!(
                    "settings schema property {key:?} must be an object"
                ));
            };
            let Some(ptype) = p.get("type").and_then(|t| t.as_str()) else {
                return err(format!(
                    "settings schema property {key:?} must declare \"type\""
                ));
            };
            if !matches!(ptype, "string" | "number" | "integer" | "boolean") {
                return err(format!(
                    "settings schema property {key:?} declares unsupported type {ptype:?}"
                ));
            }
            let (min_len, max_len) = (
                p.get("minLength").and_then(|v| v.as_u64()),
                p.get("maxLength").and_then(|v| v.as_u64()),
            );
            if let (Some(m), Some(ma)) = (min_len, max_len) {
                if ma < m {
                    return err(format!(
                        "settings schema property {key:?} has maxLength < minLength"
                    ));
                }
            }
            if let (Some(min), Some(max)) = (
                p.get("minimum").and_then(|v| v.as_f64()),
                p.get("maximum").and_then(|v| v.as_f64()),
            ) {
                if max < min {
                    return err(format!(
                        "settings schema property {key:?} has maximum < minimum"
                    ));
                }
            }
        }
        Ok(Self { raw })
    }

    fn property_type(spec: &serde_json::Map<String, serde_json::Value>) -> Option<&str> {
        spec.get("type").and_then(|t| t.as_str())
    }

    /// Validates one config VALUE against the schema (the write
    /// boundary before `plugin_install_settings.config` exists).
    /// Unknown config keys are rejected (the schema is the contract —
    /// deny-by-default); each supported key is checked for type and the
    /// optional bounds above.
    pub fn validate_config(
        &self,
        config: &serde_json::Value,
    ) -> Result<(), crate::error::DomainError> {
        let err = |m: String| Err(crate::error::DomainError::Validation(m));
        let Some(map) = config.as_object() else {
            return err("plugin settings config must be a JSON object".into());
        };
        let empty_map = serde_json::Map::new();
        let props = self
            .raw
            .get("properties")
            .and_then(|p| p.as_object())
            .unwrap_or(&empty_map);
        for key in map.keys() {
            if !props.contains_key(key) {
                return err(format!(
                    "unknown settings key {key:?} — render from the manifest schema"
                ));
            }
        }
        let required: Vec<&str> = self
            .raw
            .get("required")
            .and_then(|r| r.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        for req in required {
            if !map.contains_key(req) {
                return err(format!("settings config is missing required key {req:?}"));
            }
        }
        for (key, value) in map {
            let Some(spec) = props.get(key) else {
                return err(format!("unknown settings key {key:?}"));
            };
            let Some(spec_obj) = spec.as_object() else {
                return err(format!("settings schema key {key:?} is malformed"));
            };
            if let Some(enum_vals) = spec_obj.get("enum").and_then(|e| e.as_array()) {
                if !enum_vals.contains(value) {
                    return err(format!(
                        "settings key {key:?} is not one of the declared enum values"
                    ));
                }
                continue;
            }
            match Self::property_type(spec_obj).unwrap_or_default() {
                "string" => {
                    let Some(s) = value.as_str() else {
                        return err(format!("settings key {key:?} must be a string"));
                    };
                    let len = s.chars().count() as u64;
                    if let Some(min) = spec_obj.get("minLength").and_then(|v| v.as_u64()) {
                        if len < min {
                            return err(format!(
                                "settings key {key:?} is shorter than its declared minLength"
                            ));
                        }
                    }
                    if let Some(max) = spec_obj.get("maxLength").and_then(|v| v.as_u64()) {
                        if len > max {
                            return err(format!(
                                "settings key {key:?} is longer than its declared maxLength"
                            ));
                        }
                    }
                }
                "number" => {
                    let Some(n) = value.as_f64() else {
                        return err(format!("settings key {key:?} must be a number"));
                    };
                    if let Some(min) = spec_obj.get("minimum").and_then(|v| v.as_f64()) {
                        if n < min {
                            return err(format!(
                                "settings key {key:?} is below its declared minimum"
                            ));
                        }
                    }
                    if let Some(max) = spec_obj.get("maximum").and_then(|v| v.as_f64()) {
                        if n > max {
                            return err(format!(
                                "settings key {key:?} is above its declared maximum"
                            ));
                        }
                    }
                }
                "integer" => {
                    let n = match value.as_i64() {
                        Some(n) => n,
                        None => {
                            return err(format!("settings key {key:?} must be an integer"));
                        }
                    };
                    if let Some(min) = spec_obj.get("minimum").and_then(|v| v.as_i64()) {
                        if n < min {
                            return err(format!(
                                "settings key {key:?} is below its declared minimum"
                            ));
                        }
                    }
                    if let Some(max) = spec_obj.get("maximum").and_then(|v| v.as_i64()) {
                        if n > max {
                            return err(format!(
                                "settings key {key:?} is above its declared maximum"
                            ));
                        }
                    }
                }
                "boolean" => {
                    if !value.is_boolean() {
                        return err(format!("settings key {key:?} must be a boolean"));
                    }
                }
                other => {
                    return err(format!(
                        "settings schema key {key:?} declares unsupported type {other:?}"
                    ));
                }
            }
        }
        Ok(())
    }

    /// The UI-rendering shape: ordered property specs for the Plugins
    /// close-out lane to render FROM (never hand-authored).
    pub fn properties(&self) -> Vec<(String, serde_json::Value)> {
        self.raw
            .get("properties")
            .and_then(|p| p.as_object())
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    }
}

/// The version-consent diff for an UPDATE proposal (ADR 0009 §5):
/// declared capability keys old-declared vs new-declared, so the same
/// consent screen shows what re-consenting would enable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ConsentDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: Vec<String>,
    pub settings_schema: Option<serde_json::Value>,
}

impl ConsentDiff {
    pub fn compute(
        old_caps: &[String],
        new_caps: &[String],
        new_settings_schema: Option<serde_json::Value>,
    ) -> Self {
        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut unchanged = Vec::new();
        for old in old_caps {
            if new_caps.contains(old) {
                unchanged.push(old.clone());
            } else {
                removed.push(old.clone());
            }
        }
        for new in new_caps {
            if !old_caps.contains(new) {
                added.push(new.clone());
            }
        }
        Self {
            added,
            removed,
            unchanged,
            settings_schema: new_settings_schema,
        }
    }
}

/// The closed signing-key status vocabulary (migration 0095). ACTIVE =
/// named by new publications and part of the trust set; RETIRED = past
/// trust (no NEW publications name it) but still verifies pre-rotation
/// artifacts; REVOKED = compromise — verifies nothing, retroactively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SigningKeyStatus {
    Active,
    Retired,
    Revoked,
}

impl SigningKeyStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Retired => "retired",
            Self::Revoked => "revoked",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "retired" => Some(Self::Retired),
            "revoked" => Some(Self::Revoked),
            _ => None,
        }
    }
}

/// One row of the operator signing-key trust store (`plugin_signing_keys`,
/// migration 0095). `public_key` is verification-only material — 64 hex
/// chars of the ed25519 PUBLIC key; private keys never cross this seam.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SigningKeyRecord {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    pub key_id: String,
    pub public_key: String,
    pub status: SigningKeyStatus,
    pub label: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub created_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = "date-time"))]
    pub retired_at: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = "date-time"))]
    pub revoked_at: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub revoked_reason: Option<String>,
}

impl SigningKeyRecord {
    /// A revoked key verifies nothing — denial is retroactive on purpose.
    pub fn trust_set_member(&self) -> bool {
        matches!(
            self.status,
            SigningKeyStatus::Active | SigningKeyStatus::Retired
        )
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
    use serde_json::json;

    fn schema_sample() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "greeting": {"type": "string", "minLength": 1, "maxLength": 128},
                "verbosity": {"type": "integer", "minimum": 0, "maximum": 10},
                "ratio": {"type": "number", "minimum": 0.0},
                "verbose": {"type": "boolean"},
                "style": {"type": "string", "enum": ["concise", "detailed"]}
            },
            "required": ["greeting"]
        })
    }

    #[test]
    fn settings_schema_should_validate_supported_subsets() {
        assert!(SettingsSchema::new(schema_sample()).is_ok());
        assert!(SettingsSchema::new(json!({"type": "array"})).is_err());
        assert!(SettingsSchema::new(json!({"properties": {"x": {"type": "object"}}})).is_err());
        assert!(SettingsSchema::new(json!({"properties": {"x": {"default": 1}}})).is_err());
        assert!(SettingsSchema::new(
            json!({"properties": {"x": {"type": "string", "minLength": 5, "maxLength": 2}}})
        )
        .is_err());
    }

    #[test]
    fn settings_config_should_be_validated_deny_by_default() {
        let schema = SettingsSchema::new(schema_sample()).unwrap();
        let ok = json!({"greeting": "hi", "verbosity": 3, "ratio": 0.5, "verbose": true, "style": "concise"});
        assert!(schema.validate_config(&ok).is_ok());
        for bad in [
            json!({"greeting": "hi", "mystery": 1}),
            json!({}),
            json!({"greeting": "hi", "verbosity": 1.5}),
            json!({"greeting": "hi", "verbosity": 99}),
            json!({"greeting": "hi", "ratio": "not-a-number"}),
            json!({"greeting": "hi", "verbose": "yes"}),
            json!({"greeting": "hi", "style": "rambling"}),
            json!({"greeting": 7}),
            json!({"greeting": ""}),
            json!("not-an-object"),
        ] {
            assert!(
                schema.validate_config(&bad).is_err(),
                "config {bad} must reject"
            );
        }
    }

    #[test]
    fn consent_diff_should_show_added_removed_unchanged() {
        let old = vec!["tickets.read".to_string(), "kb.read".to_string()];
        let new = vec!["tickets.read".to_string(), "financial.read".to_string()];
        let diff = ConsentDiff::compute(&old, &new, Some(schema_sample()));
        assert_eq!(diff.added, vec!["financial.read"]);
        assert_eq!(diff.removed, vec!["kb.read"]);
        assert_eq!(diff.unchanged, vec!["tickets.read"]);
        assert_eq!(
            diff.settings_schema.as_ref().unwrap()["properties"]["greeting"]["type"],
            "string"
        );
    }

    #[test]
    fn source_kinds_should_round_trip() {
        for k in PluginSourceKind::ALL {
            assert_eq!(PluginSourceKind::parse(k.as_str()).expect("round trip"), k);
        }
    }

    #[test]
    fn provenance_entries_should_reject_bogus_actions_and_digests() {
        let base = PluginProvenanceEntry {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            install_id: Uuid::now_v7(),
            source_kind: PluginSourceKind::MarketplaceRegistry,
            source_url: "https://registry.example/catalog.json".into(),
            fetched_ref: Some("v1.2.3".into()),
            artifact_digest: "0".repeat(64).replace('0', "a"),
            uploader: Some(Uuid::now_v7()),
            action: "installed".into(),
            detail: None,
            recorded_at: chrono::Utc::now(),
        };
        assert!(base.validate().is_ok());
        let bogus_action = PluginProvenanceEntry {
            action: "deleted".into(),
            ..base.clone()
        };
        assert!(bogus_action.validate().is_err());
        let bogus_digest = PluginProvenanceEntry {
            artifact_digest: "abc".into(),
            ..base.clone()
        };
        assert!(bogus_digest.validate().is_err());
    }

    fn manifest(capabilities: &[&str]) -> PluginManifestV2 {
        PluginManifestV2 {
            name: "Ticket Summarizer".into(),
            version: "1.0.0".into(),
            interface_version: PLUGIN_RUNTIME_INTERFACE_VERSION,
            description: "Summarizes tickets".into(),
            capabilities: capabilities.iter().map(|s| s.to_string()).collect(),
            settings_schema: None,
            artifact_kind: None,
            connector_profile: None,
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
        // ADR 0006 leg (d): ciphertext-only ingress resolves onto the
        // secrets create gate — the same class of write a human record
        // author passes, never an invented authority.
        assert_eq!(
            PluginCapability::VaultIngress.permission(),
            Permission::Matrix {
                module: Module::Secrets,
                action: Action::Create
            }
        );
    }

    #[test]
    fn connector_profile_should_enforce_the_modeled_declaration_calendar() {
        let mut m = manifest(&[]);
        m.artifact_kind = Some(PluginRuntimeKind::HttpConnector);
        // A connector manifest without a profile keeps the valid default
        // (no declared webhooks); a DECLARED profile validates deeply.
        assert!(m.validate().is_ok());
        m.connector_profile = Some(ConnectorProfile {
            webhook_endpoints: vec![
                "https://hooks.example.com/m365/ingest".into(),
                "https://relay.immybot.dev/webhook".into(),
            ],
            scopes: vec!["graph:mail.read".into(), "graph:calendars.readwrite".into()],
        });
        assert!(m.validate().is_ok());

        // A wasm manifest DECLARING a connector profile refuses.
        m.artifact_kind = Some(PluginRuntimeKind::Wasm);
        assert!(m.validate().is_err());

        // The default (absent kind = wasm baseline) refuses a profile too.
        m.artifact_kind = None;
        assert!(m.validate().is_err());
    }

    #[test]
    fn connector_profile_should_refuse_http_credentials_and_oversize_declarations() {
        let refuse = |url: &str| {
            ConnectorProfile {
                webhook_endpoints: vec![url.into()],
                scopes: vec!["graph:mail.read".into()],
            }
            .validate()
            .is_err()
        };
        assert!(refuse("http://hooks.example.com/plain")); // not https
        assert!(refuse("https://user:pass@hooks.example.com/x")); // credentials
        assert!(refuse("not-a-url"));

        let ok = ConnectorProfile {
            webhook_endpoints: vec!["https://hooks.example.com/x".into()],
            scopes: vec![],
        };
        assert!(ok.validate().is_ok());
        // Over-cap declarations refuse.
        let many: Vec<String> = (0..10)
            .map(|i| format!("https://h{i}.example.com/x"))
            .collect();
        assert!(ConnectorProfile {
            webhook_endpoints: many,
            scopes: vec![],
        }
        .validate()
        .is_err());
        let scopes: Vec<String> = (0..40).map(|i| format!("scope{i}")).collect();
        assert!(ConnectorProfile {
            webhook_endpoints: vec![],
            scopes,
        }
        .validate()
        .is_err());
        assert!(ConnectorProfile {
            webhook_endpoints: vec![],
            scopes: vec!["bad scope with spaces".into()],
        }
        .validate()
        .is_err());
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

    /// The roster discriminator (lane-10, ADR 0009 §6): WASM is the
    /// declared-or-default baseline, connectors must declare, unknown
    /// keys reject at the deserialization boundary.
    #[test]
    fn runtime_kind_should_default_to_wasm_and_reject_unknown_keys() {
        let json = serde_json::json!({
            "name": "Collector HTTP",
            "version": "0.9.0",
            "interface_version": PLUGIN_RUNTIME_INTERFACE_VERSION,
            "capabilities": ["tickets.read"],
            "artifact_kind": "http_connector",
        });
        let manifest: PluginManifestV2 = serde_json::from_value(json).unwrap();
        assert_eq!(manifest.runtime_kind(), PluginRuntimeKind::HttpConnector);
        manifest.validate().expect("declared kind validates");

        let no_kind = serde_json::json!({
            "name": "Ticket Summarizer",
            "version": "1.0.0",
            "interface_version": PLUGIN_RUNTIME_INTERFACE_VERSION,
            "capabilities": ["tickets.read"],
        });
        let manifest = serde_json::from_value::<PluginManifestV2>(no_kind).unwrap();
        assert_eq!(manifest.runtime_kind(), PluginRuntimeKind::Wasm);

        let bad = serde_json::json!({
            "name": "Gadget",
            "version": "1.0.0",
            "interface_version": PLUGIN_RUNTIME_INTERFACE_VERSION,
            "capabilities": [],
            "artifact_kind": "ungoverned_widget",
        });
        assert!(serde_json::from_value::<PluginManifestV2>(bad).is_err());
    }
}
