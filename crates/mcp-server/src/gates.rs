//! Who may see a tool: the caller's own half of the filter.
//!
//! The org's capability grants say which families this organization enabled;
//! they say nothing about the caller in front of the adapter. A bound API key
//! is scoped by its service account's scopes, and a staff session by its role,
//! so a tool can be granted to the org and still be refused by the handler that
//! serves it. This module carries the second half: for every capability
//! namespace in the catalog, the permission the handler behind it checks.
//!
//! Two things are load-bearing here.
//!
//! - **The table is evidence, not preference.** Every row cites the handler
//!   that enforces it (see the lane's `CAPABILITY-MAP.md`), and a namespace
//!   whose tools do not share one gate is not squeezed into a guess: the
//!   namespace carries the common case and [`TOOL_GATES`] names the exceptions
//!   one tool at a time.
//! - **The filter never hides a tool the server would serve.** `Gate::All` and
//!   `Gate::Any` mirror the handler's own check exactly. Where a handler asks a
//!   question this table cannot answer — a role test, which `/api/v1/me` does
//!   not publish — the row names the matrix grant that role implies
//!   (`admin:view`, held by exactly the owner and admin presets), which is a
//!   superset of the passing set: a caller the handler would accept is never
//!   hidden.
//!
//! The list stays advisory. `tools/call` resolves against the full catalog and
//! the handler's own check remains the authorization boundary; this only stops
//! the adapter advertising what that boundary will refuse.

use crate::catalog::Tool;

/// One row of the caller-side filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// The handler checks no permission: an authenticated session is enough.
    /// Record scoping still applies, which is why these stay visible to every
    /// caller rather than to none.
    Session,
    /// The caller must hold every one of these permission keys.
    All(&'static [&'static str]),
    /// The caller must hold at least one of these permission keys.
    Any(&'static [&'static str]),
    /// The handler branches on the credential kind (`authed.via_api_key`):
    /// a machine key must hold `machine`, a human session `human`. An empty
    /// side means that kind of caller needs no permission for this read.
    Subject {
        machine: &'static [&'static str],
        human: &'static [&'static str],
    },
}

// The permission keys are `core_domain::policy::Permission::key()` strings: a
// matrix grant is `module:action`, a special is `special:<name>`, and the four
// module-namespaced reads carry their own names (`financial:read`,
// `org_usage:read`, `observability:read`, `directory_groups:read`, …). The
// adapter compares strings rather than linking the domain crate, so this file
// can be read beside the catalog without a new dependency.

/// What the caller must hold for a namespace's tools to appear.
///
/// One row per capability namespace in `packages/sdk/mcp-catalog.json`. A row
/// is the gate every tool in that family shares; the families where that is not
/// true are listed in [`TOOL_GATES`] and the namespace row holds the common
/// case.
pub const NAMESPACE_GATES: &[(&str, Gate)] = &[
    ("admin", Gate::All(&["admin:view"])),
    ("agent-proposals", Gate::All(&["admin:view"])),
    ("agent-skills", Gate::All(&["admin:view"])),
    ("ai", Gate::Session),
    ("alert-sources", Gate::All(&["special:manage_plugins"])),
    ("alerts", Gate::All(&["tickets:view"])),
    ("appointment-requests", Gate::All(&["scheduling:view"])),
    ("appointments", Gate::All(&["scheduling:view"])),
    ("assets", Gate::All(&["clients:view"])),
    (
        "audit-events",
        Gate::Any(&["special:view_audit_log", "observability:read"]),
    ),
    ("automation-rules", Gate::All(&["admin:view"])),
    ("availability", Gate::All(&["scheduling:view"])),
    ("billing", Gate::All(&["commercial:view"])),
    ("board-groups", Gate::All(&["tickets:view"])),
    ("boards", Gate::All(&["tickets:view"])),
    ("calendar", Gate::All(&["scheduling:view"])),
    ("calendar-exclusions", Gate::All(&["scheduling:view"])),
    ("calendar-sync", Gate::All(&["scheduling:view"])),
    ("catalog-items", Gate::All(&["commercial:view"])),
    ("client-loans", Gate::All(&["commercial:view"])),
    ("clients", Gate::All(&["clients:view"])),
    ("config-items", Gate::All(&["clients:view"])),
    ("connectors", Gate::Session),
    (
        "contracts",
        Gate::All(&["commercial:view", "financial:read"]),
    ),
    ("credit-notes", Gate::All(&["commercial:view"])),
    ("day-pack", Gate::All(&["scheduling:view"])),
    ("dedupe", Gate::All(&["admin:view"])),
    (
        "directory-groups",
        Gate::Any(&[
            "special:manage_users",
            "special:manage_roles",
            "directory_groups:read",
            "directory_groups:write",
        ]),
    ),
    (
        "dispatch-connector",
        Gate::Subject {
            machine: &["scheduling:view"],
            human: &["admin:view"],
        },
    ),
    ("dispatch", Gate::All(&["scheduling:view"])),
    ("documents", Gate::All(&["documents:view"])),
    ("entity-dedupe", Gate::All(&["clients:view"])),
    ("fx", Gate::All(&["commercial:view"])),
    (
        "imports",
        Gate::Subject {
            machine: &["clients:create", "clients:edit"],
            human: &["admin:view"],
        },
    ),
    ("invoice-candidates", Gate::All(&["financial:read"])),
    ("invoices", Gate::All(&["commercial:view"])),
    ("kb", Gate::All(&["kb:view"])),
    ("label-templates", Gate::Session),
    ("meeting-types", Gate::All(&["scheduling:view"])),
    ("notification-org-defaults", Gate::All(&["admin:view"])),
    ("notification-preferences", Gate::Session),
    ("notifications", Gate::Session),
    ("on-call-blocks", Gate::All(&["scheduling:view"])),
    ("org", Gate::Session),
    // A member reads their own org's notice state by membership; a machine key
    // presents the module-namespaced grant instead. The two page-shaped rows
    // below override the pair whose grant is different.
    (
        "orgs",
        Gate::Subject {
            machine: &["org_usage:read"],
            human: &[],
        },
    ),
    ("pipeline", Gate::All(&["commercial:view"])),
    ("plugins", Gate::All(&["plugins:view"])),
    ("pm-schedules", Gate::All(&["tickets:view"])),
    ("project-templates", Gate::All(&["projects:view"])),
    ("project-views", Gate::All(&["projects:view"])),
    ("projects", Gate::All(&["projects:view"])),
    ("provenance", Gate::All(&["special:view_audit_log"])),
    ("purchase-orders", Gate::All(&["commercial:view"])),
    ("quotes", Gate::All(&["commercial:view"])),
    ("rag", Gate::All(&["kb:view"])),
    // A snippet is composer material: staff read it, and its own writes are
    // admin-gated, so the read accepts either staff grant.
    ("reply-snippets", Gate::Any(&["tickets:view", "admin:view"])),
    ("report-catalog", Gate::All(&["reporting:view"])),
    ("report-definitions", Gate::All(&["reporting:view"])),
    ("report-schedules", Gate::All(&["admin:view"])),
    ("reports", Gate::All(&["reporting:view"])),
    (
        "roles",
        Gate::Any(&["special:manage_users", "special:manage_roles"]),
    ),
    ("scheduling-groups", Gate::All(&["scheduling:view"])),
    ("scheduling-links", Gate::All(&["scheduling:view"])),
    ("scorecards", Gate::All(&["reporting:view"])),
    (
        "search",
        Gate::Any(&["tickets:view", "clients:view", "kb:view", "documents:view"]),
    ),
    ("settings", Gate::Session),
    ("sla", Gate::All(&["admin:view"])),
    ("stock", Gate::All(&["commercial:view"])),
    ("stock-items", Gate::All(&["commercial:view"])),
    ("stock-locations", Gate::All(&["commercial:view"])),
    ("stock-serials", Gate::All(&["commercial:view"])),
    ("tags", Gate::All(&["tickets:view"])),
    ("taxonomy", Gate::Session),
    ("templates", Gate::Any(&["admin:view"])),
    ("tickets", Gate::All(&["tickets:view"])),
    ("tickets_correspondence", Gate::All(&["tickets:view"])),
    ("time-categories", Gate::All(&["time:view"])),
    ("time-entries", Gate::All(&["time:view"])),
    ("time-gaps", Gate::All(&["time:view"])),
    ("time-settings", Gate::All(&["admin:view"])),
    ("timers", Gate::All(&["time:view"])),
    ("tool-checkouts", Gate::All(&["commercial:view"])),
    ("tool-kits", Gate::All(&["commercial:view"])),
    ("tools", Gate::All(&["commercial:view"])),
    ("vendors", Gate::All(&["commercial:view"])),
    ("workflow-overrides", Gate::Session),
    ("workflows", Gate::All(&["admin:view"])),
];

/// The tools whose handler enforces something other than their namespace's
/// gate: a second permission on top (a money read beside the record read), a
/// narrower grant (portal administration), a self-scoped read with no
/// permission check at all, or a branch the namespace row cannot express.
pub const TOOL_GATES: &[(&str, Gate)] = &[
    (
        "billing_preview",
        Gate::All(&["commercial:view", "financial:read"]),
    ),
    (
        "billing_verification",
        Gate::All(&["special:manage_billing"]),
    ),
    ("board_exposures_list", Gate::All(&["special:portal_admin"])),
    ("day_pack_picks_list", Gate::All(&["tickets:view"])),
    ("dispatch_travel_draft_list", Gate::All(&["time:view"])),
    (
        "dispatch_work_next_list",
        Gate::All(&["scheduling:view", "scheduling:edit"]),
    ),
    (
        "org_custom_domain_get",
        Gate::Subject {
            machine: &["org_custom_domain:read"],
            human: &[],
        },
    ),
    ("org_manifest", Gate::All(&["admin:view"])),
    (
        "org_suspension_status_get",
        Gate::Subject {
            machine: &["org_suspension:read"],
            human: &[],
        },
    ),
    (
        "org_users_list",
        Gate::Any(&["special:manage_users", "special:manage_boards"]),
    ),
    ("plugin_surface_automation_actions", Gate::Session),
    ("plugin_surface_field_validators", Gate::Session),
    ("plugin_surface_report_renders", Gate::Session),
    ("plugin_surface_widgets", Gate::Session),
    ("portal_users_list", Gate::All(&["special:portal_admin"])),
    (
        "projects_cost_rollup",
        Gate::All(&["projects:view", "financial:read"]),
    ),
    (
        "purchase_orders_get_bill",
        Gate::All(&["commercial:view", "financial:read"]),
    ),
    ("search_pins_list", Gate::All(&["tickets:view"])),
    ("settings_self_host_get", Gate::All(&["admin:view"])),
    ("sign_off_consents_current", Gate::All(&["tickets:view"])),
    ("ticket_presence", Gate::Session),
    (
        "tickets_list_cost_lines",
        Gate::All(&["tickets:view", "financial:read"]),
    ),
    (
        "tickets_list_expenses",
        Gate::All(&["tickets:view", "financial:read"]),
    ),
    (
        "time_categories_work_type_map_list",
        Gate::All(&["special:manage_billing"]),
    ),
    ("time_entries_queue", Gate::Session),
    ("workflow_nodes_get", Gate::Session),
    ("workflow_nodes_list", Gate::Session),
    ("workflow_plugin_nodes", Gate::Session),
];

/// The caller in front of the adapter, as far as visibility is concerned.
pub struct Caller<'a> {
    /// Permission keys from `GET /api/v1/me` (`MeResponse.permissions`).
    pub permissions: &'a std::collections::HashSet<String>,
    /// `MeResponse.subject == "agent"`: a scoped API key rather than a person.
    pub machine: bool,
    /// False when the session context carried no permission list at all. A
    /// server too old to publish it, or a caller holding none, then keeps the
    /// pre-filter behaviour rather than losing every tool to a parse gap.
    pub scoped: bool,
}

impl Caller<'_> {
    fn holds_all(&self, keys: &[&str]) -> bool {
        keys.iter().all(|key| self.permissions.contains(*key))
    }

    fn holds_any(&self, keys: &[&str]) -> bool {
        keys.iter().any(|key| self.permissions.contains(*key))
    }
}

/// The gate that decides whether `tool` appears for this caller.
///
/// A namespace with no row keeps the surface it had before this module existed:
/// the tool stays visible and the handler still decides. That case cannot ship,
/// because the catalog and this table are pinned together and
/// `every_catalog_namespace_and_override_has_a_row` builds every tool the
/// committed artifact contains — but a live request should lose one guarantee,
/// not every tool, if the two ever drift apart.
pub fn gate_for(tool: &Tool) -> Gate {
    if let Some((_, gate)) = TOOL_GATES.iter().find(|(name, _)| *name == tool.name) {
        return *gate;
    }
    let namespace = namespace_of(&tool.capability);
    match NAMESPACE_GATES
        .iter()
        .find(|(row, _)| *row == namespace)
        .map(|(_, gate)| *gate)
    {
        Some(gate) => gate,
        None => {
            debug_assert!(
                false,
                "capability namespace {namespace} has no gate row; add one beside the catalog"
            );
            Gate::Session
        }
    }
}

/// Whether this caller's own permissions clear the tool's gate.
///
/// `Gate::Subject` picks its side from the credential kind, exactly as the
/// handler does. A session the adapter could not scope passes.
pub fn clears_gate(tool: &Tool, caller: &Caller<'_>) -> bool {
    if !caller.scoped {
        return true;
    }
    match gate_for(tool) {
        Gate::Session => true,
        Gate::All(keys) => caller.holds_all(keys),
        Gate::Any(keys) => caller.holds_any(keys),
        Gate::Subject { machine, human } => {
            if caller.machine {
                caller.holds_all(machine)
            } else {
                caller.holds_all(human)
            }
        }
    }
}

/// Whether this caller may be shown `tool`: the permission gate, plus — for a
/// machine key — an operation its credential can reach at all.
///
/// Core-api refuses an `x-api-key` on any path outside its machine allowance
/// before a handler runs, so a key that names such a tool gets a 401 rather than
/// a read. The catalog already carries that answer: a documented operation
/// names `x-api-key-scopes` when a scoped key may reach it.
pub fn may_see(tool: &Tool, caller: &Caller<'_>) -> bool {
    if caller.scoped && caller.machine && !tool.key_reachable {
        return false;
    }
    clears_gate(tool, caller)
}

/// The namespace side of a capability slug (`tickets:read` → `tickets`).
pub fn namespace_of(capability: &str) -> &str {
    capability.strip_suffix(":read").unwrap_or(capability)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn tool(namespace: &str) -> Tool {
        Tool {
            name: format!("{namespace}_list"),
            description: String::new(),
            capability: format!("{namespace}:read"),
            path: format!("/api/{namespace}"),
            method: "GET".into(),
            input_schema: serde_json::json!({ "type": "object" }),
            key_reachable: true,
        }
    }

    fn caller(permissions: &[&str], machine: bool) -> Caller<'static> {
        // Leaked on purpose: the table borrows the set for the test's lifetime
        // and a fixed-size leak per test is cheaper than a lifetime dance.
        let set: &'static HashSet<String> = Box::leak(Box::new(
            permissions.iter().map(|p| (*p).to_string()).collect(),
        ));
        Caller {
            permissions: set,
            machine,
            scoped: true,
        }
    }

    #[test]
    fn a_matrix_gate_admits_only_the_holder() {
        let tickets = tool("tickets");
        assert!(may_see(&tickets, &caller(&["tickets:view"], false)));
        assert!(!may_see(&tickets, &caller(&["clients:view"], false)));
    }

    #[test]
    fn a_two_key_gate_needs_both() {
        let cost_lines = Tool {
            name: "tickets_list_cost_lines".into(),
            description: String::new(),
            capability: "tickets:read".into(),
            path: "/api/tickets/{id}/cost-lines".into(),
            method: "GET".into(),
            input_schema: serde_json::json!({ "type": "object" }),
            key_reachable: true,
        };
        assert!(!may_see(&cost_lines, &caller(&["tickets:view"], false)));
        assert!(!may_see(&cost_lines, &caller(&["financial:read"], false)));
        assert!(may_see(
            &cost_lines,
            &caller(&["tickets:view", "financial:read"], false)
        ));
    }

    #[test]
    fn a_self_scoped_read_stays_visible_without_a_permission() {
        assert!(may_see(&tool("notifications"), &caller(&[], false)));
    }

    #[test]
    fn the_subject_branch_picks_its_side() {
        // A member reads their own org's usage by membership; a key needs the
        // module-namespaced grant.
        let usage = Tool {
            name: "org_usage_get".into(),
            description: String::new(),
            capability: "orgs:read".into(),
            path: "/api/orgs/{org_id}/usage".into(),
            method: "GET".into(),
            input_schema: serde_json::json!({ "type": "object" }),
            key_reachable: true,
        };
        assert!(may_see(&usage, &caller(&["tickets:view"], false)));
        assert!(!may_see(&usage, &caller(&["tickets:view"], true)));
        assert!(may_see(&usage, &caller(&["org_usage:read"], true)));
    }

    #[test]
    fn a_machine_key_does_not_see_a_path_its_credential_cannot_reach() {
        let mut notifications = tool("notifications");
        notifications.name = "notifications_list".into();
        notifications.path = "/api/notifications".into();
        notifications.key_reachable = false;
        assert!(may_see(&notifications, &caller(&[], false)));
        assert!(!may_see(&notifications, &caller(&[], true)));
    }

    #[test]
    fn an_unscoped_session_keeps_the_pre_filter_surface() {
        let tickets = tool("tickets");
        let empty: HashSet<String> = HashSet::new();
        let unscoped = Caller {
            permissions: &empty,
            machine: true,
            scoped: false,
        };
        assert!(may_see(&tickets, &unscoped));
    }

    /// The table is checked against the artifact it describes, not against a
    /// fixture: a capability added to the catalog without a gate row, or an
    /// override naming a tool that was renamed away, fails here.
    #[test]
    fn every_catalog_namespace_and_override_has_a_row() {
        let tools =
            crate::catalog::build_catalog(&crate::openapi_spec(), &crate::mcp_catalog_spec());
        assert!(!tools.is_empty(), "the committed catalog is not empty");
        for tool in &tools {
            let namespace = namespace_of(&tool.capability);
            assert!(
                NAMESPACE_GATES.iter().any(|(row, _)| *row == namespace),
                "no gate row for capability namespace {namespace} (tool {})",
                tool.name
            );
            let _ = gate_for(tool);
        }
        for (name, _) in TOOL_GATES {
            assert!(
                tools.iter().any(|tool| tool.name == *name),
                "TOOL_GATES names a tool the catalog does not contain: {name}"
            );
        }
        for (row, _) in NAMESPACE_GATES {
            assert!(
                tools
                    .iter()
                    .any(|tool| namespace_of(&tool.capability) == *row),
                "gate row {row} does not describe any catalog family"
            );
        }
    }

    /// Keys are compared against `Permission::key()` strings, so a row written
    /// in another vocabulary would silently match nothing.
    #[test]
    fn every_gate_key_is_a_permission_key() {
        let keys = NAMESPACE_GATES
            .iter()
            .map(|(_, gate)| *gate)
            .chain(TOOL_GATES.iter().map(|(_, gate)| *gate));
        for gate in keys {
            let keys: &[&str] = match gate {
                Gate::Session => &[],
                Gate::All(keys) | Gate::Any(keys) => keys,
                Gate::Subject { machine, human } => {
                    for key in machine.iter().chain(human.iter()) {
                        assert_key(key);
                    }
                    continue;
                }
            };
            for key in keys {
                assert_key(key);
            }
        }

        fn assert_key(key: &str) {
            assert!(
                key.contains(':')
                    && key
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || ":_-".contains(c)),
                "{key:?} is not a permission key"
            );
        }
    }
}
