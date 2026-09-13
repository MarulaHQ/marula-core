//! KB rich content (Wave 13 lane 4, KB1/KB2 per ADR 0004): the oEmbed
//! allowlist registry, per-article embed instances, and linked external
//! documents.

use crate::ids::{KbArticleId, KbEmbedId, KbEmbedProviderId, KbExternalDocumentId, OrgId, UserId};
use chrono::{DateTime, Utc};

/// An oEmbed allowlist entry. `org_id` is `None` for the platform-shipped
/// system rows (`is_system`, immutable from the API); org rows are the
/// admin-managed registry extensions.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct KbEmbedProvider {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: KbEmbedProviderId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub org_id: Option<OrgId>,
    pub name: String,
    /// Stable per-org handle (unique; system rows use the same space).
    pub slug: String,
    /// Exact host names this provider accepts.
    pub host_patterns: Vec<String>,
    /// Regex over the resource URL (Rust regex syntax); validated at
    /// write time by the caller.
    pub url_regex: String,
    /// oEmbed endpoint template; `{url}` is substituted once, and the
    /// substituted value is always percent-encoded before the fetch.
    pub oembed_endpoint: String,
    pub enabled: bool,
    pub is_system: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One embed instance attached to a KB article. Its sanction source is
/// the registry row resolved at write time (`provider_slug` snapshots the
/// handle; `provider_id` clears when the provider row is deleted, which
/// renders the embed as blocked until re-resolved).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct KbEmbed {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: KbEmbedId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub article_id: KbArticleId,
    pub url: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub provider_id: Option<KbEmbedProviderId>,
    #[serde(default)]
    pub provider_slug: Option<String>,
    #[serde(default)]
    pub title: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The closed external-document vocabulary. The classifier in `core-kb`
/// maps a URL to exactly one of these; `other` is the honest fallback for
/// any link that is neither SharePoint, OneDrive, nor Google Drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ExternalDocUrlKind {
    #[serde(rename = "sharepoint")]
    SharePoint,
    #[serde(rename = "onedrive")]
    OneDrive,
    #[serde(rename = "google_drive")]
    GoogleDrive,
    #[serde(rename = "other")]
    Other,
}

impl ExternalDocUrlKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SharePoint => "sharepoint",
            Self::OneDrive => "onedrive",
            Self::GoogleDrive => "google_drive",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "sharepoint" => Self::SharePoint,
            "onedrive" => Self::OneDrive,
            "google_drive" => Self::GoogleDrive,
            "other" => Self::Other,
            _ => return None,
        })
    }
}

/// Outcome of a link-check. `unreachable` covers DNS/TLS/HTTP failures —
/// the record exists but could not be verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum LinkCheckStatus {
    Ok,
    Unreachable,
}

impl LinkCheckStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Unreachable => "unreachable",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "ok" => Self::Ok,
            "unreachable" => Self::Unreachable,
            _ => return None,
        })
    }
}

/// A linked external document (KB2): a SharePoint/OneDrive/Drive URL with
/// title, permission note, and the last link-check result. `article_id`
/// attaches it to an article — client-facing exposure rides that
/// article's client-visible gate; standalone rows are staff-only.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct KbExternalDocument {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: KbExternalDocumentId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub article_id: Option<KbArticleId>,
    pub title: String,
    pub url: String,
    pub url_kind: ExternalDocUrlKind,
    #[serde(default)]
    pub permission_note: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub last_verified_at: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub last_verified_status: Option<LinkCheckStatus>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
