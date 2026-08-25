//! Semantic status categories and ticket priority — the binding behavior layer.

use std::fmt;

/// The four semantic categories every ticket status maps to (§5).
/// Customization (labels, ordering, per-board sets) can never add, remove, or
/// re-bind these: SLA timers, pause logic, automations, billing, and reports
/// bind to the category, not the label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TicketStatusCategory {
    Open,
    InProgress,
    Waiting,
    Closed,
}

impl TicketStatusCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InProgress => "in_progress",
            Self::Waiting => "waiting",
            Self::Closed => "closed",
        }
    }

    /// True when tickets in this category count against the technician's
    /// active workload (SLA clocks may run, timer may attach).
    pub const fn is_active_work(self) -> bool {
        matches!(self, Self::Open | Self::InProgress)
    }

    /// True when an SLA clock is paused for tickets in this category.
    /// Waiting pauses first-response/next-step clocks per PSA convention.
    pub const fn pauses_sla(self) -> bool {
        matches!(self, Self::Waiting)
    }

    /// True when the ticket is resolved from a billing perspective.
    pub const fn is_billable_final(self) -> bool {
        matches!(self, Self::Closed)
    }

    /// True when closing a ticket in this state requires a pre-close
    /// checklist to be satisfied.
    pub const fn requires_checklist(self) -> bool {
        matches!(self, Self::Closed)
    }
}

impl fmt::Display for TicketStatusCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for TicketStatusCategory {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "open" => Ok(Self::Open),
            "in_progress" => Ok(Self::InProgress),
            "waiting" => Ok(Self::Waiting),
            "closed" => Ok(Self::Closed),
            other => Err(DomainError::InvalidStatusCategory(other.to_string())),
        }
    }
}

use crate::error::DomainError;

/// Industry-standard four-tier priority.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    P1,
    P2,
    P3,
    P4,
}

impl Priority {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::P1 => "p1",
            Self::P2 => "p2",
            Self::P3 => "p3",
            Self::P4 => "p4",
        }
    }
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Priority {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "p1" => Ok(Self::P1),
            "p2" => Ok(Self::P2),
            "p3" => Ok(Self::P3),
            "p4" => Ok(Self::P4),
            other => Err(DomainError::InvalidPriority(other.to_string())),
        }
    }
}

/// Taxonomy of structured ticket notes (§1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    Internal,
    ClientVisible,
    InHouseUpdate,
}

impl NoteKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Internal => "internal",
            Self::ClientVisible => "client_visible",
            Self::InHouseUpdate => "in_house_update",
        }
    }

    /// Whether portal users may read a note of this kind.
    pub const fn portal_readable(self) -> bool {
        matches!(self, Self::ClientVisible)
    }
}

impl std::str::FromStr for NoteKind {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "internal" => Ok(Self::Internal),
            "client_visible" => Ok(Self::ClientVisible),
            "in_house_update" => Ok(Self::InHouseUpdate),
            other => Err(DomainError::InvalidNoteKind(other.to_string())),
        }
    }
}

impl fmt::Display for NoteKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sla_pause_binds_to_category_not_label() {
        // A custom "Pending Vendor" status bound to Waiting must pause SLA
        // exactly as the default Waiting status does.
        assert!(TicketStatusCategory::Waiting.pauses_sla());
        assert!(!TicketStatusCategory::InProgress.pauses_sla());
        assert!(!TicketStatusCategory::Open.pauses_sla());
    }

    #[test]
    fn closed_requires_checklist() {
        assert!(TicketStatusCategory::Closed.requires_checklist());
        assert!(!TicketStatusCategory::Waiting.requires_checklist());
    }

    #[test]
    fn priority_ordering_is_severity_order() {
        assert!(Priority::P1 < Priority::P2);
        assert!(Priority::P4 > Priority::P3);
    }
}
