//! Notification center entities: per-user notifications with channel
//! fan-out state, preferences, and quiet hours (Stage 2).

use crate::ids::{NotificationId, OrgId, TicketId, UserId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// The email leg of scheduled report delivery (TASKLIST R4): a
    /// recipient's preference chain — including quiet hours — governs
    /// whether their copy of a scheduled report CSV actually sends.
    ReportDelivery,
    /// AI soft-cap warning (ADR 0008, Wave 18 lane 1): the per-user or
    /// org-wide soft token cap was crossed at the completion edge; the
    /// call proceeded — warn + notify only. Hard caps refuse (typed),
    /// soft caps ride here.
    AiCapWarn,
    /// Secret rotation reminder (Wave 19 lane 3): the worker sweep
    /// claimed a record's `rotate_at` slot (at-most-once per schedule).
    SecretRotation,
    /// The Jev intelligent-dedupe workflow's run report (Wave 22 lane 2):
    /// manual or scheduled passes over the merge queues report their
    /// outcome (suggestions above threshold, auto-merges applied) to the
    /// designated users.
    DedupeReport,
}

impl NotificationKind {
    /// Every kind, stable order — drives preference screens and defaults.
    pub const ALL: [NotificationKind; 11] = [
        NotificationKind::Mention,
        NotificationKind::Assignment,
        NotificationKind::SlaRisk,
        NotificationKind::SlaBreach,
        NotificationKind::Escalation,
        NotificationKind::MissingTime,
        NotificationKind::TicketUpdate,
        NotificationKind::ReportDelivery,
        NotificationKind::AiCapWarn,
        NotificationKind::SecretRotation,
        NotificationKind::DedupeReport,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            NotificationKind::Mention => "mention",
            NotificationKind::Assignment => "assignment",
            NotificationKind::SlaRisk => "sla_risk",
            NotificationKind::SlaBreach => "sla_breach",
            NotificationKind::Escalation => "escalation",
            NotificationKind::MissingTime => "missing_time",
            NotificationKind::TicketUpdate => "ticket_update",
            NotificationKind::ReportDelivery => "report_delivery",
            NotificationKind::AiCapWarn => "ai_cap_warn",
            NotificationKind::SecretRotation => "secret_rotation",
            NotificationKind::DedupeReport => "dedupe_report",
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
            "report_delivery" => Ok(NotificationKind::ReportDelivery),
            "ai_cap_warn" => Ok(NotificationKind::AiCapWarn),
            "secret_rotation" => Ok(NotificationKind::SecretRotation),
            "dedupe_report" => Ok(NotificationKind::DedupeReport),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    InApp,
    Email,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Notification {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: NotificationId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    pub kind: NotificationKind,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<TicketId>,
    pub title: String,
    pub body: Option<String>,
    pub read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Per-user, per-channel delivery preferences with quiet hours (wall-clock
/// in the row's timezone, resolved at fan-out time). Absent rows fall back
/// to the org defaults table, then to built-in defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NotificationPreference {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    pub kind: NotificationKind,
    pub in_app: bool,
    pub email: bool,
    /// Quiet hours in the user's local time; deliveries outside urgency
    /// classes are deferred to the end of the window.
    pub quiet_from: Option<String>,
    pub quiet_until: Option<String>,
    /// IANA timezone the quiet-hours window reads in (`UTC` by default).
    #[serde(default = "default_timezone")]
    pub timezone: String,
}

fn default_timezone() -> String {
    "UTC".to_string()
}
