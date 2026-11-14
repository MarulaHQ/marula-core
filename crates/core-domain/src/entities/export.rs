//! Org-data export bundle (the private build tracker D1, portability/DR).
//!
//! Purpose: the canonical, schema-versioned JSON bundle over an org's
//! LOGICAL entities — the generalization of the U5 customization manifest
//! (same envelope discipline, wider scope). Where the manifest documents
//! what an org CUSTOMIZED, the bundle also carries the operational data
//! plane: the org profile, the client/site/contact hierarchy, tickets with
//! their comments and custom-field values, time entries, and (since the
//! G13 coverage audit) the money documents (contracts, billing runs,
//! invoices with lines and receipts, credit notes), assets with their
//! curated field tier and identity keys, the knowledge base with its portal
//! visibility, documents with their version history, tags, attachment
//! metadata, the audit stream, and the plugin/connector install state that
//! can survive a move at all.
//!
//! ## What "history" means here
//!
//! A migration carries two different histories. Ticket history is the
//! nested comment thread plus the ticket's own timestamps and status path;
//! org history is the audit stream (`audit_events`). Both are in the bundle
//! and both are needed: a ticket that arrives without its audit trail has
//! lost who closed it, and an org that arrives without its audit stream has
//! lost the record that answers that question for everything else. The
//! field-level write trail (`provenance_events`) is NOT carried: it is an
//! internal undo/audit mechanism keyed to sessions and ruleset hashes that
//! do not exist in the target, so it would describe a machine rather than
//! the org.
//!
//! ## What a credential does
//!
//! Nothing survives. Sealed credential bytes are bound to the key and
//! machine that sealed them, so copying them would move ciphertext that no
//! target can open — worse than absence, because it looks like a working
//! connection. Connections therefore travel as configuration plus
//! `has_sealed_credential`, and an import MUST require re-entry (the
//! connection cannot be `connected` on arrival). The same holds for vault
//! secret records and plugin credential bindings, which are not in the
//! bundle at all.
//!
//! ## Bytes
//!
//! Attachment payloads are out of scope: `attachments` carries metadata and
//! the content address (`sha256`), never the object. Storage keys are not
//! carried either — they are derivable from the address on the target, and
//! shipping one would point a reader at a machine it cannot reach.
//!
//! ## Residual scope (named, not hidden)
//!
//! Contract SCHEDULES (rates, fixed fees, prepaid blocks, caps, minimums,
//! escalators, SLA credits, change orders) are not in v1, nor are the
//! projects/stock subsystems. A migrated org gets its invoices and the
//! agreement each was billed under; a reopened contract's schedule is
//! re-entered. That boundary is the one an importer must state to a client.
//!
//! ## Envelope evolution contract
//!
//! Mirrors [`super::manifest::CustomizationManifest`]: `bundle_version` is 1
//! and bumps ONLY for a breaking section reshape; sections grow additively;
//! consumers tolerate unknown fields. The U5 customization sections are
//! embedded VERBATIM under `sections.customization` — the manifest stays
//! the customization inventory of record, and D1 never re-shapes it. The G13
//! coverage sections (money, assets, KB, docs, audit, plugins/connectors,
//! tags, attachments) were added WITHOUT a version bump for exactly that
//! reason: a reader that predates them ignores keys it does not know, and
//! every existing key kept its name, type and meaning.
//!
//! ## Determinism contract
//!
//! Every array is sorted by stable semantic keys (names, numbers, instants
//! — ids only as tiebreakers) by the core-db loader, so two consecutive
//! dumps of an unchanged org are byte-identical apart from `generated_at`.
//!
//! ## Secret discipline
//!
//! No secret material is representable: the customization section keeps the
//! manifest's structural exclusions (no webhook signing secrets, no API-key
//! hashes), the operational sections never read the `users` table — people
//! appear only as opaque user-id references (assignees, comment authors,
//! time-entry owners). User provisioning data is NOT part of the
//! portability surface. Connector connections carry their non-secret
//! settings and a boolean saying a sealed credential existed; the sealed
//! document itself is never read, and the vault tables are never touched.
//! Attachment rows carry the content address, never bytes.

use super::manifest::ManifestSections;
use serde::{Deserialize, Serialize};

/// The org-data export envelope, `bundle_version` 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportBundle {
    /// Envelope schema version. Bump ONLY for breaking section reshapes.
    pub bundle_version: u32,
    /// The org this bundle documents.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: uuid::Uuid,
    /// Dump instant (excluded from determinism comparisons).
    pub generated_at: chrono::DateTime<chrono::Utc>,
    /// The export sections. Always fully serialized.
    pub sections: ExportSections,
}

/// Section container — all fields always present; empty org state is an
/// empty array, never an absent key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportSections {
    /// The U5 customization-manifest sections, embedded verbatim — D1 is
    /// the manifest's generalization, not its replacement.
    pub customization: ManifestSections,
    /// Org profile identity (volatile counters excluded — a restore
    /// reseeds the ticket-number sequence from the imported tickets).
    pub org: ExportOrg,
    pub clients: Vec<ExportClient>,
    pub sites: Vec<ExportSite>,
    pub contacts: Vec<ExportContact>,
    /// Tickets with their comments nested. Parents (client/site/contact)
    /// are referenced by id into the sections above.
    pub tickets: Vec<ExportTicket>,
    pub time_entries: Vec<ExportTimeEntry>,
    /// Every custom-field VALUE in the org, flat across entity families —
    /// the data plane the U5 manifest deliberately excludes. `entity` names
    /// the family the field definition attaches to (ticket | client |
    /// contact | asset); `entity_id` is the owning row.
    pub custom_field_values: Vec<ExportFieldValue>,
    /// Recurring / retainer / T&M contract headers. Carried because an
    /// invoice's `contract_id` is NOT NULL: money documents cannot move
    /// across without the agreement they were billed under. The attached
    /// rate, fee, block, minimum and escalator SCHEDULES are a separate
    /// concern and are not in v1 (see the module note on residual scope).
    pub contracts: Vec<ExportContract>,
    /// Billing-run headers — the batch an invoice was issued in.
    pub invoice_runs: Vec<ExportInvoiceRun>,
    /// Invoices with their lines, payments and the time entries they
    /// consumed nested. A credit note is money, so it is its own section.
    pub invoices: Vec<ExportInvoice>,
    pub credit_notes: Vec<ExportCreditNote>,
    /// Assets / configurations: the instance row plus the human-curated
    /// field tier and the identity keys that let an import re-match the
    /// row instead of duplicating it.
    pub assets: Vec<ExportAsset>,
    pub kb_categories: Vec<ExportKbCategory>,
    /// KB articles including `client_visible` — whether a client can read
    /// an article is a migration decision, not a rendering detail.
    pub kb_articles: Vec<ExportKbArticle>,
    /// The board/status columns the U5 manifest does not carry. The
    /// manifest already owns board identity, statuses (name, category,
    /// sort order, entry flag, colour) and transitions; this section holds
    /// the remainder, keyed by the same ids, and never a second copy.
    pub board_operational: ExportBoardOperational,
    /// The audit stream — the org's record of who did what. Ticket history
    /// is derived from it plus the nested comment threads (see the loader
    /// note on what "history" means for a migration).
    pub audit_events: Vec<ExportAuditEvent>,
    pub plugin_installs: Vec<ExportPluginInstall>,
    /// Connector connections and their bindings. `has_sealed_credential`
    /// says a credential EXISTS; the sealed bytes never leave the source
    /// install because they are key-bound, so an import must re-authenticate.
    pub connector_connections: Vec<ExportConnectorConnection>,
    pub capability_defaults: Vec<ExportCapabilityDefault>,
    /// "This record IS that vendor object" — the vendor identity a
    /// migrated record needs so a connector resumes instead of minting a
    /// duplicate. Connection ids reference the section above.
    pub external_refs: Vec<ExportExternalRef>,
    /// Typed record-to-record edges (ticket ↔ configuration, ticket/KB ↔
    /// KB, KB ↔ configuration).
    pub record_links: Vec<ExportRecordLink>,
    /// Documents with their version bodies. Text, not bytes: attachment
    /// payloads are out of scope (see `attachments`).
    pub documents: Vec<ExportDocument>,
    pub document_links: Vec<ExportDocumentLink>,
    pub tags: Vec<ExportTag>,
    /// The ticket↔tag edge (the product's ticket-tagging state).
    pub ticket_tags: Vec<ExportTicketTag>,
    /// Attachment METADATA only. The bytes live in the ObjectStore under a
    /// content address; they are not in the bundle, and `sha256` is the
    /// identity an import re-uploads against.
    pub attachments: Vec<ExportAttachment>,
}

/// Org profile identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportOrg {
    pub name: String,
    pub slug: String,
    /// Human ticket-number prefix (e.g. "MER").
    pub ticket_prefix: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// One client company row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportClient {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    pub name: String,
    /// Optional visibility restriction: when non-empty, only these users
    /// (plus admins) can see the client. Opaque user-id references.
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<uuid::Uuid>))]
    pub restricted_to: Vec<uuid::Uuid>,
    pub notes: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// One site row; `client_id` references `clients` in this bundle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportSite {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    pub name: String,
    pub address: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub timezone: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// One contact row; `client_id`/`site_id` reference this bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportContact {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid, nullable = true))]
    pub site_id: Option<uuid::Uuid>,
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub title: Option<String>,
    pub is_primary: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// One ticket with its comment thread nested. Authorship is an opaque
/// user-id reference; the bundle never carries user records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportTicket {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    /// Human number (e.g. "MAR-12") — the bundle's stable ticket key.
    pub number: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub status_id: uuid::Uuid,
    pub title: String,
    pub body: Option<String>,
    pub priority: crate::status::Priority,
    /// Semantic category of the captured status (behavior binds to this,
    /// never to the status label).
    pub category: crate::status::TicketStatusCategory,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid, nullable = true))]
    pub client_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid, nullable = true))]
    pub site_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid, nullable = true))]
    pub contact_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid, nullable = true))]
    pub assignee_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid, nullable = true))]
    pub requester_id: Option<uuid::Uuid>,
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
    pub resolved_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub comments: Vec<ExportComment>,
}

/// One ticket comment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportComment {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid, nullable = true))]
    pub author_id: Option<uuid::Uuid>,
    /// Structured note taxonomy: internal | client_visible | in_house_update.
    pub kind: crate::status::NoteKind,
    pub body: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// One time entry; `ticket_id` and `user_id` reference this bundle's
/// tickets and the org's live users respectively.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportTimeEntry {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    /// `None` on project-scoped entries (Wave 39 lane 4): `phase_id`
    /// scopes the entry instead.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub phase_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: uuid::Uuid,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub ended_at: chrono::DateTime<chrono::Utc>,
    pub minutes: i64,
    pub billable: bool,
    pub notes: Option<String>,
    pub work_type: Option<String>,
    pub unbillable_reason: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// One typed custom-field value attached to any entity row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportFieldValue {
    /// Field-definition family the value belongs to (ticket | client |
    /// contact | asset) — resolved from the definition at load time.
    pub entity: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub entity_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub field_id: uuid::Uuid,
    /// The typed value exactly as stored (JSONB passthrough).
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub value: serde_json::Value,
}

// --- money documents ------------------------------------------------------------
//
// Enum-backed columns below travel as the stored TEXT, not a parsed domain
// enum. The ticket section predates this and keeps parsed enums because its
// `category` binds behavior (SLA, automation, reporting) and must never be
// re-interpreted by a reader; a document status is a label with a CHECK, and a
// bundle written by a newer Core must still deserialize in an older reader.

/// One contract header. The schedules attached to a contract (rates, fixed
/// fees, prepaid blocks, caps, minimums, escalators, change orders) are NOT
/// in v1 — an invoice can move with its agreement named by id, but a
/// reopened contract's schedule has to be re-entered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportContract {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    pub number: String,
    pub name: String,
    /// recurring | time_and_materials | retainer | fixed_fee | per_unit |
    /// per_incident.
    pub contract_type: String,
    /// draft | active | ended | cancelled.
    pub status: String,
    pub currency: String,
    pub effective_from: chrono::NaiveDate,
    pub effective_until: Option<chrono::NaiveDate>,
    /// monthly | quarterly | annual (null for non-recurring contracts).
    pub billing_frequency: Option<String>,
    pub billing_anchor: Option<chrono::NaiveDate>,
    pub amount_minor: Option<i64>,
    pub hourly_rate_minor: Option<i64>,
    pub included_minutes: Option<i32>,
    pub overage_rate_minor: Option<i64>,
    pub unit_price_minor: Option<i64>,
    pub unit_basis: Option<String>,
    pub incident_price_minor: Option<i64>,
    pub incident_priority: Option<String>,
    pub po_number: Option<String>,
    pub notes: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// One billing run: the batch identity an invoice was issued in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportInvoiceRun {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    pub number: String,
    pub period_from: chrono::NaiveDate,
    pub period_to: chrono::NaiveDate,
    pub invoice_count: i32,
    pub total_minor: i64,
    pub currency: Option<String>,
    pub due_in_days: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// One invoice with its lines, receipts and consumed time entries nested.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportInvoice {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    /// The billing run that issued this invoice, when one did.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub run_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    /// The agreement billed (references `contracts`).
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub contract_id: uuid::Uuid,
    /// The org-unique human number — this section's stable semantic key.
    pub number: String,
    /// draft | issued | paid | void.
    pub status: String,
    pub currency: String,
    pub period_from: Option<chrono::NaiveDate>,
    pub period_to: Option<chrono::NaiveDate>,
    pub issued_at: chrono::NaiveDate,
    pub due_on: chrono::NaiveDate,
    pub total_minor: i64,
    pub total_paid_minor: i64,
    /// The FX snapshot the invoice was issued under, when the org bills in
    /// a secondary currency (ADR 0002: a frozen rate, never recomputed).
    /// The frozen rate, as the exact decimal text Postgres stores.
    pub fx_rate_used: Option<String>,
    pub fx_rate_date: Option<chrono::NaiveDate>,
    pub fx_source: Option<String>,
    pub base_total_minor: Option<i64>,
    /// none | reminder_sent | overdue | final_notice | collections.
    pub dunning_state: String,
    pub dunning_updated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub dunning_note: Option<String>,
    pub po_number: Option<String>,
    pub notes: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub lines: Vec<ExportInvoiceLine>,
    pub payments: Vec<ExportInvoicePayment>,
    /// Time entries this invoice consumed, by id into `time_entries`. The
    /// ids are the whole relation — the consumed entry is not copied.
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<uuid::Uuid>))]
    pub billed_time_entry_ids: Vec<uuid::Uuid>,
}

/// One invoice line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportInvoiceLine {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    /// The agreement the line was billed under (references `contracts`).
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub contract_id: uuid::Uuid,
    /// recurring_base | retainer_base | retainer_overage | time_materials |
    /// fixed_fee (the engine emits further kinds the column's CHECK widens
    /// over time — carried verbatim for that reason).
    pub kind: String,
    pub description: String,
    pub period_start: chrono::NaiveDate,
    pub period_end: chrono::NaiveDate,
    pub quantity_minutes: i64,
    pub unit_amount_minor: i64,
    pub amount_minor: i64,
    pub position: i32,
    /// The frozen rate, as the exact decimal text Postgres stores.
    pub fx_rate_used: Option<String>,
    pub fx_rate_date: Option<chrono::NaiveDate>,
    pub fx_source: Option<String>,
    pub base_amount_minor: Option<i64>,
}

/// One receipt against an invoice (a credit-note application lands one of
/// these too, with `provider` = "credit_note").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportInvoicePayment {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    pub amount_minor: i64,
    pub currency: String,
    pub provider: String,
    /// The provider's own reference (or the credit-note number).
    pub provider_ref: Option<String>,
    pub notes: Option<String>,
    pub received_at: chrono::DateTime<chrono::Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<uuid::Uuid>,
    /// The frozen rate, as the exact decimal text Postgres stores.
    pub fx_rate_used: Option<String>,
    pub fx_rate_date: Option<chrono::NaiveDate>,
    pub fx_source: Option<String>,
    pub base_amount_minor: Option<i64>,
}

/// One credit note (money against a client) with its applications nested.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportCreditNote {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    pub number: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    /// The invoice the credit was raised against (issuing context only).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub issued_against_invoice_id: Option<uuid::Uuid>,
    /// issued | void. "applied"/"refunded" are derived from the counters.
    pub status: String,
    pub currency: String,
    pub total_minor: i64,
    pub applied_minor: i64,
    pub refunded_minor: i64,
    pub reason: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub applications: Vec<ExportCreditNoteApplication>,
}

/// One (credit note → invoice) allocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportCreditNoteApplication {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub invoice_id: uuid::Uuid,
    pub amount_minor: i64,
    /// The invoice payment this application inserted (the invoice-side
    /// effect), by id into the owning invoice's `payments`.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub payment_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// --- assets / configurations ------------------------------------------------------

/// One asset / configuration instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportAsset {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    /// Nullable since Wave 32: an internal-org asset (the MSP's own
    /// equipment) has no client.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub client_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub site_id: Option<uuid::Uuid>,
    /// The human asset key ("ACME-LT-01") — this section's ordering key.
    pub tag: String,
    pub name: String,
    /// Free-text compatibility class; behavior binds to `layout_id`'s
    /// semantic class, never to this label.
    pub kind: String,
    pub status: String,
    /// The asset-type layout (document_layouts) the field schema comes from.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub layout_id: Option<uuid::Uuid>,
    /// The EFFECTIVE field view (curated overlaid on observed).
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub fields: serde_json::Value,
    /// The human-curated tier alone — sticky across feeds, so it is the
    /// tier a migration must not lose. Observed values are re-observed by
    /// whichever connector resumes; the effective view above is what the
    /// user saw.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub curated_fields: serde_json::Value,
    /// Deterministic identity keys ({serial, service_tag, macs, hostname…})
    /// — what an import matches on instead of minting a duplicate.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub identity_keys: serde_json::Value,
    /// owned | leased | rented.
    pub ownership: String,
    pub rental_vendor: Option<String>,
    pub rental_start: Option<chrono::NaiveDate>,
    pub rental_end: Option<chrono::NaiveDate>,
    pub rental_cost_minor: Option<i64>,
    pub rental_currency: Option<String>,
    pub rental_cycle: Option<String>,
    pub purchase_date: Option<chrono::NaiveDate>,
    pub purchase_cost_minor: Option<i64>,
    pub purchase_currency: Option<String>,
    pub depreciation_method: String,
    pub useful_life_months: Option<i32>,
    pub salvage_value_minor: Option<i64>,
    pub depreciation_rate_bps: Option<i32>,
    /// Non-null means the row was merged into the target and survives as
    /// history; the target is canonical.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub merged_into_id: Option<uuid::Uuid>,
    /// internal | client (whose equipment the row is).
    pub scope: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

// --- knowledge base -----------------------------------------------------------------

/// One KB category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportKbCategory {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub position: i32,
    /// Whether the whole category is client-visible (ANDed with the
    /// article's own flag at the read path).
    pub client_visible: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// One KB article. `content_jsonb` is the canonical rich-text document;
/// `body` is the plain-text projection the FTS and older readers use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportKbArticle {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub category_id: Option<uuid::Uuid>,
    pub title: String,
    /// Org-unique slug — the article's stable semantic key.
    pub slug: String,
    /// draft | published | archived.
    pub status: String,
    /// THE portal visibility decision: whether a client may read this.
    pub client_visible: bool,
    pub body: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Object, nullable = true))]
    pub content_jsonb: Option<serde_json::Value>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub updated_by: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

// --- board operational remainder ----------------------------------------------------

/// The board/status columns the U5 manifest does not carry. Board identity,
/// statuses (name/category/sort order/entry/colour) and transitions stay in
/// `customization.boards`; this section is keyed by the same ids and adds
/// only the remainder, so there is one row per board and no duplicate copy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportBoardOperational {
    pub boards: Vec<ExportBoardDetails>,
    pub statuses: Vec<ExportBoardStatusDetails>,
}

/// Board columns beyond the manifest's: prose, the inbound email address,
/// and the group the board is filed under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportBoardDetails {
    /// References a board in `customization.boards`.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: uuid::Uuid,
    pub description: Option<String>,
    /// The address that creates tickets on this board, when email-in is on.
    pub inbound_address: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub board_group_id: Option<uuid::Uuid>,
}

/// Status columns beyond the manifest's: the WIP cap and portal visibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportBoardStatusDetails {
    /// References a status inside a `customization.boards` entry.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub status_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: uuid::Uuid,
    pub wip_limit: Option<i32>,
    pub client_visible: bool,
}

// --- audit history ------------------------------------------------------------------

/// One audit row. `detail` is the action's human-readable payload; no secret
/// material is written to it (see the loader's secret-discipline note).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportAuditEvent {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    /// human | agent | plugin | connector | system | portal_contact |
    /// portal_approver | portal_it_manager | import.
    pub actor_kind: String,
    /// Opaque id of the actor (a user id, an agent name, a plugin id).
    pub actor_id: Option<String>,
    pub action: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub detail: serde_json::Value,
    /// The request's source address, when the writer recorded one.
    pub ip: Option<String>,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
}

// --- plugins + connectors -----------------------------------------------------------

/// One plugin install. Secrets live in the vault and in the connector
/// connection rows, never here; `has_credentials` says a binding exists so
/// an import knows the operator must re-provision it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportPluginInstall {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub catalog_id: uuid::Uuid,
    pub version: String,
    /// active | disabled | revoked (verbatim stored state).
    pub status: String,
    /// The capability grant the operator consented to. Carried because a
    /// migrated install must not silently WIDEN its grant.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub granted_capabilities: serde_json::Value,
    /// The staged upgrade, when one is pending — so a half-consented
    /// upgrade survives (or is visibly dropped) rather than vanishing.
    pub pending_version: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub pending_manifest: Option<serde_json::Value>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub pending_capabilities: Option<serde_json::Value>,
    /// The install's non-secret settings (`plugin_install_settings.config`).
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub settings: serde_json::Value,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub installed_by: Option<uuid::Uuid>,
    pub installed_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_reason: Option<String>,
}

/// One connector connection plus its destinations. `secrets_encrypted` is
/// structurally absent: the sealed document is key-bound to the install it
/// was written on, so nothing usable could move even if it were copied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportConnectorConnection {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    /// References a plugin install in `plugin_installs`.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub installation_id: uuid::Uuid,
    /// Null is the org-wide instance; a client id is the per-client one.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub scope_client_id: Option<uuid::Uuid>,
    pub label: String,
    /// The non-secret settings the resolver reads (base URL, company id).
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub settings: serde_json::Value,
    /// Did a sealed credential exist? An import must re-authenticate it.
    pub has_sealed_credential: bool,
    /// connected | needs_attachment | auth_failed | degraded | disabled.
    pub status: String,
    pub status_detail: Option<String>,
    pub last_success_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_failure_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub bindings: Vec<ExportConnectorBinding>,
}

/// One named destination a connection may post to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportConnectorBinding {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    /// slack_channel | teams_conversation | rewst_workflow | voip_route.
    pub binding_kind: String,
    pub label: String,
    /// The vendor's own id for the destination (a channel id, not a name).
    pub external_id: String,
    pub external_parent: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub vendor_ids: serde_json::Value,
    pub enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Which connection a capability runs first, per org and optionally per
/// client. Without it, migrated chips have no primary provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportCapabilityDefault {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub scope_client_id: Option<uuid::Uuid>,
    pub capability: String,
    /// References a connection in `connector_connections`.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub connection_id: uuid::Uuid,
}

/// One vendor identity of a Core record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportExternalRef {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    /// configuration | client | site | contact | ticket | kb_article |
    /// invoice | payment | credit_note | quote | purchase_order | domain.
    pub record_type: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub record_id: uuid::Uuid,
    /// The connection this reference was learned through; null for an
    /// import-era key that predates any live connection.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub connection_id: Option<uuid::Uuid>,
    /// Display and import vocabulary ('ninjaone', 'qbo', 'connectwise').
    pub source_system: String,
    pub external_kind: String,
    pub external_id: String,
    /// A document URL the connector LEARNED from the vendor (never guessed).
    pub external_url: Option<String>,
    pub source_guid: Option<String>,
    pub first_seen_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// One typed edge between two records. Ids only — no embedded copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportRecordLink {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    /// ticket | kb_article.
    pub from_kind: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub from_id: uuid::Uuid,
    /// configuration | kb_article.
    pub to_kind: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub to_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// --- docs platform ------------------------------------------------------------------

/// One document with every stored version. The body is text, not bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportDocument {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    pub title: String,
    /// The live version number (the last entry in `versions`).
    pub current_version: i32,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub client_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub layout_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub fields: serde_json::Value,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub updated_by: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub versions: Vec<ExportDocumentVersion>,
}

/// One stored document version — the document's change history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportDocumentVersion {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    pub version: i32,
    pub title: String,
    pub body: String,
    /// Why the version was written (required at the write path).
    pub change_summary: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub author_id: Option<uuid::Uuid>,
    /// Which earlier version this one restored, when it is a restore.
    pub restored_from_version: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// One document↔record edge (document, password, asset → document, client,
/// ticket, kb_article, asset).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportDocumentLink {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    pub from_kind: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub from_id: uuid::Uuid,
    pub to_kind: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub to_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// --- tags + attachment metadata -------------------------------------------------------

/// One tag definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportTag {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    pub name: String,
    pub color: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// One ticket↔tag edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportTicketTag {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: uuid::Uuid,
    /// References a row in `tags`.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub tag_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub applied_by: Option<uuid::Uuid>,
    pub applied_at: chrono::DateTime<chrono::Utc>,
}

/// Attachment METADATA. The bytes are NOT in the bundle: they stay in the
/// source ObjectStore, identified by `sha256` (the object's content address,
/// from which an import derives its own storage key). A migration therefore
/// moves every attachment's record and none of its bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportAttachment {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    /// ticket | ticket_comment | email_message | sign_off.
    pub kind: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_comment_id: Option<uuid::Uuid>,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    /// The object's content address — the identity an import re-uploads
    /// against, and the only locator the bundle carries.
    pub sha256: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub uploaded_by_user_id: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
