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
/// A migration connector push session identifier (TASKLIST D3).
pub type MigrationSessionId = Uuid;
pub type ApiKeyId = Uuid;
pub type PortalUserId = Uuid;
pub type DocumentLayoutId = Uuid;
pub type DocumentLinkId = Uuid;
/// ADR 0003 configuration identity: the feeder's per-source record link.
pub type ConfigSourceLinkId = Uuid;
/// A network interface (MAC/IPs/VLANs) attribute of a config item.
pub type CiInterfaceId = Uuid;
/// A reviewable ambiguous-match candidate in the CI merge queue.
pub type CiMergeCandidateId = Uuid;
/// An audited, undoable CI merge (the split substrate).
pub type CiMergeId = Uuid;
/// A per-type/per-source field precedence override.
pub type CiFieldPrecedenceId = Uuid;
pub type PasswordId = Uuid;
pub type PasswordHistoryId = Uuid;
pub type ContractId = Uuid;
pub type AppointmentId = Uuid;
/// A calendar-sync connection (Wave 14 lane 2).
pub type CalendarSyncConnectionId = Uuid;
/// One mirrored external calendar event.
pub type CalendarSyncEventId = Uuid;
pub type CalendarId = Uuid;
pub type ProjectId = Uuid;
pub type PhaseId = Uuid;
pub type TaskId = Uuid;
pub type InvoiceId = Uuid;
pub type KbArticleId = Uuid;
/// Knowledge-base category identifier (native KB v1).
pub type KbCategoryId = Uuid;
/// One oEmbed allowlist registry row (KB rich content, 0069).
pub type KbEmbedProviderId = Uuid;
/// One embed instance on a KB article.
pub type KbEmbedId = Uuid;
/// One linked external document (SharePoint/OneDrive/Drive class).
pub type KbExternalDocumentId = Uuid;
/// A document's living handle (DocumentVersion entities, TASKLIST D4).
pub type DocumentId = Uuid;
/// One immutable snapshot in a document's append-only history.
pub type DocumentVersionId = Uuid;
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
