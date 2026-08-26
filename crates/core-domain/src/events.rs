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
    pub const CONTACT_CREATED: &str = "contact.created";
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketChecklistUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
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
