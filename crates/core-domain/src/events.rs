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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketCreated {
    pub ticket_id: TicketId,
    pub number: String,
    pub board_id: BoardId,
    pub title: String,
    pub priority: Priority,
    pub client_id: Option<ClientId>,
    pub assignee_id: Option<UserId>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketStatusChanged {
    pub ticket_id: TicketId,
    pub from_status: String,
    pub to_status: String,
    pub from_category: TicketStatusCategory,
    pub to_category: TicketStatusCategory,
    pub changed_by: Option<UserId>,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketAssigned {
    pub ticket_id: TicketId,
    pub assignee_id: Option<UserId>,
    pub assigned_by: Option<UserId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketCommentAdded {
    pub comment: TicketComment,
    pub mentioned_user_ids: Vec<UserId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketChecklistUpdated {
    pub ticket_id: TicketId,
    pub items: Vec<ChecklistItem>,
    pub updated_by: Option<UserId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimerStarted {
    pub session_id: TimerSessionId,
    pub user_id: UserId,
    pub ticket_id: TicketId,
    pub started_at: DateTime<Utc>,
    /// Session auto-paused by this start (single-active-timer workflow).
    pub switched_from: Option<TimerSessionId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimerPaused {
    pub session_id: TimerSessionId,
    pub user_id: UserId,
    pub ticket_id: TicketId,
    pub reason: Option<String>,
    pub elapsed_secs: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimerStopped {
    pub session_id: TimerSessionId,
    pub user_id: UserId,
    pub ticket_id: TicketId,
    pub total_secs: i64,
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
    pub user_id: UserId,
    pub kind: String,
    pub ticket_id: Option<TicketId>,
    pub title: String,
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
