//! DocumentVersion entities (the private build tracker D4, docs/10-agent-platform.md §6):
//! documents whose content exists only as immutable, append-only,
//! diffable snapshots. Restores create NEW versions; history is never
//! rewritten.

use crate::ids::{ClientId, DocumentId, DocumentLayoutId, DocumentVersionId, OrgId, UserId};
use chrono::{DateTime, Utc};

/// A document's living handle. No content lives here — content is only
/// ever a [`DocumentVersion`]; `current_version` names the newest one.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Document {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: DocumentId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    /// NULL = org-global document; set = client-scoped (docs-platform v1,
    /// migration 0053).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub client_id: Option<ClientId>,
    /// The typed layout this document instantiates; NULL = untyped.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub layout_id: Option<DocumentLayoutId>,
    /// Custom-field values validated against the layout's schema.
    #[serde(default)]
    pub fields: serde_json::Value,
    pub title: String,
    /// The newest version number in the document's history.
    pub current_version: i32,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub updated_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One immutable snapshot in a document's append-only history. Every
/// version stores its complete title + body, so any pair is diffable
/// without reconstructing intermediate state.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DocumentVersion {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: DocumentVersionId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub document_id: DocumentId,
    /// 1-based, gap-free per document.
    pub version: i32,
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// Optional human explanation of what changed in this version.
    #[serde(default)]
    pub change_summary: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub author_id: Option<UserId>,
    /// Set when this version was created by a RESTORE: the historical
    /// version whose content was copied. Restore-creates-new-version is
    /// the only way old content comes back — history is never rewritten.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i32>))]
    pub restored_from_version: Option<i32>,
    pub created_at: DateTime<Utc>,
}
