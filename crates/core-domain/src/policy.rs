//! Centralized authorization policy — the ONE module every permission check
//! flows through (§6). Unit-tested, auditable, exhaustive.

use serde::Serialize;

/// Product modules that carry a permission matrix (view/create/edit/delete/export).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Module {
    Tickets,
    Clients,
    Time,
    Scheduling,
    Projects,
    Commercial,
    Invoicing,
    Kb,
    Reporting,
    Admin,
    Plugins,
}

impl Module {
    pub const ALL: [Module; 11] = [
        Module::Tickets,
        Module::Clients,
        Module::Time,
        Module::Scheduling,
        Module::Projects,
        Module::Commercial,
        Module::Invoicing,
        Module::Kb,
        Module::Reporting,
        Module::Admin,
        Module::Plugins,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Module::Tickets => "tickets",
            Module::Clients => "clients",
            Module::Time => "time",
            Module::Scheduling => "scheduling",
            Module::Projects => "projects",
            Module::Commercial => "commercial",
            Module::Invoicing => "invoicing",
            Module::Kb => "kb",
            Module::Reporting => "reporting",
            Module::Admin => "admin",
            Module::Plugins => "plugins",
        }
    }
}

/// CRUD + export actions, plus module-independent operational grants.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Action {
    View,
    Create,
    Edit,
    Delete,
    Export,
}

impl Action {
    pub const ALL: [Action; 5] = [
        Action::View,
        Action::Create,
        Action::Edit,
        Action::Delete,
        Action::Export,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Action::View => "view",
            Action::Create => "create",
            Action::Edit => "edit",
            Action::Delete => "delete",
            Action::Export => "export",
        }
    }
}

/// A concrete grant: `module:action` (matrix) or a special operational grant.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Matrix { module: Module, action: Action },
    ManageUsers,
    ManageRoles,
    ManageBoards,
    ApproveTime,
    ViewFinancials,
    ManageBilling,
    ManagePlugins,
    ManageApiKeys,
    ViewAuditLog,
    PortalAdmin,
}

impl Permission {
    pub fn key(self) -> String {
        match self {
            Permission::Matrix { module, action } => {
                format!("{}:{}", module.as_str(), action.as_str())
            }
            other => match other {
                Permission::ManageUsers => "special:manage_users".into(),
                Permission::ManageRoles => "special:manage_roles".into(),
                Permission::ManageBoards => "special:manage_boards".into(),
                Permission::ApproveTime => "special:approve_time".into(),
                Permission::ViewFinancials => "special:view_financials".into(),
                Permission::ManageBilling => "special:manage_billing".into(),
                Permission::ManagePlugins => "special:manage_plugins".into(),
                Permission::ManageApiKeys => "special:manage_api_keys".into(),
                Permission::ViewAuditLog => "special:view_audit_log".into(),
                Permission::PortalAdmin => "special:portal_admin".into(),
                Permission::Matrix { .. } => unreachable!(),
            },
        }
    }
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.key())
    }
}

/// Built-in roles. Custom roles are arbitrary permission sets stored per org;
/// these four ship as defaults so a new org works with zero configuration
/// (defaults-first doctrine).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum BuiltinRole {
    Owner,
    Admin,
    Dispatcher,
    Technician,
    ReadOnly,
}

impl BuiltinRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            BuiltinRole::Owner => "owner",
            BuiltinRole::Admin => "admin",
            BuiltinRole::Dispatcher => "dispatcher",
            BuiltinRole::Technician => "technician",
            BuiltinRole::ReadOnly => "read_only",
        }
    }

    /// The complete permission set for this role. Owner/Admin hold everything;
    /// the matrix below is the single source of truth.
    pub fn permissions(self) -> Vec<Permission> {
        let full_matrix = |modules: &[Module]| {
            modules
                .iter()
                .flat_map(|m| {
                    Action::ALL.iter().map(move |a| Permission::Matrix {
                        module: *m,
                        action: *a,
                    })
                })
                .collect::<Vec<_>>()
        };
        let specials = |ps: &[Permission]| ps.to_vec();
        match self {
            BuiltinRole::Owner | BuiltinRole::Admin => {
                let mut p = full_matrix(&Module::ALL);
                p.extend(specials(&[
                    Permission::ManageUsers,
                    Permission::ManageRoles,
                    Permission::ManageBoards,
                    Permission::ApproveTime,
                    Permission::ViewFinancials,
                    Permission::ManageBilling,
                    Permission::ManagePlugins,
                    Permission::ManageApiKeys,
                    Permission::ViewAuditLog,
                    Permission::PortalAdmin,
                ]));
                p
            }
            BuiltinRole::Dispatcher => {
                let mut p = full_matrix(&[Module::Tickets, Module::Scheduling, Module::Clients]);
                p.extend(
                    full_matrix(&[
                        Module::Time,
                        Module::Projects,
                        Module::Commercial,
                        Module::Kb,
                        Module::Reporting,
                    ])
                    .into_iter()
                    .filter(|perm| {
                        matches!(
                            perm,
                            Permission::Matrix {
                                action: Action::View | Action::Edit,
                                ..
                            }
                        )
                    }),
                );
                p.extend(specials(&[Permission::ManageBoards]));
                p
            }
            BuiltinRole::Technician => {
                let mut p = full_matrix(&[Module::Tickets, Module::Kb]);
                p.extend([
                    Permission::Matrix {
                        module: Module::Clients,
                        action: Action::View,
                    },
                    Permission::Matrix {
                        module: Module::Time,
                        action: Action::View,
                    },
                    Permission::Matrix {
                        module: Module::Time,
                        action: Action::Create,
                    },
                    Permission::Matrix {
                        module: Module::Time,
                        action: Action::Edit,
                    },
                    Permission::Matrix {
                        module: Module::Scheduling,
                        action: Action::View,
                    },
                    Permission::Matrix {
                        module: Module::Scheduling,
                        action: Action::Edit,
                    },
                    Permission::Matrix {
                        module: Module::Reporting,
                        action: Action::View,
                    },
                ]);
                p
            }
            BuiltinRole::ReadOnly => Module::ALL
                .iter()
                .map(|m| Permission::Matrix {
                    module: *m,
                    action: Action::View,
                })
                .collect(),
        }
    }
}

/// The result of a policy check. Never a bare bool — denials carry a reason
/// for the audit log and the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum PolicyDecision {
    Allowed,
    Denied { reason: String },
}

impl PolicyDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }

    pub fn expect(self) -> Result<(), String> {
        match self {
            Self::Allowed => Ok(()),
            Self::Denied { reason } => Err(reason),
        }
    }
}

/// The single authorization entry point. `granted` is the caller's resolved
/// permission set (union of role grants); `required` is what the operation needs.
pub fn check(granted: &[Permission], required: Permission) -> PolicyDecision {
    if granted.contains(&required) {
        PolicyDecision::Allowed
    } else {
        PolicyDecision::Denied {
            reason: format!("missing permission: {}", required.key()),
        }
    }
}

/// Convenience: check any-of (a view action satisfiable by several grants).
pub fn check_any(granted: &[Permission], required: &[Permission]) -> PolicyDecision {
    if required.is_empty() {
        return PolicyDecision::Denied {
            reason: "no permission requirement configured".into(),
        };
    }
    if required.iter().any(|p| granted.contains(p)) {
        PolicyDecision::Allowed
    } else {
        PolicyDecision::Denied {
            reason: format!(
                "missing one of: {}",
                required
                    .iter()
                    .map(|p| p.key())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn technician_can_create_tickets_but_not_delete_clients() {
        let tech = BuiltinRole::Technician.permissions();
        assert!(check(
            &tech,
            Permission::Matrix {
                module: Module::Tickets,
                action: Action::Create
            }
        )
        .is_allowed());
        assert!(check(
            &tech,
            Permission::Matrix {
                module: Module::Clients,
                action: Action::Delete
            }
        )
        .is_allowed()
        .not());
    }

    #[test]
    fn owner_holds_every_matrix_permission() {
        let owner = BuiltinRole::Owner.permissions();
        for m in Module::ALL {
            for a in Action::ALL {
                assert!(check(
                    &owner,
                    Permission::Matrix {
                        module: m,
                        action: a
                    }
                )
                .is_allowed());
            }
        }
        assert!(check(&owner, Permission::ManageUsers).is_allowed());
    }

    #[test]
    fn read_only_cannot_edit_anything() {
        let ro = BuiltinRole::ReadOnly.permissions();
        for m in Module::ALL {
            for a in [Action::Create, Action::Edit, Action::Delete, Action::Export] {
                assert!(
                    check(
                        &ro,
                        Permission::Matrix {
                            module: m,
                            action: a
                        }
                    )
                    .is_allowed()
                    .not(),
                    "{m:?}:{a:?} should be denied"
                );
            }
        }
    }

    trait Not {
        fn not(self) -> Self;
    }
    impl Not for bool {
        fn not(self) -> Self {
            !self
        }
    }
}
