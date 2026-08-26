//! Pre-close checklist template entities and the derived progress view.

use crate::ids::{BoardId, OrgId};
use chrono::{DateTime, Utc};

/// A board's pre-close checklist template. Exactly one per board
/// (UNIQUE in storage): the deterministic gate consulted at close time.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChecklistTemplate {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One required step in a template. Instances on tickets link back to this
/// id (`checklist_items.template_item_id`), so enforcement follows identity,
/// never fragile text matching.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChecklistTemplateItem {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub template_id: uuid::Uuid,
    pub text: String,
    pub sort_order: i32,
}

/// Derived-at-read-time checklist progress (never materialized — see the
/// 0016 migration header for the justification). `required_open` counts
/// unchecked template-derived instances: the close-blocking set when the
/// board enforces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChecklistProgress {
    /// All instance rows (template-derived + ad-hoc).
    pub total: i64,
    /// Checked rows across both classes.
    pub checked: i64,
    /// Unchecked REQUIRED (template-derived) rows.
    pub required_open: i64,
}

impl ChecklistProgress {
    /// Whether every item is checked (`total == checked`, vacuously true
    /// for an empty checklist).
    pub const fn is_complete(self) -> bool {
        self.required_open == 0 && self.checked == self.total
    }
}
