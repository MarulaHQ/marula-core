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
    /// Documents with immutable version history (TASKLIST D4,
    /// agent-platform §6). Owner/Admin hold the full matrix; Read-Only
    /// holds view. Staff roles gain it by explicit custom-role grant —
    /// additive to widen later without breaking stored roles.
    Documents,
    /// Staff password vault (docs-platform v1, migration 0053).
    /// Owner/Admin hold the full matrix; Technician can use the vault
    /// (view/create/edit — credentials are working material); Dispatcher,
    /// Auditor, and Read-Only hold view. Reveal additionally requires edit
    /// (enforced in the route, not the matrix).
    Passwords,
    /// Native secrets vault generalization (ADR 0006, migration 0078):
    /// module-level matrix rides the SAME shape as Passwords; category
    /// visibility is additionally gated per folder through
    /// `secret_category_grants` (deny-by-default `restricted` folders —
    /// the `financial:read` gate shape). Reveal additionally requires
    /// edit (enforced in the route, not the matrix).
    Secrets,
    Reporting,
    Admin,
    Plugins,
}

impl Module {
    pub const ALL: [Module; 14] = [
        Module::Tickets,
        Module::Clients,
        Module::Time,
        Module::Scheduling,
        Module::Projects,
        Module::Commercial,
        Module::Invoicing,
        Module::Kb,
        Module::Documents,
        Module::Passwords,
        Module::Secrets,
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
            Module::Documents => "documents",
            Module::Passwords => "passwords",
            Module::Secrets => "secrets",
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
    Matrix {
        module: Module,
        action: Action,
    },
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
    ViewOrgUsage,
    /// Wave 47 lane A8 (`p47-org-custom-domains`): read a valid-type org's
    /// custom-domain state (domain, verification status, whether a
    /// DNS-provider plugin is installed). Held by SERVICE ACCOUNTS and
    /// custom roles — org members read their own state through membership,
    /// which is why no shipped preset carries it.
    ViewOrgCustomDomain,
    /// Wave 47 lane A8: set/clear a valid-type org's custom domain and run the
    /// DNS verification workflow. A human owner/admin holds the authority
    /// through the Admin matrix (`Module::Admin` + `Action::Edit`), exactly
    /// like org settings; this permission is what a machine caller (or a
    /// custom role) presents instead. The `org_custom_domain.manage` plugin
    /// capability resolves onto it.
    ManageOrgCustomDomain,
    /// Wave 47 lane A7 (`p47-org-suspension-notice`): read an org's suspension
    /// NOTICE state — when it was flagged, why, and when the notice starts
    /// showing. Held by SERVICE ACCOUNTS and custom roles (org members read
    /// their own state through membership, which is why no shipped preset
    /// carries it). The flag is informational: nothing in the core blocks,
    /// rejects or degrades a flagged org, so this grant widens what a machine
    /// may OBSERVE and nothing else.
    ViewOrgSuspension,
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
                // ADR 0004 (RB2): financial visibility is its OWN permission
                // with its own wire key — `financial:read` gates cost rates,
                // contract rates, and margin panels. It is deliberately NOT
                // namespaced under `special:` because it is a matrix-peer
                // grant orgs legitimately fork into custom roles.
                Permission::ViewFinancials => "financial:read".into(),
                Permission::ManageBilling => "special:manage_billing".into(),
                Permission::ManagePlugins => "special:manage_plugins".into(),
                Permission::ManageApiKeys => "special:manage_api_keys".into(),
                Permission::ViewAuditLog => "special:view_audit_log".into(),
                Permission::PortalAdmin => "special:portal_admin".into(),
                // Wave 47 lane A6 (org usage/quota read API): the wire key
                // mirrors the `financial:read` precedent — module-
                // namespaced rather than `special:`-prefixed — because
                // orgs legitimately fork this read-only reporting grant
                // into custom roles beside its Reporting matrix peers. A
                // human member reads their own org's usage through their
                // membership, so the permission exists for SERVICE ACCOUNTS
                // and custom roles; no shipped preset carries it, and
                // nothing anywhere enforces what the numbers say.
                Permission::ViewOrgUsage => "org_usage:read".into(),
                // Wave 47 lane A8: the same module-namespaced shape as
                // `financial:read` / `org_usage:read`. Read is a
                // machine/custom-role grant (members read by membership);
                // manage is what a service account presents in place of the
                // Admin matrix a human admin holds.
                Permission::ViewOrgCustomDomain => "org_custom_domain:read".into(),
                Permission::ManageOrgCustomDomain => "org_custom_domain:manage".into(),
                // Wave 47 lane A7: the same module-namespaced shape as
                // `org_usage:read` / `org_custom_domain:read`. A human member
                // reads their own org's notice state by membership; the
                // permission exists for SERVICE ACCOUNTS and custom roles, so
                // no shipped preset carries it.
                Permission::ViewOrgSuspension => "org_suspension:read".into(),
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
/// these ship as defaults so a new org works with zero configuration
/// (defaults-first doctrine). ADR 0004 (RB1): the presets are canonical
/// DATA — every org carries `roles` rows for them (seeded at creation,
/// backfilled by migration 0071) and their permission arrays are the same
/// matrix vocabulary custom roles use; orgs fork them into custom copies
/// and restore the canonical rows, never edit the preset in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum BuiltinRole {
    Owner,
    Admin,
    Dispatcher,
    Technician,
    /// Escalation tier above Technician: full ticket + KB control (a
    /// senior tech closes and prunes without admin help), time-entry
    /// approval rights, and NO financial visibility — cost rates stay
    /// gated by `financial:read` (ADR 0004 RB2).
    SeniorTech,
    /// Money-focused preset: full invoicing control plus contract/catalog
    /// price visibility through `financial:read`. Deliberately narrow
    /// elsewhere — a bookkeeper does not work tickets.
    Bookkeeper,
    /// Compliance preset: read everything (the full view matrix) plus the
    /// audit trail. Writes nothing, sees the trail. (The read_only preset
    /// stays untouched: a plain viewer holds no audit access.)
    Auditor,
    ReadOnly,
}

impl BuiltinRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            BuiltinRole::Owner => "owner",
            BuiltinRole::Admin => "admin",
            BuiltinRole::Dispatcher => "dispatcher",
            BuiltinRole::Technician => "technician",
            BuiltinRole::SeniorTech => "senior_tech",
            BuiltinRole::Bookkeeper => "bookkeeper",
            BuiltinRole::Auditor => "auditor",
            BuiltinRole::ReadOnly => "read_only",
        }
    }

    /// Every shipped preset in roster order (the row the roles surface
    /// renders, in ADR 0004's listing order).
    pub const PRESETS: [BuiltinRole; 8] = [
        BuiltinRole::Owner,
        BuiltinRole::Admin,
        BuiltinRole::Dispatcher,
        BuiltinRole::SeniorTech,
        BuiltinRole::Technician,
        BuiltinRole::Bookkeeper,
        BuiltinRole::Auditor,
        BuiltinRole::ReadOnly,
    ];

    /// Parses a preset name (accepts the storage spellings plus common
    /// variants; mirrors the directory picker's tolerance for `read_only`).
    pub fn parse_preset(s: &str) -> Option<Self> {
        let normalized = s.trim().to_lowercase();
        match normalized.as_str() {
            "owner" => Some(BuiltinRole::Owner),
            "admin" => Some(BuiltinRole::Admin),
            "dispatcher" => Some(BuiltinRole::Dispatcher),
            "senior_tech" | "senior-tech" | "senior tech" => Some(BuiltinRole::SeniorTech),
            "technician" => Some(BuiltinRole::Technician),
            "bookkeeper" => Some(BuiltinRole::Bookkeeper),
            "auditor" => Some(BuiltinRole::Auditor),
            "read_only" | "readonly" | "read-only" => Some(BuiltinRole::ReadOnly),
            _ => None,
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
                p.extend([
                    Permission::Matrix {
                        module: Module::Passwords,
                        action: Action::View,
                    },
                    Permission::Matrix {
                        module: Module::Secrets,
                        action: Action::View,
                    },
                ]);
                p.extend(specials(&[Permission::ManageBoards]));
                p
            }
            BuiltinRole::Technician => {
                let mut p = full_matrix(&[Module::Tickets, Module::Kb]);
                p.extend(
                    full_matrix(&[Module::Passwords, Module::Secrets])
                        .into_iter()
                        .filter(|perm| {
                            matches!(
                                perm,
                                Permission::Matrix {
                                    action: Action::View | Action::Create | Action::Edit,
                                    ..
                                }
                            )
                        }),
                );
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
            // Senior Tech: everything a Technician holds, escalated — the
            // full ticket + KB matrix (close/prune authority), time-entry
            // approval (special:approve_time), and still NO financial:read.
            // Cost visibility is not a seniority perk (ADR 0004 RB2).
            BuiltinRole::SeniorTech => {
                let mut p = full_matrix(&[Module::Tickets, Module::Kb]);
                p.extend(
                    full_matrix(&[Module::Passwords, Module::Secrets])
                        .into_iter()
                        .filter(|perm| {
                            matches!(
                                perm,
                                Permission::Matrix {
                                    action: Action::View | Action::Create | Action::Edit,
                                    ..
                                }
                            )
                        }),
                );
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
                p.extend(specials(&[Permission::ApproveTime]));
                p
            }
            // Bookkeeper: the money role. Full invoicing control (invoice
            // runs, voids, payments, credit-note work), commercial price
            // visibility, time reads (to cross-check billed hours), report
            // views + exports (the month-end pack), and the two financial
            // specials: `financial:read` + `special:manage_billing`. No
            // ticket-working grants at all.
            BuiltinRole::Bookkeeper => {
                let mut p = full_matrix(&[Module::Invoicing]);
                p.extend([
                    Permission::Matrix {
                        module: Module::Clients,
                        action: Action::View,
                    },
                    Permission::Matrix {
                        module: Module::Commercial,
                        action: Action::View,
                    },
                    Permission::Matrix {
                        module: Module::Time,
                        action: Action::View,
                    },
                    Permission::Matrix {
                        module: Module::Reporting,
                        action: Action::View,
                    },
                    Permission::Matrix {
                        module: Module::Reporting,
                        action: Action::Export,
                    },
                ]);
                p.extend(specials(&[
                    Permission::ViewFinancials,
                    Permission::ManageBilling,
                ]));
                p
            }
            // Auditor: the whole view matrix (every module) plus the audit
            // trail, and nothing else — write nothing, see the trail.
            BuiltinRole::Auditor => {
                let mut p = Module::ALL
                    .iter()
                    .map(|m| Permission::Matrix {
                        module: *m,
                        action: Action::View,
                    })
                    .collect::<Vec<_>>();
                p.extend(specials(&[Permission::ViewAuditLog]));
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

/// Per-board role tier stored on `board_members` rows (migration 0019,
/// TASKLIST 2.11). Board roles sit BESIDE the org matrix and can only
/// narrow it — see `authorize_board_scoped`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum BoardRole {
    Viewer,
    Member,
    Admin,
}

impl BoardRole {
    pub const ALL: [BoardRole; 3] = [BoardRole::Viewer, BoardRole::Member, BoardRole::Admin];

    pub const fn as_str(self) -> &'static str {
        match self {
            BoardRole::Viewer => "viewer",
            BoardRole::Member => "member",
            BoardRole::Admin => "admin",
        }
    }

    /// Parses the storage representation (`board_role` enum text).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "viewer" => Some(BoardRole::Viewer),
            "member" => Some(BoardRole::Member),
            "admin" => Some(BoardRole::Admin),
            _ => None,
        }
    }
}

/// The board-scoped operations gated by `BoardRole` (§6 board-level RBAC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoardOp {
    /// Read tickets, comments, and workflow state on this board.
    View,
    /// Open a new ticket on this board.
    CreateTicket,
    /// Mutate an existing ticket: patch fields, comment, checklist items,
    /// and status moves that do not close.
    WorkTicket,
    /// Move a ticket into a closed-category status.
    CloseTicket,
    /// Configure the board itself: statuses, transitions, checklist
    /// template, inbound address, group assignment.
    ConfigureBoard,
    /// Add, re-role, or remove board members.
    ManageMembers,
}

/// Whether a `BoardRole` permits `op` in isolation (ignoring org grants).
/// This is the single source of the role×operation matrix; the exhaustive
/// unit test below pins every cell of it.
pub const fn board_role_allows(role: BoardRole, op: BoardOp) -> bool {
    match role {
        BoardRole::Viewer => matches!(op, BoardOp::View),
        BoardRole::Member => matches!(
            op,
            BoardOp::View | BoardOp::CreateTicket | BoardOp::WorkTicket
        ),
        BoardRole::Admin => true,
    }
}

/// Composes the org-level verdict with the caller's optional board
/// membership for one board-scoped operation.
///
/// Composition rules (deny wins):
/// 1. No membership row ⇒ defaults-first passthrough: the org-level
///    verdict stands untouched (pre-RBAC behavior is preserved exactly).
/// 2. A membership row may only NARROW access: if the role denies `op`,
///    the operation is denied even when the org level would allow it.
/// 3. A membership row never EXPANDS access: an org-level denial stays a
///    denial regardless of role.
pub fn authorize_board_scoped(
    org_decision: PolicyDecision,
    role: Option<BoardRole>,
    op: BoardOp,
) -> PolicyDecision {
    let Some(role) = role else {
        return org_decision;
    };
    if !board_role_allows(role, op) {
        return PolicyDecision::Denied {
            reason: format!(
                "board role '{}' does not permit this operation",
                role.as_str()
            ),
        };
    }
    org_decision
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

// --- portal v1 role policy ---------------------------------------------------
//
// Portal authorization is deliberately NOT the staff permission matrix:
// portal users are client personnel whose entire grant is "their own
// client's tickets on exposed boards". These checks are the single
// authorization entry point for /api/portal/* — unit-tested, exhaustive.

/// Everything a portal session may attempt. There is no create/close/
/// assign/etc.: v1 portal is strictly view + reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortalAction {
    /// List/read tickets of the caller's own client on exposed boards.
    ViewTickets,
    /// Reply to a ticket of the caller's own client. The reply is ALWAYS
    /// created as a `client_visible` note regardless of tier.
    PostReply,
    /// Read `in_house_update` notes. Allowed ONLY for the it_manager tier;
    /// the notes whitelist still hard-excludes `internal` for every tier
    /// (see `notes::visible_kinds` — this action widens by one kind, never
    /// two).
    ViewInHouseUpdates,
    /// List/read the caller's client's invoices (Portal v2). Void and
    /// draft documents stay staff-internal; the query layer hides them.
    ViewInvoices,
    /// Record a payment against the caller's client's invoice through the
    /// `PaymentProvider` seam (Portal v2). With the built-in none-mode
    /// provider this is LEDGER-ONLY: no card processor is charged, the
    /// receipt records that the client settled the amount.
    PayInvoice,
    /// Browse/search client-facing KB articles and categories (Portal
    /// v2). The effective gate is published AND article flag AND category
    /// flag — the query layer owns it, this decides the tier may ask.
    ViewKb,
    /// Request an appointment slot against published availability and
    /// cancel own pending requests (Portal v2). Staff approve/decline is
    /// a STAFF action outside this matrix.
    RequestAppointment,
    /// List/read the caller's client's quotes (Portal v2 / Wave 34).
    /// Drafts stay staff-internal; the query layer hides them.
    ViewQuotes,
    /// Approve (accept) or decline a SENT quote of the caller's own
    /// client. Binds the SAME lifecycle legs as the staff quote flow —
    /// sent → accepted | declined — via the same transition guard; the
    /// conversion-to-invoice seam stays a staff action.
    DecideQuotes,
}

impl PortalAction {
    /// Wire key for the portal-tier preset registry (roles surface).
    pub const fn as_str(self) -> &'static str {
        match self {
            PortalAction::ViewTickets => "view_tickets",
            PortalAction::PostReply => "post_reply",
            PortalAction::ViewInHouseUpdates => "view_in_house_updates",
            PortalAction::ViewInvoices => "view_invoices",
            PortalAction::PayInvoice => "pay_invoice",
            PortalAction::ViewKb => "view_kb",
            PortalAction::RequestAppointment => "request_appointment",
            PortalAction::ViewQuotes => "view_quotes",
            PortalAction::DecideQuotes => "decide_quotes",
        }
    }

    /// Every portal capability, stable order for the registry.
    pub const ALL: [PortalAction; 9] = [
        PortalAction::ViewTickets,
        PortalAction::PostReply,
        PortalAction::ViewInHouseUpdates,
        PortalAction::ViewInvoices,
        PortalAction::PayInvoice,
        PortalAction::ViewKb,
        PortalAction::RequestAppointment,
        PortalAction::ViewQuotes,
        PortalAction::DecideQuotes,
    ];
}

/// The tier×capability matrix in isolation. Single source of truth: the
/// portal tier presets in the roles surface (RB1, ADR 0004) render their
/// capability lists from this table, and `check_portal` enforces it — the
/// rendered roster and the enforced rules cannot drift apart.
pub const fn portal_tier_allows(role: crate::entities::PortalRole, action: PortalAction) -> bool {
    use crate::entities::PortalRole;
    match action {
        PortalAction::ViewTickets
        | PortalAction::PostReply
        | PortalAction::ViewInvoices
        | PortalAction::PayInvoice
        | PortalAction::ViewKb
        | PortalAction::RequestAppointment
        | PortalAction::ViewQuotes
        | PortalAction::DecideQuotes => true,
        PortalAction::ViewInHouseUpdates => matches!(role, PortalRole::ItManager),
    }
}

/// The single portal authorization entry point. Denials carry the reason
/// for audit + API error surfaces. Note that client/board scoping is NOT
/// decided here — callers must additionally constrain queries to the
/// authenticated portal user's client_id and exposed boards; this function
/// decides what the TIER may do, the query layer decides WHICH rows exist.
pub fn check_portal(role: crate::entities::PortalRole, action: PortalAction) -> PolicyDecision {
    if portal_tier_allows(role, action) {
        PolicyDecision::Allowed
    } else {
        PolicyDecision::Denied {
            reason: format!(
                "portal tier '{}' does not include this capability",
                role.as_str()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::PortalRole;

    #[test]
    fn every_portal_tier_may_view_and_reply() {
        for role in [
            PortalRole::Contact,
            PortalRole::Approver,
            PortalRole::ItManager,
        ] {
            assert!(check_portal(role, PortalAction::ViewTickets).is_allowed());
            assert!(check_portal(role, PortalAction::PostReply).is_allowed());
            // Portal v2: invoices (view + pay), KB self-service, and
            // appointment requests are client-wide abilities — every
            // tier holds them, query scoping does the rest.
            assert!(check_portal(role, PortalAction::ViewInvoices).is_allowed());
            assert!(check_portal(role, PortalAction::PayInvoice).is_allowed());
            assert!(check_portal(role, PortalAction::ViewKb).is_allowed());
            assert!(check_portal(role, PortalAction::RequestAppointment).is_allowed());
        }
    }

    #[test]
    fn only_it_manager_reads_in_house_updates() {
        assert!(check_portal(PortalRole::ItManager, PortalAction::ViewInHouseUpdates).is_allowed());
        for role in [PortalRole::Contact, PortalRole::Approver] {
            let decision = check_portal(role, PortalAction::ViewInHouseUpdates);
            assert!(decision.is_allowed().not(), "{role:?} must be denied");
        }
    }

    /// The binding v1 rule: the IT-manager tier widens visibility to
    /// in-house updates and NEVER to internal notes. Pinned at the policy
    /// layer (in addition to `notes`) so both enforcement surfaces must
    /// agree before a regression can ship.
    #[test]
    fn it_manager_widening_stops_at_internal() {
        use crate::notes::{visible_kinds, visible_to, NoteAudience};
        let kinds = visible_kinds(NoteAudience::PortalItManager);
        assert!(kinds.contains(&crate::status::NoteKind::InHouseUpdate));
        assert!(kinds.contains(&crate::status::NoteKind::ClientVisible));
        assert!(kinds
            .iter()
            .all(|k| *k != crate::status::NoteKind::Internal));
        assert!(visible_to(
            crate::status::NoteKind::Internal,
            NoteAudience::PortalItManager
        )
        .not());
    }

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

    // --- RB1 preset roster (ADR 0004, Wave 13 lane 6) ---------------------

    /// The audit preset: full view matrix + the audit trail special, and
    /// every write action denied. Reveal-style edits (passwords) stay out
    /// because edit is never granted.
    #[test]
    fn auditor_sees_everything_including_audit_trail_but_writes_nothing() {
        let auditor = BuiltinRole::Auditor.permissions();
        for m in Module::ALL {
            assert!(
                check(
                    &auditor,
                    Permission::Matrix {
                        module: m,
                        action: Action::View
                    }
                )
                .is_allowed(),
                "{m:?}:view should be granted"
            );
            for a in [Action::Create, Action::Edit, Action::Delete, Action::Export] {
                assert!(
                    check(
                        &auditor,
                        Permission::Matrix {
                            module: m,
                            action: a
                        }
                    )
                    .is_allowed()
                    .not(),
                    "{m:?}:{a:?} should be denied for auditor"
                );
            }
        }
        assert!(check(&auditor, Permission::ViewAuditLog).is_allowed());
        assert!(check(&auditor, Permission::ViewFinancials)
            .is_allowed()
            .not());
    }

    /// A plain read-only viewer holds NO audit-trail access — that is what
    /// distinguishes the auditor preset from read_only.
    #[test]
    fn read_only_holds_no_audit_trail_access() {
        assert!(check(
            &BuiltinRole::ReadOnly.permissions(),
            Permission::ViewAuditLog
        )
        .is_allowed()
        .not());
    }

    /// The bookkeeper preset: full invoicing control + the two financial
    /// specials, and no ticket-working grants.
    #[test]
    fn bookkeeper_runs_invoicing_and_sees_financials_but_never_works_tickets() {
        let bookkeeper = BuiltinRole::Bookkeeper.permissions();
        assert!(check(&bookkeeper, Permission::ViewFinancials).is_allowed());
        assert!(check(&bookkeeper, Permission::ManageBilling).is_allowed());
        for a in Action::ALL {
            assert!(
                check(
                    &bookkeeper,
                    Permission::Matrix {
                        module: Module::Invoicing,
                        action: a
                    }
                )
                .is_allowed(),
                "invoicing:{a:?} should be granted"
            );
        }
        assert!(check(
            &bookkeeper,
            Permission::Matrix {
                module: Module::Tickets,
                action: Action::Create
            }
        )
        .is_allowed()
        .not());
        assert!(check(
            &bookkeeper,
            Permission::Matrix {
                module: Module::Tickets,
                action: Action::Edit
            }
        )
        .is_allowed()
        .not());
    }

    /// The senior-tech preset: a strict super-set of Technician (minus new
    /// grants the technician never had), with ticket close authority and
    /// time approvals — but deliberately NO financial visibility.
    #[test]
    fn senior_tech_escalates_tickets_and_time_but_not_financials() {
        let tech = BuiltinRole::Technician.permissions();
        let senior = BuiltinRole::SeniorTech.permissions();
        for p in &tech {
            assert!(
                senior.contains(p),
                "senior tech must hold every technician grant ({})",
                p.key()
            );
        }
        assert!(check(
            &senior,
            Permission::Matrix {
                module: Module::Tickets,
                action: Action::Delete
            }
        )
        .is_allowed());
        assert!(check(
            &senior,
            Permission::Matrix {
                module: Module::Tickets,
                action: Action::Export
            }
        )
        .is_allowed());
        assert!(check(&senior, Permission::ApproveTime).is_allowed());
        // The RB2 headline: no preset below owner/admin carries financial:read
        // unless it is the bookkeeper.
        assert!(check(&senior, Permission::ViewFinancials)
            .is_allowed()
            .not());
        assert!(check(&tech, Permission::ViewFinancials).is_allowed().not());
        assert!(check(
            &BuiltinRole::Dispatcher.permissions(),
            Permission::ViewFinancials
        )
        .is_allowed()
        .not());
    }

    /// The shipped preset roster is exactly the eight ADR-0004 roles and
    /// every one parses from its storage spelling (round-trip).
    #[test]
    fn preset_roster_is_pinned_and_round_trips() {
        assert_eq!(BuiltinRole::PRESETS.len(), 8);
        for role in BuiltinRole::PRESETS {
            assert_eq!(BuiltinRole::parse_preset(role.as_str()), Some(role));
        }
        assert_eq!(
            BuiltinRole::parse_preset("Senior Tech"),
            Some(BuiltinRole::SeniorTech)
        );
        assert_eq!(BuiltinRole::parse_preset("nope"), None);
    }

    /// `financial:read` is its own wire key (ADR 0004 RB2) — not a
    /// `special:` name, so custom roles address it like a matrix grant.
    #[test]
    fn financial_permission_carries_the_financial_read_key() {
        assert_eq!(Permission::ViewFinancials.key(), "financial:read");
    }

    /// Wave 47 lane A6: org usage is read by org members through their own
    /// membership (the endpoint needs no permission for a human), so the
    /// `org_usage:read` permission exists for SERVICE ACCOUNTS and custom
    /// roles — which is exactly why no shipped preset carries it. An owner
    /// or admin reads usage as a member like everyone else. Pinned because
    /// a preset grant here would read as "humans need a special grant to
    /// see their own usage", and the next lane copying the pattern from a
    /// preset would make that true.
    #[test]
    fn org_usage_read_is_a_machine_grant_no_preset_carries_it() {
        assert_eq!(Permission::ViewOrgUsage.key(), "org_usage:read");
        for role in BuiltinRole::PRESETS {
            assert!(
                !role.permissions().contains(&Permission::ViewOrgUsage),
                "{role:?} must not carry org_usage:read by default"
            );
        }
    }

    /// Wave 47 lane A8: the same shape as A6 — a human owner/admin manages a
    /// custom domain through the Admin matrix (`admin:edit`), and a human
    /// member reads it by membership. The two operational grants exist for
    /// SERVICE ACCOUNTS and custom roles, so no shipped preset carries them;
    /// a preset grant would read as "humans need a special permission to
    /// manage their own domain", and the next lane copying the pattern would
    /// make it so.
    #[test]
    fn org_custom_domain_grants_are_machine_only_no_preset_carries_them() {
        assert_eq!(
            Permission::ViewOrgCustomDomain.key(),
            "org_custom_domain:read"
        );
        assert_eq!(
            Permission::ManageOrgCustomDomain.key(),
            "org_custom_domain:manage"
        );
        for role in BuiltinRole::PRESETS {
            for perm in [
                Permission::ViewOrgCustomDomain,
                Permission::ManageOrgCustomDomain,
            ] {
                assert!(
                    !role.permissions().contains(&perm),
                    "{role:?} must not carry {} by default",
                    perm.key()
                );
            }
        }
    }

    // --- RB1 portal tier roster -------------------------------------------

    /// Wave 47 lane A7: the same shape as A6/A8. A human member reads the
    /// suspension notice state through their own membership, and an owner or
    /// admin manages it through the Admin matrix (`admin:edit` — the gate org
    /// settings use), so the one operational grant exists for SERVICE
    /// ACCOUNTS and custom roles. A preset grant here would read as "humans
    /// need a special permission to see a notice about their own org", and
    /// the next lane copying the pattern would make it true.
    #[test]
    fn org_suspension_read_is_a_machine_grant_no_preset_carries_it() {
        assert_eq!(Permission::ViewOrgSuspension.key(), "org_suspension:read");
        for role in BuiltinRole::PRESETS {
            assert!(
                !role.permissions().contains(&Permission::ViewOrgSuspension),
                "{role:?} must not carry org_suspension:read by default"
            );
        }
    }

    /// The portal tier matrix rendered by the roles surface must agree
    /// with the enforcement entry point for EVERY tier × capability cell.
    #[test]
    fn portal_tier_registry_agrees_with_enforcement() {
        use crate::entities::PortalRole;
        for tier in [
            PortalRole::Contact,
            PortalRole::Approver,
            PortalRole::ItManager,
        ] {
            for action in PortalAction::ALL {
                let enforced = check_portal(tier, action).is_allowed();
                let registry = portal_tier_allows(tier, action);
                assert_eq!(
                    enforced, registry,
                    "portal registry drift: {tier:?} × {action:?}"
                );
            }
        }
    }

    /// The two lower tiers never widen to in-house updates, whatever the
    /// registry refactor does (pinning the v1 invariant at the matrix).
    #[test]
    fn portal_it_manager_widening_stays_exact() {
        use crate::entities::PortalRole;
        assert!(portal_tier_allows(
            PortalRole::ItManager,
            PortalAction::ViewInHouseUpdates
        ));
        assert!(portal_tier_allows(PortalRole::Contact, PortalAction::ViewInHouseUpdates).not());
        assert!(portal_tier_allows(PortalRole::Approver, PortalAction::ViewInHouseUpdates).not());
    }

    trait Not {
        fn not(self) -> Self;
    }
    impl Not for bool {
        fn not(self) -> Self {
            !self
        }
    }

    // --- board-level RBAC matrix (TASKLIST 2.11) --------------------------

    use BoardOp as Op;
    use BoardRole as Role;

    fn org(verdict: bool) -> PolicyDecision {
        if verdict {
            PolicyDecision::Allowed
        } else {
            PolicyDecision::Denied {
                reason: "org-level denial".into(),
            }
        }
    }

    /// The complete expected role×operation matrix in isolation. Every cell
    /// is spelled out — no computed shortcuts — so a semantic change to any
    /// single cell fails this table loudly.
    fn expected_role_allows(role: Role, op: Op) -> bool {
        match (role, op) {
            (Role::Viewer, Op::View) => true,
            (Role::Viewer, Op::CreateTicket) => false,
            (Role::Viewer, Op::WorkTicket) => false,
            (Role::Viewer, Op::CloseTicket) => false,
            (Role::Viewer, Op::ConfigureBoard) => false,
            (Role::Viewer, Op::ManageMembers) => false,
            (Role::Member, Op::View) => true,
            (Role::Member, Op::CreateTicket) => true,
            (Role::Member, Op::WorkTicket) => true,
            (Role::Member, Op::CloseTicket) => false,
            (Role::Member, Op::ConfigureBoard) => false,
            (Role::Member, Op::ManageMembers) => false,
            (Role::Admin, Op::View) => true,
            (Role::Admin, Op::CreateTicket) => true,
            (Role::Admin, Op::WorkTicket) => true,
            (Role::Admin, Op::CloseTicket) => true,
            (Role::Admin, Op::ConfigureBoard) => true,
            (Role::Admin, Op::ManageMembers) => true,
        }
    }

    /// Exhaustive: 3 roles × 6 operations in isolation.
    #[test]
    fn board_role_operation_matrix_is_exhaustive_and_pinned() {
        for role in Role::ALL {
            for op in [
                Op::View,
                Op::CreateTicket,
                Op::WorkTicket,
                Op::CloseTicket,
                Op::ConfigureBoard,
                Op::ManageMembers,
            ] {
                assert_eq!(
                    board_role_allows(role, op),
                    expected_role_allows(role, op),
                    "{role:?} × {op:?} deviates from the pinned matrix"
                );
            }
        }
    }

    /// Exhaustive: 4 membership states × 6 ops × 2 org verdicts = 48
    /// compositions. Pins all three composition rules: passthrough without
    /// membership, deny-wins narrowing with one, and never-expanding.
    #[test]
    fn board_scope_composition_matrix_is_exhaustive() {
        let memberships: [Option<Role>; 4] = [
            None,
            Some(Role::Viewer),
            Some(Role::Member),
            Some(Role::Admin),
        ];
        for role in memberships {
            for op in [
                Op::View,
                Op::CreateTicket,
                Op::WorkTicket,
                Op::CloseTicket,
                Op::ConfigureBoard,
                Op::ManageMembers,
            ] {
                for org_allows in [true, false] {
                    let verdict = authorize_board_scoped(org(org_allows), role, op);
                    let expected = match role {
                        // Rule 1: no row ⇒ org verdict passes through.
                        None => org_allows,
                        // Rules 2+3: allowed only when BOTH layers allow.
                        Some(r) => org_allows && expected_role_allows(r, op),
                    };
                    assert_eq!(
                        verdict.is_allowed(),
                        expected,
                        "membership={role:?} op={op:?} org_allows={org_allows}"
                    );
                }
            }
        }
    }

    /// Deny wins even against an org superuser: an explicit viewer row on a
    /// board blocks ticket creation that the org matrix would grant.
    #[test]
    fn board_deny_beats_org_allow() {
        let verdict = authorize_board_scoped(
            PolicyDecision::Allowed,
            Some(Role::Viewer),
            Op::CreateTicket,
        );
        match verdict {
            PolicyDecision::Denied { reason } => assert!(reason.contains("viewer")),
            other => panic!("expected denial, got {other:?}"),
        }
    }

    /// A membership row never grants past an org-level denial.
    #[test]
    fn board_allow_never_expands_org_deny() {
        let verdict = authorize_board_scoped(
            PolicyDecision::Denied {
                reason: "missing permission".into(),
            },
            Some(Role::Admin),
            Op::View,
        );
        assert!(!verdict.is_allowed());
    }

    /// Round-trips through the storage representation.
    #[test]
    fn board_role_parse_round_trips_as_str() {
        for role in Role::ALL {
            assert_eq!(BoardRole::parse(role.as_str()), Some(role));
        }
        assert_eq!(BoardRole::parse("superuser"), None);
    }
}
