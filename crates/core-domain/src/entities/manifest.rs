//! Customization manifest (TASKLIST U5, upgrade-compatibility-policy.md §5).
//!
//! Purpose: the canonical, schema-versioned export describing everything an
//! org has customized — custom field definitions, per-board workflow
//! statuses/transitions with their semantic-category bindings, automation
//! rules, webhook subscriptions with their event filters, API-key metadata
//! (MINUS any secret material), settings-registry org overrides, SLA
//! calendars/policies with escalation ladders, and stored report
//! definitions (R3). It doubles as backup documentation and as the input
//! to the upgrade preflight (U6) and the org-data export (D1 generalizes
//! this envelope, same format, wider scope).
//!
//! ## Envelope evolution contract
//!
//! `manifest_version` is 1. Consumers must tolerate UNKNOWN fields
//! (documented contract, same stance as the event envelope): sections grow
//! by adding a field to [`ManifestSections`], which is additive and
//! non-breaking. Any removal, rename, or reshape of an existing field
//! requires a `manifest_version` bump and a documented deprecation window.
//!
//! Every section field is ALWAYS serialized (empty array when the org has
//! no such customization) so consumers get one stable shape instead of
//! key-present/key-absent branching.
//!
//! ## Determinism contract
//!
//! The loader (core-db `manifest`) orders every array by stable semantic
//! keys (names, sort orders, category positions — ids only as tiebreakers),
//! so two consecutive dumps of an unchanged org are byte-identical apart
//! from `generated_at`. `serde_json::Value` fields pass through Postgres
//! `jsonb`, whose canonical key normalization keeps object bytes stable.
//!
//! Non-purpose: no secrets (webhook signing secrets and API-key hashes are
//! structurally unrepresentable here), no operational data (tickets,
//! values) — that is D1 territory, no organization profile (the `/org`
//! surface owns it).

use serde::{Deserialize, Serialize};

/// The customization manifest envelope, `manifest_version` 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CustomizationManifest {
    /// Envelope schema version. Bump ONLY for breaking section reshapes.
    pub manifest_version: u32,
    /// The org this manifest documents.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: uuid::Uuid,
    /// Dump instant (excluded from determinism comparisons).
    pub generated_at: chrono::DateTime<chrono::Utc>,
    /// The customization sections. Always fully serialized.
    pub sections: ManifestSections,
}

/// Section container. All fields are always present; empty org state is an
/// empty array, never an absent key. New sections append here.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestSections {
    /// Custom field definitions across all entities (tickets, clients,
    /// contacts, assets), including archived ones.
    pub custom_fields: Vec<ManifestCustomField>,
    /// Per-board workflow: the board identity plus its statuses (each bound
    /// to a semantic category — the defaults-first guarantee) and its
    /// explicit transition adjacency. An EMPTY transitions array means the
    /// semantic-category floor applies, not "no transitions".
    pub boards: Vec<ManifestBoard>,
    /// Event-triggered automation rules.
    pub automation_rules: Vec<ManifestAutomationRule>,
    /// Webhook subscriptions with event-type filters, never secrets.
    pub webhooks: Vec<ManifestWebhook>,
    /// API-key metadata minus secret material (no hashes, no raw keys).
    pub api_keys: Vec<ManifestApiKey>,
    /// Org-scope settings-registry overrides (user-scope rows are per-user
    /// preferences, not org customization).
    pub settings: Vec<ManifestSetting>,
    /// SLA configuration: calendars and policies with escalation ladders.
    pub sla: ManifestSla,
    /// Stored report definitions (TASKLIST R3): registry-validated specs
    /// over the versioned read-model views. Covered so the upgrade
    /// preflight (U6) can flag any definition whose `kind`/spec columns
    /// reference deprecated view or column constructs (policy §5).
    pub report_definitions: Vec<ManifestReportDefinition>,
    /// Ticket type/category/subcategory trees (Wave 11 lane 3).
    /// Classification-only (§5): exported so the preflight sees the full
    /// classification surface; retired rows INCLUDED (tickets may still
    /// reference them). Categories are flat — `parent_id` reconstructs
    /// the exactly-two-level tree.
    pub taxonomy: ManifestTaxonomy,
    /// Per-client workflow overrides (Wave 12 lane 5): every client-scoped
    /// override of the org workflow defaults, rows verbatim. The override
    /// registry (`core_domain::workflow_overrides`) is the closed key set;
    /// the preflight must see which constructs each client's rows reference
    /// so a migration touching those constructs transforms the rows in the
    /// same transaction (§4.5).
    pub client_workflow_overrides: Vec<ManifestClientWorkflowOverride>,
    /// Role rows (Wave 13 lane 6, RB1 — ADR 0004: role rows are
    /// customizations-as-data covered by U5/U6 like any other section).
    /// This section is the ORG-OWNED rows only: custom roles and forked
    /// copies (including forked presets with their origin tag). Shipped
    /// preset rows are default state, not customization — they stay
    /// canonical and restorable, so the preflight sees no drift.
    pub roles: Vec<ManifestRole>,
}

/// One org-owned role row of the manifest (a custom role or a forked copy).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestRole {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    pub name: String,
    /// Shipped preset tag, `null` for org-created roles.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, nullable = true))]
    pub builtin: Option<String>,
    pub is_system: bool,
    /// Fork origin preset tag, `null` unless the row is a forked copy.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, nullable = true))]
    pub forked_from: Option<String>,
    /// The full matrix vocabulary (`{"matrix": {"module", "action"}}`
    /// entries plus bare special strings) — verbatim for the preflight.
    pub permissions: serde_json::Value,
}

/// The two classification trees of one org.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestTaxonomy {
    pub types: Vec<ManifestTaxonomyType>,
    pub categories: Vec<ManifestTaxonomyCategory>,
}

/// One ticket type (flat list).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestTaxonomyType {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    pub name: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, nullable = true))]
    pub description: Option<String>,
    pub is_retired: bool,
    pub sort_order: i32,
}

/// One category (root when `parent_id` is null) or subcategory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestTaxonomyCategory {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>, nullable = true))]
    pub parent_id: Option<uuid::Uuid>,
    pub name: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, nullable = true))]
    pub description: Option<String>,
    pub is_retired: bool,
    pub sort_order: i32,
}

/// One per-client workflow override row, verbatim. `state` is
/// `set` (value overrides the org default) or `disabled` (explicit deny);
/// an absent row means the client inherits and exports nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestClientWorkflowOverride {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    /// Stable semantic sort key (ids only break ties across re-imports).
    pub client_name: String,
    /// Registry key from `core_domain::workflow_overrides` (closed set).
    pub key: String,
    /// "set" | "disabled".
    pub state: String,
    /// The typed payload (uuid string for reference kinds, boolean for
    /// toggles); `null` when `disabled`.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>, nullable = true))]
    pub value: Option<serde_json::Value>,
}

/// SLA configuration sections.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestSla {
    pub calendars: Vec<ManifestSlaCalendar>,
    pub policies: Vec<ManifestSlaPolicy>,
}

/// One custom field definition. `config` is the kind-specific document
/// (select options, validation bounds, …) exactly as stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestCustomField {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    /// Entity the field attaches to: ticket | client | contact | asset.
    pub entity: String,
    pub name: String,
    /// Field kind: text | number | date | select | multiselect | bool | url | user.
    pub kind: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub config: serde_json::Value,
    pub required: bool,
    pub sort_order: i32,
    /// Present when the definition was archived (still a customization the
    /// preflight must see; the values it shaped may still exist).
    pub archived_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// One board's workflow customization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestBoard {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    pub name: String,
    /// The org's default intake board.
    pub is_default: bool,
    /// Pre-close checklist enforcement toggle (workflow discipline flag).
    pub require_checklist_close: bool,
    /// Custom statuses; ALWAYS one per semantic category (defaults-first
    /// seeding guarantees category coverage, never removed by edits).
    pub statuses: Vec<ManifestBoardStatus>,
    /// Explicit transition adjacency between this board's statuses. Empty
    /// means the semantic-category floor applies (any→any except identity
    /// and closed→open), not "no transitions".
    pub transitions: Vec<ManifestTransition>,
}

/// One custom status, bound to its semantic category. Renames and colors
/// never change behavior — consumers must branch on `category`, not `name`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestBoardStatus {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    pub name: String,
    /// Semantic category: open | in_progress | waiting | closed. THE
    /// binding every SLA/automation/reporting behavior keys on.
    pub category: crate::status::TicketStatusCategory,
    pub sort_order: i32,
    /// Tickets may enter this status directly at creation.
    pub is_entry: bool,
    pub color: Option<String>,
}

/// One allowed status change, `from` → `to`, both ids from the SAME board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestTransition {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub from_status_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub to_status_id: uuid::Uuid,
}

/// One event-triggered automation rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestAutomationRule {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    /// Board the rule is scoped to; null = every board in the org.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid, nullable = true))]
    pub board_id: Option<uuid::Uuid>,
    /// What the rule's subject is: `ticket` (the historical default) or
    /// `board` — a rule that fires on ticket-less envelopes (Wave 43
    /// lane 3). Absent from a pre-lane manifest, so it defaults.
    #[serde(default = "default_manifest_subject_kind")]
    pub subject_kind: String,
    pub name: String,
    /// Domain event type from the envelope, e.g. `ticket.created`.
    pub trigger_event: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub conditions: serde_json::Value,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub actions: serde_json::Value,
    pub enabled: bool,
}

/// Pre-lane manifests carry no subject declaration; the ticket subject is
/// the historical value.
fn default_manifest_subject_kind() -> String {
    "ticket".to_string()
}

/// One webhook subscription. The HMAC signing secret is structurally
/// absent: it exists only at creation time and in the sealed column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestWebhook {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    pub url: String,
    /// Event-type filter; empty array = wildcard (all event types).
    pub event_types: Vec<String>,
    pub enabled: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid, nullable = true))]
    pub created_by: Option<uuid::Uuid>,
}

/// API-key metadata. `key_hash` (the credential verifier) and the raw key
/// material are structurally absent — only the display prefix survives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestApiKey {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    pub name: String,
    /// Non-secret display identifier (e.g. `mrl_ab12cd34`).
    pub prefix: String,
    /// Capability scope strings ("tickets:read", …).
    pub scopes: Vec<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid, nullable = true))]
    pub created_by: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// One org-scope settings-registry override.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestSetting {
    pub key: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub value: serde_json::Value,
}

/// One named SLA calendar row with its flattened business calendar body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestSlaCalendar {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    pub name: String,
    /// Timezone / weekday windows / holidays / maintenance document.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub calendar: crate::entities::sla::BusinessCalendar,
}

/// One SLA policy with its targets document, embedded fallback calendar,
/// calendar attachment, and escalation ladder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestSlaPolicy {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    pub name: String,
    pub is_default: bool,
    /// Targets document keyed by priority string.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub targets: serde_json::Value,
    /// Embedded fallback calendar; governs while `calendar_id` is null.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub calendar: crate::entities::sla::BusinessCalendar,
    /// Attached named calendar; null keeps the embedded one authoritative.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid, nullable = true))]
    pub calendar_id: Option<uuid::Uuid>,
    /// Escalation ladder steps, ordered by `step_index`.
    pub escalation_steps: Vec<ManifestEscalationStep>,
}

/// One escalation ladder step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestEscalationStep {
    pub step_index: i32,
    /// risk | breach | escalate.
    pub trigger_kind: String,
    /// Percent of the target's business-time budget consumed at firing.
    pub at_pct: i32,
    /// JSON array of user ids to notify.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub notify_user_ids: serde_json::Value,
    pub enabled: bool,
}

/// One stored report definition (TASKLIST R3). The spec body rides as its
/// stored jsonb document (`{filters, group_by, measures}`) exactly like
/// custom-field `config` — the registry-validated view/viz slugs travel as
/// their own fields so a preflight can flag deprecated constructs without
/// re-deriving validation. Nothing here is secret-bearing: definitions
/// reference the read-model catalog only, never credentials or data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManifestReportDefinition {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    pub name: String,
    pub description: Option<String>,
    /// Registry-backed view slug (`tickets`, `time_entries`, …).
    pub kind: String,
    /// Spec body document: `{filters, group_by, measures}` as stored.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub spec: serde_json::Value,
    /// Visualization hint slug (`bar` | `line` | `pie` | `table`).
    pub viz: String,
    /// Visibility scope: `org` (published org-wide) or `self` (private to
    /// the creator; admins see all). Preflight-relevant because org-scope
    /// definitions are shared surface, self-scope ones are personal.
    pub scope: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub created_by: uuid::Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
