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
}
