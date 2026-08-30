//! Typed identifiers. All IDs are UUIDv7 (time-ordered, index-friendly).

use uuid::Uuid;

pub type OrgId = Uuid;
pub type UserId = Uuid;
pub type RoleId = Uuid;
pub type ClientId = Uuid;
pub type SiteId = Uuid;
pub type ContactId = Uuid;
pub type DeviceAssetId = Uuid;
pub type BoardId = Uuid;
pub type TicketId = Uuid;
pub type TicketStatusId = Uuid;
pub type TicketCommentId = Uuid;
pub type CustomFieldId = Uuid;
pub type SlaPolicyId = Uuid;
pub type SlaCalendarRowId = Uuid;
pub type EscalationStepId = Uuid;
pub type AutomationRuleId = Uuid;
pub type JobId = Uuid;
pub type TimeEntryId = Uuid;
pub type TimerSessionId = Uuid;
pub type NotificationId = Uuid;
pub type AuditEventId = Uuid;
pub type OutboxEventId = Uuid;
/// An import batch run identifier (import framework, TASKLIST D2).
pub type ImportBatchId = Uuid;
pub type ApiKeyId = Uuid;
pub type PortalUserId = Uuid;
pub type ContractId = Uuid;
pub type AppointmentId = Uuid;
pub type CalendarId = Uuid;
pub type ProjectId = Uuid;
pub type PhaseId = Uuid;
pub type TaskId = Uuid;
pub type InvoiceId = Uuid;
pub type KbArticleId = Uuid;
/// Knowledge-base category identifier (native KB v1).
pub type KbCategoryId = Uuid;
pub type SessionId = Uuid;
pub type ChecklistItemId = Uuid;

/// Human-facing ticket number, e.g. `MER-2841`. Globally unique per org,
/// assigned from a per-org sequence.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketNumber(pub String);

impl std::fmt::Display for TicketNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Extracts the numeric part of a ticket number (`MER-2841` → `2841`).
pub fn ticket_number_value(n: &TicketNumber) -> Option<u64> {
    n.0.rsplit('-').next()?.parse().ok()
}
