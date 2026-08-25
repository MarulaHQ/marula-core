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
