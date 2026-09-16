//! Knowledge-base entities (native KB v1): categories and articles with
//! client-facing visibility flags (TASKLIST Phase 3; feeds portal v2).

use crate::ids::{KbArticleId, KbCategoryId, OrgId, UserId};
use chrono::{DateTime, Utc};

/// Article lifecycle. Closed vocabulary so filtering and the client-facing
/// rule stay deterministic: `draft` (staff authoring), `published`
/// (readable, searchable), `archived` (retired, out of the active tree —
/// content edits are refused while archived; unarchive to edit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum KbArticleStatus {
    Draft,
    Published,
    Archived,
}

impl KbArticleStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
            Self::Archived => "archived",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "draft" => Self::Draft,
            "published" => Self::Published,
            "archived" => Self::Archived,
            _ => return None,
        })
    }
}

/// A KB category: grouping + ordering for the article tree.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct KbCategory {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: KbCategoryId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub name: String,
    /// Stable per-org handle (unique); portals and imports reference this,
    /// never the row id.
    pub slug: String,
    #[serde(default)]
    pub description: String,
    pub position: i32,
    /// The category half of the client-facing gate: an article is visible
    /// to portal clients only when this AND the article flag are set (and
    /// the article is published).
    pub client_visible: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A KB article.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct KbArticle {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: KbArticleId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    /// Survives category deletion (the FK is ON DELETE SET NULL).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub category_id: Option<KbCategoryId>,
    pub title: String,
    /// Stable per-org handle (unique).
    pub slug: String,
    #[serde(default)]
    pub body: String,
    /// Canonical rich content (ADR 0007): the versioned envelope
    /// `{"schema_ver": 1, "doc": <tiptap>}` — `None` for legacy rows the
    /// 0077 backfill has not seen (reads fall back to `body`).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<serde_json::Value>))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_jsonb: Option<serde_json::Value>,
    pub status: KbArticleStatus,
    /// The article half of the client-facing gate (portal v2 honors it).
    pub client_visible: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub updated_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
