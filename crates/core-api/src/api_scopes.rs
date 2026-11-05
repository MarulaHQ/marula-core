//! The machine-credential scope vocabulary, and the operations it opens.
//!
//! Purpose: one place that answers three questions the API used to answer
//! in prose only — which scopes an org may write onto an API key, what
//! permission each one grants, and which documented operations a key
//! holding it can actually reach. The OpenAPI document publishes this
//! module verbatim (`components.securitySchemes`, `x-scope-catalog`,
//! per-operation `x-api-key-scopes`), and the key-creation validator reads
//! the same list, so a scope cannot exist on one side and be absent from
//! the other.
//!
//! Non-purpose: no enforcement. The auth middleware still decides what a
//! scope grants on a request (`permissions_from_scopes`), and every handler
//! still runs its own permission and record-level gates. This module
//! describes that decision; it does not make it.

use core_domain::policy::Permission;
use core_domain::policy::{Action, Module};

/// One scope an organization can write onto an API key.
pub struct ApiScope {
    /// The wire name, e.g. `observability:read`.
    pub name: &'static str,
    /// What holding it lets a machine do. Published to API consumers.
    pub description: &'static str,
    /// Exactly the permissions the auth middleware derives from it. The
    /// parity test in `auth::middleware` compares this to the resolver, so
    /// a scope that grants nothing cannot ship.
    pub grants: &'static [Permission],
}

const fn matrix(module: Module, action: Action) -> Permission {
    Permission::Matrix { module, action }
}

/// Every mintable scope, in the order the API-key validator reports them.
///
/// Read and write are separate scopes on purpose: a write key does not
/// imply its read half, so a connector that only pushes never receives the
/// read-back surface. The org-namespaced scopes (`org_usage:read`,
/// `observability:read`, the custom-domain and directory pairs) are the
/// narrowest grants in the list — each unlocks one read or one management
/// surface and no matrix peer.
pub const SCOPES: &[ApiScope] = &[
    ApiScope {
        name: "tickets:read",
        description: "Read tickets, their comments, checklists, links and time entries.",
        grants: &[matrix(Module::Tickets, Action::View)],
    },
    ApiScope {
        name: "tickets:write",
        description: "Read tickets and create or edit them, including status changes and comments.",
        grants: &[
            matrix(Module::Tickets, Action::View),
            matrix(Module::Tickets, Action::Create),
            matrix(Module::Tickets, Action::Edit),
        ],
    },
    ApiScope {
        name: "tickets_correspondence:read",
        description: "Read a ticket's email correspondence. Its own scope so a connector can mirror mail without a general ticket write path.",
        grants: &[matrix(Module::Tickets, Action::View)],
    },
    ApiScope {
        name: "clients:read",
        description: "Read clients, sites, contacts, assets and configuration items.",
        grants: &[matrix(Module::Clients, Action::View)],
    },
    ApiScope {
        name: "clients:write",
        description: "Read clients and create or edit clients, sites, contacts, assets and configuration items.",
        grants: &[
            matrix(Module::Clients, Action::View),
            matrix(Module::Clients, Action::Create),
            matrix(Module::Clients, Action::Edit),
        ],
    },
    // No `timers:*` scope, deliberately. A timer is the signed-in person's
    // own work and the handlers read `principal.user.id`; a machine identity
    // has no timer of its own. Minting the scope produced a credential that
    // authenticated and was then refused by the path gate, so the published
    // pair promised a surface no key could reach.
    ApiScope {
        name: "migration:read",
        description: "Read the migration connector's checked-out data families. The read half of a pull.",
        grants: &[matrix(Module::Clients, Action::View)],
    },
    ApiScope {
        name: "migration:write",
        description: "Push whole entity families through the migration engine, and drive import batches.",
        grants: &[
            matrix(Module::Clients, Action::View),
            matrix(Module::Clients, Action::Create),
            matrix(Module::Clients, Action::Edit),
        ],
    },
    ApiScope {
        name: "dispatch:read",
        description: "Read the dispatch connector's schedule and technician roster.",
        grants: &[matrix(Module::Scheduling, Action::View)],
    },
    ApiScope {
        name: "dispatch:write",
        description: "Read the dispatch connector and push field-service status changes.",
        grants: &[
            matrix(Module::Scheduling, Action::View),
            matrix(Module::Scheduling, Action::Create),
            matrix(Module::Scheduling, Action::Edit),
        ],
    },
    ApiScope {
        name: "org_usage:read",
        description: "Read the caller's own org usage counts and configured quotas. One read, no reporting surface.",
        grants: &[Permission::ViewOrgUsage],
    },
    ApiScope {
        name: "rag:read",
        description: "Query the vector namespace and verify deletion receipts.",
        grants: &[matrix(Module::Kb, Action::View)],
    },
    ApiScope {
        name: "rag:write",
        description: "Ingest and delete vector sources on top of the read surface.",
        grants: &[
            matrix(Module::Kb, Action::View),
            matrix(Module::Kb, Action::Create),
            matrix(Module::Kb, Action::Edit),
            matrix(Module::Kb, Action::Delete),
        ],
    },
    ApiScope {
        name: "org_custom_domain:read",
        description: "Read the caller's own org custom-domain state. The read half a reverse proxy needs.",
        grants: &[Permission::ViewOrgCustomDomain],
    },
    ApiScope {
        name: "org_custom_domain:manage",
        description: "Set, verify and remove the caller's own org custom domain on its behalf.",
        grants: &[Permission::ManageOrgCustomDomain],
    },
    ApiScope {
        name: "org_suspension:read",
        description: "Read the caller's own org suspension notice state. Read-only: setting the flag stays an admin act.",
        grants: &[Permission::ViewOrgSuspension],
    },
    ApiScope {
        name: "observability:read",
        description: "Read the org audit trail, including the forward-only export cursor. Append-only: no write half exists.",
        grants: &[Permission::ViewObservability],
    },
    ApiScope {
        name: "directory_groups:read",
        description: "Read directory groups, their members and their role mappings.",
        grants: &[Permission::ViewDirectoryGroups],
    },
    ApiScope {
        name: "directory_groups:write",
        description: "Create, update and delete directory groups, memberships and role mappings.",
        grants: &[Permission::ManageDirectoryGroups],
    },
];

/// The scope with this wire name, if one exists.
pub fn get(name: &str) -> Option<&'static ApiScope> {
    SCOPES.iter().find(|s| s.name == name)
}

/// Every mintable scope name, for validation and error messages.
pub fn names() -> impl Iterator<Item = &'static str> {
    SCOPES.iter().map(|s| s.name)
}

/// A documented operation a machine key can reach, and the scopes that
/// grant it.
///
/// `prefix` is the canonical `/api/v1` path as it appears in the document
/// (braces included, e.g. `/api/v1/orgs/{org_id}/usage`). An empty
/// `methods` list means every method; a rule naming methods must come
/// before the all-methods rule for the same prefix, because the first
/// match wins.
pub struct MachineRule {
    pub prefix: &'static str,
    pub methods: &'static [&'static str],
    pub scopes: &'static [&'static str],
}

/// The key-reachable operations, most specific first.
///
/// Sources: the API-key path allowance in [`crate::auth::middleware`] and
/// the permission each handler checks. A path that appears here without a
/// scope is not automatically machine-reachable — the rule is the claim,
/// and `openapi::tests` checks every rule against the built document so a
/// renamed route cannot leave a stale entry behind.
pub const RULES: &[MachineRule] = &[
    // Org sub-resources are narrower than their prefix, so they come first.
    MachineRule {
        prefix: "/api/v1/orgs/{org_id}/usage",
        methods: &["get"],
        scopes: &["org_usage:read"],
    },
    MachineRule {
        prefix: "/api/v1/orgs/{org_id}/quotas",
        methods: &["get"],
        scopes: &["org_usage:read"],
    },
    MachineRule {
        prefix: "/api/v1/orgs/{org_id}/suspension-status",
        methods: &["get"],
        scopes: &["org_suspension:read"],
    },
    MachineRule {
        prefix: "/api/v1/orgs/{org_id}/custom-domain",
        methods: &["get"],
        scopes: &["org_custom_domain:read"],
    },
    MachineRule {
        prefix: "/api/v1/orgs/{org_id}/custom-domain",
        methods: &[],
        scopes: &["org_custom_domain:manage"],
    },
    // The audit trail: one read, one export contract.
    MachineRule {
        prefix: "/api/v1/audit-events",
        methods: &["get"],
        scopes: &["observability:read"],
    },
    // RAG: `query` is a POST that only reads, so it precedes the write rule.
    MachineRule {
        prefix: "/api/v1/rag/query",
        methods: &["post"],
        scopes: &["rag:read"],
    },
    MachineRule {
        prefix: "/api/v1/rag",
        methods: &["get"],
        scopes: &["rag:read"],
    },
    MachineRule {
        prefix: "/api/v1/rag",
        methods: &[],
        scopes: &["rag:write"],
    },
    MachineRule {
        prefix: "/api/v1/directory-groups",
        methods: &["get"],
        scopes: &["directory_groups:read"],
    },
    MachineRule {
        prefix: "/api/v1/directory-groups",
        methods: &[],
        scopes: &["directory_groups:write"],
    },
    // Ticket correspondence is its own capability; ordinary ticket read also
    // satisfies the handler, so either scope is enough.
    MachineRule {
        prefix: "/api/v1/tickets/{id}/emails",
        methods: &["get"],
        scopes: &["tickets_correspondence:read", "tickets:read"],
    },
    MachineRule {
        prefix: "/api/v1/tickets",
        methods: &["get"],
        scopes: &["tickets:read"],
    },
    MachineRule {
        prefix: "/api/v1/tickets",
        methods: &[],
        scopes: &["tickets:write"],
    },
    MachineRule {
        prefix: "/api/v1/clients",
        methods: &["get"],
        scopes: &["clients:read"],
    },
    MachineRule {
        prefix: "/api/v1/clients",
        methods: &[],
        scopes: &["clients:write"],
    },
    // Configuration items ride the Clients matrix, not a module of their own.
    MachineRule {
        prefix: "/api/v1/config-items",
        methods: &["get"],
        scopes: &["clients:read"],
    },
    MachineRule {
        prefix: "/api/v1/config-items",
        methods: &[],
        scopes: &["clients:write"],
    },
    // Import batches demand Create and Edit together on the machine path, so
    // either write scope satisfies them and no read scope does.
    MachineRule {
        prefix: "/api/v1/imports",
        methods: &[],
        scopes: &["clients:write", "migration:write"],
    },
    MachineRule {
        prefix: "/api/v1/migration",
        methods: &["get"],
        scopes: &["migration:read"],
    },
    MachineRule {
        prefix: "/api/v1/migration",
        methods: &[],
        scopes: &["migration:write"],
    },
    MachineRule {
        prefix: "/api/v1/dispatch-connector",
        methods: &["get"],
        scopes: &["dispatch:read"],
    },
    MachineRule {
        prefix: "/api/v1/dispatch-connector",
        methods: &[],
        scopes: &["dispatch:write"],
    },
    // `/api/v1/timers` is absent for the same reason it is absent from
    // `auth::middleware::API_KEY_PATHS`: a machine key cannot reach it, and a
    // scope published here would describe access the middleware refuses.
];

/// Whether `path` is `prefix` itself or a child of it.
fn under(prefix: &str, path: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The scopes that authorize a machine caller at this operation, if any.
///
/// `None` means no mintable scope grants what the operation requires: a key
/// may authenticate and will then be refused. That distinction is the point
/// of publishing this at all.
pub fn scopes_for(path: &str, method: &str) -> Option<&'static [&'static str]> {
    let method = method.to_ascii_lowercase();
    RULES
        .iter()
        .find(|rule| {
            under(rule.prefix, path)
                && (rule.methods.is_empty() || rule.methods.contains(&method.as_str()))
        })
        .map(|rule| rule.scopes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scope_name_is_unique_and_namespaced() {
        let mut seen = std::collections::HashSet::new();
        for scope in SCOPES {
            assert!(seen.insert(scope.name), "duplicate scope {}", scope.name);
            assert!(
                scope.name.split_once(':').is_some(),
                "{} is not module:action shaped",
                scope.name
            );
            assert!(
                !scope.description.trim().is_empty(),
                "{} publishes no description",
                scope.name
            );
        }
    }

    #[test]
    fn every_rule_names_a_mintable_scope_and_a_canonical_path() {
        for rule in RULES {
            assert!(
                rule.prefix.starts_with("/api/v1/"),
                "{} is not canonical",
                rule.prefix
            );
            for scope in rule.scopes {
                assert!(get(scope).is_some(), "{} is not a mintable scope", scope);
            }
            for method in rule.methods {
                assert_eq!(*method, method.to_ascii_lowercase());
            }
        }
    }

    /// A timer belongs to the signed-in person (`principal.user.id`), and the
    /// middleware's path gate refuses `/api/timers` for keys. The scope pair
    /// that used to be published promised a surface no credential could reach
    /// (F102), so both halves are asserted: the name is gone from the
    /// vocabulary, and no rule claims the path.
    #[test]
    fn the_timer_surface_is_not_published_to_machine_keys() {
        assert!(get("timers:read").is_none());
        assert!(get("timers:write").is_none());
        for method in ["get", "post"] {
            assert!(
                scopes_for("/api/v1/timers/active", method).is_none(),
                "{method} /api/v1/timers/active must not advertise a key scope"
            );
            assert!(
                scopes_for("/api/v1/timers/working-set", method).is_none(),
                "{method} /api/v1/timers/working-set must not advertise a key scope"
            );
        }
    }

    #[test]
    fn the_specific_rule_wins_over_the_family_rule() {
        assert_eq!(
            scopes_for("/api/v1/rag/query", "post"),
            Some(["rag:read"].as_slice())
        );
        assert_eq!(
            scopes_for("/api/v1/rag/chunks", "post"),
            Some(["rag:write"].as_slice())
        );
        assert_eq!(
            scopes_for("/api/v1/orgs/{org_id}/usage", "get"),
            Some(["org_usage:read"].as_slice())
        );
        // A child of the custom-domain prefix inherits its manage half.
        assert_eq!(
            scopes_for("/api/v1/orgs/{org_id}/custom-domain/verify", "post"),
            Some(["org_custom_domain:manage"].as_slice())
        );
        // And an operation no scope grants answers `None`, not an empty list.
        assert_eq!(scopes_for("/api/v1/agent-proposals", "get"), None);
    }
}
