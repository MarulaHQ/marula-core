//! Org-data export bundle (TASKLIST D1, portability/DR).
//!
//! Purpose: the canonical, schema-versioned JSON bundle over an org's
//! LOGICAL entities — the generalization of the U5 customization manifest
//! (same envelope discipline, wider scope). Where the manifest documents
//! what an org CUSTOMIZED, the bundle also carries the operational data
//! plane: the org profile, the client/site/contact hierarchy, boards'
//! operational identity, tickets with their comments and custom-field
//! values, and time entries.
//!
//! ## Envelope evolution contract
//!
//! Mirrors [`super::manifest::CustomizationManifest`]: `bundle_version` is 1
//! and bumps ONLY for a breaking section reshape; sections grow additively;
//! consumers tolerate unknown fields. The U5 customization sections are
//! embedded VERBATIM under `sections.customization` — the manifest stays
//! the customization inventory of record, and D1 never re-shapes it.
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
//! hashes), and the operational sections never read the `users` table —
//! people appear only as opaque user-id references (assignees, comment
//! authors, time-entry owners). User provisioning data is NOT part of the
//! portability surface.

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
    /// Human number (e.g. "MER-12") — the bundle's stable ticket key.
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
