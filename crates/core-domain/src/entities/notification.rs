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
    /// Low-stock threshold transition (Wave 32 lane 1): a per-location
    /// level dropped to/below its reorder point. Written in the SAME
    /// commit as the movement ledger row; the notified flag prevents
    /// repeats while the level stays low. The event stream carries the
    /// wider fan-out signal (`stock.level_low`).
    StockLow,
    /// The Jev intelligent-dedupe workflow's run report (Wave 22 lane 2):
    /// manual or scheduled passes over the merge queues report their
    /// outcome (suggestions above threshold, auto-merges applied) to the
    /// designated users.
    DedupeReport,
    /// Inbound malware quarantine (Wave 36 lane 3): the stage-2 scan seam
    /// quarantined an inbound message — either a signature hit or the
    /// fail-closed path when the scanner was unreachable. Org owners and
    /// admins receive it; the message never becomes a ticket.
    ScanDetection,
}

impl NotificationKind {
    /// Every kind, stable order — drives preference screens and defaults.
    pub const ALL: [NotificationKind; 13] = [
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
        NotificationKind::StockLow,
        NotificationKind::DedupeReport,
        NotificationKind::ScanDetection,
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
            NotificationKind::StockLow => "stock_low",
            NotificationKind::ScanDetection => "scan_detection",
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
            "stock_low" => Ok(NotificationKind::StockLow),
            "scan_detection" => Ok(NotificationKind::ScanDetection),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// One stable index per variant. The match is exhaustive on purpose: a new
    /// `NotificationKind` fails to compile here until it gets an index, which
    /// is the reminder that `ALL` has to grow with it. `ALL` is a fixed-length
    /// array, so its length cannot drift without an edit either.
    const fn variant_index(kind: NotificationKind) -> u8 {
        match kind {
            NotificationKind::Mention => 0,
            NotificationKind::Assignment => 1,
            NotificationKind::SlaRisk => 2,
            NotificationKind::SlaBreach => 3,
            NotificationKind::Escalation => 4,
            NotificationKind::MissingTime => 5,
            NotificationKind::TicketUpdate => 6,
            NotificationKind::ReportDelivery => 7,
            NotificationKind::AiCapWarn => 8,
            NotificationKind::SecretRotation => 9,
            NotificationKind::StockLow => 10,
            NotificationKind::DedupeReport => 11,
            NotificationKind::ScanDetection => 12,
        }
    }

    /// Bumped by hand when a variant or a match arm is added. The equality
    /// checks below fail loudly if it is not.
    const VARIANT_COUNT: u8 = 13;

    #[test]
    fn all_lists_every_kind_exactly_once() {
        let indices: BTreeSet<u8> = NotificationKind::ALL
            .iter()
            .copied()
            .map(variant_index)
            .collect();
        assert_eq!(
            NotificationKind::ALL.len(),
            VARIANT_COUNT as usize,
            "ALL length disagrees with the variant count"
        );
        assert_eq!(
            indices.len(),
            VARIANT_COUNT as usize,
            "ALL repeats a kind or omits one"
        );
        assert_eq!(
            indices,
            (0..VARIANT_COUNT).collect(),
            "ALL does not cover every variant index"
        );
    }

    #[test]
    fn stock_low_is_offerable_and_every_kind_round_trips_its_string() {
        // The preference screens iterate `ALL`; an omission is invisible to
        // them even though the notifier writes the kind and the migration
        // accepts it.
        assert!(
            NotificationKind::ALL.contains(&NotificationKind::StockLow),
            "stock_low must be in ALL so preferences can turn it off"
        );
        for kind in NotificationKind::ALL {
            assert_eq!(kind.as_str().parse::<NotificationKind>().unwrap(), kind);
        }
    }
}
