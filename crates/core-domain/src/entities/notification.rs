//! Notification center entities: per-user notifications with channel
//! fan-out state, preferences, and quiet hours (Stage 2).

use crate::ids::{NotificationId, OrgId, TicketId, UserId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    Mention,
    Assignment,
    SlaRisk,
    SlaBreach,
    Escalation,
    MissingTime,
    TicketUpdate,
}

impl NotificationKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            NotificationKind::Mention => "mention",
            NotificationKind::Assignment => "assignment",
            NotificationKind::SlaRisk => "sla_risk",
            NotificationKind::SlaBreach => "sla_breach",
            NotificationKind::Escalation => "escalation",
            NotificationKind::MissingTime => "missing_time",
            NotificationKind::TicketUpdate => "ticket_update",
        }
    }
}

impl std::str::FromStr for NotificationKind {
    type Err = crate::DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mention" => Ok(NotificationKind::Mention),
            "assignment" => Ok(NotificationKind::Assignment),
            "sla_risk" => Ok(NotificationKind::SlaRisk),
            "sla_breach" => Ok(NotificationKind::SlaBreach),
            "escalation" => Ok(NotificationKind::Escalation),
            "missing_time" => Ok(NotificationKind::MissingTime),
            "ticket_update" => Ok(NotificationKind::TicketUpdate),
            other => Err(crate::DomainError::Validation(format!(
                "invalid notification kind: {other}"
            ))),
        }
    }
}

impl std::fmt::Display for NotificationKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    InApp,
    Email,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Notification {
    pub id: NotificationId,
    pub org_id: OrgId,
    pub user_id: UserId,
    pub kind: NotificationKind,
    pub ticket_id: Option<TicketId>,
    pub title: String,
    pub body: Option<String>,
    pub read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Per-user, per-channel delivery preferences with quiet hours (UTC offsets
/// resolved against the user's timezone at send time).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NotificationPreference {
    pub user_id: UserId,
    pub kind: NotificationKind,
    pub in_app: bool,
    pub email: bool,
    /// Quiet hours in the user's local time; deliveries outside urgency
    /// classes are deferred to the end of the window.
    pub quiet_from: Option<String>,
    pub quiet_until: Option<String>,
}
