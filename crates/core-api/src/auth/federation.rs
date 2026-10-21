//! The identity-federation BUNDLE interface (planning REL-E4, the private build tracker
//! ID3/ID4): the versioned contract every federation bundle implements
//! and the shared invariants every bundle's login path MUST route
//! through.
//!
//! Doctrine (non-negotiable):
//! * Bundles ADD an identity source; they are NEVER a parallel session
//!   system. A federated login resolves into the same token machinery
//!   (`handlers::issue_tokens` — auth_sessions + JWT refresh chains)
//!   as a password login.
//! * Deny-by-default: an unknown bundle, a DISABLED bundle, and an
//!   assertion that fails verification are all rejections; nothing
//!   authenticates by default. Enabling a bundle is an explicit,
//!   audited ADMIN action.
//! * Verified-email linking: an identity source links to a local user
//!   only through the conservative decision (`decide_sso`) — explicit
//!   prior link rows, or JIT only when the org enabled it AND the
//!   source asserted a verified email.
//! * Org MFA policy applies regardless of identity source: every
//!   bundle's token issuance passes through the same MFA gate the
//!   password login uses (`handlers::federated_mfa_gate`).
//! * Audit events cover every federation lifecycle step (source
//!   saved/deleted, link created/removed, login success/failure).

use crate::auth::oidc_client::IdClaims;
use uuid::Uuid;

/// Version of the bundle interface contract. Bump on any breaking
/// change to [`FederationBundle`]; bundles declare the version they
/// were built against and the coordinator refuses mixed versions.
pub const FEDERATION_BUNDLE_INTERFACE_VERSION: u32 = 1;

/// Bundle kinds shipped with the core. A kind names the transport a
/// bundle authenticates over, not a vendor.
pub mod kind {
    /// OIDC authorization-code providers (incl. first-party presets).
    pub const OIDC: &str = "oidc";
    /// Reverse-proxy signed-JWT assertion (Cloudflare Access pattern).
    pub const EDGE_ASSERTION: &str = "edge_assertion";
}

/// Source namespace for link rows. OIDC links live in `oidc_accounts`
/// keyed by provider id; every other bundle links through
/// `federation_links` under its kind + source id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkNamespace {
    Oidc(Uuid),
    Bundle(&'static str, Uuid),
}

impl LinkNamespace {
    pub fn source_id(&self) -> Uuid {
        match self {
            LinkNamespace::Oidc(id) | LinkNamespace::Bundle(_, id) => *id,
        }
    }
}

/// An identity the bundle has VERIFIED (signature + pinning done) and
/// now submits to the shared conservative resolution. Bundles must
/// never pre-judge the decision — that is what [`decide_sso`] is for.
#[derive(Debug, Clone)]
pub struct FederatedIdentity {
    pub namespace: LinkNamespace,
    pub claims: IdClaims,
}

/// A federation bundle: one identity source attached to the session
/// machinery. Implementations exist per bundle kind; the router owns
/// the registry so a disabled or unknown bundle simply has no route
/// that can authenticate (deny-by-default by construction).
pub trait FederationBundle: Send + Sync {
    /// Machine kind (one of [`kind`]).
    ///
    /// This is a METHOD rather than an associated const on purpose: a
    /// registry holds `dyn FederationBundle`, and an associated const makes
    /// the trait not dyn-compatible, which is what kept this seam without a
    /// registry until the identity-bundle artifact made one necessary. The
    /// value is still a compile-time constant per implementation.
    fn kind(&self) -> &'static str;
    /// Interface version this bundle implements. Same reasoning: a method,
    /// so a version-mismatched bundle can be detected through the trait
    /// object instead of only at a static type.
    fn interface_version(&self) -> u32 {
        FEDERATION_BUNDLE_INTERFACE_VERSION
    }

    fn id(&self) -> Uuid;
    fn name(&self) -> &str;
    /// Admin-enabled gate. `false` = the bundle never authenticates
    /// anyone, whatever its protocol state.
    fn enabled(&self) -> bool;
    /// JIT provisioning flag. Deny-by-default: absent an explicit org
    /// opt-in this is `false`, and even `true` only provisions when
    /// the claims carry a VERIFIED email.
    fn jit_enabled(&self) -> bool;
}

/// Static reasons a federation login can be refused. They surface in
/// audit details and (for JSON surfaces) as typed envelope reason
/// codes — never as prose to parse.
pub mod denial_reasons {
    pub const LINK_REQUIRED: &str = "edge_identity_unlinked";
    pub const EMAIL_UNVERIFIED: &str = "link_email_unverified";
    pub const EMAIL_MISMATCH: &str = "link_email_mismatch";
    pub const JIT_MISSING_EMAIL: &str = "jit_missing_email";
    pub const JIT_EMAIL_UNVERIFIED: &str = "jit_email_unverified";
    /// A one-time SSO handoff code the exchange endpoint cannot redeem:
    /// unknown, expired, or already used — one code for all three so an
    /// attacker cannot fingerprint which check tripped (parity with
    /// `edge_assertion_invalid`).
    pub const SSO_HANDOFF_INVALID: &str = "sso_handoff_invalid";
}

// --- the identity-bundle registration seam (this lane) -----------------------
//
// Before this lane the trait above had no implementation and no registry: a
// federation source was hand-wired in the route that needed it. An
// `identity_bundle` artifact makes the set of sources DISCOVERED rather than
// compiled in, so the trait needs a registry that holds implementations it did
// not ship with.
//
// The executable half is deliberately thin. A bundle names an identity source;
// the adapter registered for it answers four questions (id, name, enabled,
// JIT) and nothing else. Authentication itself still runs through the core's
// login path — `resolve_and_issue` in `auth/provider.rs`, which mints the
// session with `handlers::issue_tokens` and gates MFA with
// `handlers::federated_mfa_gate`. There is no method here that returns tokens,
// and no `auth:*` plugin capability exists for a bundle to hold. That is the
// permanent boundary of this seam, not a v1 limitation.

use crate::auth::provider::ProviderContext;
use core_domain::entities::identity_bundle::{IdentityBundleInstall, IdentityProviderKind};
use core_domain::entities::plugins::PluginInstallStatus;

/// Why an install was not registered. Typed, because a log line that says
/// "bundle skipped" is useless to the operator who cannot sign in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationRefusal {
    /// A revoked install is terminal: it never re-enters the registry.
    Revoked,
    /// The artifact was authored against another interface version.
    InterfaceVersion { expected: u32, found: u32 },
}

impl std::fmt::Display for RegistrationRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistrationRefusal::Revoked => {
                f.write_str("identity bundle is revoked; re-install to register a source")
            }
            RegistrationRefusal::InterfaceVersion { expected, found } => write!(
                f,
                "identity bundle implements interface version {found} but this core speaks {expected}"
            ),
        }
    }
}

/// The answers a registered bundle gives, resolved from its install row.
/// Both wrappers below carry one of these.
#[derive(Debug, Clone)]
pub struct RegisteredIdentityBundle {
    pub install_id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub jit_enabled: bool,
}

impl RegisteredIdentityBundle {
    /// Resolves an install into a registration, refusing the two states a
    /// registry must never hold: a revoked row and a version mismatch.
    pub fn from_install(install: &IdentityBundleInstall) -> Result<Self, RegistrationRefusal> {
        if install.status == PluginInstallStatus::Revoked {
            return Err(RegistrationRefusal::Revoked);
        }
        if install.interface_version != FEDERATION_BUNDLE_INTERFACE_VERSION {
            return Err(RegistrationRefusal::InterfaceVersion {
                expected: FEDERATION_BUNDLE_INTERFACE_VERSION,
                found: install.interface_version,
            });
        }
        Ok(Self {
            install_id: install.id,
            name: install.name.clone(),
            // `Enabled` is the ONLY status that authenticates anyone; a
            // row adopted but never switched on registers as not-enabled,
            // which is what makes approval ≠ activation.
            enabled: install.status == PluginInstallStatus::Enabled,
            jit_enabled: install.jit_enabled,
        })
    }
}

/// The OIDC half of an installed bundle.
pub struct OidcIdentityBundle(pub RegisteredIdentityBundle);

impl FederationBundle for OidcIdentityBundle {
    fn kind(&self) -> &'static str {
        kind::OIDC
    }

    fn id(&self) -> Uuid {
        self.0.install_id
    }

    fn name(&self) -> &str {
        &self.0.name
    }

    fn enabled(&self) -> bool {
        self.0.enabled
    }

    fn jit_enabled(&self) -> bool {
        self.0.jit_enabled
    }
}

/// The edge-assertion half of an installed bundle.
pub struct EdgeAssertionIdentityBundle(pub RegisteredIdentityBundle);

impl FederationBundle for EdgeAssertionIdentityBundle {
    fn kind(&self) -> &'static str {
        kind::EDGE_ASSERTION
    }

    fn id(&self) -> Uuid {
        self.0.install_id
    }

    fn name(&self) -> &str {
        &self.0.name
    }

    fn enabled(&self) -> bool {
        self.0.enabled
    }

    fn jit_enabled(&self) -> bool {
        self.0.jit_enabled
    }
}

/// The bundle registry the router consults. It holds trait objects, so a
/// discovered artifact registers beside any compiled-in source without
/// either side knowing the other.
///
/// Deny-by-default by construction: `find` and `active` only ever return a
/// bundle whose install is `enabled`. A disabled bundle stays in the
/// registry so the admin surface can list what is installed but switched
/// off, and it cannot authenticate while it sits there.
#[derive(Default)]
pub struct IdentityBundleRegistry {
    bundles: Vec<Box<dyn FederationBundle>>,
}

impl IdentityBundleRegistry {
    pub fn new() -> Self {
        Self {
            bundles: Vec::new(),
        }
    }

    /// Registers every transport an install declared. One install may
    /// declare both kinds (an org running OIDC and an edge proxy), so a
    /// successful call can add two entries sharing one source id.
    ///
    /// A revoked or version-mismatched install adds none and reports why.
    pub fn register_install(
        &mut self,
        install: &IdentityBundleInstall,
    ) -> Result<usize, RegistrationRefusal> {
        let registered = RegisteredIdentityBundle::from_install(install)?;
        let mut added = 0;
        for label in &install.provider_kinds {
            let Some(provider_kind) = IdentityProviderKind::parse(label) else {
                // A row the approval path wrote cannot carry an unknown
                // kind; if one does, it came from somewhere else and
                // registering it would attach an unnamed transport.
                continue;
            };
            match provider_kind {
                IdentityProviderKind::Oidc => self
                    .bundles
                    .push(Box::new(OidcIdentityBundle(registered.clone()))),
                IdentityProviderKind::EdgeAssertion => self
                    .bundles
                    .push(Box::new(EdgeAssertionIdentityBundle(registered.clone()))),
            }
            added += 1;
        }
        Ok(added)
    }

    /// Builds a registry from an org's install rows, skipping refusals.
    /// A caller that needs the refusals (a boot-time report) uses
    /// [`Self::register_install`] directly.
    pub fn from_installs(installs: &[IdentityBundleInstall]) -> Self {
        let mut registry = Self::new();
        for install in installs {
            let _ = registry.register_install(install);
        }
        registry
    }

    pub fn len(&self) -> usize {
        self.bundles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bundles.is_empty()
    }

    /// The bundles a login attempt may route to: enabled only.
    pub fn active(&self) -> impl Iterator<Item = &dyn FederationBundle> {
        self.bundles
            .iter()
            .map(|b| b.as_ref())
            .filter(|b| b.enabled())
    }

    /// The bundle a `LinkNamespace::Bundle(kind, id)` names, if it exists
    /// and is switched on. A disabled bundle is indistinguishable from an
    /// absent one here — the denial the trait documents.
    pub fn find(&self, kind_label: &str, id: Uuid) -> Option<&dyn FederationBundle> {
        self.bundles
            .iter()
            .map(|b| b.as_ref())
            .find(|b| b.kind() == kind_label && b.id() == id && b.enabled())
    }
}

/// The bridge from a registered bundle to the EXISTING login seam:
/// `auth/provider.rs::resolve_and_issue` takes a [`ProviderContext`] and a
/// verified principal, links-or-refuses, applies the org MFA gate, and mints
/// the session with the same token machinery a password login uses.
///
/// A bundle therefore does not get its own resolve path. It supplies a
/// namespace (its kind + install id), an org, an audit label, and its JIT
/// opt-in — and the core does the rest. `jit_allowed` starts false because
/// the install starts with JIT off; turning it on is a separate admin act.
pub fn provider_context(bundle: &dyn FederationBundle, org_id: Uuid) -> ProviderContext {
    ProviderContext {
        namespace: LinkNamespace::Bundle(bundle.kind(), bundle.id()),
        org_id,
        // The audit label is a `'static` contract value, so it names the
        // TRANSPORT rather than the operator's bundle name — an
        // operator-supplied string must never become an audit action's
        // provenance.
        display_source: match bundle.kind() {
            kind::OIDC => "oidc",
            kind::EDGE_ASSERTION => "edge_assertion",
            _ => "identity_bundle",
        },
        jit_allowed: bundle.jit_enabled(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_version_is_pinned_and_kinds_are_distinct() {
        assert_eq!(FEDERATION_BUNDLE_INTERFACE_VERSION, 1);
        assert_ne!(kind::OIDC, kind::EDGE_ASSERTION);
    }
}

#[cfg(test)]
mod identity_bundle_registry_tests {
    use super::*;
    use core_domain::entities::identity_bundle::IdentityBundleManifest;

    fn install(status: PluginInstallStatus, kinds: &[&str]) -> IdentityBundleInstall {
        let manifest = IdentityBundleManifest::from_value(&serde_json::json!({
            "name": "corp-okta",
            "version": "1.4.0",
            "interface_version": FEDERATION_BUNDLE_INTERFACE_VERSION,
            "provider_kinds": kinds,
            "settings_schema": {"type": "object", "properties": {}},
            "scopes": ["openid"]
        }))
        .expect("manifest");
        let mut row = IdentityBundleInstall::from_approved_staged(
            Uuid::from_u128(7),
            Uuid::from_u128(1),
            &manifest,
            serde_json::json!({}),
            "local_zip",
            "local://zip-import",
            None,
            &"cd".repeat(32),
            Uuid::from_u128(2),
            chrono::Utc::now(),
        )
        .expect("install");
        row.status = status;
        row
    }

    /// The core contract with `auth/provider.rs`: an identity bundle adds
    /// an identity SOURCE and no authority. The registry exposes no token,
    /// no session and no capability surface, and the plugin vocabulary
    /// still has no `auth:*` key for one to hold.
    #[test]
    fn registration_confers_a_source_and_no_authority() {
        for cap in core_domain::entities::plugins::PluginCapability::ALL {
            assert!(!cap.as_str().starts_with("auth"));
        }
        let registry = IdentityBundleRegistry::from_installs(&[install(
            PluginInstallStatus::Enabled,
            &["oidc"],
        )]);
        assert_eq!(registry.len(), 1);
        let bundle = registry
            .find(kind::OIDC, Uuid::from_u128(7))
            .expect("an enabled bundle is findable");
        assert_eq!(bundle.name(), "corp-okta");
        assert!(!bundle.jit_enabled(), "JIT is off unless opted in");
    }

    #[test]
    fn approval_is_not_activation() {
        let registry = IdentityBundleRegistry::from_installs(&[install(
            PluginInstallStatus::Disabled,
            &["oidc"],
        )]);
        // Present (the admin surface can list it) but unreachable.
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.active().count(), 0);
        assert!(registry.find(kind::OIDC, Uuid::from_u128(7)).is_none());
    }

    #[test]
    fn a_revoked_install_never_registers_and_reports_why() {
        let mut registry = IdentityBundleRegistry::new();
        let err = registry
            .register_install(&install(PluginInstallStatus::Revoked, &["oidc"]))
            .expect_err("revoked refuses");
        assert_eq!(err, RegistrationRefusal::Revoked);
        assert!(registry.is_empty());
    }

    #[test]
    fn interface_version_drift_is_refused_at_registration() {
        let mut row = install(PluginInstallStatus::Enabled, &["oidc"]);
        row.interface_version = FEDERATION_BUNDLE_INTERFACE_VERSION + 1;
        let mut registry = IdentityBundleRegistry::new();
        let err = registry.register_install(&row).expect_err("drift refuses");
        assert_eq!(
            err,
            RegistrationRefusal::InterfaceVersion {
                expected: FEDERATION_BUNDLE_INTERFACE_VERSION,
                found: FEDERATION_BUNDLE_INTERFACE_VERSION + 1,
            }
        );
    }

    #[test]
    fn one_install_may_declare_both_transports() {
        let registry = IdentityBundleRegistry::from_installs(&[install(
            PluginInstallStatus::Enabled,
            &["oidc", "edge_assertion"],
        )]);
        assert_eq!(registry.len(), 2);
        assert!(registry.find(kind::OIDC, Uuid::from_u128(7)).is_some());
        assert!(registry
            .find(kind::EDGE_ASSERTION, Uuid::from_u128(7))
            .is_some());
        // The same source id under an unknown kind is absent, so a caller
        // cannot address an edge bundle through an OIDC-shaped route.
        assert!(registry.find("saml", Uuid::from_u128(7)).is_none());
    }

    /// The domain owns the provider-kind vocabulary (plugin-host validates
    /// manifests and cannot depend on this crate), so the two must agree.
    /// Drift here would let a manifest validate against a transport the
    /// registry cannot attach.
    #[test]
    fn domain_provider_kinds_match_the_federation_kind_constants() {
        assert_eq!(IdentityProviderKind::Oidc.as_str(), kind::OIDC);
        assert_eq!(
            IdentityProviderKind::EdgeAssertion.as_str(),
            kind::EDGE_ASSERTION
        );
        assert_eq!(IdentityProviderKind::ALL.len(), 2);
    }

    /// The executable half rides the EXISTING login seam: a registered
    /// bundle yields a `ProviderContext` that `resolve_and_issue` accepts,
    /// carries the install id as its source id, and inherits the install's
    /// JIT opt-in rather than assuming one.
    #[test]
    fn a_registered_bundle_produces_the_shared_login_context() {
        let registry = IdentityBundleRegistry::from_installs(&[install(
            PluginInstallStatus::Enabled,
            &["oidc", "edge_assertion"],
        )]);
        let org_id = Uuid::from_u128(1);
        let oidc = registry
            .find(kind::OIDC, Uuid::from_u128(7))
            .expect("oidc bundle");
        let ctx = provider_context(oidc, org_id);
        assert_eq!(ctx.org_id, org_id);
        assert_eq!(ctx.namespace.source_id(), Uuid::from_u128(7));
        assert!(matches!(
            ctx.namespace,
            LinkNamespace::Bundle(k, _) if k == kind::OIDC
        ));
        // JIT is off because the install's opt-in is off.
        assert!(!ctx.jit_allowed);
        // A `'static` transport label, never the operator's bundle name.
        assert_eq!(ctx.display_source, "oidc");

        let edge = registry
            .find(kind::EDGE_ASSERTION, Uuid::from_u128(7))
            .expect("edge bundle");
        assert_eq!(
            provider_context(edge, org_id).display_source,
            "edge_assertion"
        );
    }

    /// JIT follows the per-install flag and nothing else — the seam does
    /// not infer an opt-in from enablement.
    #[test]
    fn jit_follows_the_install_flag_not_enablement() {
        let mut row = install(PluginInstallStatus::Enabled, &["oidc"]);
        row.jit_enabled = true;
        let registry = IdentityBundleRegistry::from_installs(&[row]);
        let bundle = registry
            .find(kind::OIDC, Uuid::from_u128(7))
            .expect("bundle");
        assert!(provider_context(bundle, Uuid::from_u128(1)).jit_allowed);
    }
}
