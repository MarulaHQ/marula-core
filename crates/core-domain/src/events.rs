//! Domain event payloads. Every mutation emits one of these through the
//! transactional outbox (§4.2); the envelope in `core-events` wraps them.
//! Payloads are additive-evolving: never remove or retype a field.

use crate::entities::{ChecklistItem, TicketComment, TimeEntry};
use crate::ids::*;
use crate::status::{NoteKind, Priority, TicketStatusCategory};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Event type strings on the wire (`ticket.created`, `timer.started`, …).
pub mod types {
    pub const ORG_CREATED: &str = "org.created";
    pub const USER_CREATED: &str = "user.created";
    pub const CLIENT_CREATED: &str = "client.created";
    pub const CLIENT_UPDATED: &str = "client.updated";
    pub const SITE_CREATED: &str = "site.created";
    /// A site row was updated in place (import framework, TASKLIST D2).
    pub const SITE_UPDATED: &str = "site.updated";
    pub const CONTACT_CREATED: &str = "contact.created";
    /// A contact row was updated in place (import framework, TASKLIST D2).
    pub const CONTACT_UPDATED: &str = "contact.updated";
    /// Import batch lifecycle (TASKLIST D2): a run opened / closed so
    /// webhook consumers can bracket bulk-change windows. Row-level
    /// mutations inside the batch keep emitting their ordinary events.
    pub const IMPORT_BATCH_STARTED: &str = "import.batch_started";
    pub const IMPORT_BATCH_FINISHED: &str = "import.batch_finished";
    pub const BOARD_CREATED: &str = "board.created";
    pub const BOARD_STATUS_ADDED: &str = "board.status_added";
    pub const BOARD_STATUS_UPDATED: &str = "board.status_updated";
    pub const BOARD_STATUS_REMOVED: &str = "board.status_removed";
    pub const BOARD_TRANSITIONS_UPDATED: &str = "board.transitions_updated";
    /// Emitted when a board's pre-close checklist template or enforcement
    /// toggle changes (TASKLIST 2.8).
    pub const BOARD_CHECKLIST_TEMPLATE_UPDATED: &str = "board.checklist_template_updated";
    pub const TICKET_CREATED: &str = "ticket.created";
    pub const TICKET_UPDATED: &str = "ticket.updated";
    pub const TICKET_STATUS_CHANGED: &str = "ticket.status_changed";
    pub const TICKET_ASSIGNED: &str = "ticket.assigned";
    pub const TICKET_COMMENT_ADDED: &str = "ticket.comment_added";
    pub const TICKET_CHECKLIST_UPDATED: &str = "ticket.checklist_updated";
    pub const TICKET_CLOSED: &str = "ticket.closed";
    pub const TIMER_STARTED: &str = "timer.started";
    pub const TIMER_PAUSED: &str = "timer.paused";
    pub const TIMER_RESUMED: &str = "timer.resumed";
    pub const TIMER_SWITCHED: &str = "timer.switched";
    pub const TIMER_STOPPED: &str = "timer.stopped";
    pub const TIME_ENTRY_CREATED: &str = "time_entry.created";
    pub const TIME_ENTRY_UPDATED: &str = "time_entry.updated";
    pub const NOTIFICATION_CREATED: &str = "notification.created";
    /// Emitted when a user marks notifications read (single, bulk, or all).
    pub const NOTIFICATION_READ: &str = "notification.read";
    /// Emitted when a user overrides one preference row.
    pub const NOTIFICATION_PREFS_UPDATED: &str = "notification_prefs.updated";
    pub const EMAIL_MESSAGE_RECORDED: &str = "email.message_recorded";
    /// A ladder step fired for a ticket clock (risk/breach/escalate).
    pub const SLA_ESCALATION_FIRED: &str = "sla.escalation_fired";
    /// An automation rule matched and its effects were applied.
    pub const AUTOMATION_TRIGGERED: &str = "automation.triggered";
    /// SLA configuration surfaces mutated (calendars/policies/ladders).
    pub const SLA_CALENDAR_UPDATED: &str = "sla.calendar_updated";
    pub const SLA_POLICY_UPDATED: &str = "sla.policy_updated";
    /// An automation rule definition was created/changed/disabled.
    pub const AUTOMATION_RULE_UPDATED: &str = "automation.rule_updated";
    /// Emitted when an org- or user-scoped setting changes through the
    /// typed settings registry.
    pub const SETTING_UPDATED: &str = "setting.updated";
    /// Emitted when the org profile (name/ticket prefix) is edited from
    /// General settings or the first-run wizard.
    pub const ORG_UPDATED: &str = "org.updated";
    /// Board organization + board-level RBAC (TASKLIST 2.11).
    pub const BOARD_GROUP_CREATED: &str = "board.group_created";
    pub const BOARD_GROUP_UPDATED: &str = "board.group_updated";
    pub const BOARD_GROUP_REMOVED: &str = "board.group_removed";
    pub const BOARD_MEMBERSHIP_UPDATED: &str = "board.membership_updated";
    pub const BOARD_MEMBERSHIP_REMOVED: &str = "board.membership_removed";
    pub const BOARD_FAVORITE_TOGGLED: &str = "board.favorite_toggled";
    /// Portal v1 lifecycle: admin provisioned / changed / removed a client
    /// portal identity, or toggled a board×client exposure. Portal ticket
    /// replies intentionally reuse `ticket.comment_added` so every staff
    /// notification path keeps working unchanged.
    pub const PORTAL_USER_CREATED: &str = "portal_user.created";
    pub const PORTAL_USER_UPDATED: &str = "portal_user.updated";
    pub const PORTAL_USER_DELETED: &str = "portal_user.deleted";
    pub const BOARD_PORTAL_EXPOSURE_UPDATED: &str = "board_portal_exposure.updated";
    /// A subscription test-fire from the webhook manager; flows through
    /// the same stream + delivery pipeline as real domain events.
    pub const WEBHOOK_TEST: &str = "webhook.test";
    /// Scheduling/calendars/availability foundation (TASKLIST 2.12).
    pub const APPOINTMENT_CREATED: &str = "appointment.created";
    pub const APPOINTMENT_UPDATED: &str = "appointment.updated";
    pub const APPOINTMENT_CANCELLED: &str = "appointment.cancelled";
    /// Lightweight change markers — consumers refetch the user's windows.
    pub const AVAILABILITY_UPDATED: &str = "availability.updated";
    pub const ON_CALL_UPDATED: &str = "on_call.updated";
    pub const CALENDAR_EXCLUSION_UPDATED: &str = "calendar_exclusion.updated";
    /// Attachments (TASKLIST 2.14): added on upload/extraction, linked
    /// when bound to a comment, deleted when a row (and possibly its
    /// content-addressed object) goes away.
    pub const ATTACHMENT_ADDED: &str = "attachment.added";
    pub const ATTACHMENT_LINKED: &str = "attachment.linked";
    pub const ATTACHMENT_DELETED: &str = "attachment.deleted";
    /// Custom fields / dynamic forms v1 (TASKLIST 2.17): definition CRUD
    /// through the board settings surface and typed ticket-value writes.
    pub const CUSTOM_FIELD_DEF_CREATED: &str = "custom_field.def_created";
    pub const CUSTOM_FIELD_DEF_UPDATED: &str = "custom_field.def_updated";
    pub const CUSTOM_FIELD_DEF_REMOVED: &str = "custom_field.def_removed";
    pub const TICKET_FIELD_SET: &str = "ticket.field_set";
    pub const TICKET_FIELD_CLEARED: &str = "ticket.field_cleared";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketCreated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    pub number: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    pub title: String,
    pub priority: Priority,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub client_id: Option<ClientId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub assignee_id: Option<UserId>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketStatusChanged {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    /// Board the ticket lives on — drives WS fan-out to `board:<id>`.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    /// Current assignee — drives WS fan-out to `user:<id>`.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub assignee_id: Option<UserId>,
    pub from_status: String,
    pub to_status: String,
    pub from_category: TicketStatusCategory,
    pub to_category: TicketStatusCategory,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub changed_by: Option<UserId>,
    pub occurred_at: DateTime<Utc>,
}

/// A status was added to a board (custom workflow definition, §5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardStatusAdded {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    pub status: crate::entities::BoardStatus,
}

/// A board status's label/category/ordering was re-bound.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardStatusUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    pub status: crate::entities::BoardStatus,
}

/// An unused board status was removed. Carries the name because the row is
/// gone — consumers render history from this payload alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardStatusRemoved {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub status_id: crate::ids::TicketStatusId,
    pub name: String,
}

/// The board's explicit transition matrix was replaced. Empty `edges`
/// means the semantic-category floor governs again.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardTransitionsUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    pub edges: Vec<crate::workflow::TransitionEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketAssigned {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    /// Board the ticket lives on — drives WS fan-out to `board:<id>`.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub assignee_id: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub assigned_by: Option<UserId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketCommentAdded {
    pub comment: TicketComment,
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<uuid::Uuid>))]
    pub mentioned_user_ids: Vec<UserId>,
}

/// A client company was created (payload for `client.created`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientCreated {
    pub client_id: ClientId,
    pub name: String,
}

/// A site was created under a client (payload for `site.created`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteCreated {
    pub site_id: SiteId,
    pub client_id: ClientId,
    pub name: String,
}

/// A contact was created under a client (payload for `contact.created`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactCreated {
    pub contact_id: ContactId,
    pub client_id: ClientId,
    pub name: String,
}

/// A client was updated in place (payload for `client.updated`; import
/// framework update policy and future staff edit surfaces).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientUpdated {
    pub client_id: ClientId,
    pub name: String,
}

/// A site was updated in place (payload for `site.updated`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteUpdated {
    pub site_id: SiteId,
    pub client_id: ClientId,
    pub name: String,
}

/// A contact was updated in place (payload for `contact.updated`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactUpdated {
    pub contact_id: ContactId,
    pub client_id: ClientId,
    pub name: String,
}

/// An import run opened (payload for `import.batch_started`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportBatchStarted {
    pub batch_id: crate::ids::ImportBatchId,
    pub entity_kind: String,
    pub conflict_policy: String,
    pub label: String,
}

/// An import run finished (payload for `import.batch_finished`). Counters
/// are cumulative over every committed chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportBatchFinished {
    pub batch_id: crate::ids::ImportBatchId,
    pub entity_kind: String,
    pub rows_total: i32,
    pub rows_created: i32,
    pub rows_updated: i32,
    pub rows_skipped: i32,
    pub rows_failed: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketChecklistUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    /// Board the ticket lives on — drives WS fan-out to `board:<id>`.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    pub items: Vec<ChecklistItem>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub updated_by: Option<UserId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimerStarted {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub session_id: TimerSessionId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    pub started_at: DateTime<Utc>,
    /// Session auto-paused by this start (single-active-timer workflow).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub switched_from: Option<TimerSessionId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimerPaused {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub session_id: TimerSessionId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    pub reason: Option<String>,
    pub elapsed_secs: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimerStopped {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub session_id: TimerSessionId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    pub total_secs: i64,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub time_entry_id: Option<TimeEntryId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimeEntryCreated {
    pub entry: TimeEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NotificationCreated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    pub kind: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<TicketId>,
    pub title: String,
}

/// Emitted whenever an email message is recorded in the ledger (either
/// direction), with the routing outcome so consumers can correlate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EmailMessageRecorded {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub message_id: uuid::Uuid,
    pub rfc_message_id: String,
    /// "inbound" | "outbound" — kept as a string to avoid coupling
    /// consumers to the enum across versions.
    pub direction: String,
    pub outcome: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<TicketId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub comment_id: Option<uuid::Uuid>,
}

/// A board group was created (TASKLIST 2.11).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardGroupCreated {
    pub group: crate::entities::BoardGroup,
}

/// A board group's name/position changed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardGroupUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub group_id: crate::ids::OrgId,
    pub name: String,
    pub position: i32,
}

/// A board group was deleted; member boards became ungrouped.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardGroupRemoved {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub group_id: crate::ids::OrgId,
}

/// A user's board membership was created or re-roled.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardMembershipUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: crate::ids::BoardId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: crate::ids::UserId,
    pub role: crate::policy::BoardRole,
}

/// A user's board membership was removed (back to org-governed access).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardMembershipRemoved {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: crate::ids::BoardId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: crate::ids::UserId,
}

/// A user toggled a board favorite for the nav tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardFavoriteToggled {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: crate::ids::UserId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: crate::ids::BoardId,
    pub favorite: bool,
}

/// A booking was created on a user's calendar (TASKLIST 2.12).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AppointmentCreated {
    pub appointment: crate::entities::Appointment,
}

/// An appointment's editable fields or times changed (`rescheduled` marks
/// moves so dispatch consumers can re-plan travel).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AppointmentUpdated {
    pub appointment: crate::entities::Appointment,
    pub rescheduled: bool,
}

/// An appointment was cancelled; its slot is freed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AppointmentCancelled {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub appointment_id: crate::ids::AppointmentId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: crate::ids::UserId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub cancelled_by: Option<crate::ids::UserId>,
}

/// Availability templates changed for one user (create/delete).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AvailabilityChanged {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: crate::ids::UserId,
}

/// On-call rotations changed for one user.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OnCallChanged {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: crate::ids::UserId,
}

/// Time-off exclusions changed for one user.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CalendarExclusionChanged {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: crate::ids::UserId,
}

/// Extracts @mentions from a note body (`@alice`, `@alice@org` → user ids
/// resolved by the caller). Returns the distinct handle strings.
pub fn extract_mentions(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in body.split_whitespace() {
        let t = token.trim_matches(|c: char| c == ',' || c == '.' || c == ')' || c == '(');
        if let Some(handle) = t.strip_prefix('@') {
            let handle = handle.split('@').next().unwrap_or(handle);
            if !handle.is_empty() && !out.contains(&handle.to_string()) {
                out.push(handle.to_string());
            }
        }
    }
    out
}

/// attachment.added — a file was stored and its metadata row committed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AttachmentAdded {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub attachment_id: uuid::Uuid,
    pub kind: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_comment_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub email_message_id: Option<uuid::Uuid>,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    /// sha256 hex — consumers can verify integrity after download.
    pub sha256: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub uploaded_by_user_id: Option<uuid::Uuid>,
}

/// attachment.linked — an unattached upload was bound to a comment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AttachmentLinked {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub attachment_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_comment_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: uuid::Uuid,
}

/// attachment.deleted — the metadata row went away; `object_removed`
/// reports whether the backing bytes were reclaimed too (refcount hit 0).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AttachmentDeleted {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub attachment_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<uuid::Uuid>,
    pub object_removed: bool,
}

/// Convenience for building the note-kind of a comment event.
pub fn comment_kind_label(kind: NoteKind) -> &'static str {
    match kind {
        NoteKind::Internal => "internal",
        NoteKind::ClientVisible => "client-visible",
        NoteKind::InHouseUpdate => "in-house update",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mention_extraction() {
        let m = extract_mentions("ping @alice and @bob, cc @alice. also @carol@external");
        assert_eq!(m, vec!["alice", "bob", "carol"]);
    }

    #[test]
    fn no_mentions() {
        assert!(extract_mentions("no handles here, email@example.com").is_empty());
    }
}
