//! Stock + consumables (Wave 32 lane 1, `p32-stock-consumables`).
//!
//! The MSP's OWN stock — org-owned SKUs at typed physical locations, with
//! per-location quantities/thresholds, an append-only movement ledger as
//! the traceability record, and the serialized-vs-consumable distinction
//! (a serialized item is ONE physical unit; a consumable has only counts
//! — the serialized shape is the seam Wave 32 lane 3's tools and client
//! loans consume).
//!
//! Conventions: quantities are `*_milli` thousandths-integers consistent
//! with quote lines (floats never touch quantities, §5); money is integer
//! minor units + ISO 4217; the ADR 0002 inventory-receipt FX lock moment
//! is honored by freezing the rate on every `receive` row.

use crate::error::DomainError;
use crate::ids::OrgId;
use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;

/// Units of measure at `quantity_milli` precision (parity with quote
/// lines' `unit` column; additive to widen: lane 2/3 may extend the
/// shipment/tool surfaces but the EQUALITY check stays in the DB).
pub const STOCK_UNITS: [&str; 3] = ["each", "box", "meter"];

pub fn is_valid_stock_unit(unit: &str) -> bool {
    STOCK_UNITS.contains(&unit.trim())
}

pub const STOCK_LOCATION_KINDS: [&str; 4] = ["warehouse", "shelf", "van", "truck"];

pub fn is_valid_location_kind(kind: &str) -> bool {
    STOCK_LOCATION_KINDS.contains(&kind.trim())
}

/// Movement kinds on the append-only ledger. Deltas are SIGNED in the
/// ledger (`qty_milli`), direction is derived from the kind:
/// receive/transfer/checkin/loan_return → into stock;
/// consume/ship_out/checkout/loan → out; adjust → ±.
///
/// Wave 32 lane 3 extended the vocabulary additively (the sanctioned
/// extension path from lane 1): `checkout`/`checkin` are tool checkout
/// legs, `loan`/`loan_return` are client-loan legs (both carry the client).
pub const MOVEMENT_KINDS: [&str; 9] = [
    "receive",
    "consume",
    "adjust",
    "transfer",
    "ship_out",
    "checkout",
    "checkin",
    "loan",
    "loan_return",
];

/// One org-owned stock item: a catalog-linked SKU or a free-form part.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct StockItem {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub name: String,
    /// Optional SKU; unique per org (case-insensitive) when present.
    pub sku: Option<String>,
    /// Soft catalog reference (SET NULL on catalog removal); absent =
    /// free-form part carried by `name`/`part_number`.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub catalog_item_id: Option<Uuid>,
    pub part_number: Option<String>,
    /// serialized = one physical unit per serial; consumable = counts.
    pub kind: String,
    /// Wave 32 lane 3: a tool is a serialized item tracked per physical
    /// unit and eligible for tool checkout + kit membership.
    pub is_tool: bool,
    pub unit: String,
    pub currency: String,
    /// Most recent receipt's locked unit cost (display; the ledger keeps
    /// the authoritative per-receipt history).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub last_unit_cost_minor: Option<i64>,
    pub active: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl StockItem {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.name.trim().is_empty() {
            return Err(DomainError::Validation(
                "stock item name is required".into(),
            ));
        }
        if self.name.len() > 200 {
            return Err(DomainError::Validation(
                "stock item name must be at most 200 characters".into(),
            ));
        }
        if !is_valid_stock_unit(&self.unit) {
            return Err(DomainError::Validation(
                "stock item unit must be one of: each, box, meter".into(),
            ));
        }
        if self.kind != "consumable" && self.kind != "serialized" {
            return Err(DomainError::Validation(
                "stock item kind must be consumable or serialized".into(),
            ));
        }
        if self.is_tool && self.kind != "serialized" {
            return Err(DomainError::Validation(
                "a tool must be a serialized item (one physical unit per serial)".into(),
            ));
        }
        crate::money::CurrencyCode::new(&self.currency)?;
        Ok(())
    }
}

/// A typed physical location stock lives in.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct StockLocation {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub name: String,
    pub kind: String,
    pub code: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub parent_id: Option<Uuid>,
    /// Whose van/truck (optional; a tech's vehicle).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub assigned_user_id: Option<Uuid>,
    pub active: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl StockLocation {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.name.trim().is_empty() {
            return Err(DomainError::Validation("location name is required".into()));
        }
        if self.name.len() > 200 {
            return Err(DomainError::Validation(
                "location name must be at most 200 characters".into(),
            ));
        }
        if !is_valid_location_kind(&self.kind) {
            return Err(DomainError::Validation(
                "location kind must be one of: warehouse, shelf, van, truck".into(),
            ));
        }
        if self.parent_id == Some(self.id) {
            return Err(DomainError::Validation(
                "a location cannot be its own parent".into(),
            ));
        }
        Ok(())
    }
}

/// Per-(item, location) quantity + thresholds; the FRESH snapshot the
/// read models serve. The movement ledger is the ground truth; a level is
/// always the ledger's deterministic consequence, updated in the same
/// commit as the ledger row.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct StockLevel {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub stock_item_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub stock_location_id: Uuid,
    pub qty_milli: i64,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub min_qty_milli: Option<i64>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub max_qty_milli: Option<i64>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub reorder_point_milli: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl StockLevel {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.qty_milli < 0 {
            return Err(DomainError::Validation(
                "stock quantity must not be negative".into(),
            ));
        }
        for v in [
            self.min_qty_milli,
            self.max_qty_milli,
            self.reorder_point_milli,
        ]
        .into_iter()
        .flatten()
        {
            if v < 0 {
                return Err(DomainError::Validation(
                    "stock thresholds must not be negative".into(),
                ));
            }
        }
        Ok(())
    }
}

/// One row of the append-only ledger. Never mutated after insert (DB
/// trigger-guarded); every downstream checkout/loan/consumption/shipment
/// row references `id` as `movement_id` — the traceability record.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct StockMovement {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub stock_item_id: Uuid,
    pub kind: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub from_location_id: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub to_location_id: Option<Uuid>,
    /// Signed delta in milli (+ into stock, − out).
    pub qty_milli: i64,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub cost_minor: Option<i64>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub currency: Option<String>,
    /// ADR 0002 inventory-receipt FX lock (frozen at `receive`).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub fx_rate_used: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub fx_rate_date: Option<NaiveDate>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub fx_source: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub base_cost_minor: Option<i64>,
    /// Serialized seam (lane 3): which physical unit moved.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub serial_id: Option<Uuid>,
    /// ship_out destination (fleet/loan delivery).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub client_id: Option<Uuid>,
    /// On-site job the consumption/shipment belongs to.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<Uuid>,
    pub carrier: Option<String>,
    pub tracking_ref: Option<String>,
    pub note: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

impl StockMovement {
    pub fn validate(&self) -> Result<(), DomainError> {
        if !MOVEMENT_KINDS.contains(&self.kind.as_str()) {
            return Err(DomainError::Validation(format!(
                "movement kind must be one of: {}",
                MOVEMENT_KINDS.join(", ")
            )));
        }
        if self.qty_milli == 0 {
            return Err(DomainError::Validation(
                "movement quantity must be non-zero".into(),
            ));
        }
        // Direction discipline (same invariants the write path enforces):
        match self.kind.as_str() {
            "receive"
                if self.from_location_id.is_some()
                    || self.to_location_id.is_none()
                    || self.qty_milli <= 0 =>
            {
                return Err(DomainError::Validation(
                    "receive requires a destination location and positive quantity".into(),
                ));
            }
            "consume" | "ship_out" | "checkout" | "loan"
                if self.from_location_id.is_none()
                    || self.to_location_id.is_some()
                    || self.qty_milli >= 0 =>
            {
                return Err(DomainError::Validation(
                    "consume/ship_out/checkout/loan require a source location and negative quantity"
                        .into(),
                ));
            }
            "checkin" | "loan_return"
                if self.from_location_id.is_some()
                    || self.to_location_id.is_none()
                    || self.qty_milli <= 0 =>
            {
                return Err(DomainError::Validation(
                    "checkin/loan_return require a destination location and positive quantity"
                        .into(),
                ));
            }
            // Client-loan legs always carry their client (the destination
            // is a client, not a location).
            "loan" | "loan_return" if self.client_id.is_none() => {
                return Err(DomainError::Validation(
                    "loan movements require a client".into(),
                ));
            }
            "adjust" if self.from_location_id.is_some() == self.to_location_id.is_some() => {
                return Err(DomainError::Validation(
                    "adjust requires exactly one of source or destination".into(),
                ));
            }
            "transfer"
                if self.from_location_id.is_none()
                    || self.to_location_id.is_none()
                    || self.from_location_id == self.to_location_id
                    || self.qty_milli <= 0 =>
            {
                return Err(DomainError::Validation(
                    "transfer requires distinct source and destination and positive quantity"
                        .into(),
                ));
            }
            _ => {}
        }
        if let Some(f) = &self.fx_rate_used {
            f.parse::<f64>()
                .map_err(|_| DomainError::Validation("fx_rate_used must be numeric".into()))?;
        }
        Ok(())
    }
}

/// A serialized physical unit (one row per serial).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct StockSerial {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub stock_item_id: Uuid,
    pub serial_number: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub location_id: Option<Uuid>,
    pub status: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub client_id: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl StockSerial {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.serial_number.trim().is_empty() {
            return Err(DomainError::Validation("serial number is required".into()));
        }
        if self.status != "in_stock" && self.status != "out" {
            return Err(DomainError::Validation(
                "serial status must be in_stock or out".into(),
            ));
        }
        Ok(())
    }
}

/// Signed movement kind for `qty_milli` derivation; the write path uses
/// these helpers so milli math never branches on floats.
pub fn signed_delta(kind: &str, abs_qty_milli: i64) -> Option<i64> {
    match kind {
        "receive" | "transfer" | "checkin" | "loan_return" => Some(abs_qty_milli.max(0)),
        "consume" | "ship_out" | "checkout" | "loan" => Some(-abs_qty_milli.max(0)),
        "adjust" => Some(abs_qty_milli),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> StockItem {
        StockItem {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            name: "Cat6 box".into(),
            sku: Some("CAT6-305".into()),
            catalog_item_id: None,
            part_number: Some("C6-305M".into()),
            kind: "consumable".into(),
            is_tool: false,
            unit: "box".into(),
            currency: "USD".into(),
            last_unit_cost_minor: Some(12000),
            active: true,
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn movement() -> StockMovement {
        StockMovement {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            stock_item_id: Uuid::now_v7(),
            kind: "receive".into(),
            from_location_id: None,
            to_location_id: Some(Uuid::now_v7()),
            qty_milli: 25_000,
            cost_minor: Some(12_000),
            currency: Some("USD".into()),
            fx_rate_used: None,
            fx_rate_date: None,
            fx_source: None,
            base_cost_minor: None,
            serial_id: None,
            client_id: None,
            ticket_id: None,
            carrier: None,
            tracking_ref: None,
            note: None,
            created_by: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn receive_shape_holds() {
        movement().validate().unwrap();
    }

    #[test]
    fn movement_directions_are_enforced() {
        let mut m = movement();
        m.kind = "consume".into();
        m.qty_milli = -2_000;
        m.from_location_id = Some(Uuid::now_v7());
        m.to_location_id = None;
        m.validate().unwrap();

        m.qty_milli = 2_000;
        assert!(m.validate().is_err());

        m.kind = "transfer".into();
        m.qty_milli = 2_000;
        m.from_location_id = Some(Uuid::now_v7());
        m.to_location_id = Some(Uuid::now_v7());
        m.validate().unwrap();

        m.to_location_id = m.from_location_id;
        assert!(m.validate().is_err());
    }

    #[test]
    fn units_are_pinned() {
        assert!(is_valid_stock_unit("each"));
        assert!(!is_valid_stock_unit("hour"));
        assert_eq!(signed_delta("consume", 1500), Some(-1500));
    }

    #[test]
    fn item_unit_must_be_stock_unit() {
        let i = item();
        i.validate().unwrap();
        let mut i = i;
        i.unit = "hour".into();
        assert!(i.validate().is_err());
    }

    #[test]
    fn tool_must_be_serialized() {
        let mut i = item();
        i.is_tool = true;
        assert!(
            i.validate().is_err(),
            "a consumable cannot be flagged as a tool"
        );
        i.kind = "serialized".into();
        i.validate().unwrap();
    }

    #[test]
    fn checkout_movement_direction_discipline() {
        let mut m = movement();
        m.kind = "checkout".into();
        m.qty_milli = -1_000;
        m.from_location_id = Some(Uuid::now_v7());
        m.to_location_id = None;
        m.serial_id = Some(Uuid::now_v7());
        m.validate().unwrap();

        m.kind = "checkin".into();
        m.qty_milli = 1_000;
        m.from_location_id = None;
        m.to_location_id = Some(Uuid::now_v7());
        m.validate().unwrap();

        // A loan leg must carry its client.
        m.kind = "loan".into();
        m.qty_milli = -1_000;
        m.from_location_id = Some(Uuid::now_v7());
        m.to_location_id = None;
        assert!(m.validate().is_err());
        m.client_id = Some(Uuid::now_v7());
        m.validate().unwrap();
    }
}
