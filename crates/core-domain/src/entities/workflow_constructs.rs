//! Tags, ticket links, cost lines, expenses, and PM schedules — the
//! workflow-construct entities (Wave 11 lane 9, migration 0056).

use crate::ids::*;
use crate::status::Priority;
use chrono::{DateTime, NaiveTime, Utc};
use uuid::Uuid;

/// An org-wide tag: a flat named label with an optional display color.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Tag {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub name: String,
    pub color: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A typed edge between two tickets.
///
/// - `ParentChild`: `from_ticket_id` is the CHILD, `to_ticket_id` the
///   parent — child work rolls up to the parent.
/// - `MajorIncident`: `from_ticket_id` is a MEMBER, `to_ticket_id` the
///   major incident it folded into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TicketLinkType {
    ParentChild,
    MajorIncident,
}

impl TicketLinkType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ParentChild => "parent_child",
            Self::MajorIncident => "major_incident",
        }
    }

    /// Strict slug parser (the stored column's inverse); anything else
    /// fails closed.
    pub fn parse(slug: &str) -> Option<Self> {
        match slug {
            "parent_child" => Some(Self::ParentChild),
            "major_incident" => Some(Self::MajorIncident),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketLink {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub from_ticket_id: TicketId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub to_ticket_id: TicketId,
    pub link_type: TicketLinkType,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
}

/// Rollup over the children/members pointing at one ticket, plus the
/// ticket's own billable money lines.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketRollup {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    pub child_count: i64,
    /// Open (not closed, not merged) children still carrying work.
    pub child_open_count: i64,
    pub major_incident_member_count: i64,
    /// Summed minutes of folded-in time (own + children/members).
    pub time_minutes: i64,
    /// Summed minor-unit total of billable cost lines + expenses.
    pub billable_cost_minor: i64,
}

/// One money line on a ticket: `parts`, `flat`, or `other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CostLineKind {
    Parts,
    Flat,
    Other,
}

impl CostLineKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Parts => "parts",
            Self::Flat => "flat",
            Self::Other => "other",
        }
    }

    pub fn parse(slug: &str) -> Option<Self> {
        match slug {
            "parts" => Some(Self::Parts),
            "flat" => Some(Self::Flat),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketCostLine {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    pub kind: CostLineKind,
    pub description: String,
    pub quantity: f64,
    pub unit_cost_minor: i64,
    pub currency: String,
    pub billable: bool,
    /// Set once this line became an invoice line ("billed once, ever").
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub invoice_line_id: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TicketCostLine {
    pub fn total_minor(&self) -> i64 {
        (self.quantity * self.unit_cost_minor as f64).round() as i64
    }
}

/// One expense entry on a ticket.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketExpense {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    pub description: String,
    pub amount_minor: i64,
    pub currency: String,
    pub billable: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub invoice_line_id: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub incurred_by: Option<UserId>,
    pub incurred_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A recurring/scheduled ticket (PM schedule): a ticket template plus the
/// typed cadence (same shape as report schedules). The worker claims due
/// rows; core-api generates the instance and advances the cadence.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PmSchedule {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub name: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    pub title: String,
    pub body: Option<String>,
    pub priority: Priority,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub client_id: Option<ClientId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub site_id: Option<SiteId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub assignee_id: Option<UserId>,
    pub cadence: crate::cadence::Cadence,
    pub time_of_day: NaiveTime,
    /// 0 = Monday .. 6 = Sunday; weekly only.
    pub day_of_week: Option<i16>,
    /// 1–28; monthly only.
    pub day_of_month: Option<i16>,
    pub timezone: String,
    pub enabled: bool,
    pub next_run_at: DateTime<Utc>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub created_by: UserId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One append-only PM run ledger row.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PmRun {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub schedule_id: Uuid,
    pub slot_at: DateTime<Utc>,
    /// "generated" | "skipped"
    pub status: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<TicketId>,
    pub detail: serde_json::Value,
    pub created_at: DateTime<Utc>,
}
