//! Docs-platform v1 entities (Wave 11 lane 7, migration 0053): typed
//! document layouts, cross-entity relationship links, and the staff
//! password vault. Documents themselves gain client scope + layout
//! fields (see `documents::Document`).

use crate::ids::{
    ClientId, DocumentId, DocumentLayoutId, DocumentLinkId, OrgId, PasswordHistoryId, PasswordId,
    UserId,
};
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// One field in a layout's schema. The `fields` column stores an array of
/// these (as JSON); the API validates document `fields` objects against
/// them.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LayoutField {
    pub key: String,
    pub label: String,
    /// text | number | date | bool | select | url | email
    pub r#type: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub position: i32,
    /// Select-type options.
    #[serde(default)]
    pub options: Vec<String>,
}

/// An org-managed document/asset type. System layouts (the seeded
/// standard MSP set) are editable but not deletable.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DocumentLayout {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: DocumentLayoutId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    /// Stable per-org handle (unique).
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub icon: String,
    pub fields: Vec<LayoutField>,
    pub is_system: bool,
    /// CI6: the behavior-bearing semantic class this type is bound to
    /// (ADR 0003, migration 0061). Renaming the layout never changes
    /// behavior; identity keys and field schema read this class.
    #[serde(default)]
    pub semantic_class: Option<crate::entities::CiSemanticClass>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Either endpoint of a link. Documents, passwords, assets, and secret
/// records can link FROM; documents, passwords, clients, tickets, KB
/// articles, assets, and secret records can be linked TO (asset kind
/// added in migration 0054; secret kind added in migration 0078 — the
/// native vault joins the cross-entity RecordLink seam, ADR 0006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    Document,
    Password,
    Client,
    Ticket,
    KbArticle,
    Asset,
    Secret,
}

impl LinkKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            LinkKind::Document => "document",
            LinkKind::Password => "password",
            LinkKind::Client => "client",
            LinkKind::Ticket => "ticket",
            LinkKind::KbArticle => "kb_article",
            LinkKind::Asset => "asset",
            LinkKind::Secret => "secret",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "document" => LinkKind::Document,
            "password" => LinkKind::Password,
            "client" => LinkKind::Client,
            "ticket" => LinkKind::Ticket,
            "kb_article" => LinkKind::KbArticle,
            "asset" => LinkKind::Asset,
            "secret" => LinkKind::Secret,
            _ => return None,
        })
    }
}

/// A cross-entity relationship row (`document_links`, 0053).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DocumentLink {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: DocumentLinkId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub from_kind: LinkKind,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub from_id: DocumentId,
    pub to_kind: LinkKind,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub to_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
}

/// A link with its target resolved to a display label for the UI
/// (typed read model; the raw row never carries the label).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ResolvedLink {
    #[serde(flatten)]
    pub link: DocumentLink,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub target_client_id: Option<ClientId>,
    /// Present when the target row still exists.
    #[serde(default)]
    pub target_label: Option<String>,
    /// The target's own current link count — surfaced for document
    /// targets so the panel can hint at hub records.
    #[serde(default)]
    pub target_deleted: bool,
}

/// A vault credential. `secret_ciphertext` is AES-256-GCM from
/// core-api crypto::seal — plaintext NEVER reaches the database layer,
/// and this entity never carries it either (reveal is a dedicated,
/// audited endpoint response type).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PasswordEntry {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: PasswordId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    /// NULL = org-level vault entry.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub client_id: Option<ClientId>,
    pub title: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub url: String,
    pub expires_at: Option<DateTime<Utc>>,
    /// Operator-scheduled rotation due date (Wave 19 lane 3); the worker
    /// sweep reminds once past it (durable at-most-once marker on the
    /// row in the DB; the entities carry no sweep housekeeping).
    #[serde(default)]
    pub rotate_at: Option<DateTime<Utc>>,
    /// Stamped by the same update that moves `rotate_at` forward.
    #[serde(default)]
    pub last_rotated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub has_notes: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub updated_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One archived secret ciphertext from a credential's change history.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PasswordHistoryEntry {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: PasswordHistoryId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub password_id: PasswordId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub changed_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
}

/// The audited reveal response: plaintext is returned exactly once,
/// over TLS, to a caller holding `passwords:edit`; the reveal emits a
/// `password.revealed` contract event (provenance query: "who looked
/// at this credential and when").
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PasswordReveal {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: PasswordId,
    pub title: String,
    pub secret: String,
    #[serde(default)]
    pub notes: Option<String>,
}
