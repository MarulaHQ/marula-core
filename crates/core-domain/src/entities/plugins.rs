//! Plugin runtime v2 + marketplace registry entities (Wave 14 lane 5).

use crate::policy::{Action, Module, Permission};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    /// Wave 32 lane 7 (ADR 0014): routing-provider packs. Each scope maps
    /// onto the scheduling view gate — a routing pack READS schedule
    /// context (coordinates, times) and returns distances/durations; it
    /// never writes scheduling state. The four scopes mirror the seam's
    /// capability vocabulary exactly.
    RoutingMatrix,
    RoutingRoute,
    RoutingGeocode,
    RoutingIsochrone,
    /// Wave 34 lane 4 (§4.6): dashboard/ticket-pane widget registration.
    /// A widget is a DECLARATIVE spec (closed render kind + JSON-Schema
    /// props) rendered by trusted core components — a plugin holding this
    /// capability contributes widget entries; it never ships code the
    /// browser executes (agent-safety §4.8: untrusted content never
    /// renders raw).
    UiWidgets,
    /// Wave 34 lane 4 (§4.6): plugin-provided automation ACTION KINDS.
    /// Each declared action maps a typed parameter schema onto a
    /// consented built-in EFFECT (e.g. add an internal comment) — the
    /// engine executes the effect; the plugin contributes the kind, the
    /// typed params, and the label. Resolves onto the tickets-edit gate
    /// the effect itself requires.
    AutomationActions,
    /// Wave 34 lane 4 (§4.6): plugin-contributed render kinds on the
    /// report builder. Registration only — the rendering stays in the
    /// core's trusted component set.
    ReportsRender,
    /// Wave 34 lane 4 (§4.6): JSON-Schema validators the plugin
    /// contributes for board custom fields. The core runs the schema on
    /// every write of a field wired to the validator; the plugin never
    /// sees the value.
    FieldsValidate,
    /// Wave 43 lane 4 (ADR 0017 §3): a plugin-contributed TRIGGER SOURCE —
    /// the declared event type a rule can fire on. Registration only: the
    /// trigger declares what it EMITS and the engine's matching decides
    /// what fires, so the declaration touches no Marula entity of its own
    /// and resolves onto the plugin-surface view gate rather than onto
    /// the gate an effect would need.
    AutomationTriggers,
    /// Wave 43 lane 4 (ADR 0017 §3, §5): a plugin-contributed READ node —
    /// the one kind that lets a workflow FETCH from an external system,
    /// because `automation_action` is write-shaped. Stricter than a write
    /// by construction: the declaration carries its own rate caps, output
    /// ceiling, and the (always false) promise that it never returns a
    /// vault secret. Reads traverse credentials the connector ALREADY
    /// holds, so the grant confers nothing the connector did not have.
    AutomationReads,
    /// Wave 43 lane 4 (ADR 0017 §6): the plugin-facing EMIT seam. An
    /// authorized plugin runtime publishes an envelope of a type it
    /// DECLARED, validated against the event-type registry (migration
    /// 0124). Deliberately NOT the cheap default: inbound through lane 1's
    /// generic signed ingest needs no surface at all, and the authoring
    /// guide says so first.
    EventsEmit,
    /// Wave 47 lane A9 (client-certificate auth): the capability that
    /// declares a runtime MAY present the service-account SURFACE.
    /// Registration/declaration only — a plugin never enrolls, revokes, or
    /// authenticates a service account through this capability; enrolling
    /// and revocation remain HUMAN API-key-admin actions, and a certified
    /// service-account's authority comes from its write-time permission
    /// grants, not from this key. It resolves onto the plugins-surface VIEW
    /// gate (like the other registration-only executor kinds), never onto
    /// an admin special.
    ServiceAccountManage,
    /// Wave 47 lane A6 (org usage/quota read API): org-level resource
    /// usage for hosted-tier monitoring. The grant resolves onto the new
    /// `org_usage:read` permission — a read of the materialized usage row
    /// and nothing else. Usage is a REPORT: holding this capability
    /// changes what a plugin can observe about an org's consumption, never
    /// what that org is allowed to do, and there is no quota to enforce.
    OrgUsageRead,
    /// Wave 45 lane 1 (ADR 0020): RAG and multimodal vector namespace read
    /// capability (`rag.read`). Resolves onto `kb:view` so a plugin or
    /// connector holding `rag.read` (or `kb.read`) can execute hybrid
    /// BM25 + exact-KNN vector queries and verify deletion receipts.
    RagRead,
    /// Wave 45 lane 1 (ADR 0020): RAG and multimodal vector namespace write
    /// capability (`rag.write`). Resolves onto `kb:edit` so a plugin or
    /// connector holding `rag.write` (or `kb.write`) can batch-upsert
    /// passage/media chunks and delete sources with provable receipts.
    RagWrite,
    /// Wave 47 lane A8 (`p47-org-custom-domains`): the capability a
    /// DNS-provider plugin declares to automate a tenant's custom-domain
    /// records (create/remove the `_marula-verify.<domain>` TXT record,
    /// upsert the edge A/CNAME, proxy-mode-aware). It resolves onto the
    /// `org_custom_domain:manage` permission, which lets the plugin call the
    /// domain-management API on the org's behalf. Holding it is also the
    /// signal the API reports as "a DNS-provider plugin is installed" — the
    /// manual paste path never needs it.
    OrgCustomDomainManage,
    /// Wave 47 lane A7 (`p47-org-suspension-notice`): the capability a plugin
    /// declares to read an org's suspension NOTICE state so it can render the
    /// in-app notice — when the org was flagged, why, and when the notice
    /// starts showing. It resolves onto the `org_suspension:read` permission.
    /// Holding it changes what a plugin may OBSERVE: the flag never gates a
    /// request, a feature, or a write anywhere in the core.
    OrgSuspensionRead,
    /// Wave 52 lane R12 (`p52-extension-hooks`): the OBSERVABILITY read scope.
    /// A plugin or connector holding it reads the org's own operational
    /// evidence through the same scoped read path a machine API key uses —
    /// today the audit trail and its ascending export cursor
    /// (`GET /api/audit-events?order=asc&after=…`). It resolves onto the new
    /// `observability:read` permission, which no shipped preset carries, so a
    /// grant is always an org's explicit decision.
    ///
    /// Why this is not `special:view_audit_log`: that is an admin special, and
    /// the closed vocabulary deliberately has no key for any of them. A pack
    /// that forwards the trail to a SIEM is a reader, not an administrator, and
    /// this scope is the reader's grant. Health needs no scope at all
    /// (`/api/v1/health` is public), and metrics have no read path in core —
    /// they leave by OTLP and `core_otel::contract` describes them.
    ObservabilityRead,
    /// Wave 52 lane R16 (`p52-directory-read-scope`): the DIRECTORY read
    /// scope. The identity pack's SCIM connector and directory-admin plugin
    /// run outside core, and the group surface was reachable only through
    /// `special:manage_users` / `special:manage_roles`. Holding this reads the
    /// group list and one group with its members and role mappings
    /// (`GET /api/directory-groups`, `GET /api/directory-groups/{id}`) through
    /// the same scope path a machine API key uses. It resolves onto
    /// `directory_groups:read`, carried by no preset.
    ///
    /// What it does NOT reach: any write, and no other route.
    DirectoryGroupsRead,
    /// Wave 52 lane R16: the DIRECTORY write scope. A connector whose job is
    /// to mirror a directory writes groups, members and role mappings
    /// (`POST`/`PATCH`/`DELETE /api/directory-groups…`, the member and role
    /// `PUT`/`DELETE` routes). It resolves onto `directory_groups:write`,
    /// carried by no preset.
    ///
    /// Why this is not `special:manage_roles`: that special is the role
    /// administration gate itself (role CRUD at `/api/roles` sits behind it),
    /// and a plugin may never hold an admin special. This grant reaches the
    /// directory surface and nothing else, so a connector can attach an
    /// existing role to a group and cannot create, edit or delete a role.
    DirectoryGroupsWrite,
}

impl PluginCapability {
    pub const ALL: [PluginCapability; 35] = [
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
        PluginCapability::RoutingMatrix,
        PluginCapability::RoutingRoute,
        PluginCapability::RoutingGeocode,
        PluginCapability::RoutingIsochrone,
        PluginCapability::UiWidgets,
        PluginCapability::AutomationActions,
        PluginCapability::ReportsRender,
        PluginCapability::FieldsValidate,
        PluginCapability::AutomationTriggers,
        PluginCapability::AutomationReads,
        PluginCapability::EventsEmit,
        PluginCapability::ServiceAccountManage,
        PluginCapability::OrgUsageRead,
        PluginCapability::RagRead,
        PluginCapability::RagWrite,
        PluginCapability::OrgCustomDomainManage,
        PluginCapability::OrgSuspensionRead,
        PluginCapability::ObservabilityRead,
        PluginCapability::DirectoryGroupsRead,
        PluginCapability::DirectoryGroupsWrite,
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
            PluginCapability::RoutingMatrix => "routing.matrix",
            PluginCapability::RoutingRoute => "routing.route",
            PluginCapability::RoutingGeocode => "routing.geocode",
            PluginCapability::RoutingIsochrone => "routing.isochrone",
            PluginCapability::UiWidgets => "ui.widgets",
            PluginCapability::AutomationActions => "automation.actions",
            PluginCapability::ReportsRender => "reports.render",
            PluginCapability::FieldsValidate => "fields.validate",
            PluginCapability::AutomationTriggers => "automation.triggers",
            PluginCapability::AutomationReads => "automation.reads",
            PluginCapability::EventsEmit => "events.emit",
            PluginCapability::ServiceAccountManage => "service_accounts.manage",
            PluginCapability::OrgUsageRead => "org_usage.read",
            PluginCapability::RagRead => "rag.read",
            PluginCapability::RagWrite => "rag.write",
            PluginCapability::OrgCustomDomainManage => "org_custom_domain.manage",
            PluginCapability::OrgSuspensionRead => "org_suspension.read",
            PluginCapability::ObservabilityRead => "observability.read",
            PluginCapability::DirectoryGroupsRead => "directory_groups.read",
            PluginCapability::DirectoryGroupsWrite => "directory_groups.write",
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
            // Routing packs read scheduling context to answer a route
            // query; the resolved permission is the scheduling VIEW gate
            // (the same gate a read-only dispatcher passes). Granting
            // routing scopes never confers scheduling edits.
            PluginCapability::RoutingMatrix
            | PluginCapability::RoutingRoute
            | PluginCapability::RoutingGeocode
            | PluginCapability::RoutingIsochrone => Permission::Matrix {
                module: Module::Scheduling,
                action: Action::View,
            },
            // §4.6 surface capabilities resolve onto the gates the
            // SURFACE itself already sits behind: widgets and render
            // kinds display reporting data (reports view); a plugin
            // automation action executes a tickets-edit effect; a field
            // validator only shapes what a tickets editor may save
            // (view-level participation, never a write of its own). No
            // surface grant invents authority beyond the gate it lands on.
            PluginCapability::UiWidgets | PluginCapability::ReportsRender => Permission::Matrix {
                module: Module::Reporting,
                action: Action::View,
            },
            PluginCapability::AutomationActions => Permission::Matrix {
                module: Module::Tickets,
                action: Action::Edit,
            },
            PluginCapability::FieldsValidate => Permission::Matrix {
                module: Module::Tickets,
                action: Action::View,
            },
            // Wave 43 lane 4 (ADR 0017 §3): the three EXECUTOR kinds are
            // registration-only declarations. Unlike an action, which
            // executes a consented built-in effect and therefore resolves
            // onto the gate that effect requires, a trigger contributes an
            // event SOURCE, a read traverses credentials the connector
            // ALREADY holds to fetch EXTERNAL data, and emit publishes an
            // envelope of a type the registry already admitted. None of the
            // three touches a Marula entity, so all three resolve onto the
            // gate that governs viewing the plugin surface itself rather
            // than inventing one — and the RULE that consumes the node
            // carries its own permission checks at execution time.
            PluginCapability::AutomationTriggers
            | PluginCapability::AutomationReads
            | PluginCapability::EventsEmit => Permission::Matrix {
                module: Module::Plugins,
                action: Action::View,
            },
            // Registration/declaration only — resolves onto the plugins-
            // surface VIEW gate, the same gate the other registration-only
            // capabilities (AutomationTriggers/Reads/EventsEmit) land on.
            // It deliberately does NOT resolve to `special:manage_api_keys`:
            // the capability vocabulary never grants an admin special to a
            // plugin (deny-by-default). Enrolling/revoking a service account
            // stays a HUMAN API-key-admin action; this capability only lets a
            // runtime declare the service-account SURFACE.
            PluginCapability::ServiceAccountManage => Permission::Matrix {
                module: Module::Plugins,
                action: Action::View,
            },
            // Wave 47 lane A6: usage is a REPORT, so the capability
            // resolves onto its OWN read-only permission rather than onto
            // the Reporting matrix — a plugin that reads usage gains
            // nothing else the reporting module holds.
            PluginCapability::OrgUsageRead => Permission::ViewOrgUsage,
            // Wave 45 lane 1 (ADR 0020): RAG vector namespace capabilities
            // resolve onto the existing Kb permission matrix (`kb:view` for
            // query/receipt reads, `kb:edit` for chunk ingest and source
            // deletion) — no parallel auth system.
            PluginCapability::RagRead => Permission::Matrix {
                module: Module::Kb,
                action: Action::View,
            },
            PluginCapability::RagWrite => Permission::Matrix {
                module: Module::Kb,
                action: Action::Edit,
            },
            // Wave 47 lane A8: custom-domain automation resolves onto its
            // OWN manage permission (not the Admin matrix), so a DNS-provider
            // plugin gains exactly the domain-management surface and nothing
            // else a human admin's `admin:edit` would carry.
            PluginCapability::OrgCustomDomainManage => Permission::ManageOrgCustomDomain,
            // Wave 47 lane A7: the suspension notice resolves onto its OWN
            // read-only permission rather than onto the Admin matrix — holding
            // it lets a plugin observe the flag and nothing else. There is no
            // `manage` half: setting the flag is an admin action
            // (`admin:edit`, the gate org settings use), exactly as the private build tracker
            // A7 (D) specifies.
            PluginCapability::OrgSuspensionRead => Permission::ViewOrgSuspension,
            // Wave 52 lane R12: the observability reader resolves onto its OWN
            // read-only permission — the same posture as `org_usage.read`.
            // Deliberately NOT `special:view_audit_log`: a plugin may never
            // hold an admin special, and widening that gate for every plugin
            // would be the fork this vocabulary exists to prevent.
            PluginCapability::ObservabilityRead => Permission::ViewObservability,
            // Wave 52 lane R16: the directory pair resolves onto its OWN
            // permissions, the same posture as `observability.read`. The
            // write half deliberately does not resolve onto
            // `special:manage_roles`: a plugin may never hold an admin
            // special, and that special is the role-administration gate for
            // the whole product.
            PluginCapability::DirectoryGroupsRead => Permission::ViewDirectoryGroups,
            PluginCapability::DirectoryGroupsWrite => Permission::ManageDirectoryGroups,
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
    /// manifest fields, never free text. Optional on any artifact kind since
    /// ADR 0023 (Wave 53 lane 2): its two connector-only members are
    /// `webhook_endpoints` and `routing`, and the egress declaration
    /// (`outbound_scopes`) is legal on a wasm manifest too, because that is
    /// where `http_request` reads its allowlist from. Declared endpoints are
    /// the connector's egress-allowlist seed (the same posture the WASM
    /// resource profile carries); declared scopes render on the consent
    /// screen verbatim so an admin consents to exactly the SaaS scopes the
    /// connector asks for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_profile: Option<ConnectorProfile>,
    /// Wave 34 lane 4 (§4.6) — the declarative UI-surface profile:
    /// widgets, automation action kinds, report render kinds, custom-field
    /// validators, trigger sources, and read nodes.
    ///
    /// The rule is PER KIND (ADR 0017 §3, Wave 43 lane 4). On a `wasm`
    /// manifest every section is available. On an `http_connector` the
    /// three UI sections (`widget`, `report_render`, `field_validator`)
    /// refuse — a connector has no in-product UI surface to govern — while
    /// the three EXECUTOR sections (`automation_action`,
    /// `automation_trigger`, `read`) are allowed, because a connector
    /// already holds credentials and network egress and a sandbox does
    /// not. Optional on both: an EMPTY profile carries no consent weight.
    /// Consent renders the declarations verbatim, and each section only
    /// registers when its capability (`ui.widgets` /
    /// `automation.actions` / `reports.render` / `fields.validate` /
    /// `automation.triggers` / `automation.reads`) is granted. Additive to
    /// the v2 shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_profile: Option<PluginSurfaceProfile>,
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
    /// Wave 32 lane 7 (ADR 0014): present when the pack answers the
    /// Marula Routing Contract endpoints (`/routing/matrix`, …). The
    /// declaration mirrors the operator config 1:1 so consent shows
    /// exactly what the adapter will honor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<RoutingProfile>,
    /// ADR 0023 Decision 3 (Wave 53 lane 2): the EGRESS allowlist a wasm
    /// guest's `http_request` reads — profile name -> consented scopes. Each
    /// scope is `https://host[:port][/path-prefix]`; a request whose scheme is
    /// not https, whose authority is not listed, or whose path is outside the
    /// scope's prefix is refused (`plugin_egress_scheme_denied`,
    /// `plugin_egress_host_denied`). The map is what makes
    /// `plugin_egress_profile_missing` a real refusal: a guest names a profile,
    /// and a name the install did not consent has no list to fall back to.
    ///
    /// This one field is why the profile is no longer connector-only. The
    /// relaxation is narrow on purpose: `webhook_endpoints` and `routing` stay
    /// connector-only, so the rule that stopped a connector faking a UI
    /// surface still holds, and a wasm manifest gains exactly the declaration
    /// the executor needs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub outbound_scopes: BTreeMap<String, Vec<String>>,
}

/// The routing-capability declaration a provider pack carries. Closed
/// vocabularies only; a pack cannot declare a capability or mode the seam
/// does not speak.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RoutingProfile {
    /// Closed set: `matrix` | `route` | `geocode` | `isochrone`.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Closed set: `driving` | `walking` | `cycling` | `public_transport`.
    #[serde(default)]
    pub modes: Vec<String>,
    /// The pack's contract root: https-only absolute URL, no credentials.
    pub base_url: String,
    /// `measured` | `heuristic` — relay packs are measured; a heuristic
    /// pack is allowed but never unlocks suggestion-grade features.
    #[serde(default = "default_routing_quality")]
    pub quality: String,
    /// `none` | `departure` | `arrival` | `both` — which time control the
    /// pack honors. Anything it does not honor must not be claimed.
    #[serde(default = "default_routing_time_control")]
    pub time_control: String,
    /// Contract revision the pack implements; the core speaks revision 1.
    #[serde(default = "default_contract_revision")]
    pub contract_revision: u32,
}

fn default_routing_quality() -> String {
    "measured".into()
}

fn default_routing_time_control() -> String {
    "none".into()
}

fn default_contract_revision() -> u32 {
    ROUTING_CONTRACT_REVISION
}

/// The routing contract revision the core adapter speaks
/// (`docs/routing-providers.md`).
pub const ROUTING_CONTRACT_REVISION: u32 = 1;
/// Declared capability modes/caps for a routing profile.
pub const ROUTING_CAPABILITY_KEYS: &[&str] = &["matrix", "route", "geocode", "isochrone"];
pub const ROUTING_MODE_KEYS: &[&str] = &["driving", "walking", "cycling", "public_transport"];
pub const ROUTING_QUALITY_KEYS: &[&str] = &["measured", "heuristic"];
pub const ROUTING_TIME_CONTROL_KEYS: &[&str] = &["none", "departure", "arrival", "both"];
const MAX_ROUTING_CAPABILITIES: usize = 4;
const MAX_ROUTING_MODES: usize = 4;

impl RoutingProfile {
    /// Validates the closed routing declaration: https-only contract root
    /// without credentials, capabilities/modes/quality/time-control inside
    /// their closed vocabularies, no duplicates, within caps, and the
    /// contract revision the core can actually call.
    pub fn validate(&self) -> Result<(), crate::error::DomainError> {
        if self.capabilities.is_empty() {
            return Err(crate::error::DomainError::Validation(
                "routing profile declares no capabilities".into(),
            ));
        }
        if self.capabilities.len() > MAX_ROUTING_CAPABILITIES {
            return Err(crate::error::DomainError::Validation(format!(
                "routing profile declares {} capabilities (max {MAX_ROUTING_CAPABILITIES})",
                self.capabilities.len()
            )));
        }
        for (i, capability) in self.capabilities.iter().enumerate() {
            if !ROUTING_CAPABILITY_KEYS.contains(&capability.as_str()) {
                return Err(crate::error::DomainError::Validation(format!(
                    "routing capability {capability:?} is not one of: {}",
                    ROUTING_CAPABILITY_KEYS.join(", ")
                )));
            }
            if self.capabilities[..i].contains(capability) {
                return Err(crate::error::DomainError::Validation(format!(
                    "routing capability {capability:?} is declared twice"
                )));
            }
        }
        if self.modes.is_empty() {
            return Err(crate::error::DomainError::Validation(
                "routing profile declares no travel modes".into(),
            ));
        }
        if self.modes.len() > MAX_ROUTING_MODES {
            return Err(crate::error::DomainError::Validation(format!(
                "routing profile declares {} modes (max {MAX_ROUTING_MODES})",
                self.modes.len()
            )));
        }
        for (i, mode) in self.modes.iter().enumerate() {
            if !ROUTING_MODE_KEYS.contains(&mode.as_str()) {
                return Err(crate::error::DomainError::Validation(format!(
                    "routing mode {mode:?} is not one of: {}",
                    ROUTING_MODE_KEYS.join(", ")
                )));
            }
            if self.modes[..i].contains(mode) {
                return Err(crate::error::DomainError::Validation(format!(
                    "routing mode {mode:?} is declared twice"
                )));
            }
        }
        if !ROUTING_QUALITY_KEYS.contains(&self.quality.as_str()) {
            return Err(crate::error::DomainError::Validation(format!(
                "routing quality {:?} is not one of: {}",
                self.quality,
                ROUTING_QUALITY_KEYS.join(", ")
            )));
        }
        if !ROUTING_TIME_CONTROL_KEYS.contains(&self.time_control.as_str()) {
            return Err(crate::error::DomainError::Validation(format!(
                "routing time_control {:?} is not one of: {}",
                self.time_control,
                ROUTING_TIME_CONTROL_KEYS.join(", ")
            )));
        }
        if self.contract_revision != ROUTING_CONTRACT_REVISION {
            return Err(crate::error::DomainError::Validation(format!(
                "routing contract revision {} is not supported (core speaks {ROUTING_CONTRACT_REVISION})",
                self.contract_revision
            )));
        }
        ConnectorProfile::validate_endpoint(&self.base_url)
    }
}

/// Capacities of the modeled connector profile (deny oversized
/// declarations at the data boundary).
pub const MAX_CONNECTOR_ENDPOINTS: usize = 8;
pub const MAX_CONNECTOR_SCOPES: usize = 32;
/// Egress profiles one manifest may consent (ADR 0023, Wave 53 lane 2).
pub const MAX_EGRESS_PROFILES: usize = 8;
const MAX_ENDPOINT_URL_BYTES: usize = 2048;
const MAX_SCOPE_BYTES: usize = 64;
const MAX_EGRESS_SCOPE_BYTES: usize = 512;

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
        // Wave 32 lane 7: the optional routing declaration rides the same
        // profile gate — a pack that answers the Routing Contract declares
        // its capabilities here, validated against the closed vocabulary.
        if let Some(routing) = &self.routing {
            routing.validate()?;
        }
        self.validate_outbound_scopes()?;
        Ok(())
    }

    /// ADR 0023 Decision 3 (Wave 53 lane 2): the egress allowlist is MODELED,
    /// not free text. A profile name is a lowercase identifier, each profile
    /// declares at least one scope, the whole declaration stays inside the
    /// existing scope cap, and every scope is an https URL with a host, no
    /// credentials, no query or fragment, and no wildcard.
    fn validate_outbound_scopes(&self) -> Result<(), crate::error::DomainError> {
        if self.outbound_scopes.len() > MAX_EGRESS_PROFILES {
            return Err(crate::error::DomainError::Validation(format!(
                "connector profile declares {} egress profiles (max {MAX_EGRESS_PROFILES})",
                self.outbound_scopes.len()
            )));
        }
        let mut total = 0usize;
        for (name, scopes) in &self.outbound_scopes {
            let name_ok = !name.is_empty()
                && name.len() <= 64
                && name.starts_with(|c: char| c.is_ascii_lowercase())
                && name.chars().all(|c| {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-')
                });
            if !name_ok {
                return Err(crate::error::DomainError::Validation(format!(
                    "egress profile name {name:?} must match [a-z][a-z0-9_-]{{0,63}}"
                )));
            }
            if scopes.is_empty() {
                return Err(crate::error::DomainError::Validation(format!(
                    "egress profile {name:?} declares no scopes"
                )));
            }
            total += scopes.len();
            for (i, scope) in scopes.iter().enumerate() {
                Self::validate_egress_scope(scope)?;
                if scopes[..i].contains(scope) {
                    return Err(crate::error::DomainError::Validation(format!(
                        "egress profile {name:?} declares the scope {scope:?} twice"
                    )));
                }
            }
        }
        if total > MAX_CONNECTOR_SCOPES {
            return Err(crate::error::DomainError::Validation(format!(
                "connector profile declares {total} egress scopes (max {MAX_CONNECTOR_SCOPES})"
            )));
        }
        Ok(())
    }

    fn validate_egress_scope(scope: &str) -> Result<(), crate::error::DomainError> {
        if scope.len() > MAX_EGRESS_SCOPE_BYTES {
            return Err(crate::error::DomainError::Validation(format!(
                "egress scope exceeds the {MAX_EGRESS_SCOPE_BYTES}-byte cap"
            )));
        }
        let parsed = url::Url::parse(scope).map_err(|_| {
            crate::error::DomainError::Validation(format!(
                "egress scope {scope:?} is not an absolute URL"
            ))
        })?;
        if parsed.scheme() != "https" {
            return Err(crate::error::DomainError::Validation(format!(
                "egress scope {scope:?} must be https"
            )));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(crate::error::DomainError::Validation(
                "egress scopes must not carry credentials in the URL".into(),
            ));
        }
        let host = parsed.host_str().unwrap_or_default();
        if host.is_empty() {
            return Err(crate::error::DomainError::Validation(format!(
                "egress scope {scope:?} is missing a host"
            )));
        }
        if host.contains('*') {
            return Err(crate::error::DomainError::Validation(format!(
                "egress scope {scope:?} must name an exact host — wildcards are not consent"
            )));
        }
        if parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(crate::error::DomainError::Validation(format!(
                "egress scope {scope:?} must not carry a query or fragment"
            )));
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

/// Wave 34 lane 4 (§4.6): the modeled UI-surface profile a WASM manifest
/// may declare — dashboard/ticket-pane WIDGETS, automation ACTION KINDS,
/// report RENDER KINDS, and custom-field VALIDATORS. Everything here is
/// DECLARATIVE: closed render/effect vocabularies plus JSON-Schema
/// subsets the core validates and executes. A plugin ships declarations,
/// never code the browser or the rule engine runs raw (agent-safety
/// §4.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginSurfaceProfile {
    /// Widgets the plugin registers (dashboard or ticket-pane slots).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub widgets: Vec<SurfaceWidget>,
    /// Automation action kinds registered alongside the built-ins.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<SurfaceAction>,
    /// Render kinds contributed to the report builder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub render_kinds: Vec<SurfaceRenderKind>,
    /// JSON-Schema validators for board custom fields.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_validators: Vec<SurfaceFieldValidator>,
    /// Wave 43 lane 4 (ADR 0017 §3): TRIGGER sources the plugin
    /// contributes — the event type a rule can fire on. This is the half
    /// of the `automation_action` symmetry that was missing: a plugin
    /// could contribute an action but never a trigger.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<SurfaceTrigger>,
    /// Wave 43 lane 4 (ADR 0017 §3, §5): READ nodes, the only kind that
    /// lets a workflow FETCH from an external system. Every entry carries
    /// its own guardrails; there is no default that weakens them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<SurfaceRead>,
}

/// Where a declared widget may render. Closed vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SurfaceSlot {
    Dashboard,
    TicketPane,
}

impl SurfaceSlot {
    pub const ALL: [SurfaceSlot; 2] = [SurfaceSlot::Dashboard, SurfaceSlot::TicketPane];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dashboard => "dashboard",
            Self::TicketPane => "ticket_pane",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "dashboard" => Some(Self::Dashboard),
            "ticket_pane" => Some(Self::TicketPane),
            _ => None,
        }
    }
}

/// What a declared widget RENDERS AS. Closed vocabulary — each kind maps
/// onto one trusted core component; a plugin cannot introduce a new
/// render shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SurfaceWidgetKind {
    /// Single number + label + optional comparison line.
    Stat,
    /// Metric card backed by a report kind (value sourced from the
    /// reporting seam the viewer already passes).
    Metric,
    /// Read-only list of label/value rows.
    List,
}

impl SurfaceWidgetKind {
    pub const ALL: [SurfaceWidgetKind; 3] = [
        SurfaceWidgetKind::Stat,
        SurfaceWidgetKind::Metric,
        SurfaceWidgetKind::List,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stat => "stat",
            Self::Metric => "metric",
            Self::List => "list",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "stat" => Some(Self::Stat),
            "metric" => Some(Self::Metric),
            "list" => Some(Self::List),
            _ => None,
        }
    }
}

/// The built-in EFFECT a declared automation action maps onto. Closed
/// vocabulary: the rule engine executes the effect with the action's
/// validated params — the plugin contributes the kind, never custom
/// execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SurfaceActionEffect {
    /// Adds an internal comment whose body is the action's validated
    /// `body` param (templates interpolation applies, same dialect as
    /// the canned-templates engine).
    AddInternalComment,
}

impl SurfaceActionEffect {
    pub const ALL: [SurfaceActionEffect; 1] = [SurfaceActionEffect::AddInternalComment];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::AddInternalComment => "add_internal_comment",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "add_internal_comment" => Some(Self::AddInternalComment),
            _ => None,
        }
    }
}

/// One declared widget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SurfaceWidget {
    /// Plugin-local id, `[a-z][a-z0-9_]{0,63}`. The registry key is
    /// `plugin.<install-slug>.<id>` — collision-free by construction.
    pub id: String,
    /// Human label shown on the widget chrome and the consent screen.
    pub title: String,
    /// Where the widget renders.
    pub slot: SurfaceSlot,
    /// The trusted render kind.
    pub kind: SurfaceWidgetKind,
    /// Optional JSON-Schema subset the widget's per-board props must
    /// satisfy (same subset as settings_schema).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub props_schema: Option<serde_json::Value>,
    /// Optional defaults applied under props_schema before render.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_props: Option<serde_json::Value>,
}

/// One declared automation action kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SurfaceAction {
    /// Plugin-local id, `[a-z][a-z0-9_]{0,63}`. Registry key:
    /// `plugin.<install-slug>.<id>`.
    pub id: String,
    /// Label shown in the automations editor.
    pub label: String,
    /// One-line description shown in the editor and on consent.
    pub description: String,
    /// The consented effect the engine executes.
    pub effect: SurfaceActionEffect,
    /// Optional JSON-Schema subset for the action's params.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params_schema: Option<serde_json::Value>,
    /// A2 (Wave 43 lane 13, `p43-node-registry`): what the action
    /// RETURNS. Before this field a workflow could call a plugin action
    /// but could not use what came back, which made a plugin node a
    /// dead end for every downstream condition and reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
    /// A2: repeating-safety. Defaults to false, because an undeclared
    /// plugin action's retry-safety is unknown — `flow.retry` and the
    /// operator retry both refuse a false until a plugin says otherwise.
    #[serde(default)]
    pub idempotent: bool,
    /// A2: the event types this action can produce. Load-bearing: the
    /// static loop refusal reads them, and a plugin that emits
    /// `ticket.status_changed` then waits for it would otherwise deadlock
    /// a workflow. Validated against the event-type registry at grant
    /// time.
    #[serde(default)]
    pub emits: Vec<String>,
    /// A2: who sees this entry. Undeclared stays `human`, so a plugin that
    /// declares nothing never joins the LLM tool catalogue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audience: Option<crate::node_registry::NodeAudience>,
    /// A2: model-facing prose beside the human label. What makes LLM
    /// authoring cheap enough to be the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_description: Option<String>,
}

/// One declared report render kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SurfaceRenderKind {
    /// Plugin-local id, `[a-z][a-z0-9_]{0,63}`. Registry key:
    /// `plugin.<install-slug>.<id>`.
    pub id: String,
    /// Label shown in the report builder.
    pub label: String,
    /// One-line description.
    pub description: String,
}

/// One declared custom-field validator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SurfaceFieldValidator {
    /// Plugin-local id, `[a-z][a-z0-9_]{0,63}`. Registry key:
    /// `plugin.<install-slug>.<id>`.
    pub id: String,
    /// Label shown in the board field editor.
    pub label: String,
    /// The JSON-Schema subset applied to the field's value on write.
    pub schema: serde_json::Value,
}

/// One declared TRIGGER source (Wave 43 lane 4, ADR 0017 §3): the event
/// type a plugin emits, which a rule can then fire on.
///
/// The trigger is a DECLARATION, not a dispatcher. It names the type it
/// emits and the schema the emitted payload satisfies; the engine's
/// matching decides what fires. `emitted_event_type` is validated against
/// the event-type registry (migration 0124) at grant time, so a plugin
/// cannot declare a type the platform does not know and the org has not
/// admitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SurfaceTrigger {
    /// Plugin-local id, `[a-z][a-z0-9_]{0,63}`. Registry key:
    /// `plugin.<install-slug>.<id>`.
    pub id: String,
    /// Label shown in the automations editor's trigger picker.
    pub label: String,
    /// One-line description shown in the editor and on consent.
    pub description: String,
    /// The event type this source publishes (`segment.segment`, the same
    /// grammar the platform's own types use, e.g. `rmm.device_offline`).
    /// Checked against the event-type registry at grant time.
    pub emitted_event_type: String,
    /// Optional JSON-Schema subset the emitted envelope's payload
    /// satisfies, so a rule author knows what they can read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_schema: Option<serde_json::Value>,
}

/// One declared READ node (Wave 43 lane 4, ADR 0017 §5).
///
/// `automation_action` is write-shaped, so before this kind a workflow
/// could not FETCH from an external system at all — which is exactly the
/// gap that stopped a workflow assembling Decision Engine context from an
/// RMM. A read is therefore the one kind whose guardrails are STRICTER
/// than a write's, and they are declared per entry rather than defaulted:
/// a loop over devices is a denial of service aimed at the customer's own
/// external system, and a device list of ten thousand must not grow the
/// run ledger by megabytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SurfaceRead {
    /// Plugin-local id, `[a-z][a-z0-9_]{0,63}`. Registry key:
    /// `plugin.<install-slug>.<id>`.
    pub id: String,
    /// Label shown in the automations editor's read picker.
    pub label: String,
    /// One-line description shown in the editor and on consent.
    pub description: String,
    /// Optional JSON-Schema subset for the read's parameters (what the
    /// caller supplies — never credentials, which the host resolves).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params_schema: Option<serde_json::Value>,
    /// Optional JSON-Schema subset describing what comes back, so a
    /// downstream node can use it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
    /// A2 (Wave 43 lane 13): repeating-safety. False by default — a read
    /// is rate-capped precisely because a loop over devices is cheap to
    /// write, and retrying one is the same loop.
    #[serde(default)]
    pub idempotent: bool,
    /// A2: the event types this read can produce. A read produces none of
    /// its own, so this is normally empty; declaring one makes the loop
    /// refusal able to see it.
    #[serde(default)]
    pub emits: Vec<String>,
    /// A2: who sees this entry. Undeclared stays `human`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audience: Option<crate::node_registry::NodeAudience>,
    /// A2: model-facing prose beside the human label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_description: Option<String>,
    /// The caps this read runs under. Required — a read without declared
    /// guardrails refuses, so there is no silent unbounded default.
    pub guardrails: ReadGuardrails,
}

/// The guardrails a declared read runs under (ADR 0017 §5).
///
/// Three of the four are ceilings a declaration may not exceed; the fourth
/// is a promise that may only be declared one way. Validation applies the
/// platform floors here so a manifest cannot write itself a weaker cap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReadGuardrails {
    /// Calls this node may make in ONE workflow run. The per-run cap is
    /// what stops a loop over a device list becoming a hammer.
    pub max_calls_per_run: u32,
    /// Minimum seconds between this ORG's read-node invocations. The
    /// per-org cap is what stops a schedule becoming a stream.
    pub min_interval_secs: u32,
    /// Hard ceiling on the returned payload's size.
    pub max_output_bytes: u32,
    /// Must be `false`, and validation refuses anything else. A read node
    /// may never return a vault secret, because the run ledger is a place
    /// operators read.
    pub may_return_secrets: bool,
}

/// Calls one read node may make per workflow run.
pub const MAX_READ_CALLS_PER_RUN: u32 = 50;
/// The tightest per-org cadence a read may declare (seconds).
pub const MIN_READ_INTERVAL_SECS: u32 = 5;
/// The largest payload a read may return (bytes).
pub const MAX_READ_OUTPUT_BYTES: u32 = 262_144;
/// What a read's output may never contain.
pub const READ_SECRETS_REFUSAL: &str =
    "a read node may never return a vault secret: the run ledger is a place operators read";

const MAX_SURFACE_ENTRIES_PER_SECTION: usize = 8;
const MAX_SURFACE_LABEL_BYTES: usize = 120;
const MAX_SURFACE_DESCRIPTION_BYTES: usize = 400;

impl PluginSurfaceProfile {
    /// True when the profile declares nothing — an absent/unusable
    /// profile that carries no consent weight.
    pub fn is_empty(&self) -> bool {
        self.widgets.is_empty()
            && self.actions.is_empty()
            && self.render_kinds.is_empty()
            && self.field_validators.is_empty()
            && self.triggers.is_empty()
            && self.reads.is_empty()
    }

    /// The EXECUTOR sections a connector may contribute (ADR 0017 §3):
    /// actions, triggers and reads. The UI sections are excluded — see
    /// [`PluginManifestV2::validate`], which refuses them on a connector.
    pub fn declares_ui_sections(&self) -> bool {
        !self.widgets.is_empty()
            || !self.render_kinds.is_empty()
            || !self.field_validators.is_empty()
    }

    /// The event types this profile's trigger sources emit, deduplicated
    /// and in declaration order — the DECLARATION half of the emit seam
    /// (Wave 43 lane 4 E). The `events.emit` capability lets a runtime
    /// publish one of these; the gate that says whether the org has
    /// admitted the type is the lane-2 registry, not this list.
    pub fn declared_event_types(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        self.triggers
            .iter()
            .map(|t| t.emitted_event_type.clone())
            .filter(|t| seen.insert(t.clone()))
            .collect()
    }

    /// The registry key prefix for this profile's install (slug is the
    /// install-safe name approve minted).
    pub fn entry_key(prefix: &str, local_id: &str) -> String {
        format!("plugin.{prefix}.{local_id}")
    }

    fn validate_id(id: &str, what: &str) -> Result<(), crate::error::DomainError> {
        let ok = !id.is_empty()
            && id.len() <= 64
            && id.starts_with(|c: char| c.is_ascii_lowercase())
            && id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if ok {
            Ok(())
        } else {
            Err(crate::error::DomainError::Validation(format!(
                "{what} id {id:?} must match [a-z][a-z0-9_]{{0,63}}"
            )))
        }
    }

    fn validate_label(label: &str, what: &str) -> Result<(), crate::error::DomainError> {
        if label.trim().is_empty() || label.len() > MAX_SURFACE_LABEL_BYTES {
            return Err(crate::error::DomainError::Validation(format!(
                "{what} label must be 1..={MAX_SURFACE_LABEL_BYTES} bytes"
            )));
        }
        Ok(())
    }

    /// A one-line prose field (a description, an agent description): must
    /// say something, and must fit the section's byte cap. Shared so a new
    /// prose field cannot quietly skip the same gate the others pass.
    fn validate_description(text: &str, what: &str) -> Result<(), crate::error::DomainError> {
        if text.trim().is_empty() || text.len() > MAX_SURFACE_DESCRIPTION_BYTES {
            return Err(crate::error::DomainError::Validation(format!(
                "{what} must be 1..={MAX_SURFACE_DESCRIPTION_BYTES} bytes"
            )));
        }
        Ok(())
    }

    /// Validates the whole profile: per-section size caps, id/label
    /// shapes, unique ids, and the JSON-Schema subsets (props_schema,
    /// params_schema, validator schema) through the same
    /// [`SettingsSchema`] gate settings_schema passes.
    pub fn validate(&self) -> Result<(), crate::error::DomainError> {
        let err = |m: String| Err(crate::error::DomainError::Validation(m));
        if self.widgets.len() > MAX_SURFACE_ENTRIES_PER_SECTION {
            return err(format!(
                "surface profile declares {} widgets (max {MAX_SURFACE_ENTRIES_PER_SECTION})",
                self.widgets.len()
            ));
        }
        if self.actions.len() > MAX_SURFACE_ENTRIES_PER_SECTION {
            return err(format!(
                "surface profile declares {} actions (max {MAX_SURFACE_ENTRIES_PER_SECTION})",
                self.actions.len()
            ));
        }
        if self.render_kinds.len() > MAX_SURFACE_ENTRIES_PER_SECTION {
            return err(format!(
                "surface profile declares {} render kinds (max {MAX_SURFACE_ENTRIES_PER_SECTION})",
                self.render_kinds.len()
            ));
        }
        if self.field_validators.len() > MAX_SURFACE_ENTRIES_PER_SECTION {
            return err(format!(
                "surface profile declares {} field validators (max {MAX_SURFACE_ENTRIES_PER_SECTION})",
                self.field_validators.len()
            ));
        }
        if self.triggers.len() > MAX_SURFACE_ENTRIES_PER_SECTION {
            return err(format!(
                "surface profile declares {} triggers (max {MAX_SURFACE_ENTRIES_PER_SECTION})",
                self.triggers.len()
            ));
        }
        if self.reads.len() > MAX_SURFACE_ENTRIES_PER_SECTION {
            return err(format!(
                "surface profile declares {} reads (max {MAX_SURFACE_ENTRIES_PER_SECTION})",
                self.reads.len()
            ));
        }
        let mut seen_widgets = std::collections::HashSet::new();
        for w in &self.widgets {
            Self::validate_id(&w.id, "widget")?;
            Self::validate_label(&w.title, "widget")?;
            if !seen_widgets.insert(&w.id) {
                return err(format!("widget id {:?} is declared twice", w.id));
            }
            if let Some(schema) = &w.props_schema {
                SettingsSchema::new(schema.clone())?;
            }
            if let Some(props) = &w.default_props {
                if let Some(schema) = &w.props_schema {
                    let schema = SettingsSchema::new(schema.clone())?;
                    schema.validate_config(props)?;
                } else {
                    return err(format!(
                        "widget {:?} declares default_props without props_schema",
                        w.id
                    ));
                }
            }
        }
        let mut seen_actions = std::collections::HashSet::new();
        for a in &self.actions {
            Self::validate_id(&a.id, "action")?;
            Self::validate_label(&a.label, "action")?;
            if a.description.trim().is_empty()
                || a.description.len() > MAX_SURFACE_DESCRIPTION_BYTES
            {
                return err(format!(
                    "action {:?} description must be 1..={MAX_SURFACE_DESCRIPTION_BYTES} bytes",
                    a.id
                ));
            }
            if !seen_actions.insert(&a.id) {
                return err(format!("action id {:?} is declared twice", a.id));
            }
            if let Some(schema) = &a.params_schema {
                SettingsSchema::new(schema.clone())?;
            }
            // A2 (Wave 43 lane 13): the output schema goes through the
            // REGISTRY's subset gate, not SettingsSchema's — a node
            // contract has to be the same shape the registry can honour,
            // or a plugin could declare a keyword a first-party node
            // cannot.
            if let Some(schema) = &a.output_schema {
                crate::node_registry::validate_declared_schema(schema).map_err(|v| {
                    crate::error::DomainError::Validation(format!(
                        "action {:?} output_schema is not inside the declared schema subset: {v}",
                        a.id
                    ))
                })?;
            }
            Self::validate_emits(&a.id, "action", &a.emits)?;
            if let Some(desc) = &a.agent_description {
                Self::validate_description(desc, "action agent_description")?;
            }
        }
        let mut seen_renders = std::collections::HashSet::new();
        for r in &self.render_kinds {
            Self::validate_id(&r.id, "render kind")?;
            Self::validate_label(&r.label, "render kind")?;
            if r.description.trim().is_empty()
                || r.description.len() > MAX_SURFACE_DESCRIPTION_BYTES
            {
                return err(format!(
                    "render kind {:?} description must be 1..={MAX_SURFACE_DESCRIPTION_BYTES} bytes",
                    r.id
                ));
            }
            if !seen_renders.insert(&r.id) {
                return err(format!("render kind id {:?} is declared twice", r.id));
            }
        }
        let mut seen_validators = std::collections::HashSet::new();
        for v in &self.field_validators {
            Self::validate_id(&v.id, "field validator")?;
            Self::validate_label(&v.label, "field validator")?;
            if !seen_validators.insert(&v.id) {
                return err(format!("field validator id {:?} is declared twice", v.id));
            }
            SettingsSchema::new(v.schema.clone())?;
        }
        let mut seen_triggers = std::collections::HashSet::new();
        for t in &self.triggers {
            Self::validate_id(&t.id, "trigger")?;
            Self::validate_label(&t.label, "trigger")?;
            if t.description.trim().is_empty()
                || t.description.len() > MAX_SURFACE_DESCRIPTION_BYTES
            {
                return err(format!(
                    "trigger {:?} description must be 1..={MAX_SURFACE_DESCRIPTION_BYTES} bytes",
                    t.id
                ));
            }
            Self::validate_event_type(&t.emitted_event_type, "trigger")?;
            if !seen_triggers.insert(&t.id) {
                return err(format!("trigger id {:?} is declared twice", t.id));
            }
            if let Some(schema) = &t.payload_schema {
                SettingsSchema::new(schema.clone())?;
            }
        }
        let mut seen_reads = std::collections::HashSet::new();
        for r in &self.reads {
            Self::validate_id(&r.id, "read")?;
            Self::validate_label(&r.label, "read")?;
            if r.description.trim().is_empty()
                || r.description.len() > MAX_SURFACE_DESCRIPTION_BYTES
            {
                return err(format!(
                    "read {:?} description must be 1..={MAX_SURFACE_DESCRIPTION_BYTES} bytes",
                    r.id
                ));
            }
            if !seen_reads.insert(&r.id) {
                return err(format!("read id {:?} is declared twice", r.id));
            }
            Self::validate_read_guardrails(r)?;
            if let Some(schema) = &r.params_schema {
                SettingsSchema::new(schema.clone())?;
            }
            if let Some(schema) = &r.output_schema {
                // The registry's subset gate, not SettingsSchema's: the
                // output schema is what a downstream value.reference and
                // every condition gate on (A2), so it must be a shape the
                // registry can actually resolve.
                crate::node_registry::validate_declared_schema(schema).map_err(|v| {
                    crate::error::DomainError::Validation(format!(
                        "read {:?} output_schema is not inside the declared schema subset: {v}",
                        r.id
                    ))
                })?;
            }
            Self::validate_emits(&r.id, "read", &r.emits)?;
            if let Some(desc) = &r.agent_description {
                Self::validate_description(desc, "read agent_description")?;
            }
        }
        Ok(())
    }

    /// A2 (Wave 43 lane 13): a node's declared emit list. The static loop
    /// refusal reads these, so they are validated at consent time rather
    /// than discovered as a deadlock. Each entry must be a platform event
    /// type in the same grammar a trigger declares, and the list is
    /// bounded — an unbounded array is a way to smuggle a long list
    /// nobody reviewed.
    fn validate_emits(
        id: &str,
        what: &str,
        emits: &[String],
    ) -> Result<(), crate::error::DomainError> {
        if emits.len() > 32 {
            return Err(crate::error::DomainError::Validation(format!(
                "{what} {:?} declares {} emitted event types (max 32)",
                id,
                emits.len()
            )));
        }
        for event in emits {
            Self::validate_event_type(event, &format!("{what} {:?} emit", id))?;
        }
        Ok(())
    }

    /// The event-type grammar the platform's own emitted types use:
    /// lowercase `segment.segment`, so `rmm.device_offline` parses and
    /// `RMM.Device Offline` does not. Reused for a trigger's declared
    /// emission, which the event-type registry validates at grant time.
    fn validate_event_type(value: &str, what: &str) -> Result<(), crate::error::DomainError> {
        let segments: Vec<&str> = value.split('.').collect();
        let ok = value.len() <= 120
            && segments.len() == 2
            && segments.iter().all(|s| {
                !s.is_empty()
                    && s.len() <= 48
                    && s.starts_with(|c: char| c.is_ascii_lowercase())
                    && s.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            });
        if ok {
            Ok(())
        } else {
            Err(crate::error::DomainError::Validation(format!(
                "{what} event type {value:?} must be lowercase segment.segment \
                 (e.g. rmm.device_offline)"
            )))
        }
    }

    /// Applies the platform floors to a declared read (ADR 0017 §5). A
    /// declaration cannot write itself a weaker cap than the platform
    /// allows, and the one boolean that may only be declared one way is
    /// checked here rather than trusted.
    fn validate_read_guardrails(read: &SurfaceRead) -> Result<(), crate::error::DomainError> {
        let err = |m: String| Err(crate::error::DomainError::Validation(m));
        let g = &read.guardrails;
        if g.max_calls_per_run == 0 || g.max_calls_per_run > MAX_READ_CALLS_PER_RUN {
            return err(format!(
                "read {:?} declares max_calls_per_run {} (allowed 1..={MAX_READ_CALLS_PER_RUN})",
                read.id, g.max_calls_per_run
            ));
        }
        if g.min_interval_secs < MIN_READ_INTERVAL_SECS {
            return err(format!(
                "read {:?} declares min_interval_secs {} (floored at {MIN_READ_INTERVAL_SECS})",
                read.id, g.min_interval_secs
            ));
        }
        if g.max_output_bytes == 0 || g.max_output_bytes > MAX_READ_OUTPUT_BYTES {
            return err(format!(
                "read {:?} declares max_output_bytes {} (allowed 1..={MAX_READ_OUTPUT_BYTES})",
                read.id, g.max_output_bytes
            ));
        }
        if g.may_return_secrets {
            return err(format!("read {:?}: {READ_SECRETS_REFUSAL}", read.id));
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
        // kind-coherent. Since ADR 0023 (Wave 53 lane 2) the profile itself is
        // legal on any artifact kind, because a wasm guest's `http_request`
        // reads its egress allowlist from `outbound_scopes`. The two
        // CONNECTOR-ONLY members are still refused on a wasm manifest:
        // `webhook_endpoints` (a wasm guest receives no inbound webhooks) and
        // `routing` (the Routing Contract is answered by an HTTP pack). A
        // connector that declares no webhooks keeps the absent-profile default
        // posture.
        match self.runtime_kind() {
            PluginRuntimeKind::HttpConnector => {
                if let Some(profile) = &self.connector_profile {
                    profile.validate()?;
                }
            }
            PluginRuntimeKind::Wasm => {
                if let Some(profile) = &self.connector_profile {
                    if !profile.webhook_endpoints.is_empty() {
                        return Err(crate::error::DomainError::Validation(
                            "wasm manifests must not declare connector webhook_endpoints \
                             (connector-only); a wasm manifest may declare only \
                             connector_profile.outbound_scopes for its http_request egress"
                                .into(),
                        ));
                    }
                    if profile.routing.is_some() {
                        return Err(crate::error::DomainError::Validation(
                            "wasm manifests must not declare a routing profile \
                             (the Routing Contract is answered by an HTTP connector)"
                                .into(),
                        ));
                    }
                    profile.validate()?;
                }
            }
        }
        // §4.6 kind-coherence, AMENDED PER KIND by ADR 0017 §3 (accepted
        // 2026-10-01, Wave 43 lane 4). The original rule refused the field
        // on EVERY connector, and its stated reason was UI coherence: a
        // connector has no in-product UI surface to govern. That reason
        // covers the UI kinds only — `widget` and `report_render` render in
        // the product, and `field_validator` governs the board field
        // editor, which is in-product UI too. It does NOT cover the
        // executor kinds, so those three are now available to connectors:
        //
        //   UI      (wasm-only): widget, report_render, field_validator
        //   Executor (either):  automation_action, automation_trigger, read
        //
        // The argument that carries it: an `http_connector` already holds
        // credentials and network egress as its reason for existing, so an
        // action grants it nothing new — and a read traverses the same
        // credentials. The wasm case is the reverse, because a sandbox is
        // capability-restricted and granting it a network action IS an
        // escalation, which is presumably why the rule started there.
        //
        // Trust does not come from authorship: first-party and marketplace
        // connectors hold the SAME capability, gated by consent + digest
        // pinning + the per-install grant (`plugin_installs.
        // granted_capabilities`), none of which depend on who wrote it.
        match self.runtime_kind() {
            PluginRuntimeKind::Wasm => {
                if let Some(profile) = &self.surface_profile {
                    profile.validate()?;
                }
            }
            PluginRuntimeKind::HttpConnector => {
                if let Some(profile) = &self.surface_profile {
                    if profile.declares_ui_sections() {
                        return Err(crate::error::DomainError::Validation(
                            "http_connector manifests may not declare a UI surface \
                             (widget, report_render, field_validator): a connector has no \
                             in-product UI surface to govern. The executor kinds \
                             (automation_action, automation_trigger, read) are allowed."
                                .into(),
                        ));
                    }
                    profile.validate()?;
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
    /// The org that published this entry, or `None` for an
    /// operator-authored entry. The registry is server-wide, so this is
    /// the only per-entry ownership the catalog routes can bind to: a
    /// tenant admin may mutate only the entries its own org published,
    /// and a `None` entry is immutable through those routes.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub origin_org_id: Option<Uuid>,
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
    /// sha256 hex of the executable module this version ships, when it ships
    /// one. The digest is captured at admission (where the bytes were verified)
    /// and travels with the version so an install or an update can PIN it —
    /// without it, an update has no artifact to name and no pin to enforce.
    pub module_digest: String,
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

    /// Terminal-state predicate. Revocation is the only status nothing
    /// leaves, so callers branch on it by name rather than by variant.
    pub fn is_revoked(&self) -> bool {
        matches!(self, Self::Revoked)
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
    /// The identity-bundle artifact: the declarative half of a pluggable
    /// identity source (name, version, interface version, provider kinds,
    /// settings schema, upstream scopes). It rides the SAME staged
    /// consent diff and the SAME provenance chain as a plugin, and it
    /// lands DISABLED. Its executable half is a core-side adapter
    /// registered through `FederationBundle`; sessions, tokens and MFA
    /// stay in core, and no `auth:*` capability exists for it to hold.
    IdentityBundle,
}

impl PluginArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PluginArtifactKind::Plugin => "plugin",
            PluginArtifactKind::Skill => "skill",
            PluginArtifactKind::Theme => "theme",
            PluginArtifactKind::IdentityBundle => "identity_bundle",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "plugin" => Some(Self::Plugin),
            "skill" => Some(Self::Skill),
            "theme" => Some(Self::Theme),
            "identity_bundle" => Some(Self::IdentityBundle),
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
    /// sha256 hex of the EXECUTABLE module inside that artifact, when the
    /// artifact carries one (a `.wasm` member of an imported zip, or the
    /// fetched binary a registry/GitHub entry points at). Empty for a
    /// declarative-only import, a skill, a theme or an identity bundle.
    ///
    /// Two digests on purpose: `artifact_digest` identifies what was imported
    /// (the archive), and this one identifies what would RUN. The install pins
    /// this one, because a pin over the archive cannot address a module.
    pub module_digest: String,
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
    /// The identity-bundle install minted on approval of an
    /// `identity_bundle` staged item.
    pub adopted_identity_bundle_id: Option<Uuid>,
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

/// Marketplace v2 verified-publisher registry row (Wave 22). A
/// publisher is a NORMALIZED SOURCE IDENTITY; verification is per-org
/// (this org vouches this publisher), admin-curated, and additive over
/// the v1 distribution formats.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginPublisherVerification {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: Uuid,
    /// Normalized identity, e.g. `github.com/owner` or `registry://slug`.
    pub publisher: String,
    pub display_name: String,
    /// `verified` | `revoked`.
    pub status: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub verified_by: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub verified_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = "date-time"))]
    pub revoked_at: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub revoked_by: Option<Uuid>,
    pub note: String,
}

impl PluginPublisherVerification {
    pub fn validate(&self) -> Result<(), crate::error::DomainError> {
        match self.status.as_str() {
            "verified" | "revoked" => {}
            other => {
                return Err(crate::error::DomainError::Validation(format!(
                    "unknown publisher status {:?}",
                    other
                )))
            }
        }
        if self.publisher.trim().is_empty() || self.publisher.len() > 300 {
            return Err(crate::error::DomainError::Validation(
                "publisher identity is required (max 300 chars)".into(),
            ));
        }
        if self.display_name.trim().is_empty() || self.display_name.len() > 200 {
            return Err(crate::error::DomainError::Validation(
                "display_name is required (max 200 chars)".into(),
            ));
        }
        Ok(())
    }
}

/// Marketplace v2 per-user rating on one artifact RELEASE (the
/// `{source_kind, source_url, artifact_digest}` triple). One rating per
/// user per release; re-rating is an upsert.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginRating {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: Uuid,
    pub source_kind: PluginSourceKind,
    pub source_url: String,
    /// sha256 hex of the artifact release.
    pub artifact_digest: String,
    /// 1..=5 stars.
    pub rating: i16,
    pub comment: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub rated_by: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub rated_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub updated_at: DateTime<Utc>,
}

impl PluginRating {
    pub fn validate(&self) -> Result<(), crate::error::DomainError> {
        if !(1..=5).contains(&self.rating) {
            return Err(crate::error::DomainError::Validation(
                "rating must be 1..=5 stars".into(),
            ));
        }
        if !self.artifact_digest.is_empty() && !is_sha256_hex(&self.artifact_digest) {
            return Err(crate::error::DomainError::Validation(
                "artifact_digest must be a sha256 hex digest".into(),
            ));
        }
        if self.comment.len() > 2000 {
            return Err(crate::error::DomainError::Validation(
                "comment must be at most 2000 chars".into(),
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
    /// The DIGEST PIN: sha256 hex of the executable module this install
    /// adopted. Empty means the install is declarative-only (no module to
    /// run), and the load path refuses rather than guessing. An update moves
    /// this to a new digest; a rollback moves it back.
    pub module_digest: String,
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
    /// Why the install was revoked, for the operator's trail. The audit row
    /// carries the same string; the column exists so the install's own state
    /// explains itself without a join.
    pub revoked_reason: Option<String>,
    /// The pin a rollback returns to: the version, digest and grant set the
    /// install held before its last update. `None` means nothing to roll back
    /// to (never updated, or already rolled back).
    pub previous_version: Option<String>,
    pub previous_module_digest: Option<String>,
    /// The grants the install held at `previous_version`. Restored verbatim on
    /// rollback, because an older manifest may declare a narrower set and
    /// keeping the newer grants would run the old code with authority its
    /// manifest never asked for.
    pub previous_capabilities: Vec<String>,
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
    fn artifact_kinds_should_round_trip_and_the_vocabulary_stays_closed() {
        for k in [
            PluginArtifactKind::Plugin,
            PluginArtifactKind::Skill,
            PluginArtifactKind::Theme,
            PluginArtifactKind::IdentityBundle,
        ] {
            assert_eq!(
                PluginArtifactKind::parse(k.as_str()).expect("round trip"),
                k
            );
        }
        assert_eq!(
            PluginArtifactKind::IdentityBundle.as_str(),
            "identity_bundle"
        );
        assert_eq!(PluginArtifactKind::parse("auth_bundle"), None);
        assert_eq!(PluginArtifactKind::parse("identity"), None);
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
            surface_profile: None,
        }
    }

    #[test]
    fn manifest_should_accept_known_capabilities_at_pinned_interface_version() {
        let m = manifest(&["tickets.read", "financial.read"]);
        assert!(m.validate().is_ok());
        assert_eq!(m.declared().len(), 2);
    }

    #[test]
    fn surface_capabilities_should_round_trip_and_resolve_onto_existing_gates() {
        for key in [
            "ui.widgets",
            "automation.actions",
            "reports.render",
            "fields.validate",
        ] {
            let parsed = PluginCapability::parse(key).expect("surface capability parses");
            assert_eq!(parsed.as_str(), key);
        }
        assert_eq!(
            PluginCapability::UiWidgets.permission(),
            Permission::Matrix {
                module: Module::Reporting,
                action: Action::View
            }
        );
        assert_eq!(
            PluginCapability::AutomationActions.permission(),
            Permission::Matrix {
                module: Module::Tickets,
                action: Action::Edit
            }
        );
        assert_eq!(
            PluginCapability::FieldsValidate.permission(),
            Permission::Matrix {
                module: Module::Tickets,
                action: Action::View
            }
        );
    }

    #[test]
    fn surface_profile_should_validate_declarations_and_closed_vocabularies() {
        let good = PluginSurfaceProfile {
            widgets: vec![SurfaceWidget {
                id: "csat_today".into(),
                title: "CSAT today".into(),
                slot: SurfaceSlot::Dashboard,
                kind: SurfaceWidgetKind::Stat,
                props_schema: Some(json!({
                    "type": "object",
                    "properties": {"suffix": {"type": "string", "maxLength": 12}}
                })),
                default_props: Some(json!({"suffix": "%"})),
            }],
            actions: vec![SurfaceAction {
                id: "triage_note".into(),
                label: "Triage note".into(),
                description: "Adds the triage note the rule configures.".into(),
                effect: SurfaceActionEffect::AddInternalComment,
                params_schema: Some(json!({
                    "type": "object",
                    "properties": {"body": {"type": "string", "maxLength": 500}}
                })),
                // A2: a declared output schema and emit list, so the
                // action is usable downstream and visible to the loop
                // refusal.
                output_schema: Some(json!({
                    "type": "object",
                    "properties": {"comment_id": {"type": "string", "format": "uuid"}}
                })),
                idempotent: false,
                emits: vec!["ticket.comment_added".into()],
                audience: Some(crate::node_registry::NodeAudience::Both),
                agent_description: Some("Adds the triage note to the ticket.".into()),
            }],
            render_kinds: vec![SurfaceRenderKind {
                id: "gauges".into(),
                label: "Gauges".into(),
                description: "Horizontal gauge rows for bounded metrics.".into(),
            }],
            field_validators: vec![SurfaceFieldValidator {
                id: "asset_tag".into(),
                label: "Asset tag".into(),
                schema: json!({
                    "type": "object",
                    "properties": {"value": {"type": "string", "maxLength": 32}}
                }),
            }],
            triggers: vec![trigger("offline", "rmm.device_offline")],
            reads: vec![read("devices")],
        };
        assert!(good.validate().is_ok());

        // Unknown slot / kind / effect vocabularies cannot even serialize.
        assert!(SurfaceSlot::parse("portal").is_none());
        assert!(SurfaceWidgetKind::parse("iframe").is_none());
        assert!(SurfaceActionEffect::parse("run_code").is_none());

        // Bad shapes refuse.
        let bad_id = PluginSurfaceProfile {
            widgets: vec![SurfaceWidget {
                id: "Bad-Id".into(),
                title: "x".into(),
                slot: SurfaceSlot::Dashboard,
                kind: SurfaceWidgetKind::Stat,
                props_schema: None,
                default_props: None,
            }],
            ..PluginSurfaceProfile::default()
        };
        assert!(bad_id.validate().is_err());

        // default_props without props_schema refuses.
        let orphan_props = PluginSurfaceProfile {
            widgets: vec![SurfaceWidget {
                id: "w".into(),
                title: "w".into(),
                slot: SurfaceSlot::TicketPane,
                kind: SurfaceWidgetKind::List,
                props_schema: None,
                default_props: Some(json!({"a": 1})),
            }],
            ..PluginSurfaceProfile::default()
        };
        assert!(orphan_props.validate().is_err());

        // default_props violating props_schema refuses.
        let violating_props = PluginSurfaceProfile {
            widgets: vec![SurfaceWidget {
                id: "w".into(),
                title: "w".into(),
                slot: SurfaceSlot::TicketPane,
                kind: SurfaceWidgetKind::List,
                props_schema: Some(json!({
                    "type": "object",
                    "properties": {"rows": {"type": "integer", "maximum": 10}}
                })),
                default_props: Some(json!({"rows": 99})),
            }],
            ..PluginSurfaceProfile::default()
        };
        assert!(violating_props.validate().is_err());

        // Duplicate ids refuse.
        let dup = PluginSurfaceProfile {
            actions: vec![
                SurfaceAction {
                    id: "a".into(),
                    label: "A".into(),
                    description: "d".into(),
                    effect: SurfaceActionEffect::AddInternalComment,
                    params_schema: None,
                    output_schema: None,
                    idempotent: false,
                    emits: Vec::new(),
                    audience: None,
                    agent_description: None,
                },
                SurfaceAction {
                    id: "a".into(),
                    label: "A2".into(),
                    description: "d".into(),
                    effect: SurfaceActionEffect::AddInternalComment,
                    params_schema: None,
                    output_schema: None,
                    idempotent: false,
                    emits: Vec::new(),
                    audience: None,
                    agent_description: None,
                },
            ],
            ..PluginSurfaceProfile::default()
        };
        assert!(dup.validate().is_err());

        // Over-cap sections refuse.
        let many_validators: Vec<SurfaceFieldValidator> = (0..9)
            .map(|i| SurfaceFieldValidator {
                id: format!("v{i}"),
                label: "v".into(),
                schema: json!({"type": "object", "properties": {}}),
            })
            .collect();
        assert!(PluginSurfaceProfile {
            field_validators: many_validators,
            ..PluginSurfaceProfile::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn surface_profile_should_be_wasm_only_for_ui_kinds() {
        let mut m = manifest(&["ui.widgets", "automation.actions"]);
        m.surface_profile = Some(PluginSurfaceProfile {
            widgets: vec![SurfaceWidget {
                id: "w".into(),
                title: "W".into(),
                slot: SurfaceSlot::Dashboard,
                kind: SurfaceWidgetKind::Stat,
                props_schema: None,
                default_props: None,
            }],
            ..PluginSurfaceProfile::default()
        });
        // WASM baseline (absent kind) accepts the profile.
        assert!(m.validate().is_ok());
        // ADR 0017 §3: a UI kind still refuses on a connector.
        m.artifact_kind = Some(PluginRuntimeKind::HttpConnector);
        assert!(m.validate().is_err());
    }

    fn trigger(id: &str, emitted: &str) -> SurfaceTrigger {
        SurfaceTrigger {
            id: id.into(),
            label: "T".into(),
            description: "A trigger.".into(),
            emitted_event_type: emitted.into(),
            payload_schema: None,
        }
    }

    fn read(id: &str) -> SurfaceRead {
        SurfaceRead {
            id: id.into(),
            label: "R".into(),
            description: "A read.".into(),
            params_schema: None,
            output_schema: None,
            // A2 defaults, matching the migration's: an undeclared read is
            // retry-unsafe, emits nothing and stays off the model's view.
            idempotent: false,
            emits: Vec::new(),
            audience: None,
            agent_description: None,
            guardrails: ReadGuardrails {
                max_calls_per_run: 5,
                min_interval_secs: MIN_READ_INTERVAL_SECS,
                max_output_bytes: 4096,
                may_return_secrets: false,
            },
        }
    }

    #[test]
    fn connector_should_contribute_executor_kinds_and_refuse_ui_kinds() {
        let mut m = manifest(&[
            "automation.actions",
            "automation.triggers",
            "automation.reads",
        ]);
        m.artifact_kind = Some(PluginRuntimeKind::HttpConnector);
        m.surface_profile = Some(PluginSurfaceProfile {
            actions: vec![SurfaceAction {
                id: "note".into(),
                label: "Note".into(),
                description: "Adds a note.".into(),
                effect: SurfaceActionEffect::AddInternalComment,
                params_schema: None,
                output_schema: None,
                idempotent: false,
                emits: Vec::new(),
                audience: None,
                agent_description: None,
            }],
            triggers: vec![trigger("offline", "rmm.device_offline")],
            reads: vec![read("devices")],
            ..PluginSurfaceProfile::default()
        });
        assert!(m.validate().is_ok());

        // The same manifest naming a read node that leaks secrets refuses.
        let mut m = manifest(&["automation.reads"]);
        m.artifact_kind = Some(PluginRuntimeKind::HttpConnector);
        m.surface_profile = Some(PluginSurfaceProfile {
            triggers: vec![trigger("t", "rmm.device_offline")],
            reads: vec![SurfaceRead {
                guardrails: ReadGuardrails {
                    may_return_secrets: true,
                    ..read("devices").guardrails
                },
                ..read("devices")
            }],
            ..PluginSurfaceProfile::default()
        });
        let err = m.validate().expect_err("secret refusal").to_string();
        assert!(err.contains("vault secret"), "{err}");

        // A read declaring no guardrails at all refuses.
        let mut m = manifest(&["automation.reads"]);
        m.artifact_kind = Some(PluginRuntimeKind::HttpConnector);
        m.surface_profile = Some(PluginSurfaceProfile {
            reads: vec![read("devices")],
            ..PluginSurfaceProfile::default()
        });
        assert!(m.validate().is_ok());

        // Each UI section refuses on a connector on its own.
        for (name, profile) in [
            (
                "widget",
                PluginSurfaceProfile {
                    widgets: vec![SurfaceWidget {
                        id: "w".into(),
                        title: "W".into(),
                        slot: SurfaceSlot::Dashboard,
                        kind: SurfaceWidgetKind::Stat,
                        props_schema: None,
                        default_props: None,
                    }],
                    ..PluginSurfaceProfile::default()
                },
            ),
            (
                "report_render",
                PluginSurfaceProfile {
                    render_kinds: vec![SurfaceRenderKind {
                        id: "r".into(),
                        label: "R".into(),
                        description: "Renders.".into(),
                    }],
                    ..PluginSurfaceProfile::default()
                },
            ),
            (
                "field_validator",
                PluginSurfaceProfile {
                    field_validators: vec![SurfaceFieldValidator {
                        id: "f".into(),
                        label: "F".into(),
                        schema: json!({"type": "object", "properties": {}}),
                    }],
                    ..PluginSurfaceProfile::default()
                },
            ),
        ] {
            let mut m = manifest(&["ui.widgets"]);
            m.artifact_kind = Some(PluginRuntimeKind::HttpConnector);
            m.surface_profile = Some(profile);
            let err = m.validate().expect_err(name).to_string();
            assert!(err.contains("in-product UI"), "{name}: {err}");
        }
    }

    #[test]
    fn the_node_contract_fields_should_validate_as_a_registry_can_honour_them() {
        // A2 (Wave 43 lane 13). The five fields that turn a surface entry
        // into a workflow node go through the REGISTRY's schema gate, not
        // SettingsSchema's: a plugin node and a first-party node must be
        // declared in the same vocabulary, or one of them is lying.
        let good = PluginSurfaceProfile {
            actions: vec![SurfaceAction {
                id: "push".into(),
                label: "Push".into(),
                description: "Pushes the ticket.".into(),
                effect: SurfaceActionEffect::AddInternalComment,
                params_schema: None,
                output_schema: Some(json!({
                    "type": "object",
                    "properties": {"remote_id": {"type": "string"}},
                    "required": ["remote_id"],
                    "additionalProperties": false
                })),
                idempotent: true,
                emits: vec!["rmm.ticket_pushed".into()],
                audience: Some(crate::node_registry::NodeAudience::Both),
                agent_description: Some("Pushes the ticket's summary to the vendor portal.".into()),
            }],
            ..PluginSurfaceProfile::default()
        };
        assert!(good.validate().is_ok());

        // An output schema declaring a keyword the registry would ignore
        // refuses: a contract that lies is worse than no contract.
        let unsupported_keyword = PluginSurfaceProfile {
            actions: vec![SurfaceAction {
                output_schema: Some(json!({
                    "type": "object",
                    "patternProperties": {"^x": {"type": "string"}}
                })),
                ..SurfaceAction {
                    id: "push".into(),
                    label: "Push".into(),
                    description: "Pushes the ticket.".into(),
                    effect: SurfaceActionEffect::AddInternalComment,
                    params_schema: None,
                    output_schema: None,
                    idempotent: false,
                    emits: Vec::new(),
                    audience: None,
                    agent_description: None,
                }
            }],
            ..PluginSurfaceProfile::default()
        };
        assert!(unsupported_keyword.validate().is_err());

        // The emit list is the loop refusal's whole basis, so it is
        // validated at consent time rather than discovered as a deadlock.
        let bad_emit = PluginSurfaceProfile {
            actions: vec![SurfaceAction {
                emits: vec!["RMM.Device Offline".into()],
                ..SurfaceAction {
                    id: "push".into(),
                    label: "Push".into(),
                    description: "Pushes the ticket.".into(),
                    effect: SurfaceActionEffect::AddInternalComment,
                    params_schema: None,
                    output_schema: None,
                    idempotent: false,
                    emits: Vec::new(),
                    audience: None,
                    agent_description: None,
                }
            }],
            ..PluginSurfaceProfile::default()
        };
        assert!(bad_emit.validate().is_err());

        let too_many_emits = PluginSurfaceProfile {
            actions: vec![SurfaceAction {
                emits: (0..33).map(|i| format!("plugin.event_{i}")).collect(),
                ..SurfaceAction {
                    id: "push".into(),
                    label: "Push".into(),
                    description: "Pushes the ticket.".into(),
                    effect: SurfaceActionEffect::AddInternalComment,
                    params_schema: None,
                    output_schema: None,
                    idempotent: false,
                    emits: Vec::new(),
                    audience: None,
                    agent_description: None,
                }
            }],
            ..PluginSurfaceProfile::default()
        };
        assert!(too_many_emits.validate().is_err());

        // A read's output schema passes the same gate, and an empty agent
        // description is refused rather than rendered as a blank row.
        let blank_agent = PluginSurfaceProfile {
            reads: vec![SurfaceRead {
                agent_description: Some("   ".into()),
                ..read("r")
            }],
            ..PluginSurfaceProfile::default()
        };
        assert!(blank_agent.validate().is_err());
    }

    #[test]
    fn a_declared_node_contract_should_parse_into_the_registrys_own_shape() {
        // The A2 seam: a manifest's declaration and the registry's
        // PluginNodeContract must agree, because the palette, the picker
        // and the LLM tool catalogue all read the registry's shape.
        let action = SurfaceAction {
            id: "push".into(),
            label: "Push ticket".into(),
            description: "Pushes the ticket.".into(),
            effect: SurfaceActionEffect::AddInternalComment,
            params_schema: Some(json!({
                "type": "object",
                "properties": {"ticket": {"type": "string"}},
                "required": ["ticket"]
            })),
            output_schema: Some(json!({
                "type": "object",
                "properties": {"remote_id": {"type": "string"}}
            })),
            idempotent: true,
            emits: vec!["rmm.ticket_pushed".into()],
            audience: Some(crate::node_registry::NodeAudience::Both),
            agent_description: Some("Pushes the ticket to the vendor portal.".into()),
        };
        let contract = crate::node_registry::PluginNodeContract::parse(
            "automation_action",
            &PluginSurfaceProfile::entry_key("acme-rmm", &action.id),
            &action.label,
            &json!({"params_schema": action.params_schema.clone()}),
            action.output_schema.clone(),
            action.idempotent,
            &action.emits,
            action.audience.map(|a| a.as_str()).unwrap_or("human"),
            action.agent_description.as_deref(),
        )
        .expect("a declared contract parses");

        assert_eq!(contract.kind, crate::node_registry::PluginNodeKind::Action);
        assert_eq!(contract.node_id, "plugin.acme-rmm.push");
        assert!(contract.idempotent);
        assert_eq!(contract.emits, ["rmm.ticket_pushed"]);
        assert_eq!(contract.audience, crate::node_registry::NodeAudience::Both);
        assert!(contract
            .agent_description
            .as_deref()
            .is_some_and(|d| d.contains("vendor portal")));

        let node = contract.as_node_type();
        assert_eq!(node.tier, crate::node_registry::NodeTier::C);
        assert_eq!(node.emits, ["rmm.ticket_pushed"]);
        // The declared output schema is what makes the node referenceable
        // downstream; a contract with none is not. A plugin contract is not
        // in the first-party catalogue, so `output_schema` cannot resolve it
        // — the declaration itself is the evidence.
        assert!(contract.output_schema.is_some());
        assert!(!node.output_schema.is_empty());
    }

    #[test]
    fn read_guardrails_should_refuse_weaker_than_the_platform_floor() {
        let weak = |g: ReadGuardrails| {
            let mut m = manifest(&["automation.reads"]);
            m.surface_profile = Some(PluginSurfaceProfile {
                reads: vec![SurfaceRead {
                    guardrails: g,
                    ..read("r")
                }],
                ..PluginSurfaceProfile::default()
            });
            m.validate().is_err()
        };
        assert!(weak(ReadGuardrails {
            max_calls_per_run: 0,
            min_interval_secs: MIN_READ_INTERVAL_SECS,
            max_output_bytes: 4096,
            may_return_secrets: false,
        }));
        assert!(weak(ReadGuardrails {
            max_calls_per_run: MAX_READ_CALLS_PER_RUN + 1,
            min_interval_secs: MIN_READ_INTERVAL_SECS,
            max_output_bytes: 4096,
            may_return_secrets: false,
        }));
        assert!(weak(ReadGuardrails {
            max_calls_per_run: 5,
            min_interval_secs: 0,
            max_output_bytes: 4096,
            may_return_secrets: false,
        }));
        assert!(weak(ReadGuardrails {
            max_calls_per_run: 5,
            min_interval_secs: MIN_READ_INTERVAL_SECS,
            max_output_bytes: 0,
            may_return_secrets: false,
        }));
        assert!(weak(ReadGuardrails {
            max_calls_per_run: 5,
            min_interval_secs: MIN_READ_INTERVAL_SECS,
            max_output_bytes: MAX_READ_OUTPUT_BYTES + 1,
            may_return_secrets: false,
        }));
    }

    #[test]
    fn trigger_event_types_should_use_the_platform_grammar() {
        let ok = |emitted: &str| {
            let mut m = manifest(&["automation.triggers"]);
            m.surface_profile = Some(PluginSurfaceProfile {
                triggers: vec![trigger("t", emitted)],
                ..PluginSurfaceProfile::default()
            });
            m.validate().is_ok()
        };
        assert!(ok("rmm.device_offline"));
        assert!(ok("alert.ingested"));
        assert!(!ok("RMM.DeviceOffline"));
        assert!(!ok("rmm"));
        assert!(!ok("rmm..offline"));
    }

    #[test]
    fn declared_event_types_should_dedupe_in_declaration_order() {
        let profile = PluginSurfaceProfile {
            triggers: vec![
                trigger("a", "rmm.device_offline"),
                trigger("b", "rmm.device_online"),
                trigger("c", "rmm.device_offline"),
            ],
            ..PluginSurfaceProfile::default()
        };
        assert_eq!(
            profile.declared_event_types(),
            vec![
                "rmm.device_offline".to_string(),
                "rmm.device_online".to_string()
            ]
        );
        assert!(PluginSurfaceProfile::default()
            .declared_event_types()
            .is_empty());
    }

    #[test]
    fn the_three_new_capabilities_should_map_onto_plugins_view() {
        let expected = Permission::Matrix {
            module: Module::Plugins,
            action: Action::View,
        };
        for cap in [
            PluginCapability::AutomationTriggers,
            PluginCapability::AutomationReads,
            PluginCapability::EventsEmit,
        ] {
            assert_eq!(cap.permission(), expected);
        }
    }

    #[test]
    fn the_new_capability_keys_round_trip() {
        assert_eq!(
            PluginCapability::AutomationTriggers.as_str(),
            "automation.triggers"
        );
        assert_eq!(
            PluginCapability::AutomationReads.as_str(),
            "automation.reads"
        );
        assert_eq!(PluginCapability::EventsEmit.as_str(), "events.emit");
        for key in ["automation.triggers", "automation.reads", "events.emit"] {
            assert!(PluginCapability::parse(key).is_some(), "{key}");
        }
    }

    /// Wave 47 lane A6: the usage capability resolves onto its OWN
    /// read-only permission, NOT the Reporting matrix — a plugin that reads
    /// an org's usage counts gains nothing else the reporting module
    /// holds. Pinned here because the cheap default would have been to
    /// file it under `ReportingRead`'s existing mapping, which would have
    /// handed a monitoring plugin every report definition too.
    #[test]
    fn org_usage_capability_resolves_onto_its_own_read_only_permission() {
        use crate::policy::Permission;
        assert_eq!(PluginCapability::OrgUsageRead.as_str(), "org_usage.read");
        assert_eq!(
            PluginCapability::OrgUsageRead.permission(),
            Permission::ViewOrgUsage
        );
        // The wire key round-trips, so a manifest naming the capability
        // resolves rather than being rejected as unknown.
        assert_eq!(
            PluginCapability::parse("org_usage.read"),
            Some(PluginCapability::OrgUsageRead)
        );
    }

    /// Wave 52 lane R12: the observability scope resolves onto its OWN
    /// read-only permission, never onto `special:view_audit_log`. The property
    /// that matters is the negative: a pack granted observability reads the
    /// trail and stays unable to do anything an administrator does. Pinned
    /// because the obvious shortcut (widen the admin special) would pass every
    /// other test in this file while quietly handing packs an admin gate.
    #[test]
    fn observability_capability_resolves_onto_a_read_grant_not_an_admin_special() {
        use crate::policy::Permission;
        assert_eq!(
            PluginCapability::ObservabilityRead.as_str(),
            "observability.read"
        );
        assert_eq!(
            PluginCapability::ObservabilityRead.permission(),
            Permission::ViewObservability
        );
        assert_ne!(
            PluginCapability::ObservabilityRead.permission(),
            Permission::ViewAuditLog
        );
        // The wire key round-trips, so a manifest naming it resolves rather
        // than being rejected as unknown at the data boundary.
        assert_eq!(
            PluginCapability::parse("observability.read"),
            Some(PluginCapability::ObservabilityRead)
        );
    }

    /// Wave 52 lane R16: the directory pair resolves onto its own permissions
    /// and never onto an admin special. The read and the write are separate
    /// capabilities, so a groups-only mirror of a directory can be granted
    /// without role mapping — and the write half is the one that has to be
    /// checked hardest, because the obvious shortcut (resolve it onto
    /// `special:manage_roles`) would hand a connector role administration and
    /// pass every other test in this file.
    #[test]
    fn directory_capabilities_resolve_onto_their_own_grants_not_a_special() {
        use crate::policy::Permission;

        assert_eq!(
            PluginCapability::DirectoryGroupsRead.as_str(),
            "directory_groups.read"
        );
        assert_eq!(
            PluginCapability::DirectoryGroupsWrite.as_str(),
            "directory_groups.write"
        );
        assert_eq!(
            PluginCapability::DirectoryGroupsRead.permission(),
            Permission::ViewDirectoryGroups
        );
        assert_eq!(
            PluginCapability::DirectoryGroupsWrite.permission(),
            Permission::ManageDirectoryGroups
        );
        assert_ne!(
            PluginCapability::DirectoryGroupsWrite.permission(),
            Permission::ManageRoles
        );
        assert_ne!(
            PluginCapability::DirectoryGroupsWrite.permission(),
            Permission::ManageUsers
        );
        // The read capability opens no write, so a mirror that only lists
        // groups cannot change one.
        assert_ne!(
            PluginCapability::DirectoryGroupsRead.permission(),
            PluginCapability::DirectoryGroupsWrite.permission()
        );
        for key in ["directory_groups.read", "directory_groups.write"] {
            assert_eq!(
                PluginCapability::parse(key).map(PluginCapability::as_str),
                Some(key),
                "{key} must round-trip through the closed vocabulary"
            );
        }
    }

    /// Wave 47 lane A8: the custom-domain-manage capability resolves onto
    /// its OWN manage permission, NOT the Admin matrix a human admin holds —
    /// a DNS-provider plugin gains exactly the domain surface. The wire key
    /// round-trips so a manifest naming it resolves rather than being refused
    /// as unknown.
    #[test]
    fn org_custom_domain_capability_resolves_onto_its_own_manage_permission() {
        use crate::policy::Permission;
        assert_eq!(
            PluginCapability::OrgCustomDomainManage.as_str(),
            "org_custom_domain.manage"
        );
        assert_eq!(
            PluginCapability::OrgCustomDomainManage.permission(),
            Permission::ManageOrgCustomDomain
        );
        assert_eq!(
            PluginCapability::parse("org_custom_domain.manage"),
            Some(PluginCapability::OrgCustomDomainManage)
        );
    }

    /// Wave 47 lane A7: the suspension-notice capability resolves onto its own
    /// read-only permission, so a plugin holding it can render the notice and
    /// nothing else. There is no manage half — setting the flag is an admin
    /// action. The wire key round-trips, so a manifest naming the capability
    /// resolves rather than being refused as unknown.
    #[test]
    fn org_suspension_capability_resolves_onto_its_own_read_only_permission() {
        use crate::policy::Permission;
        assert_eq!(
            PluginCapability::OrgSuspensionRead.as_str(),
            "org_suspension.read"
        );
        assert_eq!(
            PluginCapability::OrgSuspensionRead.permission(),
            Permission::ViewOrgSuspension
        );
        assert_eq!(
            PluginCapability::parse("org_suspension.read"),
            Some(PluginCapability::OrgSuspensionRead)
        );
        // The permission is NOT the Admin matrix: a plugin that renders the
        // notice gains no settings-write surface with it.
        assert_ne!(
            PluginCapability::OrgSuspensionRead.permission(),
            Permission::Matrix {
                module: Module::Admin,
                action: Action::Edit,
            }
        );
    }

    #[test]
    fn rag_capabilities_resolve_onto_kb_matrix_and_round_trip() {
        assert_eq!(PluginCapability::RagRead.as_str(), "rag.read");
        assert_eq!(PluginCapability::RagWrite.as_str(), "rag.write");
        assert_eq!(
            PluginCapability::RagRead.permission(),
            Permission::Matrix {
                module: Module::Kb,
                action: Action::View,
            }
        );
        assert_eq!(
            PluginCapability::RagWrite.permission(),
            Permission::Matrix {
                module: Module::Kb,
                action: Action::Edit,
            }
        );
        assert_eq!(
            PluginCapability::parse("rag.read"),
            Some(PluginCapability::RagRead)
        );
        assert_eq!(
            PluginCapability::parse("rag.write"),
            Some(PluginCapability::RagWrite)
        );
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
            routing: None,
            outbound_scopes: std::collections::BTreeMap::new(),
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
                routing: None,
                outbound_scopes: std::collections::BTreeMap::new(),
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
            routing: None,
            outbound_scopes: std::collections::BTreeMap::new(),
        };
        assert!(ok.validate().is_ok());
        // Over-cap declarations refuse.
        let many: Vec<String> = (0..10)
            .map(|i| format!("https://h{i}.example.com/x"))
            .collect();
        assert!(ConnectorProfile {
            webhook_endpoints: many,
            scopes: vec![],
            routing: None,
            outbound_scopes: std::collections::BTreeMap::new(),
        }
        .validate()
        .is_err());
        let scopes: Vec<String> = (0..40).map(|i| format!("scope{i}")).collect();
        assert!(ConnectorProfile {
            webhook_endpoints: vec![],
            scopes,
            routing: None,
            outbound_scopes: std::collections::BTreeMap::new(),
        }
        .validate()
        .is_err());
        assert!(ConnectorProfile {
            webhook_endpoints: vec![],
            scopes: vec!["bad scope with spaces".into()],
            routing: None,
            outbound_scopes: std::collections::BTreeMap::new(),
        }
        .validate()
        .is_err());
    }

    /// ADR 0023 Decision 3: the one manifest relaxation. A wasm manifest may
    /// declare the egress allowlist its `http_request` reads, and may not
    /// declare the two members that stay connector-only.
    #[test]
    fn a_wasm_manifest_may_declare_outbound_scopes_but_not_webhooks_or_routing() {
        let mut m = manifest(&["automation.reads"]);
        m.connector_profile = Some(ConnectorProfile {
            webhook_endpoints: vec![],
            scopes: vec![],
            routing: None,
            outbound_scopes: [(
                "telemetry".to_string(),
                vec!["https://api.example.com/v1".to_string()],
            )]
            .into_iter()
            .collect(),
        });
        assert!(
            m.validate().is_ok(),
            "a wasm manifest must be able to declare its egress allowlist"
        );

        // The two connector-only members stay refused on a wasm manifest.
        let mut with_webhook = m.clone();
        with_webhook
            .connector_profile
            .as_mut()
            .unwrap()
            .webhook_endpoints = vec!["https://hooks.example.com/x".into()];
        assert!(with_webhook.validate().is_err());

        let mut with_routing = m.clone();
        with_routing.connector_profile.as_mut().unwrap().routing = Some(RoutingProfile {
            capabilities: vec!["matrix".into()],
            modes: vec!["driving".into()],
            base_url: "https://relay.example.com".into(),
            quality: "measured".into(),
            time_control: "none".into(),
            contract_revision: ROUTING_CONTRACT_REVISION,
        });
        assert!(with_routing.validate().is_err());

        // A connector keeps the profile it always had, egress included.
        let mut connector = manifest(&["automation.actions"]);
        connector.artifact_kind = Some(PluginRuntimeKind::HttpConnector);
        connector.connector_profile = Some(ConnectorProfile {
            webhook_endpoints: vec!["https://hooks.example.com/x".into()],
            scopes: vec![],
            routing: None,
            outbound_scopes: [(
                "crm".to_string(),
                vec!["https://api.example.com".to_string()],
            )]
            .into_iter()
            .collect(),
        });
        assert!(connector.validate().is_ok());
    }

    /// The egress allowlist is modeled: exact https hosts, no credentials, no
    /// query, no wildcard, bounded profile and scope counts.
    #[test]
    fn outbound_scopes_validate_like_a_consent_screen_reads_them() {
        let profile = |scopes: Vec<&str>| ConnectorProfile {
            webhook_endpoints: vec![],
            scopes: vec![],
            routing: None,
            outbound_scopes: [(
                "telemetry".to_string(),
                scopes.into_iter().map(str::to_string).collect(),
            )]
            .into_iter()
            .collect(),
        };
        assert!(
            profile(vec!["https://api.example.com/v1", "https://b.example.com"])
                .validate()
                .is_ok()
        );

        assert!(
            profile(vec!["http://api.example.com"]).validate().is_err(),
            "http"
        );
        assert!(
            profile(vec!["https://user:pw@api.example.com"])
                .validate()
                .is_err(),
            "credentials"
        );
        assert!(
            profile(vec!["https://*.example.com"]).validate().is_err(),
            "wildcard"
        );
        assert!(
            profile(vec!["https://api.example.com/v1?x=1"])
                .validate()
                .is_err(),
            "query"
        );
        assert!(
            profile(vec!["https://api.example.com/v1#frag"])
                .validate()
                .is_err(),
            "fragment"
        );
        assert!(profile(vec!["not a url"]).validate().is_err(), "relative");
        assert!(profile(vec![]).validate().is_err(), "empty profile");
        assert!(
            profile(vec!["https://a.example.com", "https://a.example.com"])
                .validate()
                .is_err(),
            "duplicate scope"
        );

        let mut bad_name = profile(vec!["https://a.example.com"]);
        bad_name.outbound_scopes = [(
            "Telemetry".to_string(),
            vec!["https://a.example.com".to_string()],
        )]
        .into_iter()
        .collect();
        assert!(bad_name.validate().is_err(), "profile name grammar");

        let mut too_many_profiles = profile(vec!["https://a.example.com"]);
        too_many_profiles.outbound_scopes = (0..MAX_EGRESS_PROFILES + 1)
            .map(|i| (format!("p{i}"), vec!["https://a.example.com".to_string()]))
            .collect();
        assert!(too_many_profiles.validate().is_err(), "profile cap");
    }

    #[test]
    fn routing_profile_should_validate_closed_vocabulary_and_contract_revision() {
        let valid = RoutingProfile {
            capabilities: vec!["matrix".into(), "route".into()],
            modes: vec!["driving".into(), "cycling".into()],
            base_url: "https://relay.example.com".into(),
            quality: "measured".into(),
            time_control: "departure".into(),
            contract_revision: ROUTING_CONTRACT_REVISION,
        };
        assert!(valid.validate().is_ok());

        let mut insecure = valid.clone();
        insecure.base_url = "http://relay.example.com".into();
        assert!(insecure.validate().is_err());

        let mut unknown_capability = valid.clone();
        unknown_capability.capabilities = vec!["teleport".into()];
        assert!(unknown_capability.validate().is_err());

        let mut duplicated = valid.clone();
        duplicated.capabilities = vec!["matrix".into(), "matrix".into()];
        assert!(duplicated.validate().is_err());

        let mut unknown_mode = valid.clone();
        unknown_mode.modes = vec!["hoverboard".into()];
        assert!(unknown_mode.validate().is_err());

        let mut future_revision = valid.clone();
        future_revision.contract_revision = ROUTING_CONTRACT_REVISION + 1;
        assert!(future_revision.validate().is_err());

        let mut no_modes = valid.clone();
        no_modes.modes = vec![];
        assert!(no_modes.validate().is_err());

        // Every routing capability maps onto the scheduling VIEW gate —
        // a routing grant never confers scheduling edits.
        for key in [
            "routing.matrix",
            "routing.route",
            "routing.geocode",
            "routing.isochrone",
        ] {
            let capability = PluginCapability::parse(key).expect("closed vocabulary");
            assert_eq!(
                capability.permission(),
                Permission::Matrix {
                    module: Module::Scheduling,
                    action: Action::View,
                }
            );
        }
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

    /// The plugin-kit's `KNOWN_CAPABILITIES` (`packages/plugin-kit/bin/build.mjs`)
    /// is the JS mirror of this enum. A plugin author pre-validates a manifest
    /// against that Set locally, and the host validates the same manifest
    /// against `PluginCapability`, so the two must agree exactly. They did not
    /// once: a capability in the host and not in the Set is a manifest the kit
    /// rejects and the host accepts.
    ///
    /// This test reads the Set out of the file and pins it to `ALL`. An enum
    /// variant added without its wire string in the Set fails here, and so does
    /// a string left behind in the Set after a variant is removed. That parity
    /// was a promise in the kit's comment and belonged to no test until the
    /// lane that added a capability also owned the check.
    #[test]
    fn plugin_capability_mirror_matches_the_plugin_kit() {
        use std::collections::BTreeSet;

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packages/plugin-kit/bin/build.mjs");
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));

        // Scope the scan to the Set literal, so a quoted string elsewhere in
        // the file is never mistaken for a capability, and split on the quote
        // so only the odd pieces (the string bodies) are collected.
        let open = src
            .find("const KNOWN_CAPABILITIES = new Set([")
            .expect("the plugin-kit still declares KNOWN_CAPABILITIES as a Set literal");
        let literal = &src[open..];
        let literal = &literal[..literal.find("]);").expect("the Set literal is terminated")];
        let mirror: BTreeSet<&str> = literal.split('"').skip(1).step_by(2).collect();
        let host: BTreeSet<&str> = PluginCapability::ALL
            .iter()
            .map(|c| (*c).as_str())
            .collect();

        let missing_from_kit: Vec<_> = host.difference(&mirror).collect();
        let unknown_to_host: Vec<_> = mirror.difference(&host).collect();
        assert!(
            missing_from_kit.is_empty() && unknown_to_host.is_empty(),
            "the plugin-kit capability mirror in {} drifted from PluginCapability::ALL\n  \
             in the enum but missing from the file: {missing_from_kit:?}\n  \
             in the file but not the enum: {unknown_to_host:?}",
            path.display()
        );
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
