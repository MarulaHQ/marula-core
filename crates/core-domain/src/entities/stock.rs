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

/// The seven live custody classes (ADR 0035 §4.1). A unit is in exactly one
/// of these or in one terminal disposition below, never in two.
pub const CUSTODY_CLASSES: [&str; 7] = [
    "sale_stock",
    "internal_fleet",
    "rental_in",
    "rental_out",
    "client_custody",
    "ewaste_custody",
    "tools",
];

/// Terminal dispositions. A disposed unit keeps its identity and its serial or
/// asset row; what changes is that it no longer sits in a live class.
pub const TERMINAL_DISPOSITIONS: [&str; 4] = ["sold", "recycled", "destroyed", "returned"];

/// Every value the `custody_class` columns admit, live and terminal. The DB
/// CHECK in migration 0232 names this same list.
pub const UNIT_CLASSES: [&str; 11] = [
    "sale_stock",
    "internal_fleet",
    "rental_in",
    "rental_out",
    "client_custody",
    "ewaste_custody",
    "tools",
    "sold",
    "recycled",
    "destroyed",
    "returned",
];

pub fn is_valid_custody_class(class: &str) -> bool {
    UNIT_CLASSES.contains(&class.trim())
}

pub fn is_live_custody_class(class: &str) -> bool {
    CUSTODY_CLASSES.contains(&class.trim())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CustodyClass {
    SaleStock,
    InternalFleet,
    RentalIn,
    RentalOut,
    ClientCustody,
    EwasteCustody,
    Tools,
    Sold,
    Recycled,
    Destroyed,
    Returned,
}

impl CustodyClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            CustodyClass::SaleStock => "sale_stock",
            CustodyClass::InternalFleet => "internal_fleet",
            CustodyClass::RentalIn => "rental_in",
            CustodyClass::RentalOut => "rental_out",
            CustodyClass::ClientCustody => "client_custody",
            CustodyClass::EwasteCustody => "ewaste_custody",
            CustodyClass::Tools => "tools",
            CustodyClass::Sold => "sold",
            CustodyClass::Recycled => "recycled",
            CustodyClass::Destroyed => "destroyed",
            CustodyClass::Returned => "returned",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        match raw.trim() {
            "sale_stock" => Ok(Self::SaleStock),
            "internal_fleet" => Ok(Self::InternalFleet),
            "rental_in" => Ok(Self::RentalIn),
            "rental_out" => Ok(Self::RentalOut),
            "client_custody" => Ok(Self::ClientCustody),
            "ewaste_custody" => Ok(Self::EwasteCustody),
            "tools" => Ok(Self::Tools),
            "sold" => Ok(Self::Sold),
            "recycled" => Ok(Self::Recycled),
            "destroyed" => Ok(Self::Destroyed),
            "returned" => Ok(Self::Returned),
            other => Err(DomainError::Validation(format!(
                "invalid custody class: {other}"
            ))),
        }
    }

    /// A terminal disposition closes the unit's life; nothing moves back out of
    /// it, and a unit in one never enters owned levels or valuation.
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            CustodyClass::Sold
                | CustodyClass::Recycled
                | CustodyClass::Destroyed
                | CustodyClass::Returned
        )
    }

    /// True when the organisation owns the unit and it may sit in owned
    /// quantity or valuation. E-waste custody, client custody and vendor
    /// rentals are all somebody else's goods.
    pub const fn counts_as_owned(self) -> bool {
        matches!(
            self,
            CustodyClass::SaleStock
                | CustodyClass::InternalFleet
                | CustodyClass::RentalOut
                | CustodyClass::Tools
        )
    }
}

/// Which projection of a unit a class movement names. A lot is a line plus a
/// quantity (ADR 0035 §1); its table arrives with the e-waste work and the
/// reference is already a UUID, never a free-text key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    Serial,
    Asset,
    Lot,
}

impl UnitKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            UnitKind::Serial => "serial",
            UnitKind::Asset => "asset",
            UnitKind::Lot => "lot",
        }
    }
}

/// The identity a movement points at: one kind and one id, never two
/// references. A lot id is the id of the lot line row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UnitReference {
    pub kind: UnitKind,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
}

impl UnitReference {
    pub const fn serial(id: Uuid) -> Self {
        Self {
            kind: UnitKind::Serial,
            id,
        }
    }

    pub const fn asset(id: Uuid) -> Self {
        Self {
            kind: UnitKind::Asset,
            id,
        }
    }

    pub const fn lot(id: Uuid) -> Self {
        Self {
            kind: UnitKind::Lot,
            id,
        }
    }
}

/// What caused a class change. `manual` is the honest source for an operator
/// correction and carries no source id; every other kind names the document.
pub const CLASS_TRANSITION_SOURCES: [&str; 9] = [
    "purchase_receipt_line",
    "stock_movement",
    "device_asset",
    "client_loan",
    "tool_checkout",
    "credit_line",
    "ewaste_intake",
    "ewaste_disposition",
    "manual",
];

pub fn is_valid_class_transition_source(source: &str) -> bool {
    CLASS_TRANSITION_SOURCES.contains(&source.trim())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ClassTransitionSource {
    PurchaseReceiptLine,
    StockMovement,
    DeviceAsset,
    ClientLoan,
    ToolCheckout,
    CreditLine,
    EwasteIntake,
    EwasteDisposition,
    Manual,
}

impl ClassTransitionSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            ClassTransitionSource::PurchaseReceiptLine => "purchase_receipt_line",
            ClassTransitionSource::StockMovement => "stock_movement",
            ClassTransitionSource::DeviceAsset => "device_asset",
            ClassTransitionSource::ClientLoan => "client_loan",
            ClassTransitionSource::ToolCheckout => "tool_checkout",
            ClassTransitionSource::CreditLine => "credit_line",
            ClassTransitionSource::EwasteIntake => "ewaste_intake",
            ClassTransitionSource::EwasteDisposition => "ewaste_disposition",
            ClassTransitionSource::Manual => "manual",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        match raw.trim() {
            "purchase_receipt_line" => Ok(Self::PurchaseReceiptLine),
            "stock_movement" => Ok(Self::StockMovement),
            "device_asset" => Ok(Self::DeviceAsset),
            "client_loan" => Ok(Self::ClientLoan),
            "tool_checkout" => Ok(Self::ToolCheckout),
            "credit_line" => Ok(Self::CreditLine),
            "ewaste_intake" => Ok(Self::EwasteIntake),
            "ewaste_disposition" => Ok(Self::EwasteDisposition),
            "manual" => Ok(Self::Manual),
            other => Err(DomainError::Validation(format!(
                "invalid class transition source: {other}"
            ))),
        }
    }

    pub const fn is_manual(self) -> bool {
        matches!(self, ClassTransitionSource::Manual)
    }
}

/// One append-only row of the class-transition ledger (migration 0232). The row
/// is the movement; the unit's `custody_class` column is its consequence.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CustodyClassMovement {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub unit: UnitReference,
    pub from_class: CustodyClass,
    pub to_class: CustodyClass,
    pub reason: String,
    pub source: ClassTransitionSource,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub source_id: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

impl CustodyClassMovement {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.from_class == self.to_class {
            return Err(DomainError::Validation(
                "a class movement must change the class".into(),
            ));
        }
        if self.from_class.is_terminal() {
            return Err(DomainError::Validation(
                "a terminal disposition cannot move again".into(),
            ));
        }
        if self.reason.trim().is_empty() {
            return Err(DomainError::Validation(
                "a class movement needs a reason".into(),
            ));
        }
        if self.reason.chars().count() > 500 {
            return Err(DomainError::Validation(
                "a class movement reason must be at most 500 characters".into(),
            ));
        }
        match (self.source.is_manual(), self.source_id) {
            (true, None) => {}
            (true, Some(_)) => {
                return Err(DomainError::Validation(
                    "a manual class movement carries no source id".into(),
                ))
            }
            (false, None) => {
                return Err(DomainError::Validation(
                    "a class movement needs the document that caused it".into(),
                ))
            }
            (false, Some(_)) => {}
        }
        if self.unit.kind == UnitKind::Lot {
            return Err(DomainError::Validation(
                "a lot class movement needs the e-waste lot table; the ledger admits serial and asset units today"
                    .into(),
            ));
        }
        Ok(())
    }
}

/// The class a new serial row starts in, from the vocabulary the row already
/// carries. This is the backfill rule in migration 0232 and the reason a tool
/// serial never starts life as sale stock.
pub fn initial_serial_class(is_tool: bool, has_client: bool) -> CustodyClass {
    if is_tool {
        CustodyClass::Tools
    } else if has_client {
        CustodyClass::RentalOut
    } else {
        CustodyClass::SaleStock
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

    fn class_movement() -> CustodyClassMovement {
        CustodyClassMovement {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            unit: UnitReference::asset(Uuid::now_v7()),
            from_class: CustodyClass::InternalFleet,
            to_class: CustodyClass::SaleStock,
            reason: "decommissioned laptop returned to sale stock".into(),
            source: ClassTransitionSource::Manual,
            source_id: None,
            created_by: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn every_custody_class_round_trips_and_stays_one_of_the_two_vocabularies() {
        for class in UNIT_CLASSES {
            let parsed = CustodyClass::parse(class).expect("known class parses");
            assert_eq!(parsed.as_str(), class);
            assert!(is_valid_custody_class(class));
            assert_eq!(
                is_live_custody_class(class),
                !parsed.is_terminal(),
                "a class is either live or terminal, never both"
            );
        }
        assert_eq!(
            UNIT_CLASSES.len(),
            CUSTODY_CLASSES.len() + TERMINAL_DISPOSITIONS.len()
        );
        assert!(CustodyClass::parse("in_stock").is_err());
    }

    #[test]
    fn a_terminal_disposition_is_not_owned_quantity() {
        for class in TERMINAL_DISPOSITIONS {
            let parsed = CustodyClass::parse(class).unwrap();
            assert!(parsed.is_terminal());
            assert!(!parsed.counts_as_owned(), "{class} is not owned stock");
        }
        assert!(!CustodyClass::EwasteCustody.counts_as_owned());
        assert!(!CustodyClass::ClientCustody.counts_as_owned());
        assert!(!CustodyClass::RentalIn.counts_as_owned());
        assert!(CustodyClass::SaleStock.counts_as_owned());
        assert!(CustodyClass::Tools.counts_as_owned());
    }

    #[test]
    fn a_new_tool_serial_starts_in_the_tools_class() {
        assert_eq!(initial_serial_class(true, false), CustodyClass::Tools);
        assert_eq!(initial_serial_class(false, true), CustodyClass::RentalOut);
        assert_eq!(initial_serial_class(false, false), CustodyClass::SaleStock);
    }

    #[test]
    fn a_class_movement_needs_a_change_a_reason_and_a_source() {
        class_movement().validate().unwrap();

        let mut m = class_movement();
        m.to_class = m.from_class;
        assert!(m.validate().is_err(), "no movement, no row");

        let mut m = class_movement();
        m.reason = "   ".into();
        assert!(m.validate().is_err(), "a reason is required");

        let mut m = class_movement();
        m.source = ClassTransitionSource::PurchaseReceiptLine;
        assert!(
            m.validate().is_err(),
            "a document source must name the document"
        );
        m.source_id = Some(Uuid::now_v7());
        m.validate().unwrap();

        let mut m = class_movement();
        m.source_id = Some(Uuid::now_v7());
        assert!(m.validate().is_err(), "manual carries no source id");

        let mut m = class_movement();
        m.from_class = CustodyClass::Destroyed;
        assert!(
            m.validate().is_err(),
            "a terminal disposition cannot move again"
        );
    }

    #[test]
    fn a_lot_movement_is_refused_until_the_lot_table_exists() {
        let mut m = class_movement();
        m.unit = UnitReference::lot(Uuid::now_v7());
        assert!(
            m.validate().is_err(),
            "no lot ledger exists yet, so no lot movement is accepted"
        );
    }
}
