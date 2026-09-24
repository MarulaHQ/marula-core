//! Vendor-neutral sync model: the ONLY shapes that cross between a
//! [`crate::DmsSource`] and a [`crate::DmsSink`]. Vendor detail stays in
//! the source parsers and push renderers; nothing vendor-specific leaks
//! into engine or sink code.
//!
//! Design notes:
//!
//! * Field kinds mirror the docs-platform layout schema vocabulary
//!   (`text | number | date | bool | select | url | email`) — external
//!   types map onto `document_layouts` with the SAME field-schema shape,
//!   so app-side validation is inherited unchanged.
//! * Every record carries its [`Identity`] (deterministic keys per ADR
//!   0003) and the external id it came from; the engine keys its ledger
//!   and reports off these, never off names.
//! * [`Credential`] holds the ONLY plaintext crossing this crate, and
//!   only when a source exposes it. Its `Debug` impl redacts the secret
//!   so a log line can never capture a vault-boundary violation; the
//!   type is not `Serialize` so it cannot ride a report or state dump.

use std::collections::BTreeMap;

use crate::identity::Identity;

/// One field in a sync layout schema — the neutral mirror of the
/// docs-platform `{key, label, type, required, position, options?}`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SyncField {
    pub key: String,
    pub label: String,
    /// One of [`FieldKind`].
    pub kind: FieldKind,
    pub required: bool,
    pub position: i32,
    /// Options for [`FieldKind::Select`].
    pub options: Vec<String>,
}

impl SyncField {
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        kind: FieldKind,
        position: i32,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            kind,
            required: false,
            position,
            options: Vec::new(),
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }
}

/// Layout field types the docs-platform schema accepts (migration 0053).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum FieldKind {
    Text,
    Number,
    Date,
    Bool,
    Select,
    Url,
    Email,
}

impl FieldKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FieldKind::Text => "text",
            FieldKind::Number => "number",
            FieldKind::Date => "date",
            FieldKind::Bool => "bool",
            FieldKind::Select => "select",
            FieldKind::Url => "url",
            FieldKind::Email => "email",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "text" => FieldKind::Text,
            "number" => FieldKind::Number,
            "date" => FieldKind::Date,
            "bool" => FieldKind::Bool,
            "select" => FieldKind::Select,
            "url" => FieldKind::Url,
            "email" => FieldKind::Email,
            _ => return None,
        })
    }
}

/// Which side of a [`LinkEndpoint`] a record sits on — the
/// `document_links` kind vocabulary (migration 0053 + the 0054 asset
/// kind). External relationships map onto these; anything an external
/// system links that has no Meridian kind is reported, not invented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkKind {
    Document,
    Asset,
    Password,
    Client,
    Ticket,
    KbArticle,
}

impl LinkKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            LinkKind::Document => "document",
            LinkKind::Asset => "asset",
            LinkKind::Password => "password",
            LinkKind::Client => "client",
            LinkKind::Ticket => "ticket",
            LinkKind::KbArticle => "kb_article",
        }
    }
}

/// One end of a relationship: the external record's id (the source's
/// own id space) plus the Meridian kind it maps to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkEndpoint {
    /// The external record id this endpoint resolves to (source id space).
    pub external_id: String,
    pub kind: LinkKind,
}

/// A cross-entity relationship from a source snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    pub from: LinkEndpoint,
    pub to: LinkEndpoint,
}

/// An external credential/password. PLAINTEXT INSIDE — see the crate
/// doc's vault boundary: transit only between an explicitly-constructed
/// source and the sink's audited vault path; never logged or retained.
#[derive(Clone)]
pub struct Credential {
    pub external_id: String,
    /// External organization this credential belongs to (client ref).
    pub client_ref: Option<String>,
    pub title: String,
    pub username: String,
    pub url: String,
    /// The secret itself; carried only to the sink's vault path.
    pub secret: String,
    pub notes: Option<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("external_id", &self.external_id)
            .field("client_ref", &self.client_ref)
            .field("title", &self.title)
            .field("username", &self.username)
            .field("url", &self.url)
            .field("secret", &"[REDACTED]")
            .field("notes", &self.notes.as_ref().map(|_| "[REDACTED]"))
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// A layout/type definition from the source side (external type → will
/// map onto a `document_layouts` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncLayout {
    /// The external type's own id in its source system.
    pub external_id: String,
    /// Human type name ("Server", "Password Asset").
    pub name: String,
    pub description: String,
    /// Suggested Meridian icon (best-effort; cosmetic only).
    pub icon: String,
    pub fields: Vec<SyncField>,
}

/// One synced record — asset or document. Kept as one struct because the
/// docs-platform models both as `layout_id` + schema-validated `fields`
/// (documents have a versioned `body`; assets have tag/kind/status).
#[derive(Debug, Clone)]
pub struct SyncRecord {
    pub external_id: String,
    pub kind: LinkKind,
    /// Records a source maps as typed assets (configurations, devices).
    pub client_ref: Option<String>,
    pub title: String,
    /// Free-text type name as the source calls it (layout mapping input).
    pub type_name: Option<String>,
    /// Extra identity values the source carries (serial, hostname...).
    pub fields: BTreeMap<String, String>,
    /// Document body (documents only; assets ignore it).
    pub body: String,
    /// Human asset tag (assets only; falls back to `title`).
    pub tag: Option<String>,
    pub credentials: Vec<Credential>,
}

/// Equality ignores credentials (they never participate in identity or
/// round-trip comparisons — the vault boundary keeps them out of
/// content semantics).
impl PartialEq for SyncRecord {
    fn eq(&self, other: &Self) -> bool {
        self.external_id == other.external_id
            && self.kind == other.kind
            && self.client_ref == other.client_ref
            && self.title == other.title
            && self.type_name == other.type_name
            && self.fields == other.fields
            && self.body == other.body
            && self.tag == other.tag
    }
}

impl SyncRecord {
    /// Deterministic identity per ADR 0003: source GUID first, then
    /// serial, then hostname+domain — derived here from the record's
    /// own fields by [`crate::identity`].
    pub fn identity(&self, source_name: &str) -> Identity {
        crate::identity::derive(source_name, &self.external_id, &self.fields)
    }
}

/// One fully-parsed source state: everything one sync pass consumes.
#[derive(Debug, Clone, Default)]
pub struct DmsSnapshot {
    /// Source system identifier ("it-glue", "hudu", "generic:acme").
    pub source: String,
    pub layouts: Vec<SyncLayout>,
    pub records: Vec<SyncRecord>,
    pub relations: Vec<Relation>,
    /// External client/organization scope: id → display name (the
    /// engine resolves record `client_ref`s through this map, then
    /// operator aliases, then Meridian client name matching).
    pub client_refs: BTreeMap<String, String>,
    /// Credentials the source exposed. EMPTY unless the source was
    /// explicitly constructed to surface them; the engine gates the
    /// sink handoff separately (`SyncConfig::map_credentials`).
    pub credentials: Vec<Credential>,
}

impl DmsSnapshot {
    /// Records grouped by their source type name, in first-seen order.
    pub fn records_by_type(&self) -> BTreeMap<String, Vec<&SyncRecord>> {
        let mut grouped: BTreeMap<String, Vec<&SyncRecord>> = BTreeMap::new();
        for record in &self.records {
            grouped
                .entry(record.type_name.clone().unwrap_or_default())
                .or_default()
                .push(record);
        }
        grouped
    }
}

/// Typed errors surfaced across the sync model seam.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SyncError {
    #[error("source parse failure: {0}")]
    Parse(String),
    #[error("sink rejected the write: {0}")]
    Sink(String),
}
