//! Attachment entity: one stored file bound to exactly one owner target —
//! a ticket, a ticket comment, an inbound email message, or the sign-off row
//! whose drawn mark it carries (TASKLIST 2.14; Wave 43 track 2 lane 12).
//!
//! Purpose: the serde/utoipa shape shared by DB mapping and the API plus
//! the [`AttachmentKind`] taxonomy. Bytes never live here: rows point at
//! content-addressed object-store keys (`core_store::attachment_key`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What an attachment belongs to. The enum value is persisted verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    /// Attached to a specific ticket comment.
    TicketComment,
    /// Extracted from one inbound email message.
    EmailMessage,
    /// Attached to the ticket itself (no comment intermediary yet).
    Ticket,
    /// The PNG payload of one sign-off (`sign_offs.attachment_id`). The
    /// attachment is the payload; the sign-off row is the fact.
    SignOff,
}

impl AttachmentKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            AttachmentKind::TicketComment => "ticket_comment",
            AttachmentKind::EmailMessage => "email_message",
            AttachmentKind::Ticket => "ticket",
            AttachmentKind::SignOff => "sign_off",
        }
    }
}

impl std::str::FromStr for AttachmentKind {
    type Err = crate::DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ticket_comment" => Ok(AttachmentKind::TicketComment),
            "email_message" => Ok(AttachmentKind::EmailMessage),
            "ticket" => Ok(AttachmentKind::Ticket),
            "sign_off" => Ok(AttachmentKind::SignOff),
            other => Err(crate::DomainError::Validation(format!(
                "invalid attachment kind: {other}"
            ))),
        }
    }
}

/// One uploaded file's metadata row; the payload lives behind `storage_key`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Attachment {
    pub id: Uuid,
    pub org_id: Uuid,
    pub kind: AttachmentKind,
    /// Set when kind is ticket_comment or ticket.
    pub ticket_id: Option<Uuid>,
    /// Set only when kind is ticket_comment.
    pub ticket_comment_id: Option<Uuid>,
    /// Set only when kind is email_message.
    pub email_message_id: Option<Uuid>,
    /// Original filename, sanitized (no path components/control chars).
    pub filename: String,
    /// Declared + magic-byte-verified content type.
    pub content_type: String,
    pub size_bytes: i64,
    /// Lowercase hex sha256 of the exact bytes; dedup + integrity anchor.
    pub sha256: String,
    /// Object-store key (`org/<id>/attachments/<sha256>`).
    pub storage_key: String,
    /// Staff uploader when uploaded through the API.
    pub uploaded_by_user_id: Option<Uuid>,
    /// Portal actor when uploaded through a portal surface (v1: none —
    /// reserved so portal uploads need no schema change).
    pub uploaded_by_portal_user_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// One stored object's scan verdict (Wave 52 lane P6a, migration 0139).
///
/// Keyed by the object, not the upload: `attachments` rows are per-owner and
/// several may share one content-addressed object, while the verdict belongs
/// to the bytes and is reached by `(org_id, sha256)`. An infected object has
/// no `Attachment` row at all — `status` says where the bytes live, and the
/// release fields carry the admin override trail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentScan {
    pub org_id: Uuid,
    /// Lowercase hex sha256 — the object's identity within the org.
    pub sha256: String,
    /// `clean` (a scanner passed it), `skipped` (none mode: nobody looked),
    /// or `infected` (signature found; the bytes are quarantined).
    pub status: super::email::ScanStatus,
    /// Signature name for an infection, or the none-mode note for a skip.
    pub detail: Option<String>,
    /// Adapter that produced the verdict (`clamd`, `none`).
    pub scanner: String,
    pub size_bytes: i64,
    pub content_type: String,
    pub filename: String,
    /// Admission key for clean/skipped objects; quarantine key when infected.
    pub storage_key: String,
    pub ticket_id: Option<Uuid>,
    pub uploaded_by_user_id: Option<Uuid>,
    pub uploaded_by_portal_user_id: Option<Uuid>,
    pub scanned_at: DateTime<Utc>,
    /// Set exactly when `status` is `infected`.
    pub quarantined_at: Option<DateTime<Utc>>,
    /// Release override, when an admin has admitted the object anyway.
    pub released_at: Option<DateTime<Utc>>,
    pub released_by_user_id: Option<Uuid>,
    pub release_reason: Option<String>,
}

impl AttachmentScan {
    /// True when the bytes were rejected and are held for an operator.
    #[must_use]
    pub fn is_quarantined(&self) -> bool {
        self.status == super::email::ScanStatus::Infected
    }
}
