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

/// Whether an inbound message's sender was already a participant on the
/// ticket the message names (Wave 55, decision 4).
///
/// The HMAC on the webhook proves the message came through the mail
/// provider. It says nothing about who wrote it, so a reply may only become
/// part of a ticket when its `From` belongs to that ticket already.
///
/// `NotRequired` is the new-ticket path (any sender may open their own
/// ticket, as designed) and the quarantined path: there is no existing
/// ticket whose confidentiality a sender check would protect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SenderBindingState {
    /// No existing ticket was in play, so nothing had to be established.
    NotRequired,
    /// The sender is a participant on the target ticket; the message
    /// threads exactly as it did before this rule existed.
    Bound,
    /// The sender is not a participant. The message is recorded with its
    /// reason and reaches no ticket until a decision is applied.
    Held,
    /// A human, or the engine under `autonomous`, decided the message
    /// joins the ticket.
    Accepted,
    /// A human, or the engine under `autonomous`, decided it does not.
    Refused,
}

impl SenderBindingState {
    pub const fn as_str(self) -> &'static str {
        match self {
            SenderBindingState::NotRequired => "not_required",
            SenderBindingState::Bound => "bound",
            SenderBindingState::Held => "held",
            SenderBindingState::Accepted => "accepted",
            SenderBindingState::Refused => "refused",
        }
    }
}

impl std::str::FromStr for SenderBindingState {
    type Err = crate::DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "not_required" => Ok(SenderBindingState::NotRequired),
            "bound" => Ok(SenderBindingState::Bound),
            "held" => Ok(SenderBindingState::Held),
            "accepted" => Ok(SenderBindingState::Accepted),
            "refused" => Ok(SenderBindingState::Refused),
            other => Err(crate::DomainError::Validation(format!(
                "invalid sender binding state: {other}"
            ))),
        }
    }
}

/// Who decided that an unbound sender's message may or may not join a
/// ticket. The distinction is the audit trail: an engine decision is
/// stamped as one, never dressed as a person's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SenderBindingDecisionSource {
    Human,
    Engine,
}

impl SenderBindingDecisionSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            SenderBindingDecisionSource::Human => "human",
            SenderBindingDecisionSource::Engine => "engine",
        }
    }
}

/// How much authority the org has granted the engine over an unbound
/// sender's message (Wave 55, decision 4). Per org, defaulting to
/// `HumanOnly`; the other two are deliberate admin acts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SenderBindingLevel {
    /// A non-participant message waits for a person. Nothing reaches the
    /// ticket until someone decides.
    #[default]
    HumanOnly,
    /// The engine's proposal is recorded beside the held message and a
    /// person still decides. Shadow exists so agreement is measurable
    /// before authority is granted.
    HumanWithSuggestions,
    /// The engine's decision is applied without a person, for orgs that
    /// choose it.
    Autonomous,
}

impl SenderBindingLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            SenderBindingLevel::HumanOnly => "human_only",
            SenderBindingLevel::HumanWithSuggestions => "shadow",
            SenderBindingLevel::Autonomous => "autonomous",
        }
    }

    /// Resolves the stored setting value. Anything unrecognised falls back
    /// to the default rather than granting authority: a value this code
    /// cannot read is not an instruction to apply the engine.
    pub fn from_setting(value: Option<&str>) -> Self {
        match value {
            Some("shadow") | Some("human_with_suggestions") => Self::HumanWithSuggestions,
            Some("autonomous") => Self::Autonomous,
            _ => Self::HumanOnly,
        }
    }

    /// True when the engine's proposal is recorded (shadow and autonomous).
    pub const fn engine_engaged(self) -> bool {
        matches!(self, Self::HumanWithSuggestions | Self::Autonomous)
    }

    /// True when the engine's decision is applied without a person.
    pub const fn applies_without_a_person(self) -> bool {
        matches!(self, Self::Autonomous)
    }
}
