//! Attachment entity: one stored file bound to exactly one owner target —
//! a ticket, a ticket comment, or an inbound email message (TASKLIST 2.14).
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
}

impl AttachmentKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            AttachmentKind::TicketComment => "ticket_comment",
            AttachmentKind::EmailMessage => "email_message",
            AttachmentKind::Ticket => "ticket",
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
