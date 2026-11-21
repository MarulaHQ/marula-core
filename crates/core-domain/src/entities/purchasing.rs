//! Purchasing + vendors (Wave 32 lane 2, `p32-purchasing`).
//!
//! The BUY side of lane 1's stock story: typed vendors the MSP orders
//! from, purchase orders with a typed lifecycle, receipt documents that
//! post into the lane-1 `stock_movements` ledger at a chosen location,
//! typed external links (reseller order portals / parcel tracking pages),
//! and the bill hook that carries a purchase into project cost rollups.
//!
//! Conventions: quantities are `*_milli` thousandths-integers and money
//! integer minor units + ISO 4217 (floats never touch either, §5); line
//! amounts ride [`crate::entities::line_amount_minor`] (half-even, parity
//! with quote lines); the ADR 0002 FX lock moment for a bill is POSTING.

use super::client::Ownership;
use super::costs::RechargeRule;
use super::stock::CustodyClass;
use crate::entities::line_amount_minor;
use crate::error::DomainError;
use crate::ids::OrgId;
use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;

/// The PO lifecycle: draft → ordered → partially_received → received →
/// closed | cancelled. Receipt legs move ordered → partially_received →
/// received; close records 'full' or 'short' (under-receipt).
pub const PO_STATUSES: [&str; 6] = [
    "draft",
    "ordered",
    "partially_received",
    "received",
    "closed",
    "cancelled",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PurchaseOrderStatus {
    Draft,
    Ordered,
    PartiallyReceived,
    Received,
    Closed,
    Cancelled,
}

impl PurchaseOrderStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            PurchaseOrderStatus::Draft => "draft",
            PurchaseOrderStatus::Ordered => "ordered",
            PurchaseOrderStatus::PartiallyReceived => "partially_received",
            PurchaseOrderStatus::Received => "received",
            PurchaseOrderStatus::Closed => "closed",
            PurchaseOrderStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        match raw {
            "draft" => Ok(Self::Draft),
            "ordered" => Ok(Self::Ordered),
            "partially_received" => Ok(Self::PartiallyReceived),
            "received" => Ok(Self::Received),
            "closed" => Ok(Self::Closed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(DomainError::Validation(format!(
                "invalid purchase order status: {other}"
            ))),
        }
    }

    /// Whether a status change `self → to` is legal. Receipt legs are NOT
    /// transitions here — the receipt engine advances the status from the
    /// line states; these are the operator moves.
    pub fn can_transition_to(self, to: Self) -> bool {
        match self {
            PurchaseOrderStatus::Draft => {
                matches!(
                    to,
                    PurchaseOrderStatus::Ordered | PurchaseOrderStatus::Cancelled
                )
            }
            PurchaseOrderStatus::Ordered => matches!(
                to,
                PurchaseOrderStatus::PartiallyReceived
                    | PurchaseOrderStatus::Received
                    | PurchaseOrderStatus::Cancelled
            ),
            PurchaseOrderStatus::PartiallyReceived => matches!(
                to,
                PurchaseOrderStatus::Received | PurchaseOrderStatus::Closed
            ),
            PurchaseOrderStatus::Received => matches!(to, PurchaseOrderStatus::Closed),
            PurchaseOrderStatus::Closed | PurchaseOrderStatus::Cancelled => false,
        }
    }

    /// Terminal states accept no further moves.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            PurchaseOrderStatus::Closed | PurchaseOrderStatus::Cancelled
        )
    }
}

/// Per-line receipt consequence, maintained by the receipt engine in the
/// same commit as the ledger row it posts.
pub const RECEIPT_STATES: [&str; 4] = ["pending", "partial", "complete", "over"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReceiptState {
    Pending,
    Partial,
    Complete,
    Over,
}

impl ReceiptState {
    pub const fn as_str(self) -> &'static str {
        match self {
            ReceiptState::Pending => "pending",
            ReceiptState::Partial => "partial",
            ReceiptState::Complete => "complete",
            ReceiptState::Over => "over",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        match raw {
            "pending" => Ok(Self::Pending),
            "partial" => Ok(Self::Partial),
            "complete" => Ok(Self::Complete),
            "over" => Ok(Self::Over),
            other => Err(DomainError::Validation(format!(
                "invalid receipt state: {other}"
            ))),
        }
    }
}

/// The typed consequence of receiving `received_milli` against
/// `ordered_milli`: zero is pending, short is partial, exact is complete,
/// and beyond is over — over/under receipts are typed states, never
/// silent.
pub fn receipt_state_for(ordered_milli: i64, received_milli: i64) -> ReceiptState {
    if received_milli <= 0 {
        ReceiptState::Pending
    } else if received_milli < ordered_milli {
        ReceiptState::Partial
    } else if received_milli == ordered_milli {
        ReceiptState::Complete
    } else {
        ReceiptState::Over
    }
}

/// Close disposition: 'full' when every line is complete/over, 'short'
/// when unreceived quantity stays behind (the typed under-receipt record).
pub const CLOSE_KINDS: [&str; 2] = ["full", "short"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CloseKind {
    Full,
    Short,
}

impl CloseKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            CloseKind::Full => "full",
            CloseKind::Short => "short",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        match raw {
            "full" => Ok(Self::Full),
            "short" => Ok(Self::Short),
            other => Err(DomainError::Validation(format!(
                "invalid close kind: {other}"
            ))),
        }
    }
}

/// Typed external-link kinds (the asset integration-links pattern:
/// document_links + LinkKind — a closed kind enum, org-scoped,
/// subject-bound).
pub const EXTERNAL_LINK_KINDS: [&str; 4] = ["order_portal", "tracking", "invoice", "other"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExternalLinkKind {
    OrderPortal,
    Tracking,
    Invoice,
    Other,
}

impl ExternalLinkKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            ExternalLinkKind::OrderPortal => "order_portal",
            ExternalLinkKind::Tracking => "tracking",
            ExternalLinkKind::Invoice => "invoice",
            ExternalLinkKind::Other => "other",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        match raw {
            "order_portal" => Ok(Self::OrderPortal),
            "tracking" => Ok(Self::Tracking),
            "invoice" => Ok(Self::Invoice),
            "other" => Ok(Self::Other),
            other => Err(DomainError::Validation(format!(
                "invalid external link kind: {other}"
            ))),
        }
    }
}

pub const EXTERNAL_LINK_SUBJECTS: [&str; 4] = [
    "vendor",
    "purchase_order",
    "purchase_order_line",
    "device_asset",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExternalLinkSubject {
    Vendor,
    PurchaseOrder,
    PurchaseOrderLine,
    DeviceAsset,
}

impl ExternalLinkSubject {
    pub const fn as_str(self) -> &'static str {
        match self {
            ExternalLinkSubject::Vendor => "vendor",
            ExternalLinkSubject::PurchaseOrder => "purchase_order",
            ExternalLinkSubject::PurchaseOrderLine => "purchase_order_line",
            ExternalLinkSubject::DeviceAsset => "device_asset",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        match raw {
            "vendor" => Ok(Self::Vendor),
            "purchase_order" => Ok(Self::PurchaseOrder),
            "purchase_order_line" => Ok(Self::PurchaseOrderLine),
            "device_asset" => Ok(Self::DeviceAsset),
            other => Err(DomainError::Validation(format!(
                "invalid external link subject: {other}"
            ))),
        }
    }
}

/// Only https links are recorded: deeplinks open in a browser from staff
/// surfaces, so plain http and non-web schemes are rejected at validation.
pub fn is_valid_external_url(url: &str) -> bool {
    let trimmed = url.trim();
    trimmed.starts_with("https://")
        && trimmed.len() > "https://".len()
        && !trimmed.chars().any(char::is_whitespace)
}

fn validate_currency(code: &str) -> Result<(), DomainError> {
    if code.len() != 3 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(DomainError::Validation(format!(
            "invalid ISO 4217 currency code: {code}"
        )));
    }
    Ok(())
}

/// Acquisition intent (ADR 0035 §2, §5.1). A purchase order may mix intents:
/// the header carries defaults and the line decides. These are the words, the
/// destination each word implies, and the validation that keeps a line from
/// claiming a destination its kind cannot reach. The purchasing work wires them
/// to columns and routes; nothing here changes a PO today.
pub const ACQUIRED_FOR_VALUES: [&str; 8] = [
    "sale_stock",
    "internal_fleet",
    "rental_pool",
    "vendor_rental",
    "client_stocked",
    "client_drop_ship",
    "project_consumable",
    "ewaste_service",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AcquiredFor {
    /// Bought to sell: lands in `sale_stock`.
    SaleStock,
    /// The organisation's own equipment: lands as an `internal_fleet` asset.
    InternalFleet,
    /// Bought for the rental pool: lands as a `rental_out` asset.
    RentalPool,
    /// Rented from a vendor: lands as a `rental_in` asset at `ownership = rented`.
    VendorRental,
    /// Bought for a client and stocked for them: `client_custody`.
    ClientStocked,
    /// Bought for a client and shipped direct: `client_custody`, no owned level.
    ClientDropShip,
    /// A consumable for a project or ticket: stocked, then consumed.
    ProjectConsumable,
    /// E-waste service or intake: custody only.
    EwasteService,
}

impl AcquiredFor {
    pub const fn as_str(self) -> &'static str {
        match self {
            AcquiredFor::SaleStock => "sale_stock",
            AcquiredFor::InternalFleet => "internal_fleet",
            AcquiredFor::RentalPool => "rental_pool",
            AcquiredFor::VendorRental => "vendor_rental",
            AcquiredFor::ClientStocked => "client_stocked",
            AcquiredFor::ClientDropShip => "client_drop_ship",
            AcquiredFor::ProjectConsumable => "project_consumable",
            AcquiredFor::EwasteService => "ewaste_service",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        match raw.trim() {
            "sale_stock" => Ok(Self::SaleStock),
            "internal_fleet" => Ok(Self::InternalFleet),
            "rental_pool" => Ok(Self::RentalPool),
            "vendor_rental" => Ok(Self::VendorRental),
            "client_stocked" => Ok(Self::ClientStocked),
            "client_drop_ship" => Ok(Self::ClientDropShip),
            "project_consumable" => Ok(Self::ProjectConsumable),
            "ewaste_service" => Ok(Self::EwasteService),
            other => Err(DomainError::Validation(format!(
                "invalid acquired_for: {other}"
            ))),
        }
    }

    /// Where a receipt for this intent lands. One intent, one destination
    /// class; a mixed PO routes per line.
    pub const fn destination_class(self) -> CustodyClass {
        match self {
            AcquiredFor::SaleStock | AcquiredFor::ProjectConsumable => CustodyClass::SaleStock,
            AcquiredFor::InternalFleet => CustodyClass::InternalFleet,
            AcquiredFor::RentalPool => CustodyClass::RentalOut,
            AcquiredFor::VendorRental => CustodyClass::RentalIn,
            AcquiredFor::ClientStocked | AcquiredFor::ClientDropShip => CustodyClass::ClientCustody,
            AcquiredFor::EwasteService => CustodyClass::EwasteCustody,
        }
    }

    /// A purchase made on a client's behalf is the one that resolves a recharge
    /// rule (§2, §5.3).
    pub const fn is_client_purchase(self) -> bool {
        matches!(
            self,
            AcquiredFor::ClientStocked | AcquiredFor::ClientDropShip
        )
    }
}

pub const DELIVERY_MODES: [&str; 3] = ["to_stock", "drop_ship_to_client", "to_site"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    ToStock,
    DropShipToClient,
    ToSite,
}

impl DeliveryMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            DeliveryMode::ToStock => "to_stock",
            DeliveryMode::DropShipToClient => "drop_ship_to_client",
            DeliveryMode::ToSite => "to_site",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        match raw.trim() {
            "to_stock" => Ok(Self::ToStock),
            "drop_ship_to_client" => Ok(Self::DropShipToClient),
            "to_site" => Ok(Self::ToSite),
            other => Err(DomainError::Validation(format!(
                "invalid delivery mode: {other}"
            ))),
        }
    }
}

/// What one PO line purchases. `pass_through` is bought and recharged without
/// ever entering owned quantity; that is the defect the schema prevents by
/// validation, not by remembering.
pub const LINE_KINDS: [&str; 6] = [
    "stock",
    "serialized_unit",
    "asset",
    "rental_unit",
    "service",
    "pass_through",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    Stock,
    SerializedUnit,
    Asset,
    RentalUnit,
    Service,
    PassThrough,
}

impl LineKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            LineKind::Stock => "stock",
            LineKind::SerializedUnit => "serialized_unit",
            LineKind::Asset => "asset",
            LineKind::RentalUnit => "rental_unit",
            LineKind::Service => "service",
            LineKind::PassThrough => "pass_through",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        match raw.trim() {
            "stock" => Ok(Self::Stock),
            "serialized_unit" => Ok(Self::SerializedUnit),
            "asset" => Ok(Self::Asset),
            "rental_unit" => Ok(Self::RentalUnit),
            "service" => Ok(Self::Service),
            "pass_through" => Ok(Self::PassThrough),
            other => Err(DomainError::Validation(format!(
                "invalid line kind: {other}"
            ))),
        }
    }

    /// A physical unit a person can hold needs a serial captured at receipt.
    pub const fn is_serialised(self) -> bool {
        matches!(
            self,
            LineKind::SerializedUnit | LineKind::Asset | LineKind::RentalUnit
        )
    }

    /// A pass-through is never owned: it cannot land in a class the
    /// organisation values.
    pub const fn can_enter_owned_quantity(self) -> bool {
        !matches!(self, LineKind::PassThrough | LineKind::Service)
    }
}

/// The header defaults a PO carries (§5.1). The line can override the class,
/// not the intent the money is recharged under.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PurchaseIntent {
    pub acquired_for: AcquiredFor,
    pub ownership: Option<Ownership>,
    pub delivery_mode: DeliveryMode,
    pub recharge_rule: Option<RechargeRule>,
}

impl PurchaseIntent {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.acquired_for == AcquiredFor::ClientDropShip
            && self.delivery_mode != DeliveryMode::DropShipToClient
        {
            return Err(DomainError::Validation(
                "a drop-ship purchase delivers straight to the client".into(),
            ));
        }
        if self.delivery_mode == DeliveryMode::DropShipToClient
            && !self.acquired_for.is_client_purchase()
        {
            return Err(DomainError::Validation(
                "only a client purchase can drop-ship to a client".into(),
            ));
        }
        if self.ownership == Some(Ownership::Rented)
            && self.acquired_for != AcquiredFor::VendorRental
        {
            return Err(DomainError::Validation(
                "a rented unit is a vendor rental".into(),
            ));
        }
        if self.acquired_for == AcquiredFor::VendorRental
            && self.ownership != Some(Ownership::Rented)
        {
            return Err(DomainError::Validation(
                "a vendor rental records ownership rented".into(),
            ));
        }
        match (&self.recharge_rule, self.acquired_for.is_client_purchase()) {
            (Some(rule), true) => rule.validate()?,
            (Some(_), false) => {
                return Err(DomainError::Validation(
                    "a recharge rule applies only to a purchase made for a client".into(),
                ))
            }
            (None, _) => {}
        }
        Ok(())
    }

    /// Validates one line against the header it belongs to. The header alone
    /// cannot see a line, and the line alone cannot see the delivery mode, so
    /// the one rule that needs both lives here: goods that ship straight to a
    /// client never enter owned quantity (ADR 0035 §2).
    pub fn validate_line(&self, line: &PurchaseLineIntent) -> Result<(), DomainError> {
        if self.delivery_mode == DeliveryMode::DropShipToClient
            && line.destination_class.counts_as_owned()
        {
            return Err(DomainError::Validation(format!(
                "a drop-ship purchase cannot land a line in {} — drop-shipped goods never enter owned quantity",
                line.destination_class.as_str()
            )));
        }
        Ok(())
    }
}

/// One line's intent and the fields that decide where it lands (§5.1).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PurchaseLineIntent {
    pub line_kind: LineKind,
    pub destination_class: CustodyClass,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub asset_layout_id: Option<Uuid>,
    pub serial_expected: bool,
    pub weight_g: Option<i32>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub sell_price_minor: Option<i64>,
    pub billable: bool,
}

impl PurchaseLineIntent {
    pub fn validate(&self) -> Result<(), DomainError> {
        if let Some(weight) = self.weight_g {
            if weight <= 0 {
                return Err(DomainError::Validation(
                    "line weight must be positive grams".into(),
                ));
            }
        }
        if let Some(price) = self.sell_price_minor {
            if price < 0 {
                return Err(DomainError::Validation(
                    "line sell price must not be negative".into(),
                ));
            }
        }
        if self.serial_expected && !self.line_kind.is_serialised() {
            return Err(DomainError::Validation(
                "only a serialised unit, asset or rental unit expects a serial".into(),
            ));
        }
        if self.line_kind == LineKind::SerializedUnit && !self.serial_expected {
            return Err(DomainError::Validation(
                "a serialized_unit line expects a serial".into(),
            ));
        }
        if !self.line_kind.can_enter_owned_quantity() && self.destination_class.counts_as_owned() {
            return Err(DomainError::Validation(
                "a pass-through or service line cannot land in owned quantity".into(),
            ));
        }
        Ok(())
    }
}

/// One org-owned supplier/reseller.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Vendor {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub name: String,
    pub code: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    /// The vendor's account number for THIS org.
    pub account_number: Option<String>,
    /// ISO 4217 default for POs to this vendor.
    pub default_currency: String,
    pub notes: Option<String>,
    pub active: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Vendor {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.name.trim().is_empty() {
            return Err(DomainError::Validation("vendor name is required".into()));
        }
        if self.name.len() > 200 {
            return Err(DomainError::Validation(
                "vendor name must be at most 200 characters".into(),
            ));
        }
        if let Some(code) = &self.code {
            if code.len() > 64 {
                return Err(DomainError::Validation(
                    "vendor code must be at most 64 characters".into(),
                ));
            }
        }
        if let Some(notes) = &self.notes {
            if notes.chars().count() > 2000 {
                return Err(DomainError::Validation(
                    "vendor notes must be at most 2000 characters".into(),
                ));
            }
        }
        validate_currency(&self.default_currency)
    }
}

/// A typed contact on a vendor.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct VendorContact {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vendor_id: Uuid,
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub role: Option<String>,
    pub is_primary: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl VendorContact {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.name.trim().is_empty() {
            return Err(DomainError::Validation("contact name is required".into()));
        }
        if self.name.len() > 200 {
            return Err(DomainError::Validation(
                "contact name must be at most 200 characters".into(),
            ));
        }
        if self.email.as_deref().is_some_and(|e| e.len() > 254) {
            return Err(DomainError::Validation(
                "contact email must be at most 254 characters".into(),
            ));
        }
        Ok(())
    }
}

/// A typed external URL bound to a subject (vendor / PO / PO line /
/// device asset). No polymorphic FK — the write path checks that the
/// subject row exists in the org.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExternalLink {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub subject_kind: ExternalLinkSubject,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub subject_id: Uuid,
    pub kind: ExternalLinkKind,
    pub url: String,
    pub label: Option<String>,
    /// Carrier that owns a tracking page (kind = tracking).
    pub carrier: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ExternalLink {
    pub fn validate(&self) -> Result<(), DomainError> {
        if !is_valid_external_url(&self.url) {
            return Err(DomainError::Validation(
                "external links must be https URLs".into(),
            ));
        }
        if self.url.len() > 2000 {
            return Err(DomainError::Validation(
                "external link url must be at most 2000 characters".into(),
            ));
        }
        if self.label.as_deref().is_some_and(|l| l.len() > 200) {
            return Err(DomainError::Validation(
                "external link label must be at most 200 characters".into(),
            ));
        }
        Ok(())
    }
}

/// One purchase order header.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PurchaseOrder {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vendor_id: Uuid,
    pub number: String,
    pub status: PurchaseOrderStatus,
    /// ISO 4217; every line cost is in this currency.
    pub currency: String,
    /// The vendor's own order reference.
    pub vendor_reference: Option<String>,
    /// Cost-object hook for project cost rollups (NULL = unassigned).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub project_id: Option<Uuid>,
    pub notes: Option<String>,
    pub ordered_at: Option<DateTime<Utc>>,
    pub expected_at: Option<NaiveDate>,
    pub closed_at: Option<DateTime<Utc>>,
    pub close_kind: Option<CloseKind>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub cancel_reason: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl PurchaseOrder {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.number.trim().is_empty() {
            return Err(DomainError::Validation(
                "purchase order number is required".into(),
            ));
        }
        if self.number.len() > 40 {
            return Err(DomainError::Validation(
                "purchase order number must be at most 40 characters".into(),
            ));
        }
        if let Some(notes) = &self.notes {
            if notes.chars().count() > 4000 {
                return Err(DomainError::Validation(
                    "purchase order notes must be at most 4000 characters".into(),
                ));
            }
        }
        validate_currency(&self.currency)
    }
}

/// One PO line: quantity at milli precision, cost in minor units.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PurchaseOrderLine {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub purchase_order_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub stock_item_id: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub catalog_item_id: Option<Uuid>,
    pub description: String,
    pub qty_ordered_milli: i64,
    pub qty_received_milli: i64,
    pub unit_cost_minor: i64,
    pub receipt_state: ReceiptState,
    pub position: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl PurchaseOrderLine {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.description.trim().is_empty() {
            return Err(DomainError::Validation(
                "line description is required".into(),
            ));
        }
        if self.description.len() > 500 {
            return Err(DomainError::Validation(
                "line description must be at most 500 characters".into(),
            ));
        }
        if self.qty_ordered_milli <= 0 {
            return Err(DomainError::Validation(
                "line ordered quantity must be positive".into(),
            ));
        }
        if self.qty_received_milli < 0 {
            return Err(DomainError::Validation(
                "line received quantity must not be negative".into(),
            ));
        }
        if self.unit_cost_minor < 0 {
            return Err(DomainError::Validation(
                "line unit cost must not be negative".into(),
            ));
        }
        Ok(())
    }

    /// Ordered amount in minor units (half-even, quote-line parity).
    pub fn ordered_amount_minor(&self) -> Result<i64, DomainError> {
        line_amount_minor(self.qty_ordered_milli, self.unit_cost_minor)
    }

    /// Received amount in minor units; zero when nothing arrived.
    pub fn received_amount_minor(&self) -> Result<i64, DomainError> {
        if self.qty_received_milli == 0 {
            return Ok(0);
        }
        line_amount_minor(self.qty_received_milli, self.unit_cost_minor)
    }
}

/// The rolled-up money on a PO: ordered and received amounts in the PO
/// currency, integer minor units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PurchaseOrderRollup {
    pub ordered_minor: i64,
    pub received_minor: i64,
}

/// Sums the line amounts; overflow is a domain error, never a wrap.
pub fn po_rollup(lines: &[PurchaseOrderLine]) -> Result<PurchaseOrderRollup, DomainError> {
    let mut ordered: i128 = 0;
    let mut received: i128 = 0;
    for line in lines {
        line.validate()?;
        ordered += i128::from(line.ordered_amount_minor()?);
        received += i128::from(line.received_amount_minor()?);
    }
    Ok(PurchaseOrderRollup {
        ordered_minor: i64::try_from(ordered)
            .map_err(|_| DomainError::MoneyOverflow(ordered.to_string(), "order total".into()))?,
        received_minor: i64::try_from(received).map_err(|_| {
            DomainError::MoneyOverflow(received.to_string(), "received total".into())
        })?,
    })
}

/// One receipt document: goods that landed at a location against a PO.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PurchaseReceipt {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub purchase_order_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub location_id: Uuid,
    pub carrier: Option<String>,
    pub tracking_ref: Option<String>,
    pub note: Option<String>,
    pub received_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// One receipt line: what moved, and the ledger row it posted.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PurchaseReceiptLine {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub receipt_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub po_line_id: Uuid,
    pub qty_milli: i64,
    /// The lane-1 append-only ledger row this line posted.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub movement_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// The vendor bill: recorded as a draft, posted (FX-locked) into project
/// cost rollups.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PurchaseBill {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub purchase_order_id: Uuid,
    pub bill_number: Option<String>,
    pub amount_minor: i64,
    pub currency: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub fx_rate_used: Option<String>,
    pub fx_rate_date: Option<NaiveDate>,
    pub fx_source: Option<String>,
    pub base_amount_minor: Option<i64>,
    pub posted_at: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_states_are_typed_for_the_whole_range() {
        assert_eq!(receipt_state_for(1_000, 0), ReceiptState::Pending);
        assert_eq!(receipt_state_for(1_000, 400), ReceiptState::Partial);
        assert_eq!(receipt_state_for(1_000, 1_000), ReceiptState::Complete);
        assert_eq!(receipt_state_for(1_000, 1_500), ReceiptState::Over);
    }

    #[test]
    fn lifecycle_allows_the_documented_moves_only() {
        use PurchaseOrderStatus::*;
        assert!(Draft.can_transition_to(Ordered));
        assert!(Draft.can_transition_to(Cancelled));
        assert!(Ordered.can_transition_to(PartiallyReceived));
        assert!(Ordered.can_transition_to(Received));
        assert!(Ordered.can_transition_to(Cancelled));
        assert!(PartiallyReceived.can_transition_to(Closed));
        assert!(Received.can_transition_to(Closed));
        assert!(!PartiallyReceived.can_transition_to(Cancelled));
        assert!(!Closed.can_transition_to(Received));
        assert!(!Cancelled.can_transition_to(Ordered));
    }

    #[test]
    fn external_links_require_https() {
        assert!(is_valid_external_url("https://reseller.example/orders/1"));
        assert!(!is_valid_external_url("http://reseller.example"));
        assert!(!is_valid_external_url("javascript:alert(1)"));
        assert!(!is_valid_external_url("https://"));
        assert!(!is_valid_external_url("https://bad url"));
    }

    #[test]
    fn rollup_uses_half_even_line_math() {
        let line = |qty: i64, received: i64, cost: i64| PurchaseOrderLine {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            purchase_order_id: Uuid::now_v7(),
            stock_item_id: None,
            catalog_item_id: None,
            description: "line".into(),
            qty_ordered_milli: qty,
            qty_received_milli: received,
            unit_cost_minor: cost,
            receipt_state: receipt_state_for(qty, received),
            position: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        // 2_500 × 3 = 7_500 milli → 7.5 minor; half-even takes the odd
        // floor (7) up to 8. Received: 1_000 × 3 → exactly 3.
        let rolled = po_rollup(&[line(2_500, 1_000, 3)]).unwrap();
        assert_eq!(rolled.ordered_minor, 8);
        assert_eq!(rolled.received_minor, 3);
        // Nothing received stays zero without erroring.
        let rolled = po_rollup(&[line(1_000, 0, 100)]).unwrap();
        assert_eq!(rolled.received_minor, 0);
    }

    fn intent() -> PurchaseIntent {
        PurchaseIntent {
            acquired_for: AcquiredFor::SaleStock,
            ownership: Some(Ownership::Owned),
            delivery_mode: DeliveryMode::ToStock,
            recharge_rule: None,
        }
    }

    fn line_intent() -> PurchaseLineIntent {
        PurchaseLineIntent {
            line_kind: LineKind::Stock,
            destination_class: CustodyClass::SaleStock,
            asset_layout_id: None,
            serial_expected: false,
            weight_g: Some(1_200),
            sell_price_minor: Some(4_900),
            billable: false,
        }
    }

    #[test]
    fn each_intent_names_one_destination_class() {
        for value in ACQUIRED_FOR_VALUES {
            let acquired = AcquiredFor::parse(value).unwrap();
            assert_eq!(acquired.as_str(), value);
            assert!(
                is_live_destination(acquired.destination_class()),
                "{value} must land in a live class"
            );
        }
        assert_eq!(
            AcquiredFor::VendorRental.destination_class(),
            CustodyClass::RentalIn
        );
        assert_eq!(
            AcquiredFor::RentalPool.destination_class(),
            CustodyClass::RentalOut
        );
        assert_eq!(
            AcquiredFor::ClientDropShip.destination_class(),
            CustodyClass::ClientCustody
        );
    }

    fn is_live_destination(class: CustodyClass) -> bool {
        !class.is_terminal()
    }

    #[test]
    fn a_drop_ship_delivers_to_the_client_and_only_for_a_client_purchase() {
        let drop_ship = PurchaseIntent {
            acquired_for: AcquiredFor::ClientDropShip,
            delivery_mode: DeliveryMode::ToStock,
            ..intent()
        };
        assert!(drop_ship.validate().is_err());

        let ok = PurchaseIntent {
            acquired_for: AcquiredFor::ClientDropShip,
            ownership: None,
            delivery_mode: DeliveryMode::DropShipToClient,
            recharge_rule: Some(RechargeRule::at_cost()),
        };
        ok.validate().unwrap();

        let wrong_owner = PurchaseIntent {
            acquired_for: AcquiredFor::SaleStock,
            delivery_mode: DeliveryMode::DropShipToClient,
            ..intent()
        };
        assert!(
            wrong_owner.validate().is_err(),
            "sale stock never drops ships to a client"
        );
    }

    #[test]
    fn rented_in_gear_is_a_vendor_rental_in_both_directions() {
        let rented = PurchaseIntent {
            acquired_for: AcquiredFor::VendorRental,
            ownership: Some(Ownership::Rented),
            delivery_mode: DeliveryMode::ToStock,
            recharge_rule: None,
        };
        rented.validate().unwrap();

        let unowned_rental = PurchaseIntent {
            acquired_for: AcquiredFor::VendorRental,
            ownership: Some(Ownership::Owned),
            ..rented.clone()
        };
        assert!(unowned_rental.validate().is_err());

        let rented_but_owned = PurchaseIntent {
            acquired_for: AcquiredFor::SaleStock,
            ownership: Some(Ownership::Rented),
            ..intent()
        };
        assert!(rented_but_owned.validate().is_err());
    }

    #[test]
    fn a_recharge_rule_belongs_to_a_client_purchase_only() {
        let client = PurchaseIntent {
            acquired_for: AcquiredFor::ClientStocked,
            ownership: None,
            delivery_mode: DeliveryMode::ToStock,
            recharge_rule: Some(RechargeRule::default_markup()),
        };
        client.validate().unwrap();

        let ours = PurchaseIntent {
            recharge_rule: Some(RechargeRule::default_markup()),
            ..intent()
        };
        assert!(
            ours.validate().is_err(),
            "a purchase for stock is not recharged to a client"
        );
    }

    #[test]
    fn a_pass_through_cannot_land_in_owned_quantity() {
        let pass_through = PurchaseLineIntent {
            line_kind: LineKind::PassThrough,
            destination_class: CustodyClass::SaleStock,
            serial_expected: false,
            weight_g: None,
            sell_price_minor: None,
            billable: true,
            asset_layout_id: None,
        };
        assert!(
            pass_through.validate().is_err(),
            "the double-count defect: a pass-through in sale stock"
        );

        let client_pass_through = PurchaseLineIntent {
            destination_class: CustodyClass::ClientCustody,
            ..pass_through
        };
        client_pass_through.validate().unwrap();

        let service = PurchaseLineIntent {
            line_kind: LineKind::Service,
            destination_class: CustodyClass::SaleStock,
            serial_expected: false,
            weight_g: None,
            sell_price_minor: None,
            billable: true,
            asset_layout_id: None,
        };
        assert!(
            service.validate().is_err(),
            "a service is not a unit and cannot enter sale stock"
        );
    }

    #[test]
    fn line_kind_validation_keeps_serials_and_weights_honest() {
        line_intent().validate().unwrap();

        let mut serial = line_intent();
        serial.line_kind = LineKind::SerializedUnit;
        assert!(
            serial.validate().is_err(),
            "a serialized unit expects a serial"
        );
        serial.serial_expected = true;
        serial.validate().unwrap();

        let mut wrong = line_intent();
        wrong.serial_expected = true;
        assert!(wrong.validate().is_err(), "stock does not expect a serial");

        let mut heavy = line_intent();
        heavy.weight_g = Some(0);
        assert!(heavy.validate().is_err());

        let mut negative_price = line_intent();
        negative_price.sell_price_minor = Some(-1);
        assert!(negative_price.validate().is_err());
    }
}
