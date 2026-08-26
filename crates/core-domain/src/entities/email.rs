//! Email message entity: the ledger row for one RFC 5322 message in either
//! direction, with its threading keys and scan outcome (Stage 2).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Whether a message entered (inbound) or left (outbound) the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum EmailDirection {
    Inbound,
    Outbound,
}

impl EmailDirection {
    pub const fn as_str(self) -> &'static str {
        match self {
            EmailDirection::Inbound => "inbound",
            EmailDirection::Outbound => "outbound",
        }
    }
}

impl std::str::FromStr for EmailDirection {
    type Err = crate::DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "inbound" => Ok(EmailDirection::Inbound),
            "outbound" => Ok(EmailDirection::Outbound),
            other => Err(crate::DomainError::Validation(format!(
                "invalid email direction: {other}"
            ))),
        }
    }
}

/// Outcome of the malware scan that runs between webhook receipt and
/// persistence. `skipped` is the documented "none" mode (no scanner
/// configured) — never a silent pass of an actual verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ScanStatus {
    Clean,
    Infected,
    Skipped,
}

impl ScanStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            ScanStatus::Clean => "clean",
            ScanStatus::Infected => "infected",
            ScanStatus::Skipped => "skipped",
        }
    }
}

impl std::str::FromStr for ScanStatus {
    type Err = crate::DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "clean" => Ok(ScanStatus::Clean),
            "infected" => Ok(ScanStatus::Infected),
            "skipped" => Ok(ScanStatus::Skipped),
            other => Err(crate::DomainError::Validation(format!(
                "invalid scan status: {other}"
            ))),
        }
    }
}

/// One persisted email message. `rfc_message_id` is unique per org and is
/// the idempotency key for inbound processing; `email_refs` carries the
/// RFC 5322 References chain (`references` itself is reserved SQL).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EmailMessage {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: uuid::Uuid,
    pub direction: EmailDirection,
    pub rfc_message_id: String,
    pub in_reply_to: Option<String>,
    pub email_refs: Vec<String>,
    pub subject: Option<String>,
    pub from_address: String,
    pub from_name: Option<String>,
    pub to_addresses: Vec<String>,
    pub body_text: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub comment_id: Option<uuid::Uuid>,
    pub scan_status: ScanStatus,
    /// Which transport adapter handled the message (postmark/smtp/…).
    pub transport: String,
    pub raw_ref: Option<String>,
    pub created_at: DateTime<Utc>,
}
