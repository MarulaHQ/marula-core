//! Tools + toolboxes (Wave 32 lane 3, `p32-tools-loans`).
//!
//! A tool is a lane-1 stock item that is `serialized` and flagged
//! `is_tool` — one physical unit per `stock_serials` row, individually
//! tracked. A toolbox is a KIT: a named `tool_kits` row grouping member
//! tools that are checked out together, as ONE transaction, while each
//! physical unit still moves on its own ledger leg.
//!
//! Check-out/check-in transactions: one `tool_checkouts` header (the
//! holder, the due date, the notes), one `tool_checkout_lines` row per
//! physical unit referencing the append-only `stock_movements` legs it
//! rode (`movement_out_id` always, `movement_in_id` on return). The
//! single-holder invariant — one tool, one person at a time — is the
//! partial unique index on open lines, mirrored by the serial status gate
//! (`checked_out`). Overdue is DERIVED (`returned_at IS NULL AND due_at <
//! now()`); it is never a stored flag that can go stale.

use crate::error::DomainError;
use crate::ids::OrgId;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Return-condition vocabulary for a physical unit coming back from a
/// checkout or a client loan. Additive-only: widen the list + the DB
/// CHECK together.
pub const RETURN_CONDITIONS: [&str; 3] = ["good", "fair", "damaged"];

pub fn is_valid_return_condition(condition: &str) -> bool {
    RETURN_CONDITIONS.contains(&condition.trim())
}

/// Checkout transaction lifecycle. "Overdue" is derived at read time from
/// `due_at`, never stored — a stored flag would go stale the moment the
/// clock moved past it.
pub const CHECKOUT_STATUSES: [&str; 2] = ["open", "returned"];

/// A toolbox: a named kit of member tools checked out as one.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ToolKit {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub name: String,
    pub code: Option<String>,
    pub note: Option<String>,
    pub active: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ToolKit {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.name.trim().is_empty() {
            return Err(DomainError::Validation("tool kit name is required".into()));
        }
        if self.name.len() > 200 {
            return Err(DomainError::Validation(
                "tool kit name must be at most 200 characters".into(),
            ));
        }
        if let Some(note) = &self.note {
            if note.chars().count() > 500 {
                return Err(DomainError::Validation(
                    "tool kit note must be at most 500 characters".into(),
                ));
            }
        }
        Ok(())
    }
}

/// One member item of a kit. Tools are serialized, so a member quantity is
/// whole units (multiples of 1000 milli) — a kit cannot hold "half a
/// torque wrench".
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ToolKitMember {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub kit_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub stock_item_id: Uuid,
    pub qty_milli: i64,
    pub created_at: DateTime<Utc>,
}

impl ToolKitMember {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.qty_milli <= 0 {
            return Err(DomainError::Validation(
                "kit member quantity must be positive".into(),
            ));
        }
        if self.qty_milli % 1000 != 0 {
            return Err(DomainError::Validation(
                "kit member quantity must be whole units (multiples of 1000 milli)".into(),
            ));
        }
        Ok(())
    }
}

/// A check-out/check-in transaction header. Mutable while open (lines
/// return into it); the ledger legs it references never change.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ToolCheckout {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    /// The single holder of the transaction.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub holder_user_id: Uuid,
    /// Set when the checkout came from a kit (toolbox checked out as one).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub kit_id: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub from_location_id: Uuid,
    pub due_at: DateTime<Utc>,
    pub notes: Option<String>,
    pub status: String,
    pub returned_at: Option<DateTime<Utc>>,
    pub return_notes: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub checked_out_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ToolCheckout {
    pub fn validate(&self) -> Result<(), DomainError> {
        if !CHECKOUT_STATUSES.contains(&self.status.as_str()) {
            return Err(DomainError::Validation(
                "checkout status must be open or returned".into(),
            ));
        }
        if (self.status == "returned") != self.returned_at.is_some() {
            return Err(DomainError::Validation(
                "a returned checkout carries returned_at; an open one does not".into(),
            ));
        }
        for text in [self.notes.as_deref(), self.return_notes.as_deref()]
            .into_iter()
            .flatten()
        {
            if text.chars().count() > 500 {
                return Err(DomainError::Validation(
                    "checkout notes must be at most 500 characters".into(),
                ));
            }
        }
        Ok(())
    }

    /// Derived, never stored: overdue while open past the due date.
    pub fn is_overdue(&self, now: DateTime<Utc>) -> bool {
        self.returned_at.is_none() && self.due_at < now
    }
}

/// One physical unit on a checkout, referencing the ledger legs it rode.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ToolCheckoutLine {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub checkout_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub stock_item_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub serial_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub movement_out_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub movement_in_id: Option<Uuid>,
    pub returned_at: Option<DateTime<Utc>>,
    pub return_condition: Option<String>,
    pub return_notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ToolCheckoutLine {
    pub fn validate(&self) -> Result<(), DomainError> {
        if let Some(condition) = &self.return_condition {
            if !is_valid_return_condition(condition) {
                return Err(DomainError::Validation(
                    "return condition must be one of: good, fair, damaged".into(),
                ));
            }
            if self.returned_at.is_none() {
                return Err(DomainError::Validation(
                    "a return condition requires the unit to be returned".into(),
                ));
            }
        }
        if let Some(notes) = &self.return_notes {
            if notes.chars().count() > 500 {
                return Err(DomainError::Validation(
                    "return notes must be at most 500 characters".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn is_returned(&self) -> bool {
        self.returned_at.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kit() -> ToolKit {
        ToolKit {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            name: "Electrical toolbox".into(),
            code: Some("TB-ELEC-1".into()),
            note: None,
            active: true,
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn checkout() -> ToolCheckout {
        ToolCheckout {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            holder_user_id: Uuid::now_v7(),
            kit_id: None,
            from_location_id: Uuid::now_v7(),
            due_at: Utc::now() + chrono::Duration::days(3),
            notes: None,
            status: "open".into(),
            returned_at: None,
            return_notes: None,
            checked_out_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn kit_shape_holds() {
        kit().validate().unwrap();
    }

    #[test]
    fn kit_member_is_whole_units() {
        let mut m = ToolKitMember {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            kit_id: Uuid::now_v7(),
            stock_item_id: Uuid::now_v7(),
            qty_milli: 1000,
            created_at: Utc::now(),
        };
        m.validate().unwrap();
        m.qty_milli = 1500;
        assert!(m.validate().is_err());
        m.qty_milli = 0;
        assert!(m.validate().is_err());
    }

    #[test]
    fn overdue_is_derived_from_due_date() {
        let mut c = checkout();
        let now = Utc::now();
        assert!(!c.is_overdue(now), "future due date is not overdue");
        c.due_at = now - chrono::Duration::hours(1);
        assert!(c.is_overdue(now), "open past due is overdue");
        c.returned_at = Some(now);
        c.status = "returned".into();
        assert!(!c.is_overdue(now), "a returned checkout is never overdue");
    }

    #[test]
    fn returned_status_and_timestamp_move_together() {
        let mut c = checkout();
        c.status = "returned".into();
        assert!(c.validate().is_err());
        c.returned_at = Some(Utc::now());
        c.validate().unwrap();
    }

    #[test]
    fn return_condition_needs_a_returned_unit() {
        let mut line = ToolCheckoutLine {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            checkout_id: Uuid::now_v7(),
            stock_item_id: Uuid::now_v7(),
            serial_id: Uuid::now_v7(),
            movement_out_id: Uuid::now_v7(),
            movement_in_id: None,
            returned_at: None,
            return_condition: Some("good".into()),
            return_notes: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(line.validate().is_err());
        line.returned_at = Some(Utc::now());
        line.validate().unwrap();
        line.return_condition = Some("melted".into());
        assert!(line.validate().is_err());
    }
}
