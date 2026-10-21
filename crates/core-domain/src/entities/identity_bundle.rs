//! The identity-bundle ARTIFACT (this lane): the declarative half of a
//! pluggable identity source, distributed like every other ADR 0009
//! artifact and adopted through the same staged-import consent diff.
//!
//! Why a bundle exists at all. `crates/core-api/src/auth/federation.rs`
//! defines the `FederationBundle` seam — one identity source attached to
//! the SAME session machinery as a password login — and
//! `crates/core-api/src/auth/provider.rs` defines the auth-provider
//! contract behind it. Neither had an artifact: an operator hand-wrote
//! rows. A bundle gives that seam a versioned, digest-pinned, signed
//! distribution unit, in the shape plugins, skills and theme packs
//! already use.
//!
//! What a bundle MAY declare: a name, a version, the interface version
//! it was built against, one or two provider kinds, the settings schema
//! its configuration is rendered from, and the upstream scopes it asks
//! its identity provider for.
//!
//! What a bundle may NEVER declare, by construction:
//!
//! * **No plugin capability.** There is no `auth:*` key in the plugin
//!   capability vocabulary and this module adds none. A bundle's
//!   `scopes` are UPSTREAM provider scopes (`openid`, `email`,
//!   `profile`, a tenant scope…) that the identity provider interprets —
//!   they confer nothing inside Marula, and [`IdentityBundleManifest::validate`]
//!   refuses any scope that collides with a plugin capability key, so
//!   the boundary is mechanical rather than a comment.
//! * **No session, token or MFA semantics.** Those stay core
//!   (`auth::handlers::issue_tokens`, the refresh-chain rotation, and
//!   `federated_mfa_gate`). A bundle names an identity source; it never
//!   mints authority.
//! * **No code.** The executable half is a core-side adapter registered
//!   through `FederationBundle`; the artifact is data, exactly like the
//!   plugin and theme manifests beside it.
//!
//! Non-purpose: persistence (core-db owns `identity_bundle_installs`),
//! the HTTP surface, and signature verification / digest pinning
//! (plugin-host's distribution adapters own both).

use crate::entities::plugins::{
    is_sha256_hex, PluginCapability, PluginInstallStatus, SettingsSchema,
};
use crate::error::DomainError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Version of the identity-bundle artifact contract. A manifest written
/// against another version is refused — versioned interface,
/// deny-by-default (the `FEDERATION_BUNDLE_INTERFACE_VERSION` posture,
/// mirrored here because the domain cannot depend on the API crate).
pub const IDENTITY_BUNDLE_INTERFACE_VERSION: u32 = 1;

/// Longest accepted bundle name / version / scope string, in bytes.
pub const MAX_BUNDLE_NAME_BYTES: usize = 64;
pub const MAX_BUNDLE_VERSION_BYTES: usize = 64;
pub const MAX_SCOPE_BYTES: usize = 128;
/// A bundle names at most one source per transport.
pub const MAX_PROVIDER_KINDS: usize = 2;
/// Ceiling on requested upstream scopes (an IdP realm rarely asks for
/// more; the cap keeps a manifest from becoming a permissions wishlist).
pub const MAX_SCOPES: usize = 32;

/// The closed provider-kind vocabulary a bundle may declare. Each kind
/// names the TRANSPORT the executable half authenticates over — never a
/// vendor. The wire strings are the ones
/// `crates/core-api/src/auth/federation.rs::kind` publishes; the domain
/// owns the vocabulary because plugin-host validates manifests and
/// cannot depend on the API crate. A test there asserts the two agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum IdentityProviderKind {
    /// OIDC authorization-code providers, first-party presets included.
    Oidc,
    /// A reverse-proxy signed-JWT assertion (the Cloudflare Access shape).
    EdgeAssertion,
}

impl IdentityProviderKind {
    pub const ALL: [IdentityProviderKind; 2] = [
        IdentityProviderKind::Oidc,
        IdentityProviderKind::EdgeAssertion,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            IdentityProviderKind::Oidc => "oidc",
            IdentityProviderKind::EdgeAssertion => "edge_assertion",
        }
    }

    /// Unknown kinds are REJECTED: the vocabulary is closed, so a bundle
    /// naming a transport the core cannot attach has no way to describe
    /// itself (deny-by-default at the data boundary).
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.as_str() == value)
    }
}

/// The declarative manifest an `identity_bundle` artifact carries. Field
/// set is deliberately EXACTLY the six the artifact needs; anything a
/// bundle might want to smuggle (capabilities, permissions, roles,
/// session settings) has no field to land in.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct IdentityBundleManifest {
    /// Stable, lowercase identifier. Doubles as the registry key and the
    /// audit label, so it is slug-shaped rather than a display name.
    pub name: String,
    pub version: String,
    /// The artifact contract this was authored against.
    pub interface_version: u32,
    /// One or two transports this bundle's executable half attaches.
    pub provider_kinds: Vec<IdentityProviderKind>,
    /// The JSON-Schema a bundle's configuration is rendered from and
    /// validated against (same supported subset as a plugin's).
    pub settings_schema: serde_json::Value,
    /// Upstream identity-provider scopes the bundle requests. These are
    /// the IdP's vocabulary, not Marula's: holding one grants nothing.
    #[serde(default)]
    pub scopes: Vec<String>,
}

impl IdentityBundleManifest {
    /// Validates the declaration itself. Every refusal is a typed
    /// [`DomainError::Validation`] naming the offending field, because
    /// the ingest gate surfaces these to the uploader.
    pub fn validate(&self) -> Result<(), DomainError> {
        validate_name(&self.name)?;
        validate_version(&self.version)?;
        if self.interface_version != IDENTITY_BUNDLE_INTERFACE_VERSION {
            return Err(DomainError::Validation(format!(
                "identity bundle was authored against interface version {} but this core \
                 speaks {IDENTITY_BUNDLE_INTERFACE_VERSION}",
                self.interface_version
            )));
        }
        if self.provider_kinds.is_empty() {
            return Err(DomainError::Validation(
                "identity bundle must declare at least one provider kind".into(),
            ));
        }
        if self.provider_kinds.len() > MAX_PROVIDER_KINDS {
            return Err(DomainError::Validation(format!(
                "identity bundle declares {} provider kinds; at most {MAX_PROVIDER_KINDS} exist",
                self.provider_kinds.len()
            )));
        }
        for (i, kind) in self.provider_kinds.iter().enumerate() {
            if self.provider_kinds[..i].contains(kind) {
                return Err(DomainError::Validation(format!(
                    "identity bundle declares provider kind {:?} twice",
                    kind.as_str()
                )));
            }
        }
        // The settings schema is the ONLY place a bundle's configuration
        // is described, so it is validated with the plugin subset rules
        // rather than passed through.
        SettingsSchema::new(self.settings_schema.clone())?;
        if self.scopes.len() > MAX_SCOPES {
            return Err(DomainError::Validation(format!(
                "identity bundle requests {} scopes; the cap is {MAX_SCOPES}",
                self.scopes.len()
            )));
        }
        for (i, scope) in self.scopes.iter().enumerate() {
            validate_scope(scope)?;
            if self.scopes[..i].contains(scope) {
                return Err(DomainError::Validation(format!(
                    "identity bundle requests scope {scope:?} twice"
                )));
            }
            // THE BOUNDARY. A scope is the identity provider's word, and
            // the plugin capability vocabulary has no `auth:*` key. If a
            // scope ever spells a capability key, the artifact is trying
            // to arrive at a grant through the settings door — refused
            // outright rather than silently dropped.
            if PluginCapability::parse(scope).is_some() {
                return Err(DomainError::Validation(format!(
                    "identity bundle scope {scope:?} is a plugin capability key; an identity \
                     bundle confers no Marula capability"
                )));
            }
        }
        Ok(())
    }

    /// The validated settings-schema handle for rendering and for
    /// write-boundary config validation. Only callable after
    /// [`Self::validate`] succeeded, but it re-checks so a caller that
    /// skipped validation still cannot render an unsupported schema.
    pub fn settings(&self) -> Result<SettingsSchema, DomainError> {
        SettingsSchema::new(self.settings_schema.clone())
    }

    /// Parses and validates a wire manifest in one step — the ingest
    /// gate's only entry point.
    pub fn from_value(value: &serde_json::Value) -> Result<Self, DomainError> {
        // A bundle has no capabilities field. A manifest carrying the key
        // is REFUSED rather than ignored: silently dropping it would let
        // an author believe a grant had been requested.
        if value.get("capabilities").is_some() || value.get("permissions").is_some() {
            return Err(DomainError::Validation(
                "identity bundle manifests declare no capabilities or permissions; an identity \
                 bundle confers no Marula capability"
                    .into(),
            ));
        }
        let manifest: Self = serde_json::from_value(value.clone())
            .map_err(|e| DomainError::Validation(format!("identity bundle manifest: {e}")))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Slug-shaped stable id derived from the declared name (the install
    /// key inside an org, mirroring theme packs).
    pub fn slug(&self) -> String {
        slug_of(&self.name)
    }
}

/// Lowercase, install-safe id derived from a bundle name.
pub fn slug_of(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn validate_name(name: &str) -> Result<(), DomainError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(DomainError::Validation(
            "identity bundle name is required".into(),
        ));
    }
    if trimmed.len() > MAX_BUNDLE_NAME_BYTES {
        return Err(DomainError::Validation(format!(
            "identity bundle name exceeds {MAX_BUNDLE_NAME_BYTES} bytes"
        )));
    }
    // Slug-shaped on purpose: the name is a registry key and an audit
    // label, so two bundles cannot differ only by case or spacing.
    let ok = trimmed
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
        && trimmed
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if !ok {
        return Err(DomainError::Validation(format!(
            "identity bundle name {trimmed:?} must be lowercase alphanumeric with '-', '_' or \
             '.' and start with a letter or digit"
        )));
    }
    Ok(())
}

fn validate_version(version: &str) -> Result<(), DomainError> {
    let trimmed = version.trim();
    if trimmed.is_empty() {
        return Err(DomainError::Validation(
            "identity bundle version is required".into(),
        ));
    }
    if trimmed.len() > MAX_BUNDLE_VERSION_BYTES {
        return Err(DomainError::Validation(format!(
            "identity bundle version exceeds {MAX_BUNDLE_VERSION_BYTES} bytes"
        )));
    }
    let ok = trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))
        && trimmed.chars().next().is_some_and(|c| c.is_ascii_digit());
    if !ok {
        return Err(DomainError::Validation(format!(
            "identity bundle version {trimmed:?} must start with a digit and hold only \
             alphanumerics plus '.', '-', '+', '_'"
        )));
    }
    Ok(())
}

fn validate_scope(scope: &str) -> Result<(), DomainError> {
    let trimmed = scope.trim();
    if trimmed.is_empty() {
        return Err(DomainError::Validation(
            "identity bundle scope entries cannot be empty".into(),
        ));
    }
    if trimmed.len() > MAX_SCOPE_BYTES {
        return Err(DomainError::Validation(format!(
            "scope {trimmed:?} exceeds {MAX_SCOPE_BYTES} bytes"
        )));
    }
    let ok = trimmed.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-' | '@' | '~' | '*')
    });
    if !ok {
        return Err(DomainError::Validation(format!(
            "scope {trimmed:?} carries characters an identity provider does not read"
        )));
    }
    Ok(())
}

// --- the install lifecycle ----------------------------------------------------

/// What an admin may ask of an installed bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum IdentityBundleAction {
    Enable,
    Disable,
    Revoke,
}

impl IdentityBundleAction {
    pub fn as_str(self) -> &'static str {
        match self {
            IdentityBundleAction::Enable => "enable",
            IdentityBundleAction::Disable => "disable",
            IdentityBundleAction::Revoke => "revoke",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "enable" => Some(Self::Enable),
            "disable" => Some(Self::Disable),
            "revoke" => Some(Self::Revoke),
            _ => None,
        }
    }
}

/// The lifecycle state machine, expressed as one total function so the
/// route layer and the tests cannot drift apart.
///
/// * an install is minted `Disabled` (deny-by-default — approving a
///   staged import adopts the artifact, it does not switch it on);
/// * enable/disable flip between `Enabled` and `Disabled` freely, and
///   asking for the state it is already in is an idempotent no-op;
/// * `Revoked` is TERMINAL: no action revives a revoked bundle, a
///   re-install is a new row with a fresh consent diff.
pub fn transition(
    current: PluginInstallStatus,
    action: IdentityBundleAction,
) -> Result<PluginInstallStatus, DomainError> {
    use PluginInstallStatus::{Disabled, Enabled, Revoked};
    match (current, action) {
        (Revoked, _) => Err(DomainError::InvalidTransition(
            current.as_str().to_string(),
            action.as_str().to_string(),
        )),
        (Disabled, IdentityBundleAction::Enable) => Ok(Enabled),
        (Enabled, IdentityBundleAction::Disable) => Ok(Disabled),
        (Disabled, IdentityBundleAction::Disable) => Ok(Disabled),
        (Enabled, IdentityBundleAction::Enable) => Ok(Enabled),
        (Disabled | Enabled, IdentityBundleAction::Revoke) => Ok(Revoked),
    }
}

/// An installed identity bundle: the adopted artifact plus the provenance
/// chain that survives it. Mirrors `theme_packs` — the row is the
/// byproduct of an APPROVED staged import, so `provenance_digest` is the
/// pinned artifact digest and never a caller-supplied value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct IdentityBundleInstall {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: Uuid,
    /// Slug of [`IdentityBundleManifest::name`] — unique per org.
    pub slug: String,
    pub name: String,
    pub version: String,
    pub interface_version: u32,
    pub provider_kinds: Vec<String>,
    pub scopes: Vec<String>,
    /// Full validated manifest snapshot, so the consent screen and the
    /// settings renderer read the same bytes the reviewer approved.
    pub manifest: serde_json::Value,
    pub provenance_source_kind: String,
    pub provenance_source_url: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub provenance_fetched_ref: Option<String>,
    /// sha256 of the verified artifact — the pinning anchor.
    pub provenance_digest: String,
    pub status: PluginInstallStatus,
    /// JIT provisioning is a per-install ADMIN opt-in and default-false;
    /// even when true the core only provisions from a verified email.
    pub jit_enabled: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub revoked_reason: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub installed_by: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub installed_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub updated_at: DateTime<Utc>,
}

impl IdentityBundleInstall {
    /// The install row a fresh approval produces: `Disabled`, JIT off,
    /// digest pinned from the staged item (never from the caller).
    #[allow(clippy::too_many_arguments)]
    pub fn from_approved_staged(
        id: Uuid,
        org_id: Uuid,
        manifest: &IdentityBundleManifest,
        manifest_json: serde_json::Value,
        source_kind: &str,
        source_url: &str,
        fetched_ref: Option<&str>,
        digest: &str,
        installed_by: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        manifest.validate()?;
        if !is_sha256_hex(digest) {
            return Err(DomainError::Validation(format!(
                "identity bundle artifact digest {digest:?} is not a sha256 hex string"
            )));
        }
        Ok(Self {
            id,
            org_id,
            slug: manifest.slug(),
            name: manifest.name.trim().to_string(),
            version: manifest.version.trim().to_string(),
            interface_version: manifest.interface_version,
            provider_kinds: manifest
                .provider_kinds
                .iter()
                .map(|k| k.as_str().to_string())
                .collect(),
            scopes: manifest.scopes.clone(),
            manifest: manifest_json,
            provenance_source_kind: source_kind.to_string(),
            provenance_source_url: source_url.to_string(),
            provenance_fetched_ref: fetched_ref.map(String::from),
            provenance_digest: digest.to_string(),
            status: PluginInstallStatus::Disabled,
            jit_enabled: false,
            revoked_reason: None,
            installed_by,
            installed_at: now,
            updated_at: now,
        })
    }

    /// The status after `action`, without mutating — the route layer
    /// audits the transition it is about to write.
    pub fn next_status(
        &self,
        action: IdentityBundleAction,
    ) -> Result<PluginInstallStatus, DomainError> {
        transition(self.status, action)
    }

    /// Applies `action`, stamping the reason on revoke. Idempotent for
    /// enable/disable; refused once revoked.
    pub fn apply(
        &mut self,
        action: IdentityBundleAction,
        reason: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<PluginInstallStatus, DomainError> {
        let next = self.next_status(action)?;
        self.status = next;
        self.updated_at = now;
        if next == PluginInstallStatus::Revoked {
            self.revoked_reason = reason;
        }
        Ok(next)
    }

    /// True iff this install is the artifact the pinned digest names —
    /// the re-fetch check every activation path runs before a bundle is
    /// allowed to register.
    pub fn pins(&self, artifact_bytes: &[u8]) -> bool {
        plugin_digest_of(artifact_bytes) == self.provenance_digest.trim().to_lowercase()
    }
}

/// sha256 hex over arbitrary bytes. Duplicated from plugin-host on
/// purpose: the domain must not depend on the distribution crate, and a
/// digest helper is cheaper than a dependency edge.
pub fn plugin_digest_of(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_json() -> serde_json::Value {
        serde_json::json!({
            "name": "corp-okta",
            "version": "1.4.0",
            "interface_version": IDENTITY_BUNDLE_INTERFACE_VERSION,
            "provider_kinds": ["oidc"],
            "settings_schema": {
                "type": "object",
                "properties": {
                    "issuer": { "type": "string" },
                    "client_id": { "type": "string" }
                }
            },
            "scopes": ["openid", "email", "profile"]
        })
    }

    fn manifest() -> IdentityBundleManifest {
        IdentityBundleManifest::from_value(&manifest_json()).expect("baseline manifest is valid")
    }

    #[test]
    fn manifest_round_trips_and_validates() {
        let m = manifest();
        assert_eq!(m.slug(), "corp-okta");
        assert_eq!(m.provider_kinds, vec![IdentityProviderKind::Oidc]);
        assert_eq!(m.scopes, vec!["openid", "email", "profile"]);
        assert!(m.settings().is_ok());
    }

    #[test]
    fn interface_version_drift_is_refused() {
        let mut value = manifest_json();
        value["interface_version"] = serde_json::json!(IDENTITY_BUNDLE_INTERFACE_VERSION + 1);
        let err = IdentityBundleManifest::from_value(&value).expect_err("drift refuses");
        assert!(err.to_string().contains("interface version"), "{err}");
    }

    #[test]
    fn unknown_and_duplicate_provider_kinds_are_refused() {
        let mut value = manifest_json();
        value["provider_kinds"] = serde_json::json!(["saml"]);
        assert!(IdentityBundleManifest::from_value(&value).is_err());

        let mut value = manifest_json();
        value["provider_kinds"] = serde_json::json!(["oidc", "oidc"]);
        let err = IdentityBundleManifest::from_value(&value).expect_err("dupe refuses");
        assert!(err.to_string().contains("twice"), "{err}");

        let mut value = manifest_json();
        value["provider_kinds"] = serde_json::json!([]);
        assert!(IdentityBundleManifest::from_value(&value).is_err());
    }

    #[test]
    fn off_schema_settings_are_refused() {
        let mut value = manifest_json();
        value["settings_schema"] = serde_json::json!({
            "type": "object",
            "properties": { "claim_map": { "type": "array" } }
        });
        let err = IdentityBundleManifest::from_value(&value).expect_err("array refuses");
        assert!(err.to_string().contains("unsupported type"), "{err}");
    }

    /// THE BOUNDARY, stated as an executable claim: an identity bundle
    /// arrives at no plugin capability. Two doors are closed — the
    /// manifest has no capabilities field (a manifest carrying one is
    /// refused, not silently stripped) and no scope may spell a
    /// capability key.
    #[test]
    fn identity_bundle_confers_no_plugin_capability() {
        let mut value = manifest_json();
        value["capabilities"] = serde_json::json!(["tickets.read"]);
        let err = IdentityBundleManifest::from_value(&value).expect_err("capabilities refuse");
        assert!(
            err.to_string().contains("confers no Marula capability"),
            "{err}"
        );

        let mut value = manifest_json();
        value["permissions"] = serde_json::json!(["special:manage_plugins"]);
        assert!(IdentityBundleManifest::from_value(&value).is_err());

        let mut value = manifest_json();
        value["scopes"] = serde_json::json!(["openid", "tickets.read"]);
        let err = IdentityBundleManifest::from_value(&value).expect_err("capability scope refuses");
        assert!(err.to_string().contains("plugin capability key"), "{err}");
    }

    /// The vocabulary itself: no plugin capability key carries auth
    /// authority, and the identity-bundle artifact kind adds none. This
    /// is the lane's contract with the auth core, asserted rather than
    /// documented.
    #[test]
    fn no_plugin_capability_grants_auth_authority() {
        for cap in PluginCapability::ALL {
            let key = cap.as_str();
            assert!(
                !key.starts_with("auth"),
                "capability {key} would grant auth authority"
            );
            assert!(
                !key.contains("session"),
                "capability {key} reaches sessions"
            );
            assert!(!key.contains("token"), "capability {key} reaches tokens");
            assert!(!key.contains("mfa"), "capability {key} reaches MFA");
        }
        // A plausible-looking key the vocabulary must keep refusing.
        assert!(PluginCapability::parse("auth.login").is_none());
        assert!(PluginCapability::parse("auth.session").is_none());
        assert!(PluginCapability::parse("auth.mfa").is_none());
    }

    #[test]
    fn scopes_carry_provider_vocabulary_only() {
        let m = manifest();
        for scope in &m.scopes {
            assert!(PluginCapability::parse(scope).is_none(), "{scope}");
        }
        let mut value = manifest_json();
        value["scopes"] = serde_json::json!(["openid", "openid"]);
        assert!(IdentityBundleManifest::from_value(&value).is_err());
        let mut value = manifest_json();
        value["scopes"] = serde_json::json!(["open id"]);
        assert!(IdentityBundleManifest::from_value(&value).is_err());
    }

    #[test]
    fn names_are_slug_shaped_so_two_bundles_cannot_collide_by_case() {
        for bad in ["", "  ", "Corp Okta", "Corp-Okta", "-okta", "corp okta"] {
            let mut value = manifest_json();
            value["name"] = serde_json::json!(bad);
            assert!(
                IdentityBundleManifest::from_value(&value).is_err(),
                "{bad:?} should refuse"
            );
        }
        let mut value = manifest_json();
        value["name"] = serde_json::json!("corp-okta.eu");
        assert!(IdentityBundleManifest::from_value(&value).is_ok());
    }

    fn install() -> IdentityBundleInstall {
        IdentityBundleInstall::from_approved_staged(
            Uuid::nil(),
            Uuid::nil(),
            &manifest(),
            manifest_json(),
            "local_zip",
            "local://zip-import",
            None,
            &"ab".repeat(32),
            Uuid::nil(),
            Utc::now(),
        )
        .expect("install builds")
    }

    #[test]
    fn installs_are_minted_disabled_with_jit_off_and_the_digest_pinned() {
        let row = install();
        assert_eq!(row.status, PluginInstallStatus::Disabled);
        assert!(!row.jit_enabled);
        assert_eq!(row.provenance_digest, "ab".repeat(32));
        assert_eq!(row.provider_kinds, vec!["oidc"]);
        assert!(row.pins(b"artifact bytes") == false);
        assert!(row
            .pins(&[])
            .eq(&(row.provenance_digest == plugin_digest_of(&[]))));
    }

    #[test]
    fn a_non_sha256_pin_is_refused() {
        let err = IdentityBundleInstall::from_approved_staged(
            Uuid::nil(),
            Uuid::nil(),
            &manifest(),
            manifest_json(),
            "local_zip",
            "local://zip-import",
            None,
            "not-a-digest",
            Uuid::nil(),
            Utc::now(),
        )
        .expect_err("bad digest refuses");
        assert!(err.to_string().contains("sha256"), "{err}");
    }

    #[test]
    fn enable_and_disable_are_idempotent_and_revoke_is_terminal() {
        use PluginInstallStatus::{Disabled, Enabled, Revoked};
        assert_eq!(
            transition(Disabled, IdentityBundleAction::Enable),
            Ok(Enabled)
        );
        assert_eq!(
            transition(Enabled, IdentityBundleAction::Enable),
            Ok(Enabled)
        );
        assert_eq!(
            transition(Enabled, IdentityBundleAction::Disable),
            Ok(Disabled)
        );
        assert_eq!(
            transition(Disabled, IdentityBundleAction::Disable),
            Ok(Disabled)
        );
        assert_eq!(
            transition(Enabled, IdentityBundleAction::Revoke),
            Ok(Revoked)
        );
        assert_eq!(
            transition(Disabled, IdentityBundleAction::Revoke),
            Ok(Revoked)
        );
        for action in [
            IdentityBundleAction::Enable,
            IdentityBundleAction::Disable,
            IdentityBundleAction::Revoke,
        ] {
            assert!(
                transition(Revoked, action).is_err(),
                "revoked is terminal for {action:?}"
            );
        }

        let mut row = install();
        row.apply(IdentityBundleAction::Enable, None, Utc::now())
            .expect("enable");
        row.apply(
            IdentityBundleAction::Revoke,
            Some("publisher withdrew the bundle".into()),
            Utc::now(),
        )
        .expect("revoke");
        assert_eq!(row.status, Revoked);
        assert_eq!(
            row.revoked_reason.as_deref(),
            Some("publisher withdrew the bundle")
        );
        assert!(row
            .apply(IdentityBundleAction::Enable, None, Utc::now())
            .is_err());
    }

    #[test]
    fn provider_kind_vocabulary_is_closed_and_round_trips() {
        for kind in IdentityProviderKind::ALL {
            assert_eq!(IdentityProviderKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(IdentityProviderKind::parse("saml"), None);
        assert_eq!(IdentityProviderKind::Oidc.as_str(), "oidc");
        assert_eq!(
            IdentityProviderKind::EdgeAssertion.as_str(),
            "edge_assertion"
        );
    }
}
