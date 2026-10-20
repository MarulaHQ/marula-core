//! Board and board-status entities. Custom statuses bind to semantic
//! categories (§5); the category column is what behavior reads.

use crate::ids::{BoardId, OrgId, TicketStatusId, UserId};
use crate::status::TicketStatusCategory;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Board {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: BoardId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub name: String,
    pub description: Option<String>,
    pub is_default: bool,
    /// Pre-close checklist enforcement toggle (defaults OFF; closing is
    /// gated only when this AND a checklist template are both present —
    /// defaults-first doctrine, migration 0016).
    pub require_checklist_close: bool,
    /// Users whose favorite this board is (board groups/favorites, Stage 2).
    pub created_at: DateTime<Utc>,
}

/// A per-board status. `category` is the binding contract — labels are
/// presentation, category is behavior.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardStatus {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: TicketStatusId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub name: String,
    pub category: TicketStatusCategory,
    pub sort_order: i32,
    /// Whether tickets may enter this status directly from New.
    pub is_entry: bool,
    pub color: Option<String>,
    /// Per-column WIP cap (0089): occupancy of this column is enforced
    /// server-side at ticket status change. `None` = no limit.
    pub wip_limit: Option<i32>,
    /// Wave 41 lane 12: whether the portal shows this status's own label to
    /// a client. `false` shows the semantic category word instead, so an
    /// internal label ("Escalated to L2") never reaches a client while the
    /// ticket still reports where it stands.
    pub client_visible: bool,
}

/// A user's favorite marking of a board (drives board groups + my-work).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardFavorite {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    pub sort_order: i32,
}

/// A named, ordered nav grouping of boards within one org (TASKLIST 2.11).
/// Groups are organization-wide; `owner_id` records who controls the group.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardGroup {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub owner_id: UserId,
    pub name: String,
    pub position: i32,
    pub created_at: DateTime<Utc>,
}

/// An explicit per-user membership on one board carrying a board-level
/// role tier (§6 board-level RBAC). Rows NARROW org grants; absence of a
/// row leaves org-level governance untouched.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardMember {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    pub role: crate::policy::BoardRole,
    pub created_at: DateTime<Utc>,
}
