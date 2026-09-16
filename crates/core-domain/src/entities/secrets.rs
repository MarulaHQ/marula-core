//! Native secrets vault entities (ADR 0006, migration 0078): secret
//! category trees (client- AND org-scoped), SecretRecord generalization
//! beside PasswordEntry, and the role × folder category grants.
//!
//! Posture this crate pins: plaintext NEVER reaches the database layer —
//! `SecretRecord.fields` carries metadata only (key + shape, no
//! ciphertext, no plaintext); plaintext exists exactly twice: in the
//! write request and in the audited reveal response (`SecretReveal`).
//! Ciphertext rides `secret_fields` (bytea), sealed AES-256-GCM by
//! core-api `crypto::seal` with an AAD binding the ciphertext to its
//! `(org_id, record_id, field_key)` coordinate.

use crate::ids::{ClientId, OrgId, UserId};
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A secret category folder (`secret_folders`). Exactly one of org/client
/// scope rows: org-scope rows carry `org_id` (client_id NULL),
/// client-scope rows carry `client_id` (org_id NULL — the tenancy spine
/// rides the owning client's org via the RLS policy). Seeded defaults
/// (`is_default_seed`) are immutable ADR-0004 template rows: orgs fork,
/// never edit in place. `restricted` is deny-by-default: invisible to a
/// role without a `secret_category_grants` row.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretFolder {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    /// Org-scope tenancy root; NULL on client-scope rows.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub org_id: Option<OrgId>,
    /// Client scope; NULL on org-scope rows.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub client_id: Option<ClientId>,
    /// Parent folder; NULL = root of its scope tree.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub parent_id: Option<Uuid>,
    /// Sibling-unique within (scope, parent); lowercase spellings.
    pub slug: String,
    pub label: String,
    pub kind: SecretFolderKind,
    /// Immutable shipped default (ADR 0004 template doctrine).
    pub is_default_seed: bool,
    /// Deny-by-default category flag (financial:read gate shape).
    pub restricted: bool,
    /// The seed row this folder forked from (customization provenance).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub forked_from: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub updated_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The closed seeded-configuration vocabulary for folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SecretFolderKind {
    Passwords,
    ApiKeys,
    Certificates,
    RecoveryCodes,
    Documents,
}

impl SecretFolderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passwords => "passwords",
            Self::ApiKeys => "api-keys",
            Self::Certificates => "certificates",
            Self::RecoveryCodes => "recovery-codes",
            Self::Documents => "documents",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "passwords" => Self::Passwords,
            "api-keys" => Self::ApiKeys,
            "certificates" => Self::Certificates,
            "recovery-codes" => Self::RecoveryCodes,
            "documents" => Self::Documents,
            _ => return None,
        })
    }

    /// The record kind that classically lives in this folder kind.
    pub fn record_kind(self) -> SecretRecordKind {
        match self {
            Self::Passwords => SecretRecordKind::Password,
            Self::ApiKeys => SecretRecordKind::ApiKey,
            Self::Certificates => SecretRecordKind::Certificate,
            Self::RecoveryCodes => SecretRecordKind::RecoveryCode,
            Self::Documents => SecretRecordKind::Document,
        }
    }
}

/// The kind of a `secret_records` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SecretRecordKind {
    Password,
    ApiKey,
    Certificate,
    RecoveryCode,
    Document,
}

impl SecretRecordKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::ApiKey => "api-key",
            Self::Certificate => "certificate",
            Self::RecoveryCode => "recovery-code",
            Self::Document => "document",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "password" => Self::Password,
            "api-key" => Self::ApiKey,
            "certificate" => Self::Certificate,
            "recovery-code" => Self::RecoveryCode,
            "document" => Self::Document,
            _ => return None,
        })
    }
}

/// A material field of a secret record — METADATA ONLY view. The
/// ciphertext and plaintext never ride this entity; `value_ciphertext`
/// lives in `secret_fields` only.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretFieldMeta {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub record_id: Uuid,
    /// Closed write-path vocabulary: `secret`, `notes`, `username`, `url`,
    /// plus the document-field keys (metadata for non-material pointers).
    pub field_key: String,
    /// Whether a sealed value exists for this key.
    #[serde(default)]
    pub has_value: bool,
    pub updated_at: DateTime<Utc>,
}

/// A SecretRecord (`secret_records`, migration 0078) — the generalization
/// beside the shipped `PasswordEntry` (whose UI and responses stay byte-
/// compatible; the vault never becomes a destructive cutover). Scope
/// mirrors its owning folder; every material field is sealed in
/// `secret_fields` and reachable ONLY via the audited reveal path.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretRecord {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    /// Org-scope tenancy root; NULL on client-scope rows.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub org_id: Option<OrgId>,
    /// Client scope; NULL on org-scope rows.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub client_id: Option<ClientId>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub folder_id: Uuid,
    pub kind: SecretRecordKind,
    pub title: String,
    pub expires_at: Option<DateTime<Utc>>,
    /// Document-kind files ride object storage as ciphertext with
    /// pointer+hash rows — never material blobs in this table.
    #[serde(default)]
    pub object_key: Option<String>,
    #[serde(default)]
    pub file_sha256: Option<String>,
    #[serde(default)]
    pub file_bytes: Option<i64>,
    /// Metadata-only field views (NEVER carries ciphertext or plaintext).
    #[serde(default)]
    pub fields: Vec<SecretFieldMeta>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub updated_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The audited reveal response: the ONLY plaintext carrier for secret
/// records, mirroring `/passwords/{id}/reveal` (requires secrets:edit,
/// emits `secret.revealed`, plaintext returned exactly once). Reveal
/// payloads carry no provenance detail beyond the title — provenance
/// queries never carry the secret.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretReveal {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    pub title: String,
    /// field_key → plaintext for every sealed field.
    pub fields: Vec<SecretRevealField>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretRevealField {
    pub field_key: String,
    pub value: String,
}

/// A role × folder category grant (`secret_category_grants`). Grants are
/// DATA flowing through the central policy module: the matrix permission
/// (`secrets:read`/`secrets:edit`) is checked FIRST, then the category
/// gate — restricted folders deny unless a grant row for one of the
/// caller's roles allows it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretCategoryGrant {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub role_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub folder_id: Uuid,
    pub can_read: bool,
    pub can_edit: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub updated_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
