//! The plugin-runtime SANDBOX SEAM (Wave 14 lane 5) — the versioned
//! contract every plugin runtime implements and the deny-by-default
//! authorization every invocation MUST route through. This mirrors the
//! identity-federation bundle interface posture
//! (`FEDERATION_BUNDLE_INTERFACE_VERSION`, core-api
//! `auth::federation`): versioned interface, deny-by-default,
//! admin-enabled, audited lifecycle.
//!
//! Doctrine (non-negotiable):
//! * **No eval-style dynamic loading.** A plugin is a DECLARATIVE
//!   manifest (name/version/capabilities — data, never code) adopted
//!   fork-like from the marketplace registry catalog. Nothing in this
//!   seam accepts, compiles, or executes inline code; the v1 runtime
//!   seam below authorizes invocations and resolves grants — the
//!   Extism WASM host lands behind this same interface later.
//! * **Capability grants are DATA.** The closed capability vocabulary
//!   (`core_domain::entities::plugins::PluginCapability`) maps onto the
//!   EXISTING permission matrix — a grant resolves to a standard
//!   `Permission` and every authorization flows through
//!   `core_domain::policy` like any human actor's. No parallel auth
//!   system.
//! * **Deny-by-default, admin-enabled.** An install is created
//!   DISABLED; only an explicit admin action enables it. A revoked
//!   catalog entry or install is terminal. An unknown capability, a
//!   grant outside the manifest's declared set, and an interface-
//!   version mismatch are all rejections — nothing authorizes by
//!   default.
//! * **Audit events cover every lifecycle step** (catalog entry
//!   published/revoked, install created/enabled/disabled/uninstalled/
//!   revoked) — emitted by the route layer through the existing
//!   append-only `audit_events` table.

pub mod distribution;
pub mod surface;

use core_domain::entities::{
    PluginCapability, PluginInstall, PluginInstallStatus, PLUGIN_RUNTIME_INTERFACE_VERSION,
};
use core_domain::policy::{self, Permission};

/// Version of the runtime interface contract. Bump on any breaking
/// change to the manifest v2 shape or the capability vocabulary;
/// catalog entries declare the version they were published against and
/// the runtime refuses mixed versions.
pub const PLUGIN_HOST_INTERFACE_VERSION: u32 = PLUGIN_RUNTIME_INTERFACE_VERSION;

/// Static reasons an invocation can be refused. They surface in audit
/// details and (for JSON surfaces) as typed envelope reason codes —
/// never as prose to parse.
pub mod denial_reasons {
    pub const ENTRY_REVOKED: &str = "plugin_entry_revoked";
    pub const INSTALL_REVOKED: &str = "plugin_install_revoked";
    pub const INSTALL_DISABLED: &str = "plugin_install_disabled";
    pub const INTERFACE_MISMATCH: &str = "plugin_interface_mismatch";
    pub const CAPABILITY_NOT_GRANTED: &str = "plugin_capability_not_granted";
}

/// A plugin runtime: one sandboxed execution context attached to the
/// invocation machinery. Implementations exist per host kind; the
/// router owns the registry so a disabled or unknown runtime simply has
/// no route that can execute (deny-by-default by construction).
pub trait PluginRuntime: Send + Sync {
    /// Machine kind of the host (e.g. `"wasm"` for the future Extism
    /// host).
    const KIND: &'static str;
    /// Interface version this runtime implements.
    const INTERFACE_VERSION: u32 = PLUGIN_HOST_INTERFACE_VERSION;

    fn id(&self) -> uuid::Uuid;
    fn name(&self) -> &str;
    /// Admin-enabled gate. `false` = the runtime never executes
    /// anything, whatever its protocol state.
    fn enabled(&self) -> bool;
}

/// The sandbox decision for one invocation: the resolved
/// [`Permission`] the capability maps onto, once every deny-by-default
/// gate has passed. Enforcement stays in the standard policy path —
/// the caller runs `policy::check(install.granted_permissions(), …)`
/// with the SAME granted set, exactly as for a human actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationGrant {
    pub permission: Permission,
    pub capability: PluginCapability,
}

/// Authorizes one invocation of `required` under `install`.
///
/// Order of gates (each denial is a distinct typed reason — never
/// prose):
/// 1. interface version: the catalog entry the install adopted must
///    speak the runtime's interface version;
/// 2. registry revocation: a revoked catalog entry (or a revoked
///    install) is terminal;
/// 3. admin-enabled: a DISABLED install never runs;
/// 4. grant check: the capability must be in the install's granted
///    set — itself a subset of the pinned manifest's declared set
///    (enforced at the write boundary);
/// 5. resolution: the capability maps onto the existing permission
///    matrix. No step invents authority.
pub fn authorize_invocation(
    install: &PluginInstall,
    entry_revoked: bool,
    entry_interface_version: u32,
    required: PluginCapability,
) -> Result<InvocationGrant, &'static str> {
    if entry_interface_version != PLUGIN_HOST_INTERFACE_VERSION {
        return Err(denial_reasons::INTERFACE_MISMATCH);
    }
    if entry_revoked {
        return Err(denial_reasons::ENTRY_REVOKED);
    }
    match install.status {
        PluginInstallStatus::Revoked => return Err(denial_reasons::INSTALL_REVOKED),
        PluginInstallStatus::Disabled => return Err(denial_reasons::INSTALL_DISABLED),
        PluginInstallStatus::Enabled => {}
    }
    let key = required.as_str();
    if !install.granted_capabilities.iter().any(|g| g == key) {
        return Err(denial_reasons::CAPABILITY_NOT_GRANTED);
    }
    Ok(InvocationGrant {
        permission: required.permission(),
        capability: required,
    })
}

/// The granted permission set of an install, for the standard policy
/// path (`policy::check` / `check_any`) — the same vocabulary a human
/// role carries.
pub fn granted_permissions(install: &PluginInstall) -> Vec<Permission> {
    install.granted_permissions()
}

/// Convenience: is `required` allowed under `install`'s grants by the
/// standard policy check? (Sandbox preconditions are
/// [`authorize_invocation`]'s job; this is the matrix half.)
pub fn policy_allows(install: &PluginInstall, required: PluginCapability) -> bool {
    policy::check(&granted_permissions(install), required.permission()).is_allowed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn sample_install(status: PluginInstallStatus, grants: &[&str]) -> PluginInstall {
        PluginInstall {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            catalog_id: Uuid::now_v7(),
            slug: "ticket-summarizer".into(),
            version: "1.0.0".into(),
            status,
            granted_capabilities: grants.iter().map(|s| s.to_string()).collect(),
            installed_by: None,
            installed_at: Utc::now(),
            updated_at: Utc::now(),
            revoked_at: None,
            revoked_by: None,
        }
    }

    #[test]
    fn disabled_install_should_be_denied_even_with_grants() {
        let install = sample_install(PluginInstallStatus::Disabled, &["tickets.read"]);
        let err = authorize_invocation(
            &install,
            false,
            PLUGIN_HOST_INTERFACE_VERSION,
            PluginCapability::TicketsRead,
        )
        .expect_err("disabled must deny");
        assert_eq!(err, denial_reasons::INSTALL_DISABLED);
    }

    #[test]
    fn revoked_entry_and_revoked_install_should_deny_distinctly() {
        let install = sample_install(PluginInstallStatus::Enabled, &["tickets.read"]);
        assert_eq!(
            authorize_invocation(
                &install,
                true,
                PLUGIN_HOST_INTERFACE_VERSION,
                PluginCapability::TicketsRead
            )
            .expect_err("revoked entry must deny"),
            denial_reasons::ENTRY_REVOKED
        );
        let revoked = sample_install(PluginInstallStatus::Revoked, &["tickets.read"]);
        assert_eq!(
            authorize_invocation(
                &revoked,
                false,
                PLUGIN_HOST_INTERFACE_VERSION,
                PluginCapability::TicketsRead
            )
            .expect_err("revoked install must deny"),
            denial_reasons::INSTALL_REVOKED
        );
    }

    #[test]
    fn interface_mismatch_should_deny_before_anything_else() {
        let install = sample_install(PluginInstallStatus::Enabled, &["tickets.read"]);
        assert_eq!(
            authorize_invocation(
                &install,
                false,
                PLUGIN_HOST_INTERFACE_VERSION + 1,
                PluginCapability::TicketsRead
            )
            .expect_err("mismatch must deny"),
            denial_reasons::INTERFACE_MISMATCH
        );
    }

    #[test]
    fn ungranted_capability_should_deny_and_granted_should_resolve_to_matrix() {
        let install = sample_install(PluginInstallStatus::Enabled, &["tickets.read"]);
        assert_eq!(
            authorize_invocation(
                &install,
                false,
                PLUGIN_HOST_INTERFACE_VERSION,
                PluginCapability::TicketsWrite
            )
            .expect_err("ungranted must deny"),
            denial_reasons::CAPABILITY_NOT_GRANTED
        );
        let grant = authorize_invocation(
            &install,
            false,
            PLUGIN_HOST_INTERFACE_VERSION,
            PluginCapability::TicketsRead,
        )
        .expect("granted must authorize");
        assert_eq!(
            grant.permission,
            Permission::Matrix {
                module: core_domain::policy::Module::Tickets,
                action: core_domain::policy::Action::View
            }
        );
        assert!(policy_allows(&install, PluginCapability::TicketsRead));
        assert!(!policy_allows(&install, PluginCapability::TicketsWrite));
    }

    #[test]
    fn financial_grant_should_map_onto_financial_read() {
        let install = sample_install(PluginInstallStatus::Enabled, &["financial.read"]);
        let grant = authorize_invocation(
            &install,
            false,
            PLUGIN_HOST_INTERFACE_VERSION,
            PluginCapability::FinancialRead,
        )
        .expect("financial.read grant must authorize");
        assert_eq!(grant.permission, Permission::ViewFinancials);
    }

    #[test]
    fn interface_version_is_pinned() {
        assert_eq!(PLUGIN_HOST_INTERFACE_VERSION, 1);
    }
}
