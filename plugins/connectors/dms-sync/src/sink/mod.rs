//! The Meridian-side apply seam ([`DmsSink`]) plus the documented
//! [`NoneDmsSink`].
//!
//! The sink owns the WIRE mechanics (REST calls, auth, envelopes); the
//! engine owns SEMANTICS (identity resolution, conflicts, reports). The
//! sink therefore deals only in concrete Meridian-side facts:
//! layouts (`document_layouts`), clients, assets (`device_assets`),
//! documents, links (`document_links`), and the vault path.
//!
//! Shipped implementations:
//!
//! * [`meridian::MarulaRestSink`] — the public v1 REST API over a
//!   scoped machine API key (x-api-key); instant-first idempotency keys
//!   derived from identity keys make retried creates free replays.
//! * [`NoneDmsSink`] — zero I/O, every call returns
//!   [`SinkError::Disabled`]. THE shipped default: an org with no
//!   connection configured operates standalone, and "would have synced"
//!   is the honest answer.

use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

use crate::model::LinkKind;

/// Typed errors surfaced by a sink.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SinkError {
    /// None mode: no credentials configured; zero network performed.
    #[error("dms sink disabled: no Meridian connection configured (none mode)")]
    Disabled,
    /// The caller omitted something the sink requires.
    #[error("invalid sink request: {0}")]
    InvalidRequest(String),
    /// The Meridian API rejected the credential (401/403) — the
    /// operator rotates the scoped key.
    #[error("sink unauthorized: {0}")]
    Unauthorized(String),
    #[error("sink rate limited: {0}")]
    RateLimited(String),
    /// Target record vanished mid-sync (record deleted Meridian-side).
    #[error("sink record not found: {0}")]
    NotFound(String),
    #[error("sink conflict: {0}")]
    Conflict(String),
    #[error("sink transport failure: {0}")]
    Network(String),
}

/// A layout as the sink sees it: the `document_layouts` mirror.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutRecord {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    /// System layouts are read-only for the connector — matched, never
    /// mutated (binding per the lane brief).
    pub is_system: bool,
    /// Schema keys + kinds + required flags (schema-validated writes).
    pub fields: Vec<crate::model::SyncField>,
}

/// A new org layout the connector wants created (is_system=false).
#[derive(Debug, Clone)]
pub struct NewLayout {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub fields: Vec<crate::model::SyncField>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClientRecord {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssetRecord {
    pub id: Uuid,
    pub client_id: Uuid,
    pub tag: String,
    pub name: String,
    pub status: String,
    pub layout_id: Option<Uuid>,
    pub fields: Value,
}

#[derive(Debug, Clone)]
pub struct AssetCreate {
    pub client_id: Uuid,
    pub tag: String,
    pub name: String,
    pub layout_id: Option<Uuid>,
    pub fields: Value,
    /// Deterministic idempotency key (`dms-{source}-asset-{identity}`):
    /// a retried create replays the stored result — never a twin.
    pub idempotency_key: String,
    /// Provenance change summary (audit trail rides the envelope).
    pub change_note: String,
}

#[derive(Debug, Clone, Default)]
pub struct AssetUpdate {
    pub name: Option<String>,
    pub status: Option<String>,
    pub layout_id: Option<Uuid>,
    pub fields: Option<Value>,
    pub change_note: Option<String>,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DocumentRecord {
    pub id: Uuid,
    pub client_id: Option<Uuid>,
    pub title: String,
    pub layout_id: Option<Uuid>,
    pub fields: Value,
    /// The current version's body.
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct DocumentCreate {
    pub client_id: Option<Uuid>,
    pub title: String,
    pub layout_id: Option<Uuid>,
    pub fields: Value,
    pub body: String,
    pub idempotency_key: String,
    pub change_note: String,
}

#[derive(Debug, Clone, Default)]
pub struct DocumentUpdate {
    pub title: Option<String>,
    pub layout_id: Option<Uuid>,
    pub fields: Option<Value>,
    pub body: Option<String>,
    pub change_note: Option<String>,
    pub idempotency_key: String,
}

/// One link plan: from a Meridian record (already resolved) to another
/// resolved record. Unresolved ends never reach the sink — they are
/// reported by the engine instead (broken targets surfaced, never
/// dropped and never invented).
#[derive(Debug, Clone)]
pub struct LinkCreate {
    pub from_kind: LinkKind,
    pub from_id: Uuid,
    pub to_kind: LinkKind,
    pub to_id: Uuid,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinkRecord {
    pub id: Uuid,
    pub from_kind: LinkKind,
    pub from_id: Uuid,
    pub to_kind: LinkKind,
    pub to_id: Uuid,
}

/// A vault-path credential push. The secret crosses ONLY here; sink
/// implementations must not log, cache, or serialize it.
#[derive(Debug, Clone)]
pub struct CredentialCreate {
    pub client_id: Option<Uuid>,
    pub title: String,
    pub username: String,
    pub url: String,
    pub secret: String,
    pub notes: Option<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CredentialReceipt {
    pub password_id: Uuid,
}

/// The sink seam. Every method treats `SinkError::Disabled` as
/// "would have synced" (none mode).
#[async_trait]
pub trait DmsSink: Send + Sync {
    fn name(&self) -> &str;

    async fn list_layouts(&self) -> Result<Vec<LayoutRecord>, SinkError>;
    /// Creates an ORG layout (never a system row — the sink must
    /// refuse `is_system` creation by construction).
    async fn create_layout(&self, layout: &NewLayout) -> Result<LayoutRecord, SinkError>;
    async fn list_clients(&self) -> Result<Vec<ClientRecord>, SinkError>;
    async fn list_assets(&self) -> Result<Vec<AssetRecord>, SinkError>;
    async fn create_asset(&self, plan: &AssetCreate) -> Result<AssetRecord, SinkError>;
    async fn update_asset(&self, id: Uuid, plan: &AssetUpdate) -> Result<AssetRecord, SinkError>;
    async fn list_documents(&self) -> Result<Vec<DocumentRecord>, SinkError>;
    /// One document INCLUDING its current body (list responses carry
    /// metadata only; the engine reads bodies lazily when a record's
    /// fingerprint changed, so a sync does not version-churn).
    async fn get_document(&self, id: Uuid) -> Result<DocumentRecord, SinkError>;
    async fn create_document(&self, plan: &DocumentCreate) -> Result<DocumentRecord, SinkError>;
    async fn update_document(
        &self,
        id: Uuid,
        plan: &DocumentUpdate,
    ) -> Result<DocumentRecord, SinkError>;
    /// Existing links FROM one record (idempotent link-ensure input).
    async fn list_links(
        &self,
        from_kind: LinkKind,
        from_id: Uuid,
    ) -> Result<Vec<LinkRecord>, SinkError>;
    async fn create_link(&self, plan: &LinkCreate) -> Result<LinkRecord, SinkError>;
    /// Vault path. Only reached when the engine's
    /// `map_credentials` gate is explicitly on; implementations must
    /// route through the audited password-create path server-side.
    async fn create_password(
        &self,
        plan: &CredentialCreate,
    ) -> Result<CredentialReceipt, SinkError>;
}

/// Built-in none-mode sink: disabled, zero network, zero side effects.
/// The shipped default sink for an org with no Meridian connection.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoneDmsSink;

#[async_trait]
impl DmsSink for NoneDmsSink {
    fn name(&self) -> &str {
        "none"
    }

    async fn list_layouts(&self) -> Result<Vec<LayoutRecord>, SinkError> {
        Err(SinkError::Disabled)
    }

    async fn create_layout(&self, _layout: &NewLayout) -> Result<LayoutRecord, SinkError> {
        Err(SinkError::Disabled)
    }

    async fn list_clients(&self) -> Result<Vec<ClientRecord>, SinkError> {
        Err(SinkError::Disabled)
    }

    async fn list_assets(&self) -> Result<Vec<AssetRecord>, SinkError> {
        Err(SinkError::Disabled)
    }

    async fn create_asset(&self, _plan: &AssetCreate) -> Result<AssetRecord, SinkError> {
        Err(SinkError::Disabled)
    }

    async fn update_asset(&self, _id: Uuid, _plan: &AssetUpdate) -> Result<AssetRecord, SinkError> {
        Err(SinkError::Disabled)
    }

    async fn list_documents(&self) -> Result<Vec<DocumentRecord>, SinkError> {
        Err(SinkError::Disabled)
    }

    async fn get_document(&self, _id: Uuid) -> Result<DocumentRecord, SinkError> {
        Err(SinkError::Disabled)
    }

    async fn create_document(&self, _plan: &DocumentCreate) -> Result<DocumentRecord, SinkError> {
        Err(SinkError::Disabled)
    }

    async fn update_document(
        &self,
        _id: Uuid,
        _plan: &DocumentUpdate,
    ) -> Result<DocumentRecord, SinkError> {
        Err(SinkError::Disabled)
    }

    async fn list_links(
        &self,
        _from_kind: LinkKind,
        _from_id: Uuid,
    ) -> Result<Vec<LinkRecord>, SinkError> {
        Err(SinkError::Disabled)
    }

    async fn create_link(&self, _plan: &LinkCreate) -> Result<LinkRecord, SinkError> {
        Err(SinkError::Disabled)
    }

    async fn create_password(
        &self,
        _plan: &CredentialCreate,
    ) -> Result<CredentialReceipt, SinkError> {
        Err(SinkError::Disabled)
    }
}

pub mod marula;
