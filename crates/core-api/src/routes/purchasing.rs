//! Purchasing + vendors API (Wave 32 lane 2, `p32-purchasing`).
//!
//! The IF1–IF3 instant-first envelope over `vendors` (+ typed contacts),
//! typed `external_links` (reseller order portals / parcel tracking), the
//! purchase-order lifecycle (draft → ordered → partially_received →
//! received → closed | cancelled), receipt documents that post into the
//! lane-1 append-only `stock_movements` ledger at a chosen location, and
//! the vendor bill that carries a purchase into project cost rollups.
//!
//! Typed honesty (never silent):
//!   * over-receipt: refused with a typed 409 unless the request carries
//!     `allow_over_receipt = true`; the line then reads `receipt_state =
//!     'over'` (persisted, queryable).
//!   * under-receipt: closing a partially received order stamps
//!     `close_kind = 'short'` (persisted, queryable) — the unreceived
//!     quantity is recorded, not dropped.
//!   * a catalog-only line cannot receive into stock (typed 409): the
//!     ledger needs a stock item, and the API never invents one.
//!
//! ADR 0002: each receipt leg that lands at the same moment as a movement
//! gets the standard FX-lock treatment (routes/fx.rs `lock_snapshot`);
//! bill POSTING is the bill's lock moment. Money stays integer minor
//! units, quantities `*_milli`; cost fields serialize `null` without
//! `financial:read`.
//!
//! Non-purpose: no UI (lane 4), no settings surface, no live carrier API
//! (tracking links are operator-recorded URLs).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::http::Extensions;
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use chrono::NaiveDate;
use core_db::purchasing::{BillFxSnapshot, OrderIntent, ProjectCostRollup};
use core_domain::entities::{
    AcquiredFor, Actor, AuditEvent, CloseKind, CostLineKind, CustodyClass, DeliveryMode,
    ExternalLink, ExternalLinkKind, ExternalLinkSubject, LineKind, Ownership, PurchaseBill,
    PurchaseIntent, PurchaseLineIntent, PurchaseOrder, PurchaseOrderLine, PurchaseOrderRollup,
    PurchaseOrderStatus, PurchaseReceipt, PurchaseReceiptLine, ReceiptState, RechargeRule,
    RechargeRuleKind, Vendor, VendorContact,
};
use core_domain::policy::{check, Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

const VENDOR_ENTITY: &str = "vendor";
const ORDER_ENTITY: &str = "purchase_order";
const LINK_ENTITY: &str = "external_link";
const RECEIPT_ENTITY: &str = "purchase_receipt";
const BILL_ENTITY: &str = "purchase_bill";
const LIST_LIMIT: i64 = 500;

fn commercial(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Commercial,
        action,
    }
}

fn financial_read(authed: &crate::auth::AuthedUser) -> bool {
    check(&authed.principal.permissions, Permission::ViewFinancials).is_allowed()
}

/// Maps a unique-violation to a typed 409 (vendor code / PO number).
fn unique_conflict(what: &'static str) -> impl Fn(core_db::DbError) -> ApiError {
    move |e| {
        let unique = matches!(
            &e,
            core_db::DbError::Sql(sqlx::Error::Database(db_err)) if db_err.is_unique_violation()
        );
        if unique {
            ApiError::Conflict(format!("that {what} already exists"))
        } else {
            e.into()
        }
    }
}

/// Tri-state PATCH semantics: absent → `None` (keep), `null` → `Some(None)`
/// (clear), value → `Some(Some(v))` (set).
fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    serde::Deserialize::deserialize(deserializer).map(Some)
}

pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/vendors"),
            get(list_vendors).post(create_vendor),
        )
        .route(
            &format!("{base}/vendors/{{id}}"),
            get(get_vendor).patch(update_vendor),
        )
        .route(
            &format!("{base}/vendors/{{id}}/contacts"),
            get(list_vendor_contacts).post(replace_vendor_contacts_route),
        )
        .route(
            &format!("{base}/vendors/{{id}}/links"),
            get(list_vendor_links).post(create_vendor_link),
        )
        .route(
            &format!("{base}/purchase-orders"),
            get(list_orders).post(create_order),
        )
        .route(
            &format!("{base}/purchase-orders/{{id}}"),
            get(get_order).patch(update_order),
        )
        .route(
            &format!("{base}/purchase-orders/{{id}}/lines"),
            post(add_order_line),
        )
        .route(
            &format!("{base}/purchase-orders/{{id}}/lines/{{line_id}}"),
            patch(update_order_line).delete(delete_order_line),
        )
        .route(
            &format!("{base}/purchase-orders/{{id}}/order"),
            post(place_order),
        )
        .route(
            &format!("{base}/purchase-orders/{{id}}/receive"),
            post(receive_order),
        )
        .route(
            &format!("{base}/purchase-orders/{{id}}/close"),
            post(close_order_route),
        )
        .route(
            &format!("{base}/purchase-orders/{{id}}/cancel"),
            post(cancel_order_route),
        )
        .route(
            &format!("{base}/purchase-orders/{{id}}/links"),
            get(list_order_links).post(create_order_link),
        )
        .route(
            &format!("{base}/purchase-orders/{{id}}/deeplinks"),
            get(order_deeplinks),
        )
        .route(
            &format!("{base}/purchase-orders/{{id}}/recharges"),
            get(order_recharges),
        )
        .route(
            &format!("{base}/purchase-orders/{{id}}/bill"),
            get(get_order_bill).put(record_order_bill),
        )
        .route(
            &format!("{base}/purchase-orders/{{id}}/bill/post"),
            post(post_order_bill),
        )
        .route(
            &format!("{base}/external-links/{{id}}"),
            delete(delete_external_link_route),
        )
        .route(
            &format!("{base}/projects/{{id}}/cost-rollup"),
            get(project_cost_rollup_route),
        )
}

// --- wire + view structs ------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct ListVendorsQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub include_inactive: bool,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct VendorContactInput {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub is_primary: bool,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateVendorRequest {
    pub name: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub website: Option<String>,
    #[serde(default)]
    pub account_number: Option<String>,
    #[serde(default)]
    pub default_currency: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// Initial contact list (replace-on-write semantics).
    #[serde(default)]
    pub contacts: Vec<VendorContactInput>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateVendorRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<Option<String>>)]
    pub code: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<Option<String>>)]
    pub email: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<Option<String>>)]
    pub phone: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<Option<String>>)]
    pub website: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<Option<String>>)]
    pub account_number: Option<Option<String>>,
    #[serde(default)]
    pub default_currency: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<Option<String>>)]
    pub notes: Option<Option<String>>,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct VendorMutationResponse {
    #[serde(flatten)]
    pub vendor: Vendor,
    pub mutation: instant::MutationMeta,
}

/// Vendor detail: the entity plus its typed contacts and external links.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct VendorDetail {
    pub vendor: Vendor,
    pub contacts: Vec<VendorContact>,
    pub links: Vec<ExternalLink>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ReplaceContactsRequest {
    pub contacts: Vec<VendorContactInput>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ContactsMutationResponse {
    pub contacts: Vec<VendorContact>,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateExternalLinkRequest {
    pub kind: ExternalLinkKind,
    pub url: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub carrier: Option<String>,
    /// PO-line links only: the line inside the route's order.
    #[serde(default)]
    pub line_id: Option<Uuid>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LinkMutationResponse {
    pub link: ExternalLink,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct ListOrdersQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub vendor_id: Option<Uuid>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub q: Option<String>,
}

/// The header intent a client reads (plan §5.1): what the purchase is for, who
/// it serves, how it is delivered and the recharge rule it carries.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PurchaseOrderIntentView {
    pub acquired_for: AcquiredFor,
    #[schema(value_type = Option<Uuid>)]
    pub client_id: Option<Uuid>,
    #[schema(value_type = Option<Uuid>)]
    pub ticket_id: Option<Uuid>,
    #[schema(value_type = Option<Uuid>)]
    pub project_id: Option<Uuid>,
    pub ownership: Option<Ownership>,
    pub delivery_mode: DeliveryMode,
    pub recharge_rule: Option<RechargeRule>,
}

impl From<&OrderIntent> for PurchaseOrderIntentView {
    fn from(intent: &OrderIntent) -> Self {
        Self {
            acquired_for: intent.acquired_for,
            client_id: intent.client_id,
            ticket_id: intent.ticket_id,
            project_id: intent.project_id,
            ownership: intent.ownership,
            delivery_mode: intent.delivery_mode,
            recharge_rule: intent.recharge_rule.clone(),
        }
    }
}

/// One receipt line's consequence, as the routing engine resolved it. The
/// destination class and the asset a line created are what makes the chain in
/// §4.3 readable from the receipt alone.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReceiptLineView {
    #[serde(flatten)]
    pub line: PurchaseReceiptLine,
    pub destination_class: CustodyClass,
    #[schema(value_type = Option<Uuid>)]
    pub device_asset_id: Option<Uuid>,
}

/// One issued recharge (plan §5.3). The resolved rule, the basis it was applied
/// to, both currencies and both FX snapshots travel together because an issued
/// recharge is never recomputed.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PurchaseRechargeView {
    #[schema(value_type = Uuid)]
    pub id: Uuid,
    #[schema(value_type = Uuid)]
    pub purchase_order_id: Uuid,
    #[schema(value_type = Uuid)]
    pub po_line_id: Uuid,
    #[schema(value_type = Uuid)]
    pub receipt_line_id: Uuid,
    #[schema(value_type = Option<Uuid>)]
    pub client_id: Option<Uuid>,
    #[schema(value_type = Option<Uuid>)]
    pub ticket_id: Option<Uuid>,
    #[schema(value_type = Option<Uuid>)]
    pub project_id: Option<Uuid>,
    pub rule: RechargeRule,
    /// `purchase_order | ticket | project | client | organisation` — the rung
    /// that supplied the rule.
    pub rule_source: String,
    #[schema(value_type = i64)]
    pub basis_amount_minor: i64,
    pub basis_currency: String,
    pub cost_fx: Option<RechargeFxView>,
    #[schema(value_type = i64)]
    pub charge_amount_minor: i64,
    pub charge_currency: String,
    pub charge_fx: Option<RechargeFxView>,
    #[schema(value_type = Option<Uuid>)]
    pub cost_line_id: Option<Uuid>,
    pub issued_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RechargeFxView {
    pub rate_used: String,
    pub rate_date: NaiveDate,
    pub source: String,
    #[schema(value_type = i64)]
    pub base_amount_minor: i64,
}

impl From<core_db::purchasing::Recharge> for PurchaseRechargeView {
    fn from(r: core_db::purchasing::Recharge) -> Self {
        Self {
            id: r.id,
            purchase_order_id: r.purchase_order_id,
            po_line_id: r.po_line_id,
            receipt_line_id: r.receipt_line_id,
            client_id: r.client_id,
            ticket_id: r.ticket_id,
            project_id: r.project_id,
            rule: r.rule,
            rule_source: r.rule_source.as_str().to_string(),
            basis_amount_minor: r.basis_amount_minor,
            basis_currency: r.basis_currency,
            cost_fx: r.cost_fx.map(RechargeFxView::from),
            charge_amount_minor: r.charge_amount_minor,
            charge_currency: r.charge_currency,
            charge_fx: r.charge_fx.map(RechargeFxView::from),
            cost_line_id: r.cost_line_id,
            issued_at: r.issued_at,
        }
    }
}

impl From<core_db::purchasing::FxColumns> for RechargeFxView {
    fn from(fx: core_db::purchasing::FxColumns) -> Self {
        Self {
            rate_used: fx.rate_used,
            rate_date: fx.rate_date,
            source: fx.source,
            base_amount_minor: fx.base_amount_minor,
        }
    }
}

/// One row of the order list: the order itself plus its own totals.
///
/// The order fields are flattened, so this is the order's old wire shape plus
/// `totals` and `intent` keys — a client that ignores the new keys keeps
/// working, and one that reads them stops needing a request per order to know
/// what an order buys or what it is worth. `totals` is `null` for a caller
/// without `ViewFinancials`, which is the same gate the detail endpoint's
/// totals obey: the list never leaks money the detail page would hide.
#[derive(Debug, Serialize, ToSchema)]
pub struct PurchaseOrderSummary {
    #[serde(flatten)]
    pub order: PurchaseOrder,
    pub intent: PurchaseOrderIntentView,
    pub totals: Option<PurchaseOrderRollup>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct OrderLineInput {
    #[serde(default)]
    pub stock_item_id: Option<Uuid>,
    #[serde(default)]
    pub catalog_item_id: Option<Uuid>,
    pub description: String,
    pub qty_ordered_milli: i64,
    pub unit_cost_minor: i64,
    #[serde(default)]
    pub position: Option<i32>,
    /// What this line buys (plan §5.1). Absent → `stock`.
    #[serde(default)]
    pub line_kind: Option<LineKind>,
    /// Where the line lands when it differs from the header's intent. Absent →
    /// inherit the header, so a draft follows a header change instead of
    /// freezing a destination the operator never named.
    #[serde(default)]
    pub destination_class: Option<CustodyClass>,
    /// The asset layout a receipt for this line should instantiate.
    #[serde(default)]
    pub asset_layout_id: Option<Uuid>,
    /// Absent → true for a `serialized_unit` line, false otherwise.
    #[serde(default)]
    pub serial_expected: Option<bool>,
    #[serde(default)]
    pub weight_g: Option<i32>,
    #[serde(default)]
    pub sell_price_minor: Option<i64>,
    /// Absent → true when the line lands in client custody.
    #[serde(default)]
    pub billable: Option<bool>,
}

/// POST /purchase-orders/{id}/lines body: the line plus IF1 params.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct AddOrderLineRequest {
    #[serde(flatten)]
    pub line: OrderLineInput,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateOrderRequest {
    pub vendor_id: Uuid,
    /// Absent → generated `PO-NNNNNN` from the org sequence.
    #[serde(default)]
    pub number: Option<String>,
    /// Absent → the vendor's default currency.
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub vendor_reference: Option<String>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub expected_at: Option<NaiveDate>,
    /// What the order buys (plan §5.1). Absent → `sale_stock`, delivered
    /// `to_stock`: the behaviour every existing caller already gets.
    #[serde(default)]
    pub acquired_for: Option<AcquiredFor>,
    /// The client the purchase is made for. Required for a client purchase and
    /// refused for anything else; derived from the ticket when a ticket is given
    /// and the client omitted.
    #[serde(default)]
    pub client_id: Option<Uuid>,
    #[serde(default)]
    pub ticket_id: Option<Uuid>,
    #[serde(default)]
    pub ownership: Option<Ownership>,
    #[serde(default)]
    pub delivery_mode: Option<DeliveryMode>,
    /// The rule this purchase resolves to when it is received. Absent → the
    /// ticket/project → client → organisation ladder (§5.3).
    #[serde(default)]
    pub recharge_rule: Option<RechargeRule>,
    #[serde(default)]
    pub lines: Vec<OrderLineInput>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateOrderRequest {
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<Option<String>>)]
    pub vendor_reference: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<Option<Uuid>>)]
    pub project_id: Option<Option<Uuid>>,
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<Option<String>>)]
    pub notes: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<Option<NaiveDate>>)]
    pub expected_at: Option<Option<NaiveDate>>,
    /// Header intent edits. Absent keeps the current value; `null` clears a
    /// nullable one (`client_id`, `ticket_id`, `ownership`, `recharge_rule`).
    #[serde(default)]
    pub acquired_for: Option<AcquiredFor>,
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<Option<Uuid>>)]
    pub client_id: Option<Option<Uuid>>,
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<Option<Uuid>>)]
    pub ticket_id: Option<Option<Uuid>>,
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<Option<Ownership>>)]
    pub ownership: Option<Option<Ownership>>,
    #[serde(default)]
    pub delivery_mode: Option<DeliveryMode>,
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<Option<RechargeRule>>)]
    pub recharge_rule: Option<Option<RechargeRule>>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Ledger-visible PO line (cost fields are `financial:read`-gated).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PurchaseOrderLineView {
    #[serde(rename = "id")]
    #[schema(value_type = Uuid)]
    pub id: Uuid,
    #[schema(value_type = Uuid)]
    pub purchase_order_id: Uuid,
    #[schema(value_type = Option<Uuid>)]
    pub stock_item_id: Option<Uuid>,
    #[schema(value_type = Option<Uuid>)]
    pub catalog_item_id: Option<Uuid>,
    pub description: String,
    pub qty_ordered_milli: i64,
    pub qty_received_milli: i64,
    pub receipt_state: ReceiptState,
    /// What this line buys and where it lands (plan §5.1). The destination is
    /// the resolved one — the line's own when it named a class, the header's
    /// otherwise — so a client reads what will happen rather than two words it
    /// has to combine.
    pub line_kind: LineKind,
    pub destination_class: CustodyClass,
    #[schema(value_type = Option<Uuid>)]
    pub asset_layout_id: Option<Uuid>,
    pub serial_expected: bool,
    pub weight_g: Option<i32>,
    pub billable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sell_price_minor: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_cost_minor: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordered_amount_minor: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub received_amount_minor: Option<i64>,
    pub position: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl PurchaseOrderLineView {
    fn view(
        line: &PurchaseOrderLine,
        intent: &core_db::purchasing::LineIntent,
        header: &OrderIntent,
        financial: bool,
    ) -> Self {
        Self {
            id: line.id,
            purchase_order_id: line.purchase_order_id,
            stock_item_id: line.stock_item_id,
            catalog_item_id: line.catalog_item_id,
            description: line.description.clone(),
            qty_ordered_milli: line.qty_ordered_milli,
            qty_received_milli: line.qty_received_milli,
            receipt_state: line.receipt_state,
            line_kind: intent.line_kind,
            destination_class: resolve_destination(intent, header),
            asset_layout_id: intent.asset_layout_id,
            serial_expected: intent.serial_expected,
            weight_g: intent.weight_g,
            billable: intent.billable,
            sell_price_minor: financial.then_some(intent.sell_price_minor).flatten(),
            unit_cost_minor: financial.then_some(line.unit_cost_minor),
            ordered_amount_minor: financial
                .then(|| line.ordered_amount_minor().ok())
                .flatten(),
            received_amount_minor: financial
                .then(|| line.received_amount_minor().ok())
                .flatten(),
            position: line.position,
            created_at: line.created_at,
            updated_at: line.updated_at,
        }
    }
}

/// A receipt document with its traceability lines (each stamps the consequence
/// it produced: a ledger movement, an asset, or neither for a client lot).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PurchaseReceiptView {
    #[serde(flatten)]
    pub receipt: PurchaseReceipt,
    pub lines: Vec<ReceiptLineView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PurchaseOrderDetail {
    pub order: PurchaseOrder,
    pub intent: PurchaseOrderIntentView,
    pub lines: Vec<PurchaseOrderLineView>,
    pub receipts: Vec<PurchaseReceiptView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bill: Option<PurchaseBill>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub totals: Option<PurchaseOrderRollup>,
    /// Recharges this order issued. Withheld entirely without
    /// `financial:read`, like `totals`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recharges: Option<Vec<PurchaseRechargeView>>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OrderMutationResponse {
    #[serde(flatten)]
    pub detail: PurchaseOrderDetail,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReceiveLineInput {
    pub line_id: Uuid,
    pub qty_milli: i64,
    /// Serialized stock items only: one serial per unit (qty_milli must be
    /// `count × 1000`). Each serial posts its own ledger row + receipt line.
    #[serde(default)]
    pub serial_numbers: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ReceiveOrderRequest {
    /// Where the goods landed (lane-1 stock location).
    pub location_id: Uuid,
    #[serde(default)]
    pub carrier: Option<String>,
    #[serde(default)]
    pub tracking_ref: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// Explicit acknowledgement for an over-receipt; absent/false refuses
    /// with a typed 409.
    #[serde(default)]
    pub allow_over_receipt: bool,
    pub lines: Vec<ReceiveLineInput>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReceiptMutationResponse {
    pub receipt: PurchaseReceiptView,
    pub order: PurchaseOrderDetail,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CloseOrderRequest {
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CancelOrderRequest {
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RecordBillRequest {
    #[serde(default)]
    pub bill_number: Option<String>,
    /// Absent → the received total from the PO lines.
    #[serde(default)]
    pub amount_minor: Option<i64>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct BillMutationResponse {
    pub bill: PurchaseBill,
    pub mutation: instant::MutationMeta,
}

/// One resolved one-click link on a PO, in open order.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeeplinkEntry {
    pub link_id: Uuid,
    pub kind: ExternalLinkKind,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub carrier: Option<String>,
    /// vendor | purchase_order | purchase_order_line
    pub subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_position: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeeplinkVendor {
    pub id: Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_number: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PurchaseOrderDeeplinks {
    pub order_id: Uuid,
    pub number: String,
    pub vendor: DeeplinkVendor,
    pub links: Vec<DeeplinkEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RollupBillView {
    pub bill_id: Uuid,
    pub purchase_order_id: Uuid,
    pub purchase_order_number: String,
    pub amount_minor: i64,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_amount_minor: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posted_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProjectCostRollupView {
    pub project_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_minor: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_currency: Option<String>,
    pub base_currency: String,
    pub po_bill_base_minor: i64,
    pub bills: Vec<RollupBillView>,
}

impl From<ProjectCostRollup> for ProjectCostRollupView {
    fn from(r: ProjectCostRollup) -> Self {
        Self {
            project_id: r.project_id,
            budget_minor: r.budget_minor,
            budget_currency: r.budget_currency,
            base_currency: r.base_currency,
            po_bill_base_minor: r.po_bill_base_minor,
            bills: r
                .bills
                .into_iter()
                .map(|b| RollupBillView {
                    bill_id: b.bill_id,
                    purchase_order_id: b.purchase_order_id,
                    purchase_order_number: b.purchase_order_number,
                    amount_minor: b.amount_minor,
                    currency: b.currency,
                    base_amount_minor: b.base_amount_minor,
                    posted_at: b.posted_at,
                })
                .collect(),
        }
    }
}

// --- shared helpers -----------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn vendor_from_parts(
    org_id: Uuid,
    user_id: Uuid,
    name: String,
    code: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    website: Option<String>,
    account_number: Option<String>,
    default_currency: Option<String>,
    notes: Option<String>,
) -> Vendor {
    let now = chrono::Utc::now();
    Vendor {
        id: Uuid::now_v7(),
        org_id,
        name: name.trim().to_string(),
        code: code.map(|c| c.trim().to_string()).filter(|c| !c.is_empty()),
        email,
        phone,
        website,
        account_number,
        default_currency: default_currency.unwrap_or_else(|| "USD".into()),
        notes,
        active: true,
        created_by: Some(user_id),
        created_at: now,
        updated_at: now,
    }
}

fn contacts_from_input(
    org_id: Uuid,
    vendor_id: Uuid,
    inputs: &[VendorContactInput],
) -> ApiResult<Vec<VendorContact>> {
    let mut contacts = Vec::with_capacity(inputs.len());
    for input in inputs {
        let now = chrono::Utc::now();
        let contact = VendorContact {
            id: Uuid::now_v7(),
            org_id,
            vendor_id,
            name: input.name.trim().to_string(),
            email: input.email.clone(),
            phone: input.phone.clone(),
            role: input.role.clone(),
            is_primary: input.is_primary,
            created_at: now,
            updated_at: now,
        };
        contact.validate().map_err(ApiError::from)?;
        contacts.push(contact);
    }
    if contacts.iter().filter(|c| c.is_primary).count() > 1 {
        return Err(ApiError::Validation(
            "at most one vendor contact can be primary".into(),
        ));
    }
    Ok(contacts)
}

/// The class a line's receipt lands in: its own when it named one, the header
/// intent's otherwise (plan §5.1 — the header carries defaults, the line
/// decides).
fn resolve_destination(
    intent: &core_db::purchasing::LineIntent,
    header: &OrderIntent,
) -> CustodyClass {
    intent
        .destination_class
        .unwrap_or_else(|| header.acquired_for.destination_class())
}

/// The rule in force for a client purchase, applied to one receipt line's
/// basis. Basis points are integer math with the house half-even rule; `fixed`
/// stands alone and `none` never produces a charge. A `fixed` rule charges its
/// amount per receipt line, and a named unit is its own receipt line, so a
/// fixed price is a fixed price per unit.
fn recharge_charge_minor(basis_minor: i64, rule: &RechargeRule) -> ApiResult<i64> {
    match rule.kind {
        RechargeRuleKind::AtCost => Ok(basis_minor),
        RechargeRuleKind::MarkupBps => {
            let bps = i128::from(rule.markup_bps.unwrap_or(0));
            let markup = core_domain::fx::div_half_even(i128::from(basis_minor) * bps, 10_000);
            i64::try_from(i128::from(basis_minor) + markup)
                .map_err(|_| ApiError::Validation("recharge amount out of range".into()))
        }
        RechargeRuleKind::Fixed => rule
            .fixed_minor
            .ok_or_else(|| ApiError::Validation("a fixed recharge rule carries no amount".into())),
        RechargeRuleKind::None => Err(ApiError::Validation(
            "a none recharge rule issues no charge".into(),
        )),
    }
}

/// The reporting class a recharge line carries (plan §6.4). A pass-through is
/// bought only to be passed on, so its cost is `cogs`; a service is
/// `service_cost`; stock, assets and rentals are `inventory`; e-waste is its own
/// class because the vendor bill is the point.
fn recharge_reporting_class(acquired_for: AcquiredFor, line_kind: LineKind) -> &'static str {
    if acquired_for == AcquiredFor::EwasteService {
        return "ewaste_cost";
    }
    match line_kind {
        LineKind::Service => "service_cost",
        LineKind::PassThrough => "cogs",
        _ => "inventory",
    }
}

/// A line may name its destination, not the intent the money is recharged
/// under, so the pair has to agree on the things the receipt engine cannot
/// invent: a client, a rented vendor unit, an asset shape.
fn guard_line_destination(
    destination: CustodyClass,
    line_kind: LineKind,
    header: &OrderIntent,
) -> ApiResult<()> {
    if destination == CustodyClass::ClientCustody && header.client_id.is_none() {
        return Err(ApiError::Validation(
            "a line that lands in client custody needs a client on the purchase order".into(),
        ));
    }
    if destination == CustodyClass::RentalIn && header.ownership != Some(Ownership::Rented) {
        return Err(ApiError::Validation(
            "a vendor rental line records ownership rented on the purchase order".into(),
        ));
    }
    if matches!(
        destination,
        CustodyClass::InternalFleet | CustodyClass::RentalIn | CustodyClass::RentalOut
    ) && !matches!(
        line_kind,
        LineKind::SerializedUnit | LineKind::Asset | LineKind::RentalUnit
    ) {
        return Err(ApiError::Validation(format!(
            "a line landing in {} buys an asset or rental unit, not a {} line",
            destination.as_str(),
            line_kind.as_str()
        )));
    }
    if destination == CustodyClass::SaleStock
        && matches!(line_kind, LineKind::Asset | LineKind::RentalUnit)
    {
        return Err(ApiError::Validation(
            "an asset or rental unit line cannot land in sale stock".into(),
        ));
    }
    if destination == CustodyClass::EwasteCustody && line_kind != LineKind::Service {
        return Err(ApiError::Validation(
            "an e-waste line is a service; units arrive through an e-waste intake".into(),
        ));
    }
    Ok(())
}

/// Proves a line's asset layout belongs to this organisation and can be
/// satisfied by the asset a receipt creates. The receipt engine passes empty
/// fields, so a layout that requires values the purchase does not carry is
/// refused here — naming the missing fields — rather than producing an asset
/// that violates its own schema. Type-specific values are added on the asset
/// surface after receipt.
async fn validate_line_layout(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    intent: &core_db::purchasing::LineIntent,
) -> ApiResult<()> {
    let Some(layout_id) = intent.asset_layout_id else {
        return Ok(());
    };
    let layout = core_db::document_layouts::get_layout(&mut *conn, org_id, layout_id).await?;
    let empty = serde_json::json!({});
    crate::routes::documents::validate_fields(&layout, &empty)
}

/// One line's intent, validated against the header it belongs to.
fn line_intent_from_input(
    input: &OrderLineInput,
    header: &OrderIntent,
) -> ApiResult<core_db::purchasing::LineIntent> {
    let line_kind = input.line_kind.unwrap_or(LineKind::Stock);
    let destination = input
        .destination_class
        .unwrap_or_else(|| header.acquired_for.destination_class());
    let serial_expected = input
        .serial_expected
        .unwrap_or(line_kind == LineKind::SerializedUnit);
    let billable = input
        .billable
        .unwrap_or(destination == CustodyClass::ClientCustody);
    PurchaseLineIntent {
        line_kind,
        destination_class: destination,
        asset_layout_id: input.asset_layout_id,
        serial_expected,
        weight_g: input.weight_g,
        sell_price_minor: input.sell_price_minor,
        billable,
    }
    .validate()
    .map_err(ApiError::from)?;
    guard_line_destination(destination, line_kind, header)?;
    Ok(core_db::purchasing::LineIntent {
        line_kind,
        // Only an explicit override is stored; NULL keeps the line following
        // the header, which is what the column means.
        destination_class: input.destination_class,
        asset_layout_id: input.asset_layout_id,
        serial_expected,
        weight_g: input.weight_g,
        sell_price_minor: input.sell_price_minor,
        billable,
    })
}

/// The header a create request asks for, validated on its own before any row is
/// written.
fn header_intent_from_create(req: &CreateOrderRequest) -> ApiResult<OrderIntent> {
    let intent = OrderIntent {
        acquired_for: req.acquired_for.unwrap_or(AcquiredFor::SaleStock),
        client_id: req.client_id,
        ticket_id: req.ticket_id,
        project_id: req.project_id,
        ownership: req.ownership,
        delivery_mode: req.delivery_mode.unwrap_or(DeliveryMode::ToStock),
        recharge_rule: req.recharge_rule.clone(),
    };
    let domain = PurchaseIntent {
        acquired_for: intent.acquired_for,
        ownership: intent.ownership,
        delivery_mode: intent.delivery_mode,
        recharge_rule: intent.recharge_rule.clone(),
    };
    domain.validate().map_err(ApiError::from)?;
    Ok(intent)
}

/// Resolves the references a header names and proves the caller may see them:
/// a restricted client cannot be bought for, and a ticket's client is the
/// purchase's client. Mutates the derived client into the intent.
async fn validate_order_intent(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    principal: &core_domain::entities::Principal,
    intent: &mut OrderIntent,
) -> ApiResult<()> {
    if let Some(ticket_id) = intent.ticket_id {
        let ticket = core_db::tickets::get_ticket(&mut *conn, org_id, ticket_id).await?;
        // Naming a ticket the caller cannot see would leak its existence, and
        // receiving against it writes a cost line on that board.
        crate::routes::board_access::authorize_board_op(
            &mut *conn,
            principal,
            ticket.board_id,
            core_domain::policy::BoardOp::View,
        )
        .await?;
        match (intent.client_id, ticket.client_id) {
            (Some(client_id), Some(ticket_client)) if client_id != ticket_client => {
                return Err(ApiError::Validation(
                    "the ticket belongs to a different client than the purchase order".into(),
                ));
            }
            (None, Some(ticket_client)) => intent.client_id = Some(ticket_client),
            _ => {}
        }
    }
    if let Some(client_id) = intent.client_id {
        crate::routes::clients::require_client_visible(&mut *conn, org_id, client_id, principal)
            .await?;
    }
    if intent.acquired_for.is_client_purchase() && intent.client_id.is_none() {
        return Err(ApiError::Validation(
            "a purchase made for a client needs a client or a ticket that has one".into(),
        ));
    }
    let domain = PurchaseIntent {
        acquired_for: intent.acquired_for,
        ownership: intent.ownership,
        delivery_mode: intent.delivery_mode,
        recharge_rule: intent.recharge_rule.clone(),
    };
    domain.validate().map_err(ApiError::from)
}

/// Re-checks every existing line against a header that is about to change. A
/// draft edit that moves the header to an intent one line cannot satisfy (a
/// client-custody line losing its client, a vendor rental that is no longer
/// rented) is refused now rather than at the receipt.
async fn validate_existing_lines(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    order_id: Uuid,
    header: &OrderIntent,
) -> ApiResult<()> {
    let lines = core_db::purchasing::list_po_lines(&mut *conn, org_id, order_id).await?;
    let intents = core_db::purchasing::list_line_intents(&mut *conn, org_id, order_id).await?;
    for line in lines {
        let intent = intents
            .get(&line.id)
            .cloned()
            .unwrap_or_else(core_db::purchasing::LineIntent::stock);
        let destination = resolve_destination(&intent, header);
        PurchaseLineIntent {
            line_kind: intent.line_kind,
            destination_class: destination,
            asset_layout_id: intent.asset_layout_id,
            serial_expected: intent.serial_expected,
            weight_g: intent.weight_g,
            sell_price_minor: intent.sell_price_minor,
            billable: intent.billable,
        }
        .validate()
        .map_err(|e| {
            ApiError::Validation(format!(
                "line \"{}\" no longer fits the order: {e}",
                line.description
            ))
        })?;
        guard_line_destination(destination, intent.line_kind, header).map_err(|e| {
            ApiError::Validation(format!(
                "line \"{}\" no longer fits the order: {e}",
                line.description
            ))
        })?;
    }
    Ok(())
}

/// Proves an order is still visible to this caller before a replayed response
/// is handed back. Permission and client restrictions can change between the
/// request that wrote and the replay that reads, and a stored response must not
/// outlive the visibility rule that produced it.
async fn assert_order_visible(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    principal: &core_domain::entities::Principal,
    order_id: Uuid,
) -> ApiResult<()> {
    let order = core_db::purchasing::get_purchase_order(&mut *conn, org_id, order_id).await?;
    let header = core_db::purchasing::get_order_intent(&mut *conn, org_id, order.id).await?;
    if let Some(client_id) = header.client_id {
        crate::routes::clients::require_client_visible(&mut *conn, org_id, client_id, principal)
            .await?;
    }
    Ok(())
}

async fn load_detail(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    order: PurchaseOrder,
    financial: bool,
) -> ApiResult<PurchaseOrderDetail> {
    let lines = core_db::purchasing::list_po_lines(&mut *conn, org_id, order.id).await?;
    let intent = core_db::purchasing::get_order_intent(&mut *conn, org_id, order.id).await?;
    let line_intents = core_db::purchasing::list_line_intents(&mut *conn, org_id, order.id).await?;
    let receipts = core_db::purchasing::list_receipts(&mut *conn, org_id, order.id).await?;
    let receipt_lines =
        core_db::purchasing::list_receipt_lines_for_order(&mut *conn, org_id, order.id).await?;
    let receipt_views = receipts
        .into_iter()
        .map(|receipt| PurchaseReceiptView {
            lines: receipt_lines
                .iter()
                .filter(|l| l.line.receipt_id == receipt.id)
                .map(|l| ReceiptLineView {
                    line: l.line.clone(),
                    destination_class: l
                        .destination_class
                        .unwrap_or(intent.acquired_for.destination_class()),
                    device_asset_id: l.device_asset_id,
                })
                .collect(),
            receipt,
        })
        .collect();
    let bill = if financial {
        core_db::purchasing::get_bill(&mut *conn, org_id, order.id).await?
    } else {
        None
    };
    let totals = if financial {
        Some(core_domain::entities::po_rollup(&lines).map_err(ApiError::from)?)
    } else {
        None
    };
    let recharges = if financial {
        Some(
            core_db::purchasing::list_recharges_for_order(&mut *conn, org_id, order.id)
                .await?
                .into_iter()
                .map(PurchaseRechargeView::from)
                .collect(),
        )
    } else {
        None
    };
    Ok(PurchaseOrderDetail {
        order,
        intent: PurchaseOrderIntentView::from(&intent),
        lines: lines
            .iter()
            .map(|l| {
                let line_intent = line_intents
                    .get(&l.id)
                    .cloned()
                    .unwrap_or_else(core_db::purchasing::LineIntent::stock);
                PurchaseOrderLineView::view(l, &line_intent, &intent, financial)
            })
            .collect(),
        receipts: receipt_views,
        bill,
        totals,
        recharges,
    })
}

async fn require_draft(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    order_id: Uuid,
) -> ApiResult<PurchaseOrder> {
    let order = core_db::purchasing::get_purchase_order(&mut *conn, org_id, order_id).await?;
    if order.status != PurchaseOrderStatus::Draft {
        return Err(ApiError::Conflict(format!(
            "purchase order {} is {} — only drafts are editable",
            order.number,
            order.status.as_str()
        )));
    }
    Ok(order)
}

async fn validate_project(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    project_id: Option<Uuid>,
) -> ApiResult<()> {
    if let Some(pid) = project_id {
        core_db::projects::get_project(&mut *conn, org_id, pid).await?;
    }
    Ok(())
}

async fn line_from_input(
    org_id: Uuid,
    order_id: Uuid,
    input: &OrderLineInput,
    position: i32,
) -> ApiResult<PurchaseOrderLine> {
    let line = PurchaseOrderLine {
        id: Uuid::now_v7(),
        org_id,
        purchase_order_id: order_id,
        stock_item_id: input.stock_item_id,
        catalog_item_id: input.catalog_item_id,
        description: input.description.trim().to_string(),
        qty_ordered_milli: input.qty_ordered_milli,
        qty_received_milli: 0,
        unit_cost_minor: input.unit_cost_minor,
        receipt_state: ReceiptState::Pending,
        position: input.position.unwrap_or(position),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    line.validate().map_err(ApiError::from)?;
    Ok(line)
}

// --- vendors ------------------------------------------------------------------

/// GET /api/vendors
#[utoipa::path(operation_id = "vendors_list", get, path = "/api/vendors", tag = "purchasing", security(("bearer" = [])),
    params(ListVendorsQuery),
    responses((status = 200, body = [Vendor])))]
pub async fn list_vendors(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<ListVendorsQuery>,
) -> ApiResult<Json<Vec<Vendor>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::purchasing::list_vendors(
        &mut tx,
        org_id,
        q.q.as_deref(),
        !q.include_inactive,
        LIST_LIMIT,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// GET /api/vendors/{id} — entity + contacts + links.
#[utoipa::path(operation_id = "vendors_get", get, path = "/api/vendors/{id}", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = VendorDetail),
              (status = 404, description = "vendor not visible in scope")))]
pub async fn get_vendor(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<VendorDetail>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let vendor = core_db::purchasing::get_vendor(&mut *tx, org_id, id).await?;
    let contacts = core_db::purchasing::list_vendor_contacts(&mut tx, org_id, id).await?;
    let links =
        core_db::purchasing::list_external_links(&mut tx, org_id, ExternalLinkSubject::Vendor, id)
            .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(VendorDetail {
        vendor,
        contacts,
        links,
    }))
}

/// POST /api/vendors
#[utoipa::path(operation_id = "vendors_create", post, path = "/api/vendors", tag = "purchasing", security(("bearer" = [])),
    request_body = CreateVendorRequest,
    responses((status = 200, body = VendorMutationResponse),
              (status = 409, description = "vendor code already exists")))]
pub async fn create_vendor(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<CreateVendorRequest>,
) -> ApiResult<Json<VendorMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let vendor = vendor_from_parts(
        org_id,
        user_id,
        req.name.clone(),
        req.code.clone(),
        req.email.clone(),
        req.phone.clone(),
        req.website.clone(),
        req.account_number.clone(),
        req.default_currency.clone(),
        req.notes.clone(),
    );
    vendor.validate().map_err(ApiError::from)?;
    let contacts = contacts_from_input(org_id, vendor.id, &req.contacts)?;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: VENDOR_ENTITY,
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        core_db::purchasing::insert_vendor(&mut *ctx.tx, &vendor)
            .await
            .map_err(unique_conflict("vendor code"))?;
        if !contacts.is_empty() {
            core_db::purchasing::replace_vendor_contacts(&mut ctx.tx, org_id, vendor.id, &contacts)
                .await?;
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: VENDOR_ENTITY,
                entity_id: vendor.id,
                version: None,
            },
        )
        .await?;
        ctx.emit(
            core_domain::events::types::VENDOR_CREATED,
            VENDOR_ENTITY,
            Some(vendor.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::VendorCreated {
                vendor: vendor.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "purchase.vendor_created".into(),
                entity_type: Some(VENDOR_ENTITY.into()),
                entity_id: Some(vendor.id.to_string()),
                detail: serde_json::json!({
                    "name": vendor.name,
                    "code": vendor.code,
                    "default_currency": vendor.default_currency,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let response = VendorMutationResponse {
            vendor: vendor.clone(),
            mutation: instant::record(
                &mut ctx,
                instant::RecordSpec {
                    entity: VENDOR_ENTITY,
                    entity_id: vendor.id,
                    op: "create",
                    seq: write_seq,
                    before: None,
                    after: serde_json::to_value(&vendor).unwrap_or(Value::Null),
                    updated_at: vendor.updated_at,
                    key: key.as_deref(),
                    payload: &vendor,
                    provenance: Default::default(),
                },
            )
            .await?,
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/vendors/{id}
#[utoipa::path(operation_id = "vendors_update", patch, path = "/api/vendors/{id}", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateVendorRequest,
    responses((status = 200, body = VendorMutationResponse),
              (status = 404, description = "vendor not visible in scope"),
              (status = 409, description = "vendor code already exists or version conflict")))]
pub async fn update_vendor(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<UpdateVendorRequest>,
) -> ApiResult<Json<VendorMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: VENDOR_ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::purchasing::get_vendor(&mut *ctx.tx, org_id, id).await?;
        let mut vendor = before.clone();
        if let Some(name) = &req.name {
            vendor.name = name.trim().to_string();
        }
        if let Some(code) = &req.code {
            vendor.code = code
                .clone()
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty());
        }
        if let Some(email) = &req.email {
            vendor.email = email.clone();
        }
        if let Some(phone) = &req.phone {
            vendor.phone = phone.clone();
        }
        if let Some(website) = &req.website {
            vendor.website = website.clone();
        }
        if let Some(account) = &req.account_number {
            vendor.account_number = account.clone();
        }
        if let Some(currency) = &req.default_currency {
            vendor.default_currency = currency.clone();
        }
        if let Some(notes) = &req.notes {
            vendor.notes = notes.clone();
        }
        if let Some(active) = req.active {
            vendor.active = active;
        }
        vendor.updated_at = chrono::Utc::now();
        vendor.validate().map_err(ApiError::from)?;
        core_db::purchasing::update_vendor(&mut *ctx.tx, &vendor)
            .await
            .map_err(unique_conflict("vendor code"))?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: VENDOR_ENTITY,
                entity_id: id,
                version,
            },
        )
        .await?;
        ctx.emit(
            core_domain::events::types::VENDOR_UPDATED,
            VENDOR_ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::VendorUpdated {
                vendor: vendor.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "purchase.vendor_updated".into(),
                entity_type: Some(VENDOR_ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "name": vendor.name, "active": vendor.active }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let response = VendorMutationResponse {
            vendor: vendor.clone(),
            mutation: instant::record(
                &mut ctx,
                instant::RecordSpec {
                    entity: VENDOR_ENTITY,
                    entity_id: id,
                    op: "update",
                    seq: write_seq,
                    before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                    after: serde_json::to_value(&vendor).unwrap_or(Value::Null),
                    updated_at: vendor.updated_at,
                    key: key.as_deref(),
                    payload: &vendor,
                    provenance: Default::default(),
                },
            )
            .await?,
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// GET /api/vendors/{id}/contacts
#[utoipa::path(operation_id = "vendors_list_contacts", get, path = "/api/vendors/{id}/contacts", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [VendorContact])))]
pub async fn list_vendor_contacts(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<Vec<VendorContact>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    core_db::purchasing::get_vendor(&mut *tx, org_id, id).await?;
    let contacts = core_db::purchasing::list_vendor_contacts(&mut tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(contacts))
}

/// POST /api/vendors/{id}/contacts — replace-on-write.
#[utoipa::path(operation_id = "vendors_replace_contacts", post, path = "/api/vendors/{id}/contacts", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ReplaceContactsRequest,
    responses((status = 200, body = ContactsMutationResponse),
              (status = 404, description = "vendor not visible in scope")))]
pub async fn replace_vendor_contacts_route(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<ReplaceContactsRequest>,
) -> ApiResult<Json<ContactsMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let contacts = contacts_from_input(org_id, id, &req.contacts)?;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "vendor_contacts",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        core_db::purchasing::get_vendor(&mut *ctx.tx, org_id, id).await?;
        let stored =
            core_db::purchasing::replace_vendor_contacts(&mut ctx.tx, org_id, id, &contacts)
                .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "vendor_contacts",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "purchase.vendor_contacts_updated".into(),
                entity_type: Some(VENDOR_ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "contact_count": stored.len() }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let response = ContactsMutationResponse {
            contacts: stored.clone(),
            mutation: instant::record(
                &mut ctx,
                instant::RecordSpec {
                    entity: "vendor_contacts",
                    entity_id: id,
                    op: "update",
                    seq: write_seq,
                    before: None,
                    after: serde_json::to_value(&stored).unwrap_or(Value::Null),
                    updated_at: chrono::Utc::now(),
                    key: key.as_deref(),
                    payload: &stored,
                    provenance: Default::default(),
                },
            )
            .await?,
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- external links -----------------------------------------------------------

async fn insert_link(
    state: AppState,
    extensions: Extensions,
    subject_kind: ExternalLinkSubject,
    subject_id: Uuid,
    expected_order: Option<Uuid>,
    req: CreateExternalLinkRequest,
) -> ApiResult<Json<LinkMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let link = ExternalLink {
        id: Uuid::now_v7(),
        org_id,
        subject_kind,
        subject_id,
        kind: req.kind,
        url: req.url.trim().to_string(),
        label: req.label.clone(),
        carrier: req.carrier.clone(),
        created_by: Some(user_id),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    link.validate().map_err(ApiError::from)?;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: LINK_ENTITY,
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        if let Some(order_id) = expected_order {
            let line = core_db::purchasing::get_po_line(&mut *ctx.tx, org_id, subject_id).await?;
            if line.purchase_order_id != order_id {
                return Err(ApiError::Validation(
                    "the line does not belong to this purchase order".into(),
                ));
            }
        }
        let exists = core_db::purchasing::external_link_subject_exists(
            &mut ctx.tx,
            org_id,
            subject_kind,
            subject_id,
        )
        .await?;
        if !exists {
            return Err(ApiError::NotFound);
        }
        core_db::purchasing::insert_external_link(&mut *ctx.tx, &link).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: LINK_ENTITY,
                entity_id: link.id,
                version: None,
            },
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "purchase.link_added".into(),
                entity_type: Some(LINK_ENTITY.into()),
                entity_id: Some(link.id.to_string()),
                detail: serde_json::json!({
                    "subject_kind": subject_kind.as_str(),
                    "subject_id": subject_id,
                    "kind": link.kind.as_str(),
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let response = LinkMutationResponse {
            link: link.clone(),
            mutation: instant::record(
                &mut ctx,
                instant::RecordSpec {
                    entity: LINK_ENTITY,
                    entity_id: link.id,
                    op: "create",
                    seq: write_seq,
                    before: None,
                    after: serde_json::to_value(&link).unwrap_or(Value::Null),
                    updated_at: link.updated_at,
                    key: key.as_deref(),
                    payload: &link,
                    provenance: Default::default(),
                },
            )
            .await?,
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// GET /api/vendors/{id}/links
#[utoipa::path(operation_id = "vendors_list_links", get, path = "/api/vendors/{id}/links", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [ExternalLink])))]
pub async fn list_vendor_links(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<Vec<ExternalLink>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    core_db::purchasing::get_vendor(&mut *tx, org_id, id).await?;
    let links =
        core_db::purchasing::list_external_links(&mut tx, org_id, ExternalLinkSubject::Vendor, id)
            .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(links))
}

/// POST /api/vendors/{id}/links
#[utoipa::path(operation_id = "vendors_add_link", post, path = "/api/vendors/{id}/links", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CreateExternalLinkRequest,
    responses((status = 200, body = LinkMutationResponse),
              (status = 404, description = "vendor not visible in scope"),
              (status = 422, description = "the url must be https")))]
pub async fn create_vendor_link(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<CreateExternalLinkRequest>,
) -> ApiResult<Json<LinkMutationResponse>> {
    insert_link(
        state,
        extensions,
        ExternalLinkSubject::Vendor,
        id,
        None,
        req,
    )
    .await
}

/// GET /api/purchase-orders/{id}/links
#[utoipa::path(operation_id = "purchase_orders_list_links", get, path = "/api/purchase-orders/{id}/links", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [ExternalLink])))]
pub async fn list_order_links(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<Vec<ExternalLink>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    core_db::purchasing::get_purchase_order(&mut *tx, org_id, id).await?;
    let mut links = core_db::purchasing::list_external_links(
        &mut tx,
        org_id,
        ExternalLinkSubject::PurchaseOrder,
        id,
    )
    .await?;
    let line_ids: Vec<Uuid> = core_db::purchasing::list_po_lines(&mut tx, org_id, id)
        .await?
        .into_iter()
        .map(|l| l.id)
        .collect();
    links.extend(
        core_db::purchasing::list_external_links_for_lines(&mut tx, org_id, &line_ids).await?,
    );
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(links))
}

/// POST /api/purchase-orders/{id}/links — PO-level or (with `line_id`) a
/// per-line order/tracking URL.
#[utoipa::path(operation_id = "purchase_orders_add_link", post, path = "/api/purchase-orders/{id}/links", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CreateExternalLinkRequest,
    responses((status = 200, body = LinkMutationResponse),
              (status = 404, description = "order or line not visible in scope"),
              (status = 422, description = "line not in this order, or non-https url")))]
pub async fn create_order_link(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<CreateExternalLinkRequest>,
) -> ApiResult<Json<LinkMutationResponse>> {
    match req.line_id {
        Some(line_id) => {
            let mut req = req;
            req.line_id = None;
            insert_link(
                state,
                extensions,
                ExternalLinkSubject::PurchaseOrderLine,
                line_id,
                Some(id),
                req,
            )
            .await
        }
        None => {
            // The order must exist (subject existence is checked again
            // inside the tx; this pre-check keeps the typed 404 shape).
            let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
            let org_id = authed.principal.user.org_id;
            let user_id = authed.principal.user.id;
            let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
            core_db::set_org_and_user(&mut tx, org_id, user_id)
                .await
                .map_err(ApiError::from)?;
            core_db::purchasing::get_purchase_order(&mut *tx, org_id, id).await?;
            tx.commit().await.map_err(ApiError::from)?;
            insert_link(
                state,
                extensions,
                ExternalLinkSubject::PurchaseOrder,
                id,
                None,
                req,
            )
            .await
        }
    }
}

/// DELETE /api/external-links/{id}
#[utoipa::path(operation_id = "external_links_delete", delete, path = "/api/external-links/{id}", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 204),
              (status = 404, description = "link not visible in scope")))]
pub async fn delete_external_link_route(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    core_db::purchasing::delete_external_link(&mut *tx, org_id, id).await?;
    core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(user_id),
            action: "purchase.link_removed".into(),
            entity_type: Some(LINK_ENTITY.into()),
            entity_id: Some(id.to_string()),
            detail: serde_json::json!({}),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// --- purchase orders ----------------------------------------------------------

/// GET /api/purchase-orders
#[utoipa::path(operation_id = "purchase_orders_list", get, path = "/api/purchase-orders", tag = "purchasing", security(("bearer" = [])),
    params(ListOrdersQuery),
    responses((status = 200, body = [PurchaseOrderSummary])))]
pub async fn list_orders(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<ListOrdersQuery>,
) -> ApiResult<Json<Vec<PurchaseOrderSummary>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let financial = financial_read(&authed);
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::purchasing::list_purchase_orders(
        &mut tx,
        org_id,
        q.status.as_deref(),
        q.vendor_id,
        q.project_id,
        q.q.as_deref(),
        LIST_LIMIT,
    )
    .await?;

    // The same totals, by the same function, the detail endpoint reports — so
    // a row's value and the order's own total can never disagree. Money is
    // only summed for a caller allowed to see it; without that permission each
    // row carries a null and the client shows the value as withheld, not zero.
    let totals_by_order: HashMap<Uuid, PurchaseOrderRollup> = if financial && !rows.is_empty() {
        let ids: Vec<Uuid> = rows.iter().map(|order| order.id).collect();
        let lines = core_db::purchasing::list_po_lines_for_orders(&mut tx, org_id, &ids).await?;
        let mut grouped: HashMap<Uuid, Vec<PurchaseOrderLine>> = HashMap::new();
        for line in lines {
            grouped
                .entry(line.purchase_order_id)
                .or_default()
                .push(line);
        }
        grouped
            .into_iter()
            .map(|(order_id, lines)| {
                core_domain::entities::po_rollup(&lines)
                    .map(|totals| (order_id, totals))
                    .map_err(ApiError::from)
            })
            .collect::<ApiResult<HashMap<Uuid, PurchaseOrderRollup>>>()?
    } else {
        HashMap::new()
    };

    // One query for the whole page's intents, never one per row.
    let ids: Vec<Uuid> = rows.iter().map(|order| order.id).collect();
    let intents = core_db::purchasing::list_order_intents(&mut tx, org_id, &ids).await?;

    let out = rows
        .into_iter()
        .map(|order| {
            let totals = totals_by_order.get(&order.id).cloned();
            let intent = intents.get(&order.id).ok_or_else(|| {
                ApiError::Internal(anyhow::anyhow!(
                    "purchase order {} has no intent row",
                    order.id
                ))
            })?;
            Ok(PurchaseOrderSummary {
                order,
                intent: PurchaseOrderIntentView::from(intent),
                totals,
            })
        })
        .collect::<ApiResult<Vec<PurchaseOrderSummary>>>()?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(out))
}

/// GET /api/purchase-orders/{id} — header + lines + receipts + bill.
#[utoipa::path(operation_id = "purchase_orders_get", get, path = "/api/purchase-orders/{id}", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = PurchaseOrderDetail),
              (status = 404, description = "order not visible in scope")))]
pub async fn get_order(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<PurchaseOrderDetail>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let financial = financial_read(&authed);
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let order = core_db::purchasing::get_purchase_order(&mut *tx, org_id, id).await?;
    let detail = load_detail(&mut tx, org_id, order, financial).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(detail))
}

/// POST /api/purchase-orders
#[utoipa::path(operation_id = "purchase_orders_create", post, path = "/api/purchase-orders", tag = "purchasing", security(("bearer" = [])),
    request_body = CreateOrderRequest,
    responses((status = 200, body = OrderMutationResponse),
              (status = 404, description = "vendor or project not visible in scope"),
              (status = 409, description = "order number already exists")))]
pub async fn create_order(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<CreateOrderRequest>,
) -> ApiResult<Json<OrderMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let financial = financial_read(&authed);
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ORDER_ENTITY,
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let vendor = core_db::purchasing::get_vendor(&mut *ctx.tx, org_id, req.vendor_id).await?;
        if !vendor.active {
            return Err(ApiError::Conflict(format!(
                "vendor {} is archived",
                vendor.id
            )));
        }
        validate_project(&mut ctx.tx, org_id, req.project_id).await?;
        let mut intent = header_intent_from_create(&req)?;
        validate_order_intent(&mut ctx.tx, org_id, &authed.principal, &mut intent).await?;
        let number = match &req.number {
            Some(n) if !n.trim().is_empty() => n.trim().to_string(),
            _ => core_db::purchasing::next_purchase_order_number(&mut *ctx.tx, org_id).await?,
        };
        let now = chrono::Utc::now();
        let order = PurchaseOrder {
            id: Uuid::now_v7(),
            org_id,
            vendor_id: req.vendor_id,
            number,
            status: PurchaseOrderStatus::Draft,
            currency: req
                .currency
                .clone()
                .unwrap_or_else(|| vendor.default_currency.clone()),
            vendor_reference: req.vendor_reference.clone(),
            project_id: req.project_id,
            notes: req.notes.clone(),
            ordered_at: None,
            expected_at: req.expected_at,
            closed_at: None,
            close_kind: None,
            cancelled_at: None,
            cancel_reason: None,
            created_by: Some(user_id),
            created_at: now,
            updated_at: now,
        };
        order.validate().map_err(ApiError::from)?;
        core_db::purchasing::insert_purchase_order(&mut *ctx.tx, &order)
            .await
            .map_err(unique_conflict("purchase order number"))?;
        core_db::purchasing::set_order_intent(&mut *ctx.tx, org_id, order.id, &intent).await?;
        let mut lines = Vec::with_capacity(req.lines.len());
        for (idx, input) in req.lines.iter().enumerate() {
            let line = line_from_input(org_id, order.id, input, idx as i32).await?;
            let line_intent = line_intent_from_input(input, &intent)?;
            validate_line_layout(&mut ctx.tx, org_id, &line_intent).await?;
            core_db::purchasing::insert_po_line(&mut *ctx.tx, &line).await?;
            core_db::purchasing::set_line_intent(&mut *ctx.tx, org_id, line.id, &line_intent)
                .await?;
            lines.push(line);
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ORDER_ENTITY,
                entity_id: order.id,
                version: None,
            },
        )
        .await?;
        ctx.emit(
            core_domain::events::types::PURCHASE_ORDER_CREATED,
            ORDER_ENTITY,
            Some(order.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::PurchaseOrderCreated {
                order: order.clone(),
                lines: lines.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "purchase.order_created".into(),
                entity_type: Some(ORDER_ENTITY.into()),
                entity_id: Some(order.id.to_string()),
                detail: serde_json::json!({
                    "number": order.number,
                    "vendor_id": order.vendor_id,
                    "line_count": lines.len(),
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let detail = load_detail(&mut ctx.tx, org_id, order.clone(), financial).await?;
        let response = OrderMutationResponse {
            detail: detail.clone(),
            mutation: instant::record(
                &mut ctx,
                instant::RecordSpec {
                    entity: ORDER_ENTITY,
                    entity_id: order.id,
                    op: "create",
                    seq: write_seq,
                    before: None,
                    after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                    updated_at: order.updated_at,
                    key: key.as_deref(),
                    payload: &detail,
                    provenance: Default::default(),
                },
            )
            .await?,
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/purchase-orders/{id} — draft-only header edits.
#[utoipa::path(operation_id = "purchase_orders_update", patch, path = "/api/purchase-orders/{id}", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateOrderRequest,
    responses((status = 200, body = OrderMutationResponse),
              (status = 409, description = "order is not a draft"),
              (status = 404, description = "order not visible in scope")))]
pub async fn update_order(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<UpdateOrderRequest>,
) -> ApiResult<Json<OrderMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let financial = financial_read(&authed);
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ORDER_ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            assert_order_visible(&mut ctx.tx, org_id, &authed.principal, id).await?;
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = require_draft(&mut ctx.tx, org_id, id).await?;
        let mut order = before.clone();
        if let Some(reference) = &req.vendor_reference {
            order.vendor_reference = reference.clone();
        }
        if let Some(project_id) = &req.project_id {
            validate_project(&mut ctx.tx, org_id, *project_id).await?;
            order.project_id = *project_id;
        }
        if let Some(notes) = &req.notes {
            order.notes = notes.clone();
        }
        if let Some(expected_at) = &req.expected_at {
            order.expected_at = *expected_at;
        }
        let mut intent = core_db::purchasing::get_order_intent(&mut *ctx.tx, org_id, id).await?;
        if let Some(acquired_for) = req.acquired_for {
            intent.acquired_for = acquired_for;
        }
        if let Some(client_id) = &req.client_id {
            intent.client_id = *client_id;
        }
        if let Some(ticket_id) = &req.ticket_id {
            intent.ticket_id = *ticket_id;
        }
        if let Some(ownership) = &req.ownership {
            intent.ownership = *ownership;
        }
        if let Some(delivery_mode) = req.delivery_mode {
            intent.delivery_mode = delivery_mode;
        }
        if let Some(recharge_rule) = &req.recharge_rule {
            intent.recharge_rule = recharge_rule.clone();
        }
        intent.project_id = order.project_id;
        validate_order_intent(&mut ctx.tx, org_id, &authed.principal, &mut intent).await?;
        validate_existing_lines(&mut ctx.tx, org_id, id, &intent).await?;
        order.updated_at = chrono::Utc::now();
        order.validate().map_err(ApiError::from)?;
        core_db::purchasing::update_purchase_order(&mut *ctx.tx, &order).await?;
        core_db::purchasing::set_order_intent(&mut *ctx.tx, org_id, id, &intent).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ORDER_ENTITY,
                entity_id: id,
                version,
            },
        )
        .await?;
        let lines = core_db::purchasing::list_po_lines(&mut ctx.tx, org_id, id).await?;
        ctx.emit(
            core_domain::events::types::PURCHASE_ORDER_UPDATED,
            ORDER_ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::PurchaseOrderUpdated {
                order: order.clone(),
                lines: lines.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "purchase.order_updated".into(),
                entity_type: Some(ORDER_ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "number": order.number }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let detail = load_detail(&mut ctx.tx, org_id, order.clone(), financial).await?;
        let response = OrderMutationResponse {
            detail: detail.clone(),
            mutation: instant::record(
                &mut ctx,
                instant::RecordSpec {
                    entity: ORDER_ENTITY,
                    entity_id: id,
                    op: "update",
                    seq: write_seq,
                    before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                    after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                    updated_at: order.updated_at,
                    key: key.as_deref(),
                    payload: &detail,
                    provenance: Default::default(),
                },
            )
            .await?,
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/purchase-orders/{id}/lines — draft-only line add.
#[utoipa::path(operation_id = "purchase_orders_add_line", post, path = "/api/purchase-orders/{id}/lines", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = OrderLineInput,
    responses((status = 200, body = OrderMutationResponse),
              (status = 409, description = "order is not a draft"),
              (status = 422, description = "line validation failed")))]
pub async fn add_order_line(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<AddOrderLineRequest>,
) -> ApiResult<Json<OrderMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let financial = financial_read(&authed);
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "purchase_order_line",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            assert_order_visible(&mut ctx.tx, org_id, &authed.principal, id).await?;
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let mut order = require_draft(&mut ctx.tx, org_id, id).await?;
        let header = core_db::purchasing::get_order_intent(&mut *ctx.tx, org_id, id).await?;
        let existing = core_db::purchasing::list_po_lines(&mut ctx.tx, org_id, id).await?;
        let line = line_from_input(org_id, id, &req.line, existing.len() as i32).await?;
        let line_intent = line_intent_from_input(&req.line, &header)?;
        validate_line_layout(&mut ctx.tx, org_id, &line_intent).await?;
        core_db::purchasing::insert_po_line(&mut *ctx.tx, &line).await?;
        core_db::purchasing::set_line_intent(&mut *ctx.tx, org_id, line.id, &line_intent).await?;
        order.updated_at = chrono::Utc::now();
        core_db::purchasing::update_purchase_order(&mut *ctx.tx, &order).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "purchase_order_line",
                entity_id: line.id,
                version: None,
            },
        )
        .await?;
        let lines = core_db::purchasing::list_po_lines(&mut ctx.tx, org_id, id).await?;
        ctx.emit(
            core_domain::events::types::PURCHASE_ORDER_UPDATED,
            ORDER_ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::PurchaseOrderUpdated {
                order: order.clone(),
                lines: lines.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "purchase.order_line_added".into(),
                entity_type: Some("purchase_order_line".into()),
                entity_id: Some(line.id.to_string()),
                detail: serde_json::json!({ "order_id": id }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let detail = load_detail(&mut ctx.tx, org_id, order.clone(), financial).await?;
        let response = OrderMutationResponse {
            detail: detail.clone(),
            mutation: instant::record(
                &mut ctx,
                instant::RecordSpec {
                    entity: "purchase_order_line",
                    entity_id: line.id,
                    op: "create",
                    seq: write_seq,
                    before: None,
                    after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                    updated_at: order.updated_at,
                    key: key.as_deref(),
                    payload: &detail,
                    provenance: Default::default(),
                },
            )
            .await?,
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/purchase-orders/{id}/lines/{line_id} — draft-only line edit.
#[utoipa::path(operation_id = "purchase_orders_update_line", patch, path = "/api/purchase-orders/{id}/lines/{line_id}", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("line_id" = Uuid, Path)),
    request_body = AddOrderLineRequest,
    responses((status = 200, body = OrderMutationResponse),
              (status = 404, description = "order or line not visible in scope"),
              (status = 409, description = "order is not a draft or the line already received goods")))]
pub async fn update_order_line(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path((id, line_id)): axum::extract::Path<(Uuid, Uuid)>,
    axum::extract::Json(req): axum::extract::Json<AddOrderLineRequest>,
) -> ApiResult<Json<OrderMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let financial = financial_read(&authed);
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "purchase_order_line",
                entity_id: Some(line_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            assert_order_visible(&mut ctx.tx, org_id, &authed.principal, id).await?;
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let mut order = require_draft(&mut ctx.tx, org_id, id).await?;
        let before = core_db::purchasing::get_po_line(&mut *ctx.tx, org_id, line_id).await?;
        if before.purchase_order_id != id {
            return Err(ApiError::Validation(
                "the line does not belong to this purchase order".into(),
            ));
        }
        if before.qty_received_milli > 0 {
            return Err(ApiError::Conflict(
                "a line that already received goods cannot be edited".into(),
            ));
        }
        let mut line = before.clone();
        line.stock_item_id = req.line.stock_item_id;
        line.catalog_item_id = req.line.catalog_item_id;
        line.description = req.line.description.trim().to_string();
        line.qty_ordered_milli = req.line.qty_ordered_milli;
        line.unit_cost_minor = req.line.unit_cost_minor;
        if let Some(position) = req.line.position {
            line.position = position;
        }
        line.receipt_state = ReceiptState::Pending;
        line.updated_at = chrono::Utc::now();
        line.validate().map_err(ApiError::from)?;
        let header = core_db::purchasing::get_order_intent(&mut *ctx.tx, org_id, id).await?;
        let before_intent = core_db::purchasing::list_line_intents(&mut ctx.tx, org_id, id)
            .await?
            .get(&line_id)
            .cloned()
            .unwrap_or_else(core_db::purchasing::LineIntent::stock);
        // Intent fields are keep-when-absent here, unlike the stock fields
        // above: clearing an override by omission would silently move a line to
        // the header's destination. A named destination stays until it is named
        // differently.
        let merged = OrderLineInput {
            stock_item_id: req.line.stock_item_id,
            catalog_item_id: req.line.catalog_item_id,
            description: req.line.description.clone(),
            qty_ordered_milli: req.line.qty_ordered_milli,
            unit_cost_minor: req.line.unit_cost_minor,
            position: req.line.position,
            line_kind: req.line.line_kind.or(Some(before_intent.line_kind)),
            destination_class: req
                .line
                .destination_class
                .or(before_intent.destination_class),
            asset_layout_id: req.line.asset_layout_id.or(before_intent.asset_layout_id),
            serial_expected: req
                .line
                .serial_expected
                .or(Some(before_intent.serial_expected)),
            weight_g: req.line.weight_g.or(before_intent.weight_g),
            sell_price_minor: req.line.sell_price_minor.or(before_intent.sell_price_minor),
            billable: req.line.billable.or(Some(before_intent.billable)),
        };
        let line_intent = line_intent_from_input(&merged, &header)?;
        validate_line_layout(&mut ctx.tx, org_id, &line_intent).await?;
        core_db::purchasing::update_po_line(&mut *ctx.tx, &line).await?;
        core_db::purchasing::set_line_intent(&mut *ctx.tx, org_id, line_id, &line_intent).await?;
        order.updated_at = chrono::Utc::now();
        core_db::purchasing::update_purchase_order(&mut *ctx.tx, &order).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "purchase_order_line",
                entity_id: line_id,
                version,
            },
        )
        .await?;
        let lines = core_db::purchasing::list_po_lines(&mut ctx.tx, org_id, id).await?;
        ctx.emit(
            core_domain::events::types::PURCHASE_ORDER_UPDATED,
            ORDER_ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::PurchaseOrderUpdated {
                order: order.clone(),
                lines: lines.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "purchase.order_line_updated".into(),
                entity_type: Some("purchase_order_line".into()),
                entity_id: Some(line_id.to_string()),
                detail: serde_json::json!({ "order_id": id }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let detail = load_detail(&mut ctx.tx, org_id, order.clone(), financial).await?;
        let response = OrderMutationResponse {
            detail: detail.clone(),
            mutation: instant::record(
                &mut ctx,
                instant::RecordSpec {
                    entity: "purchase_order_line",
                    entity_id: line_id,
                    op: "update",
                    seq: write_seq,
                    before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                    after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                    updated_at: line.updated_at,
                    key: key.as_deref(),
                    payload: &detail,
                    provenance: Default::default(),
                },
            )
            .await?,
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct DeleteOrderLineRequest {
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// DELETE /api/purchase-orders/{id}/lines/{line_id} — draft-only.
#[utoipa::path(operation_id = "purchase_orders_delete_line", delete, path = "/api/purchase-orders/{id}/lines/{line_id}", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("line_id" = Uuid, Path)),
    request_body = DeleteOrderLineRequest,
    responses((status = 200, body = OrderMutationResponse),
              (status = 404, description = "order or line not visible in scope"),
              (status = 409, description = "order is not a draft")))]
pub async fn delete_order_line(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path((id, line_id)): axum::extract::Path<(Uuid, Uuid)>,
    axum::extract::Json(req): axum::extract::Json<DeleteOrderLineRequest>,
) -> ApiResult<Json<OrderMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let financial = financial_read(&authed);
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "purchase_order_line",
                entity_id: Some(line_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            assert_order_visible(&mut ctx.tx, org_id, &authed.principal, id).await?;
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let mut order = require_draft(&mut ctx.tx, org_id, id).await?;
        let before = core_db::purchasing::get_po_line(&mut *ctx.tx, org_id, line_id).await?;
        if before.purchase_order_id != id {
            return Err(ApiError::Validation(
                "the line does not belong to this purchase order".into(),
            ));
        }
        core_db::purchasing::delete_po_line(&mut *ctx.tx, org_id, line_id).await?;
        order.updated_at = chrono::Utc::now();
        core_db::purchasing::update_purchase_order(&mut *ctx.tx, &order).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "purchase_order_line",
                entity_id: line_id,
                version: None,
            },
        )
        .await?;
        let lines = core_db::purchasing::list_po_lines(&mut ctx.tx, org_id, id).await?;
        ctx.emit(
            core_domain::events::types::PURCHASE_ORDER_UPDATED,
            ORDER_ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::PurchaseOrderUpdated {
                order: order.clone(),
                lines: lines.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "purchase.order_line_removed".into(),
                entity_type: Some("purchase_order_line".into()),
                entity_id: Some(line_id.to_string()),
                detail: serde_json::json!({ "order_id": id }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let detail = load_detail(&mut ctx.tx, org_id, order.clone(), financial).await?;
        let response = OrderMutationResponse {
            detail: detail.clone(),
            mutation: instant::record(
                &mut ctx,
                instant::RecordSpec {
                    entity: "purchase_order_line",
                    entity_id: line_id,
                    op: "delete",
                    seq: write_seq,
                    before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                    after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                    updated_at: order.updated_at,
                    key: key.as_deref(),
                    payload: &detail,
                    provenance: Default::default(),
                },
            )
            .await?,
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// Emits the lifecycle event + audit row for an operator status move.
async fn finish_status_change(
    ctx: &mut org_tx::OrgTx<'_>,
    org_id: Uuid,
    user_id: Uuid,
    action: &str,
    from: &PurchaseOrderStatus,
    order: &PurchaseOrder,
) -> ApiResult<()> {
    ctx.emit(
        core_domain::events::types::PURCHASE_ORDER_STATUS_CHANGED,
        ORDER_ENTITY,
        Some(order.id.to_string().as_str()),
        &serde_json::to_value(core_domain::events::PurchaseOrderStatusChanged {
            order_id: order.id,
            number: order.number.clone(),
            from: from.as_str().to_string(),
            to: order.status.as_str().to_string(),
            close_kind: order.close_kind.map(|k| k.as_str().to_string()),
            occurred_at: chrono::Utc::now(),
        })
        .unwrap_or_else(|_| serde_json::json!({})),
    )
    .await?;
    core_db::audit::insert(
        &mut *ctx.tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(user_id),
            action: action.to_string(),
            entity_type: Some(ORDER_ENTITY.into()),
            entity_id: Some(order.id.to_string()),
            detail: serde_json::json!({
                "number": order.number,
                "from": from.as_str(),
                "to": order.status.as_str(),
                "close_kind": order.close_kind.map(|k| k.as_str()),
            }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    Ok(())
}

/// POST /api/purchase-orders/{id}/order — draft → ordered.
#[utoipa::path(operation_id = "purchase_orders_send", post, path = "/api/purchase-orders/{id}/order", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CloseOrderRequest,
    responses((status = 200, body = OrderMutationResponse),
              (status = 409, description = "order is not a draft or has no lines")))]
pub async fn place_order(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<CloseOrderRequest>,
) -> ApiResult<Json<OrderMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let financial = financial_read(&authed);
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ORDER_ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            assert_order_visible(&mut ctx.tx, org_id, &authed.principal, id).await?;
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = require_draft(&mut ctx.tx, org_id, id).await?;
        let lines = core_db::purchasing::list_po_lines(&mut ctx.tx, org_id, id).await?;
        if lines.is_empty() {
            return Err(ApiError::Conflict(
                "a purchase order needs at least one line before it can be ordered".into(),
            ));
        }
        let order = core_db::purchasing::mark_ordered(&mut *ctx.tx, org_id, id).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ORDER_ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        finish_status_change(
            &mut ctx,
            org_id,
            user_id,
            "purchase.order_ordered",
            &before.status,
            &order,
        )
        .await?;
        let detail = load_detail(&mut ctx.tx, org_id, order.clone(), financial).await?;
        let response = OrderMutationResponse {
            detail: detail.clone(),
            mutation: instant::record(
                &mut ctx,
                instant::RecordSpec {
                    entity: ORDER_ENTITY,
                    entity_id: id,
                    op: "status_change",
                    seq: write_seq,
                    before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                    after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                    updated_at: order.updated_at,
                    key: key.as_deref(),
                    payload: &detail,
                    provenance: Default::default(),
                },
            )
            .await?,
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/purchase-orders/{id}/close — received|partially_received →
/// closed; the close disposition is the typed under-receipt record.
#[utoipa::path(operation_id = "purchase_orders_close", post, path = "/api/purchase-orders/{id}/close", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CloseOrderRequest,
    responses((status = 200, body = OrderMutationResponse),
              (status = 409, description = "order is not in a closable state")))]
pub async fn close_order_route(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<CloseOrderRequest>,
) -> ApiResult<Json<OrderMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let financial = financial_read(&authed);
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ORDER_ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            assert_order_visible(&mut ctx.tx, org_id, &authed.principal, id).await?;
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::purchasing::get_purchase_order(&mut *ctx.tx, org_id, id).await?;
        if !matches!(
            before.status,
            PurchaseOrderStatus::Received | PurchaseOrderStatus::PartiallyReceived
        ) {
            return Err(ApiError::Conflict(format!(
                "purchase order {} is {} — only received or partially received orders close",
                before.number,
                before.status.as_str()
            )));
        }
        let lines = core_db::purchasing::list_po_lines(&mut ctx.tx, org_id, id).await?;
        let all_complete = lines
            .iter()
            .all(|l| matches!(l.receipt_state, ReceiptState::Complete | ReceiptState::Over));
        let kind = if all_complete {
            CloseKind::Full
        } else {
            CloseKind::Short
        };
        let order = core_db::purchasing::close_order(&mut *ctx.tx, org_id, id, kind).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ORDER_ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        finish_status_change(
            &mut ctx,
            org_id,
            user_id,
            "purchase.order_closed",
            &before.status,
            &order,
        )
        .await?;
        let detail = load_detail(&mut ctx.tx, org_id, order.clone(), financial).await?;
        let response = OrderMutationResponse {
            detail: detail.clone(),
            mutation: instant::record(
                &mut ctx,
                instant::RecordSpec {
                    entity: ORDER_ENTITY,
                    entity_id: id,
                    op: "status_change",
                    seq: write_seq,
                    before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                    after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                    updated_at: order.updated_at,
                    key: key.as_deref(),
                    payload: &detail,
                    provenance: Default::default(),
                },
            )
            .await?,
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/purchase-orders/{id}/cancel — draft|ordered, nothing received.
#[utoipa::path(operation_id = "purchase_orders_cancel", post, path = "/api/purchase-orders/{id}/cancel", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CancelOrderRequest,
    responses((status = 200, body = OrderMutationResponse),
              (status = 409, description = "order already received goods or is terminal")))]
pub async fn cancel_order_route(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<CancelOrderRequest>,
) -> ApiResult<Json<OrderMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let financial = financial_read(&authed);
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ORDER_ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            assert_order_visible(&mut ctx.tx, org_id, &authed.principal, id).await?;
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::purchasing::get_purchase_order(&mut *ctx.tx, org_id, id).await?;
        if !matches!(
            before.status,
            PurchaseOrderStatus::Draft | PurchaseOrderStatus::Ordered
        ) {
            return Err(ApiError::Conflict(format!(
                "purchase order {} is {} — received quantities are closed short, not cancelled",
                before.number,
                before.status.as_str()
            )));
        }
        let lines = core_db::purchasing::list_po_lines(&mut ctx.tx, org_id, id).await?;
        if lines.iter().any(|l| l.qty_received_milli > 0) {
            return Err(ApiError::Conflict(
                "the order already received goods — close it short instead of cancelling".into(),
            ));
        }
        let order =
            core_db::purchasing::cancel_order(&mut *ctx.tx, org_id, id, req.reason.as_deref())
                .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ORDER_ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        finish_status_change(
            &mut ctx,
            org_id,
            user_id,
            "purchase.order_cancelled",
            &before.status,
            &order,
        )
        .await?;
        let detail = load_detail(&mut ctx.tx, org_id, order.clone(), financial).await?;
        let response = OrderMutationResponse {
            detail: detail.clone(),
            mutation: instant::record(
                &mut ctx,
                instant::RecordSpec {
                    entity: ORDER_ENTITY,
                    entity_id: id,
                    op: "status_change",
                    seq: write_seq,
                    before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                    after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                    updated_at: order.updated_at,
                    key: key.as_deref(),
                    payload: &detail,
                    provenance: Default::default(),
                },
            )
            .await?,
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// One receipt line's plan, resolved before anything is written: what the line
/// buys, where it lands, the stock item it names (when any), the serials it
/// carries and the quantity that will be received.
struct ReceiptPlan {
    line: PurchaseOrderLine,
    intent: core_db::purchasing::LineIntent,
    destination: CustodyClass,
    /// `None` when the line names no stock item — a fleet or client asset line
    /// needs an identity, not a SKU.
    item: Option<core_domain::entities::StockItem>,
    serials: Vec<String>,
    qty_milli: i64,
    creates_asset: bool,
}

fn fx_columns(snapshot: crate::routes::fx::LockedSnapshot) -> core_db::purchasing::FxColumns {
    core_db::purchasing::FxColumns {
        rate_used: snapshot.rate_used,
        rate_date: snapshot.rate_date,
        source: snapshot.source,
        base_amount_minor: snapshot.base_amount_minor,
    }
}

/// Creates the asset a receipt line lands as (plan §5.2). Scope and ownership
/// come from the destination class, the capital basis and its FX snapshot are
/// stamped from the receipt, and a class the row's own words cannot express
/// (the rental pool, a vendor rental) is reached by a class movement that cites
/// the receipt line. `core_db::assets::create_asset` stays the only writer of
/// the asset model.
#[allow(clippy::too_many_arguments)]
async fn create_receipt_asset(
    ctx: &mut crate::org_tx::OrgTx<'_>,
    order: &PurchaseOrder,
    header: &OrderIntent,
    plan: &ReceiptPlan,
    serial: Option<&str>,
    receipt_line_id: Uuid,
    user_id: Uuid,
    now: chrono::DateTime<chrono::Utc>,
) -> ApiResult<Uuid> {
    let org_id = ctx.org_id;
    let (scope, client_id, derived) = match plan.destination {
        CustodyClass::InternalFleet | CustodyClass::RentalIn | CustodyClass::RentalOut => {
            ("internal_org", None, CustodyClass::InternalFleet)
        }
        CustodyClass::ClientCustody => ("client", header.client_id, CustodyClass::ClientCustody),
        other => {
            return Err(ApiError::Validation(format!(
                "{} does not create an asset row",
                other.as_str()
            )))
        }
    };
    let asset_id = Uuid::now_v7();
    // The order's number plus the receipt line's short id: unique per unit, and
    // still readable on a label.
    let tag = format!("{}-{}", order.number, short_id(receipt_line_id));
    let name: String = plan.line.description.chars().take(200).collect();
    let fields = serde_json::json!({});
    core_db::assets::create_asset(
        &mut *ctx.tx,
        org_id,
        asset_id,
        client_id,
        scope,
        None,
        &tag,
        &name,
        "device",
        "active",
        plan.intent.asset_layout_id,
        &fields,
    )
    .await?;
    if let Some(serial) = serial {
        core_db::config_identity::refresh_identity_keys(
            &mut *ctx.tx,
            org_id,
            asset_id,
            &serde_json::json!({ "serial": serial }),
        )
        .await?;
    }
    let mut patch = core_db::assets::AssetLifecyclePatch {
        purchase_date: Some(now.date_naive()),
        purchase_cost_minor: Some(plan.line.unit_cost_minor),
        purchase_currency: Some(order.currency.clone()),
        ..Default::default()
    };
    if plan.destination == CustodyClass::RentalIn {
        patch.ownership = Some(Ownership::Rented.as_str().to_string());
    }
    let locked = crate::routes::fx::lock_snapshot(
        &mut ctx.tx,
        org_id,
        plan.line.unit_cost_minor,
        &order.currency,
        now.date_naive(),
    )
    .await?;
    if let Some(snapshot) = locked {
        patch.purchase_fx_rate_used = Some(snapshot.rate_used);
        patch.purchase_fx_rate_date = Some(snapshot.rate_date);
        patch.purchase_fx_source = Some(snapshot.source);
        patch.base_purchase_cost_minor = Some(snapshot.base_amount_minor);
    }
    core_db::assets::update_asset_lifecycle(&mut *ctx.tx, org_id, asset_id, &patch).await?;
    if derived != plan.destination {
        core_db::purchasing::record_asset_class_movement(
            &mut *ctx.tx,
            org_id,
            asset_id,
            derived,
            plan.destination,
            &format!("received on purchase order {}", order.number),
            receipt_line_id,
            user_id,
        )
        .await?;
    }
    ctx.emit(
        core_domain::events::types::ASSET_CREATED,
        "asset",
        Some(asset_id.to_string().as_str()),
        &serde_json::json!({
            "asset_id": asset_id,
            "client_id": client_id,
            "purchase_order_id": order.id,
            "receipt_line_id": receipt_line_id,
            "custody_class": plan.destination.as_str(),
        }),
    )
    .await?;
    Ok(asset_id)
}

fn short_id(seed: Uuid) -> String {
    seed.simple().to_string().chars().take(8).collect()
}

/// Issues the recharge a client purchase line resolves to (plan §5.3): the
/// resolved rule applied to the receipt line's basis, a billable cost line on
/// the ticket the purchase serves (when it names one), and the recharge row
/// that keeps the rule, the basis, both currencies and both FX snapshots.
#[allow(clippy::too_many_arguments)]
async fn issue_recharge(
    ctx: &mut crate::org_tx::OrgTx<'_>,
    principal: &core_domain::entities::Principal,
    order: &PurchaseOrder,
    header: &OrderIntent,
    plan: &ReceiptPlan,
    resolved: &core_db::purchasing::ResolvedRechargeRule,
    receipt_line: &PurchaseReceiptLine,
    leg_qty_milli: i64,
    user_id: Uuid,
    now: chrono::DateTime<chrono::Utc>,
) -> ApiResult<()> {
    let org_id = ctx.org_id;
    let basis = core_domain::fx::fact_amount_minor(
        leg_qty_milli as f64 / 1000.0,
        plan.line.unit_cost_minor,
    )
    .ok_or_else(|| ApiError::Validation("recharge basis out of range".into()))?;
    let charge = recharge_charge_minor(basis, &resolved.rule)?;
    let charge_currency = match resolved.rule.kind {
        RechargeRuleKind::Fixed => resolved.rule.currency.clone().ok_or_else(|| {
            ApiError::Validation("a fixed recharge rule carries no currency".into())
        })?,
        _ => order.currency.clone(),
    };
    let cost_fx = crate::routes::fx::lock_snapshot(
        &mut ctx.tx,
        org_id,
        basis,
        &order.currency,
        now.date_naive(),
    )
    .await?
    .map(fx_columns);
    let description = format!("Recharge {}: {}", order.number, plan.line.description);
    let (cost_line_id, charge_fx) = match header.ticket_id {
        Some(ticket_id) => {
            let kind = if plan.intent.line_kind == LineKind::Service {
                CostLineKind::Other
            } else {
                CostLineKind::Parts
            };
            let (line, snapshot) = crate::routes::costs::write_recharge_cost_line(
                ctx,
                principal,
                crate::routes::costs::RechargeCostLine {
                    ticket_id,
                    kind,
                    description: &description,
                    amount_minor: charge,
                    currency: &charge_currency,
                    reporting_class: recharge_reporting_class(
                        header.acquired_for,
                        plan.intent.line_kind,
                    ),
                    receipt_line_id: receipt_line.id,
                    billable: plan.intent.billable,
                },
            )
            .await?;
            (Some(line.id), snapshot.map(fx_columns))
        }
        None => {
            // No ticket to carry the charge, so the recharge row is the record
            // and the invoice join cites the receipt line. The charge snapshot
            // is still locked, because the amount is a fact either way.
            let snapshot = crate::routes::fx::lock_snapshot(
                &mut ctx.tx,
                org_id,
                charge,
                &charge_currency,
                now.date_naive(),
            )
            .await?;
            (None, snapshot.map(fx_columns))
        }
    };
    core_db::purchasing::insert_recharge(
        &mut *ctx.tx,
        &core_db::purchasing::NewRecharge {
            id: Uuid::now_v7(),
            org_id,
            purchase_order_id: order.id,
            po_line_id: plan.line.id,
            receipt_line_id: receipt_line.id,
            client_id: header.client_id,
            ticket_id: header.ticket_id,
            project_id: header.project_id,
            rule: resolved.rule.clone(),
            rule_source: resolved.source,
            basis_amount_minor: basis,
            basis_currency: order.currency.clone(),
            cost_fx,
            charge_amount_minor: charge,
            charge_currency,
            charge_fx,
            cost_line_id,
            created_by: user_id,
            issued_at: now,
        },
    )
    .await?;
    Ok(())
}

/// POST /api/purchase-orders/{id}/receive — the receipt that posts into
/// the lane-1 movement ledger at `location_id`.
#[utoipa::path(operation_id = "purchase_orders_receive", post, path = "/api/purchase-orders/{id}/receive", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ReceiveOrderRequest,
    responses((status = 200, body = ReceiptMutationResponse),
              (status = 404, description = "order or location not visible in scope"),
              (status = 409, description = "order not receivable, over-receipt without acknowledgement, or a line without a stock item"),
              (status = 422, description = "serial numbers inconsistent with the received quantity")))]
pub async fn receive_order(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<ReceiveOrderRequest>,
) -> ApiResult<Json<ReceiptMutationResponse>> {
    if req.lines.is_empty() {
        return Err(ApiError::Validation(
            "a receipt needs at least one line".into(),
        ));
    }
    for line in &req.lines {
        if line.qty_milli <= 0 {
            return Err(ApiError::Validation(
                "receipt line quantities must be positive".into(),
            ));
        }
    }
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let financial = financial_read(&authed);
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: RECEIPT_ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            assert_order_visible(&mut ctx.tx, org_id, &authed.principal, id).await?;
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // Row lock: concurrent receipts against this PO serialize, so the
        // line states the over-receipt gate reads cannot go stale.
        let order = core_db::purchasing::lock_purchase_order(&mut ctx.tx, org_id, id).await?;
        if !matches!(
            order.status,
            PurchaseOrderStatus::Ordered | PurchaseOrderStatus::PartiallyReceived
        ) {
            return Err(ApiError::Conflict(if order.status == PurchaseOrderStatus::Draft {
                format!(
                    "purchase order {} is a draft — order it before receiving",
                    order.number
                )
            } else {
                format!(
                    "purchase order {} is {} — it cannot receive more goods",
                    order.number,
                    order.status.as_str()
                )
            }));
        }
        let location =
            core_db::stock::get_stock_location(&mut *ctx.tx, org_id, req.location_id).await?;
        if !location.active {
            return Err(ApiError::Conflict(format!(
                "stock location {} is archived",
                location.id
            )));
        }
        let existing = core_db::purchasing::list_po_lines(&mut ctx.tx, org_id, id).await?;
        let by_id: std::collections::HashMap<Uuid, PurchaseOrderLine> =
            existing.iter().map(|l| (l.id, l.clone())).collect();
        let header = core_db::purchasing::get_order_intent(&mut *ctx.tx, org_id, id).await?;
        // A client purchase writes the client's goods and their charge, so the
        // restricted-client rule is enforced in the transaction that writes,
        // not only where the order was created.
        if let Some(client_id) = header.client_id {
            crate::routes::clients::require_client_visible(
                &mut *ctx.tx,
                org_id,
                client_id,
                &authed.principal,
            )
            .await?;
        }
        let line_intents = core_db::purchasing::list_line_intents(&mut ctx.tx, org_id, id).await?;

        // Plan the receipt: validate each line, its destination and its
        // serial/quantity consistency BEFORE anything is written. Every refusal
        // here is typed and names the line.
        let mut planned: Vec<ReceiptPlan> = Vec::with_capacity(req.lines.len());
        let mut over: Vec<String> = Vec::new();
        let mut seen_serials: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for input in &req.lines {
            let line = by_id.get(&input.line_id).cloned().ok_or_else(|| {
                ApiError::Validation(format!("line {} is not part of this order", input.line_id))
            })?;
            let intent = line_intents
                .get(&input.line_id)
                .cloned()
                .unwrap_or_else(core_db::purchasing::LineIntent::stock);
            let destination = resolve_destination(&intent, &header);
            // A pass-through or service line is bought to be recharged, not to
            // become ours; the schema refuses the pair, and the receipt refuses
            // it again because a historical line may predate the constraint.
            if !intent.line_kind.can_enter_owned_quantity() && destination.counts_as_owned() {
                return Err(ApiError::Validation(format!(
                    "line \"{}\" is a {} line — it cannot be received into {} (owned quantity)",
                    line.description,
                    intent.line_kind.as_str(),
                    destination.as_str()
                )));
            }
            if destination == CustodyClass::ClientCustody && header.client_id.is_none() {
                return Err(ApiError::Validation(format!(
                    "line \"{}\" lands in client custody but the purchase order names no client",
                    line.description
                )));
            }
            if destination == CustodyClass::RentalIn && header.ownership != Some(Ownership::Rented)
            {
                return Err(ApiError::Validation(format!(
                    "line \"{}\" is a vendor rental but the purchase order is not rented",
                    line.description
                )));
            }
            if destination == CustodyClass::EwasteCustody && intent.line_kind != LineKind::Service {
                return Err(ApiError::Conflict(format!(
                    "line \"{}\" lands in e-waste custody — units are received through an e-waste intake, not a purchase receipt",
                    line.description
                )));
            }
            let item = match line.stock_item_id {
                Some(stock_item_id) => {
                    let item =
                        core_db::stock::get_stock_item(&mut *ctx.tx, org_id, stock_item_id).await?;
                    if !item.active {
                        return Err(ApiError::Conflict(format!(
                            "stock item {} is archived",
                            item.id
                        )));
                    }
                    Some(item)
                }
                None => None,
            };
            if destination == CustodyClass::SaleStock && item.is_none() {
                return Err(ApiError::Conflict(format!(
                    "line \"{}\" has no stock item — a catalog-only line cannot receive into the stock ledger",
                    line.description
                )));
            }
            let serialized_item = item.as_ref().is_some_and(|i| i.kind == "serialized");
            let creates_asset = match destination {
                CustodyClass::InternalFleet
                | CustodyClass::RentalIn
                | CustodyClass::RentalOut => true,
                CustodyClass::ClientCustody => {
                    intent.line_kind.is_serialised() || serialized_item
                }
                _ => false,
            };
            let mut serials: Vec<String> = Vec::new();
            for serial in input.serial_numbers.clone().unwrap_or_default() {
                let trimmed = serial.trim().to_string();
                if trimmed.is_empty() {
                    return Err(ApiError::Validation(
                        "serial numbers must not be empty".into(),
                    ));
                }
                if !seen_serials.insert(trimmed.to_lowercase()) {
                    return Err(ApiError::Validation(format!(
                        "serial \"{trimmed}\" appears twice in this receipt"
                    )));
                }
                serials.push(trimmed);
            }
            let serial_required = intent.serial_expected || serialized_item;
            if serial_required && serials.is_empty() {
                return Err(ApiError::Validation(format!(
                    "line \"{}\" is a serialized line — pass one serial per unit",
                    line.description
                )));
            }
            // A serial names a physical unit. Sale stock tracks units in the
            // ledger; every other class does it through an asset row. A lot
            // (client custody of a consumable, a service) has no unit to name,
            // and a consumable stock line never had one either.
            let names_a_unit = creates_asset
                || (destination == CustodyClass::SaleStock && serialized_item);
            if !names_a_unit && !serials.is_empty() {
                return Err(ApiError::Validation(format!(
                    "line \"{}\" does not become a unit — it must not carry serials",
                    line.description
                )));
            }
            let qty_milli = if serials.is_empty() {
                input.qty_milli
            } else {
                1000 * serials.len() as i64
            };
            if !serials.is_empty() && input.qty_milli != qty_milli {
                return Err(ApiError::Validation(format!(
                    "line \"{}\" received quantity must be 1000 × the serial count",
                    line.description
                )));
            }
            if creates_asset && serials.is_empty() && qty_milli % 1000 != 0 {
                return Err(ApiError::Validation(format!(
                    "line \"{}\" receives whole units into {} — {} milli is not a whole count",
                    line.description,
                    destination.as_str(),
                    qty_milli
                )));
            }
            if !req.allow_over_receipt
                && line.qty_received_milli + qty_milli > line.qty_ordered_milli
            {
                over.push(format!(
                    "{} (already {} + {} > ordered {})",
                    line.description, line.qty_received_milli, qty_milli, line.qty_ordered_milli
                ));
            }
            planned.push(ReceiptPlan {
                line,
                intent,
                destination,
                item,
                serials,
                qty_milli,
                creates_asset,
            });
        }
        if !over.is_empty() {
            return Err(ApiError::Conflict(format!(
                "over-receipt refused for: {} — pass allow_over_receipt=true to accept the overage",
                over.join(", ")
            )));
        }

        let now = chrono::Utc::now();
        let receipt = PurchaseReceipt {
            id: Uuid::now_v7(),
            org_id,
            purchase_order_id: id,
            location_id: req.location_id,
            carrier: req.carrier.clone(),
            tracking_ref: req.tracking_ref.clone(),
            note: req.note.clone(),
            received_at: now,
            created_by: Some(user_id),
            created_at: now,
        };
        core_db::purchasing::insert_receipt(&mut *ctx.tx, &receipt).await?;

        // The ladder resolves once for the whole receipt: every line on one
        // order serves the same client, ticket and project.
        let resolved_rule = if header.client_id.is_some() {
            Some(
                core_db::purchasing::resolve_recharge_rule(
                    &mut ctx.tx,
                    org_id,
                    header.recharge_rule.as_ref(),
                    header.ticket_id,
                    header.project_id,
                    header.client_id,
                )
                .await?,
            )
        } else {
            None
        };

        let mut receipt_lines: Vec<PurchaseReceiptLine> = Vec::new();
        let mut receipt_line_views: Vec<ReceiptLineView> = Vec::new();
        let mut movement_count = 0usize;
        let mut asset_count = 0usize;
        let mut recharge_count = 0usize;
        for plan in &planned {
            // One leg is one physical unit for a named unit, and one leg is the
            // whole quantity for a lot or a service. Each leg is exactly one
            // receipt line, which is exactly one consequence.
            let legs: Vec<(i64, Option<String>)> = if plan.creates_asset {
                if plan.serials.is_empty() {
                    // No serials named: one asset per whole unit. Planning
                    // refused a fractional count, so this is exact.
                    (0..plan.qty_milli / 1000)
                        .map(|_| (1000i64, None))
                        .collect()
                } else {
                    plan.serials
                        .iter()
                        .map(|s| (1000i64, Some(s.clone())))
                        .collect()
                }
            } else if plan.destination == CustodyClass::SaleStock && !plan.serials.is_empty() {
                plan.serials
                    .iter()
                    .map(|s| (1000i64, Some(s.clone())))
                    .collect()
            } else {
                vec![(plan.qty_milli, None)]
            };
            for (qty, serial) in legs {
                let receipt_line_id = Uuid::now_v7();
                let (movement_id, device_asset_id, leg_qty) = match plan.destination {
                    CustodyClass::SaleStock => {
                        let item = plan.item.as_ref().ok_or_else(|| {
                            ApiError::Conflict(format!(
                                "line \"{}\" has no stock item — a catalog-only line cannot receive into the stock ledger",
                                plan.line.description
                            ))
                        })?;
                        let mut movement = core_domain::entities::StockMovement {
                            id: Uuid::now_v7(),
                            org_id,
                            stock_item_id: item.id,
                            kind: "receive".into(),
                            from_location_id: None,
                            to_location_id: Some(req.location_id),
                            qty_milli: qty,
                            cost_minor: Some(plan.line.unit_cost_minor),
                            currency: Some(order.currency.clone()),
                            fx_rate_used: None,
                            fx_rate_date: None,
                            fx_source: None,
                            base_cost_minor: None,
                            serial_id: None,
                            client_id: None,
                            ticket_id: None,
                            carrier: req.carrier.clone(),
                            tracking_ref: req.tracking_ref.clone(),
                            note: None,
                            created_by: Some(user_id),
                            created_at: now,
                        };
                        // ADR 0002 receipt lock: the same rule as the direct
                        // stock receive — the movement carries the frozen
                        // snapshot when the cost currency differs from base.
                        if let Some(cost) = movement.cost_minor {
                            let today = movement.created_at.date_naive();
                            let locked = crate::routes::fx::lock_snapshot(
                                &mut ctx.tx,
                                org_id,
                                cost,
                                &order.currency,
                                today,
                            )
                            .await?;
                            if let Some(snap) = locked {
                                movement.fx_rate_used = Some(snap.rate_used);
                                movement.fx_rate_date = Some(snap.rate_date);
                                movement.fx_source = Some(snap.source);
                                movement.base_cost_minor = Some(snap.base_amount_minor);
                            }
                        }
                        let ledger_out = core_db::stock::record_movement(
                            &mut ctx.tx,
                            &core_db::stock::LedgerInput {
                                movement: movement.clone(),
                                item_name: item.name.clone(),
                                item_kind: item.kind.clone(),
                                serial_number: serial,
                            },
                        )
                        .await
                        .map_err(ApiError::from)?;
                        let movement = ledger_out.movement;
                        movement_count += 1;
                        ctx.emit(
                            core_domain::events::types::STOCK_MOVEMENT_RECORDED,
                            "stock_movement",
                            Some(movement.id.to_string().as_str()),
                            &serde_json::to_value(core_domain::events::StockMovementRecorded {
                                movement: movement.clone(),
                            })
                            .unwrap_or_else(|_| serde_json::json!({})),
                        )
                        .await?;
                        (Some(movement.id), None, movement.qty_milli)
                    }
                    _ if plan.creates_asset => {
                        let asset = create_receipt_asset(
                            &mut ctx,
                            &order,
                            &header,
                            plan,
                            serial.as_deref(),
                            receipt_line_id,
                            user_id,
                            now,
                        )
                        .await?;
                        asset_count += 1;
                        (None, Some(asset), 1000)
                    }
                    _ => {
                        // Client custody of a lot, or a service: the receipt
                        // line is the record and no owned level moves.
                        (None, None, qty)
                    }
                };
                let receipt_line = PurchaseReceiptLine {
                    id: receipt_line_id,
                    org_id,
                    receipt_id: receipt.id,
                    po_line_id: plan.line.id,
                    qty_milli: leg_qty,
                    movement_id,
                    created_at: now,
                };
                core_db::purchasing::insert_routed_receipt_line(
                    &mut *ctx.tx,
                    &receipt_line,
                    plan.destination,
                    device_asset_id,
                )
                .await?;
                if plan.destination == CustodyClass::ClientCustody {
                    if let Some(resolved) = &resolved_rule {
                        if resolved.rule.kind != RechargeRuleKind::None {
                            issue_recharge(
                                &mut ctx,
                                &authed.principal,
                                &order,
                                &header,
                                plan,
                                resolved,
                                &receipt_line,
                                leg_qty,
                                user_id,
                                now,
                            )
                            .await?;
                            recharge_count += 1;
                        }
                    }
                }
                receipt_lines.push(receipt_line.clone());
                receipt_line_views.push(ReceiptLineView {
                    line: receipt_line,
                    destination_class: plan.destination,
                    device_asset_id,
                });
            }
            core_db::purchasing::bump_line_received(
                &mut ctx.tx,
                org_id,
                plan.line.id,
                plan.qty_milli,
                req.allow_over_receipt,
            )
            .await?;
        }

        let order =
            core_db::purchasing::advance_order_receipt_status(&mut *ctx.tx, org_id, id).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: RECEIPT_ENTITY,
                entity_id: receipt.id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            core_domain::events::types::PURCHASE_RECEIPT_RECORDED,
            RECEIPT_ENTITY,
            Some(receipt.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::PurchaseReceiptRecorded {
                receipt: receipt.clone(),
                lines: receipt_lines.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "purchase.order_received".into(),
                entity_type: Some(RECEIPT_ENTITY.into()),
                entity_id: Some(receipt.id.to_string()),
                detail: serde_json::json!({
                    "order_id": id,
                    "location_id": req.location_id,
                    "line_count": receipt_lines.len(),
                    "movement_count": movement_count,
                    "asset_count": asset_count,
                    "recharge_count": recharge_count,
                    "allow_over_receipt": req.allow_over_receipt,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let detail = load_detail(&mut ctx.tx, org_id, order.clone(), financial).await?;
        let response = ReceiptMutationResponse {
            receipt: PurchaseReceiptView {
                receipt: receipt.clone(),
                lines: receipt_line_views.clone(),
            },
            order: detail.clone(),
            mutation: instant::record(
                &mut ctx,
                instant::RecordSpec {
                    entity: RECEIPT_ENTITY,
                    entity_id: receipt.id,
                    op: "receive",
                    seq: write_seq,
                    before: None,
                    after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                    updated_at: receipt.received_at,
                    key: key.as_deref(),
                    payload: &detail,
                    provenance: Default::default(),
                },
            )
            .await?,
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// GET /api/purchase-orders/{id}/recharges — every charge this order issued to
/// a client, with the rule and the FX snapshots that explain each amount.
#[utoipa::path(operation_id = "purchase_orders_recharges", get, path = "/api/purchase-orders/{id}/recharges", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [PurchaseRechargeView]),
              (status = 403, description = "the caller cannot read financials"),
              (status = 404, description = "order not visible in scope")))]
pub async fn order_recharges(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<Vec<PurchaseRechargeView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    require_permission(&authed.principal, Permission::ViewFinancials)?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    // The order read is the org-scope proof: a missing order is the same 404 as
    // an order outside the caller's scope.
    core_db::purchasing::get_purchase_order(&mut *tx, org_id, id).await?;
    let recharges = core_db::purchasing::list_recharges_for_order(&mut tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        recharges
            .into_iter()
            .map(PurchaseRechargeView::from)
            .collect(),
    ))
}

/// GET /api/purchase-orders/{id}/deeplinks — the one-click open list:
/// vendor portal links first, then PO-level links, then per-line links
/// (with their line context), each https-validated at write time.
#[utoipa::path(operation_id = "purchase_orders_deeplinks", get, path = "/api/purchase-orders/{id}/deeplinks", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = PurchaseOrderDeeplinks),
              (status = 404, description = "order not visible in scope")))]
pub async fn order_deeplinks(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<PurchaseOrderDeeplinks>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let order = core_db::purchasing::get_purchase_order(&mut *tx, org_id, id).await?;
    let vendor = core_db::purchasing::get_vendor(&mut *tx, org_id, order.vendor_id).await?;
    let lines = core_db::purchasing::list_po_lines(&mut tx, org_id, id).await?;
    let vendor_links = core_db::purchasing::list_external_links(
        &mut tx,
        org_id,
        ExternalLinkSubject::Vendor,
        vendor.id,
    )
    .await?;
    let order_links = core_db::purchasing::list_external_links(
        &mut tx,
        org_id,
        ExternalLinkSubject::PurchaseOrder,
        id,
    )
    .await?;
    let line_ids: Vec<Uuid> = lines.iter().map(|l| l.id).collect();
    let line_links =
        core_db::purchasing::list_external_links_for_lines(&mut tx, org_id, &line_ids).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let mut links = Vec::with_capacity(vendor_links.len() + order_links.len() + line_links.len());
    for link in vendor_links {
        links.push(DeeplinkEntry {
            link_id: link.id,
            kind: link.kind,
            url: link.url,
            label: link.label,
            carrier: link.carrier,
            subject: "vendor".into(),
            line_id: None,
            line_description: None,
            line_position: None,
        });
    }
    for link in order_links {
        links.push(DeeplinkEntry {
            link_id: link.id,
            kind: link.kind,
            url: link.url,
            label: link.label,
            carrier: link.carrier,
            subject: "purchase_order".into(),
            line_id: None,
            line_description: None,
            line_position: None,
        });
    }
    for link in line_links {
        let line = lines.iter().find(|l| l.id == link.subject_id);
        links.push(DeeplinkEntry {
            link_id: link.id,
            kind: link.kind,
            url: link.url,
            label: link.label,
            carrier: link.carrier,
            subject: "purchase_order_line".into(),
            line_id: Some(link.subject_id),
            line_description: line.map(|l| l.description.clone()),
            line_position: line.map(|l| l.position),
        });
    }
    Ok(Json(PurchaseOrderDeeplinks {
        order_id: order.id,
        number: order.number,
        vendor: DeeplinkVendor {
            id: vendor.id,
            name: vendor.name,
            website: vendor.website,
            account_number: vendor.account_number,
        },
        links,
    }))
}

/// GET /api/purchase-orders/{id}/bill
#[utoipa::path(operation_id = "purchase_orders_get_bill", get, path = "/api/purchase-orders/{id}/bill", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = PurchaseBill),
              (status = 404, description = "no bill recorded for this order")))]
pub async fn get_order_bill(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<PurchaseBill>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    require_permission(&authed.principal, Permission::ViewFinancials)?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    core_db::purchasing::get_purchase_order(&mut *tx, org_id, id).await?;
    let bill = core_db::purchasing::get_bill(&mut tx, org_id, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(bill))
}

/// PUT /api/purchase-orders/{id}/bill — record or edit the draft bill.
#[utoipa::path(operation_id = "purchase_orders_record_bill", put, path = "/api/purchase-orders/{id}/bill", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = RecordBillRequest,
    responses((status = 200, body = BillMutationResponse),
              (status = 409, description = "order not billable yet, or the bill is already posted")))]
pub async fn record_order_bill(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<RecordBillRequest>,
) -> ApiResult<Json<BillMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    require_permission(&authed.principal, Permission::ViewFinancials)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: BILL_ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let order = core_db::purchasing::get_purchase_order(&mut *ctx.tx, org_id, id).await?;
        if !matches!(
            order.status,
            PurchaseOrderStatus::PartiallyReceived
                | PurchaseOrderStatus::Received
                | PurchaseOrderStatus::Closed
        ) {
            return Err(ApiError::Conflict(format!(
                "purchase order {} is {} — goods must be received before its bill is recorded",
                order.number,
                order.status.as_str()
            )));
        }
        let amount_minor = match req.amount_minor {
            Some(amount) => {
                if amount < 0 {
                    return Err(ApiError::Validation(
                        "bill amount must not be negative".into(),
                    ));
                }
                amount
            }
            None => {
                let lines = core_db::purchasing::list_po_lines(&mut ctx.tx, org_id, id).await?;
                let mut total: i128 = 0;
                for line in &lines {
                    total += i128::from(line.received_amount_minor().map_err(ApiError::from)?);
                }
                i64::try_from(total)
                    .map_err(|_| ApiError::Validation("received total overflows i64".into()))?
            }
        };
        let currency = req
            .currency
            .clone()
            .unwrap_or_else(|| order.currency.clone());
        let bill = core_db::purchasing::upsert_bill(
            &mut ctx.tx,
            org_id,
            id,
            req.bill_number.as_deref(),
            amount_minor,
            &currency,
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: BILL_ENTITY,
                entity_id: bill.id,
                version: req.instant.version,
            },
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "purchase.bill_recorded".into(),
                entity_type: Some(BILL_ENTITY.into()),
                entity_id: Some(bill.id.to_string()),
                detail: serde_json::json!({
                    "order_id": id,
                    "amount_minor": bill.amount_minor,
                    "currency": bill.currency,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let response = BillMutationResponse {
            bill: bill.clone(),
            mutation: instant::record(
                &mut ctx,
                instant::RecordSpec {
                    entity: BILL_ENTITY,
                    entity_id: bill.id,
                    op: "update",
                    seq: write_seq,
                    before: None,
                    after: serde_json::to_value(&bill).unwrap_or(Value::Null),
                    updated_at: bill.updated_at,
                    key: key.as_deref(),
                    payload: &bill,
                    provenance: Default::default(),
                },
            )
            .await?,
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/purchase-orders/{id}/bill/post — the rollup moment: locks
/// the ADR 0002 FX snapshot and stamps `posted_at`.
#[utoipa::path(operation_id = "purchase_orders_post_bill", post, path = "/api/purchase-orders/{id}/bill/post", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CloseOrderRequest,
    responses((status = 200, body = BillMutationResponse),
              (status = 404, description = "no bill recorded"),
              (status = 409, description = "the bill is already posted")))]
pub async fn post_order_bill(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<CloseOrderRequest>,
) -> ApiResult<Json<BillMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    require_permission(&authed.principal, Permission::ViewFinancials)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: BILL_ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        core_db::purchasing::get_purchase_order(&mut *ctx.tx, org_id, id).await?;
        let bill = core_db::purchasing::get_bill(&mut ctx.tx, org_id, id)
            .await?
            .ok_or(ApiError::NotFound)?;
        if bill.posted_at.is_some() {
            return Err(ApiError::Conflict("the bill is already posted".into()));
        }
        let today = chrono::Utc::now().date_naive();
        let locked = crate::routes::fx::lock_snapshot(
            &mut ctx.tx,
            org_id,
            bill.amount_minor,
            &bill.currency,
            today,
        )
        .await?;
        let snapshot = locked.map(|snap| BillFxSnapshot {
            rate_used: snap.rate_used,
            rate_date: snap.rate_date,
            source: snap.source,
            base_amount_minor: snap.base_amount_minor,
        });
        let bill =
            core_db::purchasing::post_bill(&mut ctx.tx, org_id, id, snapshot.as_ref()).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: BILL_ENTITY,
                entity_id: bill.id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            core_domain::events::types::PURCHASE_BILL_POSTED,
            BILL_ENTITY,
            Some(bill.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::PurchaseBillPosted { bill: bill.clone() })
                .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "purchase.bill_posted".into(),
                entity_type: Some(BILL_ENTITY.into()),
                entity_id: Some(bill.id.to_string()),
                detail: serde_json::json!({
                    "order_id": id,
                    "amount_minor": bill.amount_minor,
                    "currency": bill.currency,
                    "base_amount_minor": bill.base_amount_minor,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let response = BillMutationResponse {
            bill: bill.clone(),
            mutation: instant::record(
                &mut ctx,
                instant::RecordSpec {
                    entity: BILL_ENTITY,
                    entity_id: bill.id,
                    op: "post",
                    seq: write_seq,
                    before: None,
                    after: serde_json::to_value(&bill).unwrap_or(Value::Null),
                    updated_at: bill.updated_at,
                    key: key.as_deref(),
                    payload: &bill,
                    provenance: Default::default(),
                },
            )
            .await?,
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// GET /api/projects/{id}/cost-rollup — posted PO bills in base currency
/// (ADR 0002 locked) plus the project budget for comparison.
#[utoipa::path(operation_id = "projects_cost_rollup", get, path = "/api/projects/{id}/cost-rollup", tag = "purchasing", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ProjectCostRollupView),
              (status = 404, description = "project not visible in scope")))]
pub async fn project_cost_rollup_route(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<ProjectCostRollupView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Projects,
            action: Action::View,
        },
    )?;
    require_permission(&authed.principal, Permission::ViewFinancials)?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rollup = core_db::purchasing::project_cost_rollup(&mut tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rollup.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(acquired_for: AcquiredFor) -> OrderIntent {
        OrderIntent {
            acquired_for,
            client_id: None,
            ticket_id: None,
            project_id: None,
            ownership: None,
            delivery_mode: DeliveryMode::ToStock,
            recharge_rule: None,
        }
    }

    fn line_input(
        line_kind: Option<LineKind>,
        destination_class: Option<CustodyClass>,
    ) -> OrderLineInput {
        OrderLineInput {
            stock_item_id: None,
            catalog_item_id: None,
            description: "Dell Latitude".into(),
            qty_ordered_milli: 1000,
            unit_cost_minor: 120_000,
            position: None,
            line_kind,
            destination_class,
            asset_layout_id: None,
            serial_expected: None,
            weight_g: None,
            sell_price_minor: None,
            billable: None,
        }
    }

    fn line_intent(destination_class: Option<CustodyClass>) -> core_db::purchasing::LineIntent {
        core_db::purchasing::LineIntent {
            line_kind: LineKind::Stock,
            destination_class,
            asset_layout_id: None,
            serial_expected: false,
            weight_g: None,
            sell_price_minor: None,
            billable: false,
        }
    }

    #[test]
    fn a_line_inherits_the_header_until_it_names_a_class() {
        let ours = header(AcquiredFor::InternalFleet);
        assert_eq!(
            resolve_destination(&line_intent(None), &ours),
            CustodyClass::InternalFleet
        );
        assert_eq!(
            resolve_destination(&line_intent(Some(CustodyClass::SaleStock)), &ours),
            CustodyClass::SaleStock,
            "the line decides, the header only defaults"
        );
    }

    #[test]
    fn a_line_intent_defaults_follow_the_header() {
        let ours = header(AcquiredFor::SaleStock);
        let stock = line_intent_from_input(&line_input(None, None), &ours).unwrap();
        assert_eq!(stock.line_kind, LineKind::Stock);
        assert_eq!(
            stock.destination_class, None,
            "an inherited class is not stored"
        );
        assert!(!stock.billable);

        let serialized =
            line_intent_from_input(&line_input(Some(LineKind::SerializedUnit), None), &ours)
                .unwrap();
        assert!(
            serialized.serial_expected,
            "a serialized unit expects its serial"
        );

        let mut client = header(AcquiredFor::ClientStocked);
        client.client_id = Some(Uuid::now_v7());
        let custody = line_intent_from_input(&line_input(None, None), &client).unwrap();
        assert!(
            custody.billable,
            "a line landing in client custody is billable by default"
        );
    }

    #[test]
    fn an_asset_destination_refuses_a_stock_line() {
        let ours = header(AcquiredFor::InternalFleet);
        let refused = line_intent_from_input(&line_input(None, None), &ours).unwrap_err();
        assert!(matches!(refused, ApiError::Validation(_)));
        assert!(line_intent_from_input(&line_input(Some(LineKind::Asset), None), &ours).is_ok());
    }

    #[test]
    fn a_pass_through_cannot_land_in_owned_quantity() {
        let ours = header(AcquiredFor::SaleStock);
        let refused = line_intent_from_input(&line_input(Some(LineKind::PassThrough), None), &ours)
            .unwrap_err();
        assert!(matches!(refused, ApiError::Validation(_)));
        // It is legal once the line names a class nobody owns.
        let mut client = header(AcquiredFor::ClientStocked);
        client.client_id = Some(Uuid::now_v7());
        assert!(line_intent_from_input(
            &line_input(
                Some(LineKind::PassThrough),
                Some(CustodyClass::ClientCustody)
            ),
            &client
        )
        .is_ok());
    }

    #[test]
    fn client_custody_needs_a_client_and_a_vendor_rental_needs_rented() {
        let ours = header(AcquiredFor::SaleStock);
        assert!(line_intent_from_input(
            &line_input(Some(LineKind::Asset), Some(CustodyClass::ClientCustody)),
            &ours
        )
        .is_err());

        let mut rented = header(AcquiredFor::VendorRental);
        rented.ownership = Some(Ownership::Owned);
        assert!(
            line_intent_from_input(&line_input(Some(LineKind::RentalUnit), None), &rented).is_err()
        );
        rented.ownership = Some(Ownership::Rented);
        assert!(
            line_intent_from_input(&line_input(Some(LineKind::RentalUnit), None), &rented).is_ok()
        );
    }

    #[test]
    fn e_waste_is_a_service_and_not_a_unit() {
        let ours = header(AcquiredFor::EwasteService);
        assert!(line_intent_from_input(&line_input(None, None), &ours).is_err());
        assert!(line_intent_from_input(&line_input(Some(LineKind::Service), None), &ours).is_ok());
    }

    #[test]
    fn the_percentage_zero_default_recharges_at_cost() {
        let basis = 120_000;
        assert_eq!(
            recharge_charge_minor(basis, &RechargeRule::default_markup()).unwrap(),
            basis,
            "0 bps is a real rule: the charge is the cost"
        );
        assert_eq!(
            recharge_charge_minor(basis, &RechargeRule::at_cost()).unwrap(),
            basis
        );
        let percent = RechargeRule {
            kind: RechargeRuleKind::MarkupBps,
            markup_bps: Some(2_500),
            fixed_minor: None,
            currency: None,
        };
        assert_eq!(recharge_charge_minor(10_000, &percent).unwrap(), 12_500);
        let fixed = RechargeRule {
            kind: RechargeRuleKind::Fixed,
            markup_bps: None,
            fixed_minor: Some(9_999),
            currency: Some("USD".into()),
        };
        assert_eq!(recharge_charge_minor(basis, &fixed).unwrap(), 9_999);
        let none = RechargeRule {
            kind: RechargeRuleKind::None,
            markup_bps: None,
            fixed_minor: None,
            currency: None,
        };
        assert!(recharge_charge_minor(basis, &none).is_err());
    }

    #[test]
    fn a_percentage_markup_rounds_half_even() {
        let rule = RechargeRule {
            kind: RechargeRuleKind::MarkupBps,
            markup_bps: Some(1),
            fixed_minor: None,
            currency: None,
        };
        // 50 × 1bp = 0.005 minor → 0; 150 × 1bp = 0.015 → 0; 5000 × 1bp = 0.5
        // → 0 (even), 15000 × 1bp = 1.5 → 2 (even).
        assert_eq!(recharge_charge_minor(50, &rule).unwrap(), 50);
        assert_eq!(recharge_charge_minor(5_000, &rule).unwrap(), 5_000);
        assert_eq!(recharge_charge_minor(15_000, &rule).unwrap(), 15_002);
    }

    #[test]
    fn the_reporting_class_names_the_cost() {
        assert_eq!(
            recharge_reporting_class(AcquiredFor::ClientStocked, LineKind::SerializedUnit),
            "inventory"
        );
        assert_eq!(
            recharge_reporting_class(AcquiredFor::ClientStocked, LineKind::Service),
            "service_cost"
        );
        assert_eq!(
            recharge_reporting_class(AcquiredFor::EwasteService, LineKind::Service),
            "ewaste_cost"
        );
    }
}
