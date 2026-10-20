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
    const KIND: &'static str;
    /// Interface version this bundle implements.
    const INTERFACE_VERSION: u32 = FEDERATION_BUNDLE_INTERFACE_VERSION;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_version_is_pinned_and_kinds_are_distinct() {
        assert_eq!(FEDERATION_BUNDLE_INTERFACE_VERSION, 1);
        assert_ne!(kind::OIDC, kind::EDGE_ASSERTION);
    }
}
