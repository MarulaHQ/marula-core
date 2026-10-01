//! Ticket entity, structured comments, and pre-close checklist items.

use crate::entities::board::BoardStatus;
use crate::ids::*;
use crate::status::{NoteKind, Priority, TicketStatusCategory};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Ticket {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: TicketId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub number: TicketNumber,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub status_id: TicketStatusId,
    pub title: String,
    pub body: Option<String>,
    pub priority: Priority,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub client_id: Option<ClientId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub site_id: Option<SiteId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub contact_id: Option<ContactId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub assignee_id: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub requester_id: Option<UserId>,
    /// Denormalized from the board status for cheap filtering/sorting.
    pub category: TicketStatusCategory,
    pub due_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Merge marker (Wave 11 lane 9): the survivor this ticket folded
    /// into, when it was absorbed by a merge. The row stays queryable
    /// (doctrine #9) but is closed and never re-opened.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub merged_into_ticket_id: Option<crate::ids::TicketId>,
    /// Classification (Wave 11 lane 3; read-view exposure Wave 12 lane 6):
    /// the org taxonomy nodes carried on the ticket. PATCH accepts them;
    /// the read view now returns them so ticket detail can display the
    /// current classification. Purely descriptive — classification-only
    /// doctrine (§5) binds behavior to semantic categories, never these.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub type_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub category_id: Option<uuid::Uuid>,
}

impl Ticket {
    /// Whether this ticket may transition into `target`. Category rules are
    /// the floor (any→any is legal except closed→open without reopen intent);
    /// per-board transition tables (Stage 2) tighten this further.
    pub fn can_transition_to(&self, target: &BoardStatus) -> bool {
        if self.board_id != target.board_id {
            return false;
        }
        !matches!(self.category, TicketStatusCategory::Closed)
            || !matches!(target.category, TicketStatusCategory::Open)
    }
}

/// A structured ticket note. Taxonomy: internal / client-visible /
/// in-house update. `kind` gates portal visibility and email fan-out.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketComment {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: TicketCommentId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub author_id: Option<UserId>,
    pub kind: NoteKind,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    /// Server-composed workflow-note provenance (Wave 17 lane 11,
    /// migration 0080): `{kind, auto, item_ids}` on notes the server
    /// appended for task-state transitions; NULL/unsent for every
    /// user-written note. Machine marking lives HERE, never in body text
    /// (spoofable, unqueryable). Additive-only contract: absent keys
    /// deserialize to None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_meta: Option<serde_json::Value>,
}

/// One line of a ticket's pre-close checklist. Items instantiated from the
/// board's template carry `template_item_id`; those are REQUIRED when the
/// board enforces pre-close checklists (see `crate::notes`). Ad-hoc items
/// (`template_item_id == None`) never block closure.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChecklistItem {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: ChecklistItemId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    pub text: String,
    pub checked: bool,
    /// Shared "not applicable" state (Wave 12 lane 6): server truth for the
    /// tasks-panel tri-state, set by the toggle endpoint in the same tx as
    /// `checked`. Mutually exclusive with `checked` (enforced at the API);
    /// N-A REQUIRED items are exempt from pre-close enforcement and derived
    /// progress. `#[serde(default)]` keeps older payloads (no `na` key)
    /// deserializing as false — additive-only contract.
    #[serde(default)]
    pub na: bool,
    pub sort_order: i32,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub checked_by: Option<UserId>,
    pub checked_at: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    /// Template provenance: `Some` ⇒ required under enforcement.
    pub template_item_id: Option<uuid::Uuid>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::TicketStatusCategory;
    use uuid::Uuid;

    fn status(board: Uuid, cat: TicketStatusCategory) -> BoardStatus {
        BoardStatus {
            id: Uuid::now_v7(),
            board_id: board,
            org_id: Uuid::now_v7(),
            name: "Done".into(),
            category: cat,
            sort_order: 0,
            client_visible: true,
            is_entry: false,
            color: None,
            wip_limit: None,
        }
    }

    #[test]
    fn closed_ticket_cannot_reopen_to_open() {
        let board = Uuid::now_v7();
        let open = status(board, TicketStatusCategory::Open);
        let waiting = status(board, TicketStatusCategory::Waiting);
        let t = Ticket {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            number: TicketNumber("MAR-1".into()),
            board_id: board,
            status_id: Uuid::now_v7(),
            title: "t".into(),
            body: None,
            priority: Priority::P3,
            client_id: None,
            site_id: None,
            contact_id: None,
            assignee_id: None,
            requester_id: None,
            category: TicketStatusCategory::Closed,
            due_at: None,
            resolved_at: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            merged_into_ticket_id: None,
            type_id: None,
            category_id: None,
        };
        assert!(!t.can_transition_to(&open));
        assert!(t.can_transition_to(&waiting)); // reopen via intermediate is allowed
    }
}
