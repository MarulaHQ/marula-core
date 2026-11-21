//! Purchasing + vendors persistence (Wave 32 lane 2, `p32-purchasing`;
//! migration 0103).
//!
//! Vendors + their typed contacts, typed external links (order portals /
//! tracking pages), purchase orders with their lines, the receipt
//! documents that post into the lane-1 append-only `stock_movements`
//! ledger, and the vendor bill that carries a purchase into project cost
//! rollups.
//!
//! Every read and write carries an explicit `org_id` predicate on top of
//! the migration's RLS policies (same posture as the rest of the crate);
//! quantity math stays i64 `*_milli` and money integer minor units. The
//! receipt engine itself lives in the API layer (`routes/purchasing.rs`)
//! because the ADR 0002 FX lock is computed there — this module provides
//! the primitives it drives (`lock_purchase_order` + guarded line bumps
//! keep concurrent receipts honest).
//!
//! Statements here are built from the const column lists above and executed
//! with `sqlx::query`, not the checked macros. That is this module's
//! long-standing shape: a read that gains a column changes one const instead of
//! a prepare cycle in every environment. Nothing in this module is an unchecked
//! string built from caller input — every value is bound — and the macros that
//! exist elsewhere in the crate keep their `.sqlx/` coverage untouched.

use crate::{DbError, DbResult};
use chrono::{DateTime, NaiveDate, Utc};
use core_domain::entities::{
    AcquiredFor, CloseKind, CustodyClass, DeliveryMode, ExternalLink, ExternalLinkKind,
    ExternalLinkSubject, InventoryPolicy, LandedCostMode, LineKind, Ownership, PurchaseBill,
    PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus, PurchaseReceipt, PurchaseReceiptLine,
    ReceiptState, RechargeRule, RechargeRuleKind, Vendor, VendorContact,
};
use sqlx::{Postgres, Row};
use std::collections::HashMap;
use uuid::Uuid;

const VENDOR_COLS: &str = "id, org_id, name, code, email, phone, website, account_number, \
     default_currency, notes, active, created_by, created_at, updated_at";
const CONTACT_COLS: &str = "id, org_id, vendor_id, name, email, phone, role, is_primary, \
     created_at, updated_at";
const LINK_COLS: &str = "id, org_id, subject_kind, subject_id, kind, url, label, carrier, \
     created_by, created_at, updated_at";
const ORDER_COLS: &str = "id, org_id, vendor_id, number, status, currency, vendor_reference, \
     project_id, notes, ordered_at, expected_at, closed_at, close_kind, cancelled_at, \
     cancel_reason, created_by, created_at, updated_at";
const LINE_COLS: &str = "id, org_id, purchase_order_id, stock_item_id, catalog_item_id, \
     description, qty_ordered_milli, qty_received_milli, unit_cost_minor, receipt_state, \
     position, created_at, updated_at";
const RECEIPT_COLS: &str = "id, org_id, purchase_order_id, location_id, carrier, tracking_ref, \
     note, received_at, created_by, created_at";
const BILL_COLS: &str = "id, org_id, purchase_order_id, bill_number, amount_minor, currency, \
     fx_rate_used::text AS fx_rate_used, fx_rate_date, fx_source, base_amount_minor, posted_at, \
     created_by, created_at, updated_at";

fn decode_err(column: &str, e: core_domain::DomainError) -> sqlx::Error {
    sqlx::Error::ColumnDecode {
        index: column.into(),
        source: Box::new(e),
    }
}

fn vendor_row(row: sqlx::postgres::PgRow) -> Vendor {
    Vendor {
        id: row.get("id"),
        org_id: row.get("org_id"),
        name: row.get("name"),
        code: row.get("code"),
        email: row.get("email"),
        phone: row.get("phone"),
        website: row.get("website"),
        account_number: row.get("account_number"),
        default_currency: row.get("default_currency"),
        notes: row.get("notes"),
        active: row.get("active"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn contact_row(row: sqlx::postgres::PgRow) -> VendorContact {
    VendorContact {
        id: row.get("id"),
        org_id: row.get("org_id"),
        vendor_id: row.get("vendor_id"),
        name: row.get("name"),
        email: row.get("email"),
        phone: row.get("phone"),
        role: row.get("role"),
        is_primary: row.get("is_primary"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn link_row(row: sqlx::postgres::PgRow) -> DbResult<ExternalLink> {
    Ok(ExternalLink {
        id: row.get("id"),
        org_id: row.get("org_id"),
        subject_kind: ExternalLinkSubject::parse(row.get::<String, _>("subject_kind").as_str())
            .map_err(|e| decode_err("subject_kind", e))?,
        subject_id: row.get("subject_id"),
        kind: ExternalLinkKind::parse(row.get::<String, _>("kind").as_str())
            .map_err(|e| decode_err("kind", e))?,
        url: row.get("url"),
        label: row.get("label"),
        carrier: row.get("carrier"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn order_row(row: sqlx::postgres::PgRow) -> DbResult<PurchaseOrder> {
    Ok(PurchaseOrder {
        id: row.get("id"),
        org_id: row.get("org_id"),
        vendor_id: row.get("vendor_id"),
        number: row.get("number"),
        status: PurchaseOrderStatus::parse(row.get::<String, _>("status").as_str())
            .map_err(|e| decode_err("status", e))?,
        currency: row.get("currency"),
        vendor_reference: row.get("vendor_reference"),
        project_id: row.get("project_id"),
        notes: row.get("notes"),
        ordered_at: row.get("ordered_at"),
        expected_at: row.get("expected_at"),
        closed_at: row.get("closed_at"),
        close_kind: row
            .get::<Option<String>, _>("close_kind")
            .map(|raw| CloseKind::parse(&raw).map_err(|e| decode_err("close_kind", e)))
            .transpose()?,
        cancelled_at: row.get("cancelled_at"),
        cancel_reason: row.get("cancel_reason"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn line_row(row: sqlx::postgres::PgRow) -> DbResult<PurchaseOrderLine> {
    Ok(PurchaseOrderLine {
        id: row.get("id"),
        org_id: row.get("org_id"),
        purchase_order_id: row.get("purchase_order_id"),
        stock_item_id: row.get("stock_item_id"),
        catalog_item_id: row.get("catalog_item_id"),
        description: row.get("description"),
        qty_ordered_milli: row.get("qty_ordered_milli"),
        qty_received_milli: row.get("qty_received_milli"),
        unit_cost_minor: row.get("unit_cost_minor"),
        receipt_state: ReceiptState::parse(row.get::<String, _>("receipt_state").as_str())
            .map_err(|e| decode_err("receipt_state", e))?,
        position: row.get("position"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn receipt_row(row: sqlx::postgres::PgRow) -> PurchaseReceipt {
    PurchaseReceipt {
        id: row.get("id"),
        org_id: row.get("org_id"),
        purchase_order_id: row.get("purchase_order_id"),
        location_id: row.get("location_id"),
        carrier: row.get("carrier"),
        tracking_ref: row.get("tracking_ref"),
        note: row.get("note"),
        received_at: row.get("received_at"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
    }
}

fn receipt_line_row(row: sqlx::postgres::PgRow) -> PurchaseReceiptLine {
    PurchaseReceiptLine {
        id: row.get("id"),
        org_id: row.get("org_id"),
        receipt_id: row.get("receipt_id"),
        po_line_id: row.get("po_line_id"),
        qty_milli: row.get("qty_milli"),
        movement_id: row.get("movement_id"),
        created_at: row.get("created_at"),
    }
}

fn bill_row(row: sqlx::postgres::PgRow) -> PurchaseBill {
    PurchaseBill {
        id: row.get("id"),
        org_id: row.get("org_id"),
        purchase_order_id: row.get("purchase_order_id"),
        bill_number: row.get("bill_number"),
        amount_minor: row.get("amount_minor"),
        currency: row.get("currency"),
        fx_rate_used: row.get("fx_rate_used"),
        fx_rate_date: row.get("fx_rate_date"),
        fx_source: row.get("fx_source"),
        base_amount_minor: row.get("base_amount_minor"),
        posted_at: row.get("posted_at"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

// Vendors ---------------------------------------------------------------------

/// INSERT a vendor; code uniqueness is DB-enforced (partial unique index
/// on lower(code)) — the API layer maps the unique violation to 409.
pub async fn insert_vendor<'e, E>(executor: E, vendor: &Vendor) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"INSERT INTO vendors (id, org_id, name, code, email, phone, website, account_number,
                                default_currency, notes, active, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"#,
    )
    .bind(vendor.id)
    .bind(vendor.org_id)
    .bind(vendor.name.trim())
    .bind(vendor.code.as_deref().map(str::trim))
    .bind(vendor.email.as_deref())
    .bind(vendor.phone.as_deref())
    .bind(vendor.website.as_deref())
    .bind(vendor.account_number.as_deref())
    .bind(&vendor.default_currency)
    .bind(vendor.notes.as_deref())
    .bind(vendor.active)
    .bind(vendor.created_by)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_vendor<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Vendor>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        "SELECT {VENDOR_COLS} FROM vendors WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(vendor_row(row))
}

/// Substring search over name/code/email; `only_active = false` returns
/// archived vendors too (archival is a flag, never a delete).
pub async fn list_vendors(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    search: Option<&str>,
    only_active: bool,
    limit: i64,
) -> DbResult<Vec<Vendor>> {
    let grep = search.map(|s| format!("%{}%", s.trim().to_lowercase()));
    let rows = sqlx::query(&format!(
        r#"SELECT {VENDOR_COLS}
           FROM vendors
           WHERE org_id = $1
             AND ($2::text IS NULL
                  OR lower(name) LIKE $2
                  OR lower(coalesce(code, '')) LIKE $2
                  OR lower(coalesce(email, '')) LIKE $2)
             AND (NOT $3::boolean OR active)
           ORDER BY lower(name)
           LIMIT $4"#
    ))
    .bind(org_id)
    .bind(grep)
    .bind(only_active)
    .bind(limit)
    .fetch_all(&mut *executor)
    .await?;
    Ok(rows.into_iter().map(vendor_row).collect())
}

/// PATCH — the route loads the current row first and passes every field;
/// only the fields the request carries differ from the stored values.
pub async fn update_vendor<'e, E>(executor: E, vendor: &Vendor) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let n = sqlx::query(
        r#"UPDATE vendors
             SET name = $3, code = $4, email = $5, phone = $6, website = $7,
                 account_number = $8, default_currency = $9, notes = $10,
                 active = $11, updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
    )
    .bind(vendor.org_id)
    .bind(vendor.id)
    .bind(vendor.name.trim())
    .bind(vendor.code.as_deref().map(str::trim))
    .bind(vendor.email.as_deref())
    .bind(vendor.phone.as_deref())
    .bind(vendor.website.as_deref())
    .bind(vendor.account_number.as_deref())
    .bind(&vendor.default_currency)
    .bind(vendor.notes.as_deref())
    .bind(vendor.active)
    .execute(executor)
    .await?;
    if n.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Replace-on-write for the vendor's contact list: the request carries the
/// full desired list, the write path deletes the vendor's rows and inserts
/// the new set in one transaction (ids in the request are ignored — a
/// contact is a value on the vendor, not an addressed sub-resource).
pub async fn replace_vendor_contacts(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    vendor_id: Uuid,
    contacts: &[VendorContact],
) -> DbResult<Vec<VendorContact>> {
    sqlx::query("DELETE FROM vendor_contacts WHERE org_id = $1 AND vendor_id = $2")
        .bind(org_id)
        .bind(vendor_id)
        .execute(&mut *executor)
        .await?;
    let mut out = Vec::with_capacity(contacts.len());
    for contact in contacts {
        let id = if contact.id.is_nil() {
            Uuid::now_v7()
        } else {
            contact.id
        };
        sqlx::query(
            r#"INSERT INTO vendor_contacts (id, org_id, vendor_id, name, email, phone, role, is_primary)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        )
        .bind(id)
        .bind(org_id)
        .bind(vendor_id)
        .bind(contact.name.trim())
        .bind(contact.email.as_deref())
        .bind(contact.phone.as_deref())
        .bind(contact.role.as_deref())
        .bind(contact.is_primary)
        .execute(&mut *executor)
        .await?;
        out.push(VendorContact {
            id,
            org_id,
            vendor_id,
            name: contact.name.trim().to_string(),
            email: contact.email.clone(),
            phone: contact.phone.clone(),
            role: contact.role.clone(),
            is_primary: contact.is_primary,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
    }
    Ok(out)
}

pub async fn list_vendor_contacts(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    vendor_id: Uuid,
) -> DbResult<Vec<VendorContact>> {
    let rows = sqlx::query(&format!(
        r#"SELECT {CONTACT_COLS} FROM vendor_contacts
           WHERE org_id = $1 AND vendor_id = $2
           ORDER BY is_primary DESC, lower(name)"#
    ))
    .bind(org_id)
    .bind(vendor_id)
    .fetch_all(&mut *executor)
    .await?;
    Ok(rows.into_iter().map(contact_row).collect())
}

// External links ---------------------------------------------------------------

/// Whether the typed subject row exists in this org (the write path
/// checks before recording a link; no polymorphic FK by design).
pub async fn external_link_subject_exists(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    subject_kind: ExternalLinkSubject,
    subject_id: Uuid,
) -> DbResult<bool> {
    let sql = match subject_kind {
        ExternalLinkSubject::Vendor => {
            "SELECT EXISTS(SELECT 1 FROM vendors WHERE org_id = $1 AND id = $2)"
        }
        ExternalLinkSubject::PurchaseOrder => {
            "SELECT EXISTS(SELECT 1 FROM purchase_orders WHERE org_id = $1 AND id = $2)"
        }
        ExternalLinkSubject::PurchaseOrderLine => {
            "SELECT EXISTS(SELECT 1 FROM purchase_order_lines WHERE org_id = $1 AND id = $2)"
        }
        ExternalLinkSubject::DeviceAsset => {
            "SELECT EXISTS(SELECT 1 FROM device_assets WHERE org_id = $1 AND id = $2)"
        }
    };
    let exists: bool = sqlx::query_scalar(sql)
        .bind(org_id)
        .bind(subject_id)
        .fetch_one(&mut *executor)
        .await?;
    Ok(exists)
}

pub async fn insert_external_link<'e, E>(executor: E, link: &ExternalLink) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"INSERT INTO external_links (id, org_id, subject_kind, subject_id, kind, url, label,
                                       carrier, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#,
    )
    .bind(link.id)
    .bind(link.org_id)
    .bind(link.subject_kind.as_str())
    .bind(link.subject_id)
    .bind(link.kind.as_str())
    .bind(link.url.trim())
    .bind(link.label.as_deref())
    .bind(link.carrier.as_deref())
    .bind(link.created_by)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_external_link<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<ExternalLink>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        "SELECT {LINK_COLS} FROM external_links WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    link_row(row)
}

pub async fn delete_external_link<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let n = sqlx::query("DELETE FROM external_links WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(executor)
        .await?;
    if n.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

pub async fn list_external_links(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    subject_kind: ExternalLinkSubject,
    subject_id: Uuid,
) -> DbResult<Vec<ExternalLink>> {
    let rows = sqlx::query(&format!(
        r#"SELECT {LINK_COLS} FROM external_links
           WHERE org_id = $1 AND subject_kind = $2 AND subject_id = $3
           ORDER BY kind, created_at"#
    ))
    .bind(org_id)
    .bind(subject_kind.as_str())
    .bind(subject_id)
    .fetch_all(&mut *executor)
    .await?;
    rows.into_iter().map(link_row).collect()
}

/// Every link attached to a PO line's parent order line set — used by
/// deeplink resolution to gather per-line tracking pages in one query.
pub async fn list_external_links_for_lines(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    line_ids: &[Uuid],
) -> DbResult<Vec<ExternalLink>> {
    if line_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(&format!(
        r#"SELECT {LINK_COLS} FROM external_links
           WHERE org_id = $1 AND subject_kind = 'purchase_order_line'
             AND subject_id = ANY($2)
           ORDER BY kind, created_at"#
    ))
    .bind(org_id)
    .bind(line_ids)
    .fetch_all(&mut *executor)
    .await?;
    rows.into_iter().map(link_row).collect()
}

// Purchase orders --------------------------------------------------------------

/// `PO-NNNNNN` from the org's purchase-order sequence (the invoice/quote
/// numbering convention, migration 0103).
pub async fn next_purchase_order_number<'e, E>(executor: E, org_id: Uuid) -> DbResult<String>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let seq: i64 = sqlx::query_scalar(
        "UPDATE orgs SET purchase_order_sequence = purchase_order_sequence + 1
         WHERE id = $1 RETURNING purchase_order_sequence",
    )
    .bind(org_id)
    .fetch_one(executor)
    .await?;
    Ok(format!("PO-{seq:06}"))
}

pub async fn insert_purchase_order<'e, E>(executor: E, order: &PurchaseOrder) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"INSERT INTO purchase_orders (id, org_id, vendor_id, number, status, currency,
                                        vendor_reference, project_id, notes, expected_at, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
    )
    .bind(order.id)
    .bind(order.org_id)
    .bind(order.vendor_id)
    .bind(order.number.trim())
    .bind(order.status.as_str())
    .bind(&order.currency)
    .bind(order.vendor_reference.as_deref())
    .bind(order.project_id)
    .bind(order.notes.as_deref())
    .bind(order.expected_at)
    .bind(order.created_by)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_purchase_order<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<PurchaseOrder>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        "SELECT {ORDER_COLS} FROM purchase_orders WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    order_row(row)
}

/// Row lock for the receipt engine: concurrent receipts against the same
/// PO serialize here, so the line states they read cannot go stale under
/// each other (the over-receipt gate depends on it).
pub async fn lock_purchase_order(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<PurchaseOrder> {
    let row = sqlx::query(&format!(
        "SELECT {ORDER_COLS} FROM purchase_orders WHERE org_id = $1 AND id = $2 FOR UPDATE"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *executor)
    .await?
    .ok_or(DbError::NotFound)?;
    order_row(row)
}

pub async fn list_purchase_orders(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    status: Option<&str>,
    vendor_id: Option<Uuid>,
    project_id: Option<Uuid>,
    search: Option<&str>,
    limit: i64,
) -> DbResult<Vec<PurchaseOrder>> {
    let grep = search.map(|s| format!("%{}%", s.trim().to_lowercase()));
    let rows = sqlx::query(&format!(
        r#"SELECT {ORDER_COLS}
           FROM purchase_orders
           WHERE org_id = $1
             AND ($2::text IS NULL OR status = $2)
             AND ($3::uuid IS NULL OR vendor_id = $3)
             AND ($4::uuid IS NULL OR project_id = $4)
             AND ($5::text IS NULL
                  OR lower(number) LIKE $5
                  OR lower(coalesce(vendor_reference, '')) LIKE $5)
           ORDER BY created_at DESC
           LIMIT $6"#
    ))
    .bind(org_id)
    .bind(status)
    .bind(vendor_id)
    .bind(project_id)
    .bind(grep)
    .bind(limit)
    .fetch_all(&mut *executor)
    .await?;
    rows.into_iter().map(order_row).collect()
}

/// Header edits (draft only — enforced by the API layer): vendor,
/// reference, project hook, notes, expected date, currency.
pub async fn update_purchase_order<'e, E>(executor: E, order: &PurchaseOrder) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let n = sqlx::query(
        r#"UPDATE purchase_orders
             SET vendor_id = $3, currency = $4, vendor_reference = $5, project_id = $6,
                 notes = $7, expected_at = $8, updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
    )
    .bind(order.org_id)
    .bind(order.id)
    .bind(order.vendor_id)
    .bind(&order.currency)
    .bind(order.vendor_reference.as_deref())
    .bind(order.project_id)
    .bind(order.notes.as_deref())
    .bind(order.expected_at)
    .execute(executor)
    .await?;
    if n.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// draft → ordered.
pub async fn mark_ordered<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<PurchaseOrder>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        r#"UPDATE purchase_orders
             SET status = 'ordered', ordered_at = COALESCE(ordered_at, now()), updated_at = now()
           WHERE org_id = $1 AND id = $2
           RETURNING {ORDER_COLS}"#
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    order_row(row)
}

/// Close with the typed disposition: 'full' (every line complete/over) or
/// 'short' (unreceived quantity stays behind — recorded, never silent).
pub async fn close_order<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    kind: CloseKind,
) -> DbResult<PurchaseOrder>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        r#"UPDATE purchase_orders
             SET status = 'closed', closed_at = now(), close_kind = $3, updated_at = now()
           WHERE org_id = $1 AND id = $2
           RETURNING {ORDER_COLS}"#
    ))
    .bind(org_id)
    .bind(id)
    .bind(kind.as_str())
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    order_row(row)
}

pub async fn cancel_order<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    reason: Option<&str>,
) -> DbResult<PurchaseOrder>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        r#"UPDATE purchase_orders
             SET status = 'cancelled', cancelled_at = now(), cancel_reason = $3, updated_at = now()
           WHERE org_id = $1 AND id = $2
           RETURNING {ORDER_COLS}"#
    ))
    .bind(org_id)
    .bind(id)
    .bind(reason)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    order_row(row)
}

/// Receipt-driven status advance: every line complete/over → 'received';
/// any quantity in → 'partially_received'; nothing in yet → unchanged.
/// Terminal states are never reopened.
pub async fn advance_order_receipt_status<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<PurchaseOrder>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        r#"UPDATE purchase_orders SET
             status = CASE
               WHEN status IN ('draft','ordered','partially_received')
                    AND NOT EXISTS (SELECT 1 FROM purchase_order_lines l
                                    WHERE l.purchase_order_id = purchase_orders.id
                                      AND l.receipt_state NOT IN ('complete','over'))
                 THEN 'received'
               WHEN status IN ('draft','ordered','partially_received')
                    AND EXISTS (SELECT 1 FROM purchase_order_lines l
                                WHERE l.purchase_order_id = purchase_orders.id
                                  AND l.qty_received_milli > 0)
                 THEN 'partially_received'
               ELSE status
             END,
             updated_at = now()
           WHERE org_id = $1 AND id = $2
           RETURNING {ORDER_COLS}"#
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    order_row(row)
}

// Purchase order lines ---------------------------------------------------------

pub async fn insert_po_line<'e, E>(executor: E, line: &PurchaseOrderLine) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"INSERT INTO purchase_order_lines (id, org_id, purchase_order_id, stock_item_id,
                                             catalog_item_id, description, qty_ordered_milli,
                                             qty_received_milli, unit_cost_minor, receipt_state,
                                             position)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
    )
    .bind(line.id)
    .bind(line.org_id)
    .bind(line.purchase_order_id)
    .bind(line.stock_item_id)
    .bind(line.catalog_item_id)
    .bind(line.description.trim())
    .bind(line.qty_ordered_milli)
    .bind(line.qty_received_milli)
    .bind(line.unit_cost_minor)
    .bind(line.receipt_state.as_str())
    .bind(line.position)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_po_line<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<PurchaseOrderLine>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        "SELECT {LINE_COLS} FROM purchase_order_lines WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    line_row(row)
}

pub async fn list_po_lines(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    purchase_order_id: Uuid,
) -> DbResult<Vec<PurchaseOrderLine>> {
    let rows = sqlx::query(&format!(
        r#"SELECT {LINE_COLS} FROM purchase_order_lines
           WHERE org_id = $1 AND purchase_order_id = $2
           ORDER BY position, created_at"#
    ))
    .bind(org_id)
    .bind(purchase_order_id)
    .fetch_all(&mut *executor)
    .await?;
    rows.into_iter().map(line_row).collect()
}

/// Every line of every order in `purchase_order_ids`, in one round trip.
///
/// The list endpoint prices a page of orders so each row can carry its own
/// total, which would otherwise be a query per order. The rows stay org-scoped
/// on `org_id` exactly like every other read here, so RLS applies unchanged.
/// Ordered by order then position so each order's lines arrive in the same
/// order `list_po_lines` would have given.
pub async fn list_po_lines_for_orders(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    purchase_order_ids: &[Uuid],
) -> DbResult<Vec<PurchaseOrderLine>> {
    if purchase_order_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(&format!(
        r#"SELECT {LINE_COLS} FROM purchase_order_lines
           WHERE org_id = $1 AND purchase_order_id = ANY($2)
           ORDER BY purchase_order_id, position, created_at"#
    ))
    .bind(org_id)
    .bind(purchase_order_ids)
    .fetch_all(&mut *executor)
    .await?;
    rows.into_iter().map(line_row).collect()
}

/// Draft line edits (full-row write: the route loads, patches, saves).
pub async fn update_po_line<'e, E>(executor: E, line: &PurchaseOrderLine) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let n = sqlx::query(
        r#"UPDATE purchase_order_lines
             SET stock_item_id = $3, catalog_item_id = $4, description = $5,
                 qty_ordered_milli = $6, unit_cost_minor = $7, position = $8, updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
    )
    .bind(line.org_id)
    .bind(line.id)
    .bind(line.stock_item_id)
    .bind(line.catalog_item_id)
    .bind(line.description.trim())
    .bind(line.qty_ordered_milli)
    .bind(line.unit_cost_minor)
    .bind(line.position)
    .execute(executor)
    .await?;
    if n.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

pub async fn delete_po_line<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let n = sqlx::query("DELETE FROM purchase_order_lines WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(executor)
        .await?;
    if n.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// The receipt engine's guarded line bump: increments the received
/// quantity and stamps the typed state atomically. `allow_over = false`
/// refuses the bump that would exceed the ordered quantity (typed
/// conflict), so a concurrent receipt can never silently over-receive.
pub async fn bump_line_received(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    line_id: Uuid,
    add_milli: i64,
    allow_over: bool,
) -> DbResult<PurchaseOrderLine> {
    let row = sqlx::query(&format!(
        r#"UPDATE purchase_order_lines SET
             qty_received_milli = qty_received_milli + $3,
             receipt_state = CASE
               WHEN qty_received_milli + $3 > qty_ordered_milli THEN 'over'
               WHEN qty_received_milli + $3 = qty_ordered_milli THEN 'complete'
               ELSE 'partial'
             END,
             updated_at = now()
           WHERE org_id = $1 AND id = $2
             AND ($4::boolean OR qty_received_milli + $3 <= qty_ordered_milli)
           RETURNING {LINE_COLS}"#
    ))
    .bind(org_id)
    .bind(line_id)
    .bind(add_milli)
    .bind(allow_over)
    .fetch_optional(&mut *executor)
    .await?
    .ok_or_else(|| {
        DbError::Conflict(format!(
            "receipt of {add_milli} milli against line {line_id} exceeds the ordered quantity"
        ))
    })?;
    line_row(row)
}

// Receipts ---------------------------------------------------------------------

pub async fn insert_receipt<'e, E>(executor: E, receipt: &PurchaseReceipt) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"INSERT INTO purchase_receipts (id, org_id, purchase_order_id, location_id, carrier,
                                           tracking_ref, note, received_at, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#,
    )
    .bind(receipt.id)
    .bind(receipt.org_id)
    .bind(receipt.purchase_order_id)
    .bind(receipt.location_id)
    .bind(receipt.carrier.as_deref())
    .bind(receipt.tracking_ref.as_deref())
    .bind(receipt.note.as_deref())
    .bind(receipt.received_at)
    .bind(receipt.created_by)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn insert_receipt_line<'e, E>(executor: E, line: &PurchaseReceiptLine) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"INSERT INTO purchase_receipt_lines (id, org_id, receipt_id, po_line_id, qty_milli,
                                                movement_id)
           VALUES ($1, $2, $3, $4, $5, $6)"#,
    )
    .bind(line.id)
    .bind(line.org_id)
    .bind(line.receipt_id)
    .bind(line.po_line_id)
    .bind(line.qty_milli)
    .bind(line.movement_id)
    .execute(executor)
    .await?;
    Ok(())
}

/// One receipt line as the routing read side sees it: the line, the class its
/// consequence landed in, and the asset row when the consequence was an asset.
/// A stock consequence names its ledger row through the line's own
/// `movement_id`; a client-lot consequence names none and is the lot identity
/// ADR 0035 §2 describes (a line plus a quantity).
#[derive(Debug, Clone)]
pub struct ReceiptLine {
    pub line: PurchaseReceiptLine,
    pub destination_class: Option<CustodyClass>,
    pub device_asset_id: Option<Uuid>,
}

/// The receipt engine's insert: the line plus its one consequence in the same
/// statement. `insert_receipt_line` stays the plain form the demo seed uses, so
/// neither writer has to pretend the other's columns do not exist.
pub async fn insert_routed_receipt_line<'e, E>(
    executor: E,
    line: &PurchaseReceiptLine,
    destination_class: CustodyClass,
    device_asset_id: Option<Uuid>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"INSERT INTO purchase_receipt_lines
             (id, org_id, receipt_id, po_line_id, qty_milli, movement_id,
              destination_class, device_asset_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
    )
    .bind(line.id)
    .bind(line.org_id)
    .bind(line.receipt_id)
    .bind(line.po_line_id)
    .bind(line.qty_milli)
    .bind(line.movement_id)
    .bind(destination_class.as_str())
    .bind(device_asset_id)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn list_receipts(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    purchase_order_id: Uuid,
) -> DbResult<Vec<PurchaseReceipt>> {
    let rows = sqlx::query(&format!(
        r#"SELECT {RECEIPT_COLS} FROM purchase_receipts
           WHERE org_id = $1 AND purchase_order_id = $2
           ORDER BY received_at DESC"#
    ))
    .bind(org_id)
    .bind(purchase_order_id)
    .fetch_all(&mut *executor)
    .await?;
    Ok(rows.into_iter().map(receipt_row).collect())
}

/// All receipt lines for an order (joined through its receipts) — the
/// per-line movement or asset trace lives here.
pub async fn list_receipt_lines_for_order(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    purchase_order_id: Uuid,
) -> DbResult<Vec<ReceiptLine>> {
    let rows = sqlx::query(
        r#"SELECT rl.id, rl.org_id, rl.receipt_id, rl.po_line_id, rl.qty_milli, rl.movement_id,
                  rl.created_at, rl.destination_class, rl.device_asset_id
           FROM purchase_receipt_lines rl
           JOIN purchase_receipts r
             ON r.org_id = rl.org_id AND r.id = rl.receipt_id
           WHERE rl.org_id = $1 AND r.purchase_order_id = $2
           ORDER BY rl.created_at"#,
    )
    .bind(org_id)
    .bind(purchase_order_id)
    .fetch_all(&mut *executor)
    .await?;
    rows.into_iter()
        .map(|row| {
            let destination_class = row
                .get::<Option<String>, _>("destination_class")
                .map(|raw| {
                    CustodyClass::parse(&raw).map_err(|e| decode_err("destination_class", e))
                })
                .transpose()?;
            let device_asset_id: Option<Uuid> = row.get("device_asset_id");
            Ok(ReceiptLine {
                line: receipt_line_row(row),
                destination_class,
                device_asset_id,
            })
        })
        .collect::<DbResult<Vec<_>>>()
}

// Intent, routing and recharge (ADR 0035 §2, plan §5; migration 0235) ---------
//
// The intent lives beside the order and the line rather than on the domain
// structs: `PurchaseIntent` and `PurchaseLineIntent` are the vocabulary, these
// rows are what the columns hold. A line's `destination_class` is `None` when it
// inherits the header's intent, which is why the receipt engine resolves the
// class instead of reading a frozen copy.

/// The header intent as `purchase_orders` holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderIntent {
    pub acquired_for: AcquiredFor,
    pub client_id: Option<Uuid>,
    pub ticket_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub ownership: Option<Ownership>,
    pub delivery_mode: DeliveryMode,
    pub recharge_rule: Option<RechargeRule>,
}

/// One line's intent as `purchase_order_lines` holds it. `destination_class`
/// is `None` for a line that inherits the header intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIntent {
    pub line_kind: LineKind,
    pub destination_class: Option<CustodyClass>,
    pub asset_layout_id: Option<Uuid>,
    pub serial_expected: bool,
    pub weight_g: Option<i32>,
    pub sell_price_minor: Option<i64>,
    pub billable: bool,
}

impl LineIntent {
    /// The stock defaults a line carries when the caller says nothing.
    pub fn stock() -> Self {
        Self {
            line_kind: LineKind::Stock,
            destination_class: None,
            asset_layout_id: None,
            serial_expected: false,
            weight_g: None,
            sell_price_minor: None,
            billable: false,
        }
    }
}

fn recharge_rule_from_columns(
    kind: Option<String>,
    markup_bps: Option<i32>,
    fixed_minor: Option<i64>,
    currency: Option<String>,
) -> DbResult<Option<RechargeRule>> {
    let Some(raw) = kind else {
        return Ok(None);
    };
    let rule = RechargeRule {
        kind: RechargeRuleKind::parse(&raw).map_err(|e| decode_err("recharge_rule_kind", e))?,
        markup_bps,
        fixed_minor,
        currency,
    };
    rule.validate()
        .map_err(|e| decode_err("recharge_rule", e))?;
    Ok(Some(rule))
}

fn order_intent_row(row: sqlx::postgres::PgRow) -> DbResult<(Uuid, OrderIntent)> {
    let id: Uuid = row.get("id");
    let acquired_for = AcquiredFor::parse(row.get::<String, _>("acquired_for").as_str())
        .map_err(|e| decode_err("acquired_for", e))?;
    let delivery_mode = DeliveryMode::parse(row.get::<String, _>("delivery_mode").as_str())
        .map_err(|e| decode_err("delivery_mode", e))?;
    let ownership = row
        .get::<Option<String>, _>("ownership")
        .map(|raw| Ownership::parse(&raw).map_err(|e| decode_err("ownership", e)))
        .transpose()?;
    let recharge_rule = recharge_rule_from_columns(
        row.get("recharge_rule_kind"),
        row.get("recharge_markup_bps"),
        row.get("recharge_fixed_minor"),
        row.get("recharge_currency"),
    )?;
    Ok((
        id,
        OrderIntent {
            acquired_for,
            client_id: row.get("client_id"),
            ticket_id: row.get("ticket_id"),
            project_id: row.get("project_id"),
            ownership,
            delivery_mode,
            recharge_rule,
        },
    ))
}

fn line_intent_row(row: sqlx::postgres::PgRow) -> DbResult<(Uuid, LineIntent)> {
    let id: Uuid = row.get("id");
    let line_kind = LineKind::parse(row.get::<String, _>("line_kind").as_str())
        .map_err(|e| decode_err("line_kind", e))?;
    let destination_class = row
        .get::<Option<String>, _>("destination_class")
        .map(|raw| CustodyClass::parse(&raw).map_err(|e| decode_err("destination_class", e)))
        .transpose()?;
    Ok((
        id,
        LineIntent {
            line_kind,
            destination_class,
            asset_layout_id: row.get("asset_layout_id"),
            serial_expected: row.get("serial_expected"),
            weight_g: row.get("weight_g"),
            sell_price_minor: row.get("sell_price_minor"),
            billable: row.get("billable"),
        },
    ))
}

const ORDER_INTENT_COLS: &str = "id, acquired_for, client_id, ticket_id, project_id, ownership, \
     delivery_mode, recharge_rule_kind, recharge_markup_bps, recharge_fixed_minor, \
     recharge_currency";
const LINE_INTENT_COLS: &str = "id, line_kind, destination_class, asset_layout_id, \
     serial_expected, weight_g, sell_price_minor, billable";

pub async fn get_order_intent<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<OrderIntent>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        "SELECT {ORDER_INTENT_COLS} FROM purchase_orders WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(order_intent_row(row)?.1)
}

/// Every header intent for a page of orders in one round trip: the list
/// endpoint must not run one query per order to know what each one buys.
pub async fn list_order_intents(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    ids: &[Uuid],
) -> DbResult<HashMap<Uuid, OrderIntent>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query(&format!(
        "SELECT {ORDER_INTENT_COLS} FROM purchase_orders WHERE org_id = $1 AND id = ANY($2)"
    ))
    .bind(org_id)
    .bind(ids)
    .fetch_all(&mut *executor)
    .await?;
    rows.into_iter()
        .map(order_intent_row)
        .collect::<DbResult<HashMap<_, _>>>()
}

pub async fn set_order_intent<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    intent: &OrderIntent,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rule = intent.recharge_rule.as_ref();
    let n = sqlx::query(
        r#"UPDATE purchase_orders SET
             acquired_for = $3, client_id = $4, ticket_id = $5, ownership = $6,
             delivery_mode = $7, recharge_rule_kind = $8, recharge_markup_bps = $9,
             recharge_fixed_minor = $10, recharge_currency = $11, updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
    )
    .bind(org_id)
    .bind(id)
    .bind(intent.acquired_for.as_str())
    .bind(intent.client_id)
    .bind(intent.ticket_id)
    .bind(intent.ownership.map(Ownership::as_str))
    .bind(intent.delivery_mode.as_str())
    .bind(rule.map(|r| r.kind.as_str()))
    .bind(rule.and_then(|r| r.markup_bps))
    .bind(rule.and_then(|r| r.fixed_minor))
    .bind(rule.and_then(|r| r.currency.as_deref()))
    .execute(executor)
    .await?;
    if n.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

pub async fn list_line_intents(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    purchase_order_id: Uuid,
) -> DbResult<HashMap<Uuid, LineIntent>> {
    let rows = sqlx::query(&format!(
        r#"SELECT {LINE_INTENT_COLS} FROM purchase_order_lines
           WHERE org_id = $1 AND purchase_order_id = $2"#
    ))
    .bind(org_id)
    .bind(purchase_order_id)
    .fetch_all(&mut *executor)
    .await?;
    rows.into_iter()
        .map(line_intent_row)
        .collect::<DbResult<HashMap<_, _>>>()
}

pub async fn set_line_intent<'e, E>(
    executor: E,
    org_id: Uuid,
    line_id: Uuid,
    intent: &LineIntent,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let n = sqlx::query(
        r#"UPDATE purchase_order_lines SET
             line_kind = $3, destination_class = $4, asset_layout_id = $5,
             serial_expected = $6, weight_g = $7, sell_price_minor = $8,
             billable = $9, updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
    )
    .bind(org_id)
    .bind(line_id)
    .bind(intent.line_kind.as_str())
    .bind(intent.destination_class.map(CustodyClass::as_str))
    .bind(intent.asset_layout_id)
    .bind(intent.serial_expected)
    .bind(intent.weight_g)
    .bind(intent.sell_price_minor)
    .bind(intent.billable)
    .execute(executor)
    .await?;
    if n.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// The organisation's inventory money policy (migration 0234 columns). The
/// defaults are the column defaults, so an org that never configured a rule
/// reads back the percentage-at-zero rule the database holds.
pub async fn get_inventory_policy<'e, E>(executor: E, org_id: Uuid) -> DbResult<InventoryPolicy>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(
        r#"SELECT landed_cost_mode, recharge_rule_kind, recharge_markup_bps,
                  recharge_fixed_minor, recharge_currency
           FROM orgs WHERE id = $1"#,
    )
    .bind(org_id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    let policy = InventoryPolicy {
        landed_cost_mode: LandedCostMode::parse(row.get::<String, _>("landed_cost_mode").as_str())
            .map_err(|e| decode_err("landed_cost_mode", e))?,
        recharge_rule_kind: RechargeRuleKind::parse(
            row.get::<String, _>("recharge_rule_kind").as_str(),
        )
        .map_err(|e| decode_err("recharge_rule_kind", e))?,
        recharge_markup_bps: row.get("recharge_markup_bps"),
        recharge_fixed_minor: row.get("recharge_fixed_minor"),
        recharge_currency: row.get("recharge_currency"),
    };
    policy
        .validate()
        .map_err(|e| decode_err("inventory_policy", e))?;
    Ok(policy)
}

/// Which rung of the resolution order produced a recharge rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RechargeRuleSource {
    /// The rule the purchase order itself carries.
    PurchaseOrder,
    /// An override on the ticket the purchase serves.
    Ticket,
    /// An override on the project the purchase serves.
    Project,
    /// An override on the client the purchase is made for.
    Client,
    /// The organisation default (`orgs.recharge_*`).
    Organisation,
}

impl RechargeRuleSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            RechargeRuleSource::PurchaseOrder => "purchase_order",
            RechargeRuleSource::Ticket => "ticket",
            RechargeRuleSource::Project => "project",
            RechargeRuleSource::Client => "client",
            RechargeRuleSource::Organisation => "organisation",
        }
    }

    pub fn parse(raw: &str) -> DbResult<Self> {
        match raw.trim() {
            "purchase_order" => Ok(Self::PurchaseOrder),
            "ticket" => Ok(Self::Ticket),
            "project" => Ok(Self::Project),
            "client" => Ok(Self::Client),
            "organisation" => Ok(Self::Organisation),
            other => Err(DbError::Conflict(format!(
                "unknown recharge rule source: {other}"
            ))),
        }
    }
}

/// The rule in force for one purchase, with the rung that supplied it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRechargeRule {
    pub rule: RechargeRule,
    pub source: RechargeRuleSource,
}

/// Plan §5.3, in order: the rule written on the purchase, then the ticket or
/// project it serves, then the client, then the organisation default. Zero
/// markup is a real percentage rule (at cost), which is why the default is
/// read as a rule rather than treated as "unset".
pub async fn resolve_recharge_rule(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    header_rule: Option<&RechargeRule>,
    ticket_id: Option<Uuid>,
    project_id: Option<Uuid>,
    client_id: Option<Uuid>,
) -> DbResult<ResolvedRechargeRule> {
    if let Some(rule) = header_rule {
        return Ok(ResolvedRechargeRule {
            rule: rule.clone(),
            source: RechargeRuleSource::PurchaseOrder,
        });
    }
    for (scope_kind, scope_id, source) in [
        ("ticket", ticket_id, RechargeRuleSource::Ticket),
        ("project", project_id, RechargeRuleSource::Project),
        ("client", client_id, RechargeRuleSource::Client),
    ] {
        let Some(scope_id) = scope_id else {
            continue;
        };
        let row = sqlx::query(
            r#"SELECT rule_kind, markup_bps, fixed_minor, currency
               FROM purchase_recharge_rule_overrides
               WHERE org_id = $1 AND scope_kind = $2 AND scope_id = $3"#,
        )
        .bind(org_id)
        .bind(scope_kind)
        .bind(scope_id)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some(row) = row {
            let kind = RechargeRuleKind::parse(row.get::<String, _>("rule_kind").as_str())
                .map_err(|e| decode_err("rule_kind", e))?;
            let rule = RechargeRule {
                kind,
                markup_bps: row.get("markup_bps"),
                fixed_minor: row.get("fixed_minor"),
                currency: row.get("currency"),
            };
            rule.validate()
                .map_err(|e| decode_err("recharge_rule", e))?;
            return Ok(ResolvedRechargeRule { rule, source });
        }
    }
    let policy = get_inventory_policy(&mut *conn, org_id).await?;
    Ok(ResolvedRechargeRule {
        rule: policy
            .resolved_recharge_rule()
            .map_err(|e| decode_err("recharge_rule", e))?,
        source: RechargeRuleSource::Organisation,
    })
}

/// One issued recharge, ready to persist.
#[derive(Debug, Clone)]
pub struct NewRecharge {
    pub id: Uuid,
    pub org_id: Uuid,
    pub purchase_order_id: Uuid,
    pub po_line_id: Uuid,
    pub receipt_line_id: Uuid,
    pub client_id: Option<Uuid>,
    pub ticket_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub rule: RechargeRule,
    pub rule_source: RechargeRuleSource,
    pub basis_amount_minor: i64,
    pub basis_currency: String,
    pub cost_fx: Option<FxColumns>,
    pub charge_amount_minor: i64,
    pub charge_currency: String,
    pub charge_fx: Option<FxColumns>,
    pub cost_line_id: Option<Uuid>,
    pub created_by: Uuid,
    pub issued_at: DateTime<Utc>,
}

/// A locked FX snapshot's four columns, as every money table stores them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FxColumns {
    pub rate_used: String,
    pub rate_date: NaiveDate,
    pub source: String,
    pub base_amount_minor: i64,
}

/// The one recharge row per receipt line. The unique index on
/// `(org_id, receipt_line_id)` is the idempotency guard, so a second write is a
/// conflict rather than a double charge.
pub async fn insert_recharge<'e, E>(executor: E, recharge: &NewRecharge) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"INSERT INTO purchase_recharges
             (id, org_id, purchase_order_id, po_line_id, receipt_line_id, client_id,
              ticket_id, project_id, rule_kind, rule_source, rule_markup_bps,
              rule_fixed_minor, rule_currency, basis_amount_minor, basis_currency,
              cost_fx_rate_used, cost_fx_rate_date, cost_fx_source, cost_base_amount_minor,
              charge_amount_minor, charge_currency, charge_fx_rate_used, charge_fx_rate_date,
              charge_fx_source, charge_base_amount_minor, cost_line_id, issued_at, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
                   $16::text::numeric, $17, $18, $19, $20, $21, $22::text::numeric, $23,
                   $24, $25, $26, $27, $28)"#,
    )
    .bind(recharge.id)
    .bind(recharge.org_id)
    .bind(recharge.purchase_order_id)
    .bind(recharge.po_line_id)
    .bind(recharge.receipt_line_id)
    .bind(recharge.client_id)
    .bind(recharge.ticket_id)
    .bind(recharge.project_id)
    .bind(recharge.rule.kind.as_str())
    .bind(recharge.rule_source.as_str())
    .bind(recharge.rule.markup_bps)
    .bind(recharge.rule.fixed_minor)
    .bind(recharge.rule.currency.as_deref())
    .bind(recharge.basis_amount_minor)
    .bind(&recharge.basis_currency)
    .bind(recharge.cost_fx.as_ref().map(|fx| fx.rate_used.clone()))
    .bind(recharge.cost_fx.as_ref().map(|fx| fx.rate_date))
    .bind(recharge.cost_fx.as_ref().map(|fx| fx.source.clone()))
    .bind(recharge.cost_fx.as_ref().map(|fx| fx.base_amount_minor))
    .bind(recharge.charge_amount_minor)
    .bind(&recharge.charge_currency)
    .bind(recharge.charge_fx.as_ref().map(|fx| fx.rate_used.clone()))
    .bind(recharge.charge_fx.as_ref().map(|fx| fx.rate_date))
    .bind(recharge.charge_fx.as_ref().map(|fx| fx.source.clone()))
    .bind(recharge.charge_fx.as_ref().map(|fx| fx.base_amount_minor))
    .bind(recharge.cost_line_id)
    .bind(recharge.issued_at)
    .bind(recharge.created_by)
    .execute(executor)
    .await?;
    Ok(())
}

/// One issued recharge as the read side sees it.
#[derive(Debug, Clone)]
pub struct Recharge {
    pub id: Uuid,
    pub purchase_order_id: Uuid,
    pub po_line_id: Uuid,
    pub receipt_line_id: Uuid,
    pub client_id: Option<Uuid>,
    pub ticket_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub rule: RechargeRule,
    pub rule_source: RechargeRuleSource,
    pub basis_amount_minor: i64,
    pub basis_currency: String,
    pub cost_fx: Option<FxColumns>,
    pub charge_amount_minor: i64,
    pub charge_currency: String,
    pub charge_fx: Option<FxColumns>,
    pub cost_line_id: Option<Uuid>,
    pub issued_at: DateTime<Utc>,
}

fn fx_columns(
    rate_used: Option<String>,
    rate_date: Option<NaiveDate>,
    source: Option<String>,
    base_amount_minor: Option<i64>,
) -> DbResult<Option<FxColumns>> {
    match (rate_used, rate_date, source, base_amount_minor) {
        (None, None, None, None) => Ok(None),
        (Some(rate_used), Some(rate_date), Some(source), Some(base_amount_minor)) => {
            Ok(Some(FxColumns {
                rate_used,
                rate_date,
                source,
                base_amount_minor,
            }))
        }
        _ => Err(DbError::Conflict(
            "a recharge FX snapshot is stored as four columns or none".into(),
        )),
    }
}

pub async fn list_recharges_for_order(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    purchase_order_id: Uuid,
) -> DbResult<Vec<Recharge>> {
    let rows = sqlx::query(
        r#"SELECT id, purchase_order_id, po_line_id, receipt_line_id, client_id, ticket_id,
                  project_id, rule_kind, rule_source, rule_markup_bps, rule_fixed_minor,
                  rule_currency, basis_amount_minor, basis_currency,
                  cost_fx_rate_used::text AS cost_fx_rate_used, cost_fx_rate_date,
                  cost_fx_source, cost_base_amount_minor, charge_amount_minor,
                  charge_currency, charge_fx_rate_used::text AS charge_fx_rate_used,
                  charge_fx_rate_date, charge_fx_source, charge_base_amount_minor,
                  cost_line_id, issued_at
           FROM purchase_recharges
           WHERE org_id = $1 AND purchase_order_id = $2
           ORDER BY issued_at"#,
    )
    .bind(org_id)
    .bind(purchase_order_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.into_iter()
        .map(|row| {
            let rule_kind = RechargeRuleKind::parse(row.get::<String, _>("rule_kind").as_str())
                .map_err(|e| decode_err("rule_kind", e))?;
            let rule = RechargeRule {
                kind: rule_kind,
                markup_bps: row.get("rule_markup_bps"),
                fixed_minor: row.get("rule_fixed_minor"),
                currency: row.get("rule_currency"),
            };
            rule.validate()
                .map_err(|e| decode_err("recharge_rule", e))?;
            Ok(Recharge {
                id: row.get("id"),
                purchase_order_id: row.get("purchase_order_id"),
                po_line_id: row.get("po_line_id"),
                receipt_line_id: row.get("receipt_line_id"),
                client_id: row.get("client_id"),
                ticket_id: row.get("ticket_id"),
                project_id: row.get("project_id"),
                rule,
                rule_source: RechargeRuleSource::parse(
                    row.get::<String, _>("rule_source").as_str(),
                )?,
                basis_amount_minor: row.get("basis_amount_minor"),
                basis_currency: row.get("basis_currency"),
                cost_fx: fx_columns(
                    row.get("cost_fx_rate_used"),
                    row.get("cost_fx_rate_date"),
                    row.get("cost_fx_source"),
                    row.get("cost_base_amount_minor"),
                )?,
                charge_amount_minor: row.get("charge_amount_minor"),
                charge_currency: row.get("charge_currency"),
                charge_fx: fx_columns(
                    row.get("charge_fx_rate_used"),
                    row.get("charge_fx_rate_date"),
                    row.get("charge_fx_source"),
                    row.get("charge_base_amount_minor"),
                )?,
                cost_line_id: row.get("cost_line_id"),
                issued_at: row.get("issued_at"),
            })
        })
        .collect()
}

/// Stamps the source identity and reporting class on the cost line a recharge
/// produced. Kept as a second statement on purpose: `core_db::costs`'s insert
/// is shared with the ticket surface, and the recharge columns (migration 0234)
/// are written here so the billing join can cite the receipt line later.
pub async fn stamp_cost_line_source<'e, E>(
    executor: E,
    org_id: Uuid,
    cost_line_id: Uuid,
    source_id: Uuid,
    reporting_class: &str,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let n = sqlx::query(
        r#"UPDATE ticket_cost_lines
              SET source_kind = 'purchase_receipt_line', source_id = $3,
                  reporting_class = $4
            WHERE org_id = $1 AND id = $2 AND source_kind IS NULL"#,
    )
    .bind(org_id)
    .bind(cost_line_id)
    .bind(source_id)
    .bind(reporting_class)
    .execute(executor)
    .await?;
    if n.rows_affected() == 0 {
        return Err(DbError::Conflict(format!(
            "cost line {cost_line_id} already carries a source identity"
        )));
    }
    Ok(())
}

/// Records the class a receipt'd asset moved into when the class it lands in is
/// not the one its scope and ownership imply. The insert trigger in migration
/// 0232 applies the new class and refuses a stale `from_class`, so this is the
/// only sanctioned way to put a purchased unit in the rental pool or a vendor
/// rental, and the movement cites the receipt line that caused it.
#[allow(clippy::too_many_arguments)]
pub async fn record_asset_class_movement<'e, E>(
    executor: E,
    org_id: Uuid,
    device_asset_id: Uuid,
    from_class: CustodyClass,
    to_class: CustodyClass,
    reason: &str,
    receipt_line_id: Uuid,
    created_by: Uuid,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"INSERT INTO inventory_class_movements
             (id, org_id, unit_kind, device_asset_id, from_class, to_class, reason,
              source_kind, source_id, created_by)
           VALUES ($1, $2, 'asset', $3, $4, $5, $6, 'purchase_receipt_line', $7, $8)"#,
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(device_asset_id)
    .bind(from_class.as_str())
    .bind(to_class.as_str())
    .bind(reason)
    .bind(receipt_line_id)
    .bind(created_by)
    .execute(executor)
    .await?;
    Ok(())
}

// Bills + project cost rollup --------------------------------------------------

/// Create-or-update the PO's draft bill; a posted bill is frozen (typed
/// conflict — corrections are a new posted fact, never an edit).
pub async fn upsert_bill(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    purchase_order_id: Uuid,
    bill_number: Option<&str>,
    amount_minor: i64,
    currency: &str,
) -> DbResult<PurchaseBill> {
    let posted: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT posted_at FROM purchase_bills WHERE org_id = $1 AND purchase_order_id = $2",
    )
    .bind(org_id)
    .bind(purchase_order_id)
    .fetch_optional(&mut *executor)
    .await?
    .flatten();
    if posted.is_some() {
        return Err(DbError::Conflict(
            "the bill for this purchase order is posted and cannot be edited".into(),
        ));
    }
    let row = sqlx::query(&format!(
        r#"INSERT INTO purchase_bills (id, org_id, purchase_order_id, bill_number, amount_minor, currency)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (purchase_order_id) DO UPDATE
             SET bill_number = EXCLUDED.bill_number,
                 amount_minor = EXCLUDED.amount_minor,
                 currency = EXCLUDED.currency,
                 updated_at = now()
             WHERE purchase_bills.posted_at IS NULL
           RETURNING {BILL_COLS}"#
    ))
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(purchase_order_id)
    .bind(bill_number)
    .bind(amount_minor)
    .bind(currency)
    .fetch_optional(&mut *executor)
    .await?
    // The DO UPDATE guard returning nothing means the bill was posted
    // between the check above and this write.
    .ok_or_else(|| {
        DbError::Conflict("the bill for this purchase order is posted and cannot be edited".into())
    })?;
    Ok(bill_row(row))
}

pub async fn get_bill(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    purchase_order_id: Uuid,
) -> DbResult<Option<PurchaseBill>> {
    let row = sqlx::query(&format!(
        "SELECT {BILL_COLS} FROM purchase_bills WHERE org_id = $1 AND purchase_order_id = $2"
    ))
    .bind(org_id)
    .bind(purchase_order_id)
    .fetch_optional(&mut *executor)
    .await?;
    Ok(row.map(bill_row))
}

/// A locked ADR 0002 snapshot ready to persist (computed by the API layer
/// via `routes/fx.rs::lock_snapshot`).
#[derive(Debug, Clone)]
pub struct BillFxSnapshot {
    pub rate_used: String,
    pub rate_date: NaiveDate,
    pub source: String,
    pub base_amount_minor: i64,
}

/// Post the bill: locks the FX snapshot (all-NULL group for base-currency
/// facts) and stamps `posted_at` — the moment it enters project rollups.
pub async fn post_bill(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    purchase_order_id: Uuid,
    fx: Option<&BillFxSnapshot>,
) -> DbResult<PurchaseBill> {
    let row = sqlx::query(&format!(
        r#"UPDATE purchase_bills SET
             posted_at = now(),
             fx_rate_used = $3::numeric, fx_rate_date = $4, fx_source = $5,
             base_amount_minor = $6,
             updated_at = now()
           WHERE org_id = $1 AND purchase_order_id = $2 AND posted_at IS NULL
           RETURNING {BILL_COLS}"#
    ))
    .bind(org_id)
    .bind(purchase_order_id)
    .bind(fx.map(|f| f.rate_used.clone()))
    .bind(fx.map(|f| f.rate_date))
    .bind(fx.map(|f| f.source.clone()))
    .bind(fx.map(|f| f.base_amount_minor))
    .fetch_optional(&mut *executor)
    .await?
    .ok_or_else(|| DbError::Conflict("no unposted bill exists for this purchase order".into()))?;
    Ok(bill_row(row))
}

/// One posted bill on a project rollup.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RollupBill {
    pub bill_id: Uuid,
    pub purchase_order_id: Uuid,
    pub purchase_order_number: String,
    pub amount_minor: i64,
    pub currency: String,
    pub base_amount_minor: Option<i64>,
    pub posted_at: Option<DateTime<Utc>>,
}

/// The project material-cost read model: posted PO bills, base-currency
/// sums (ADR 0002 locked), plus the project's budget for comparison.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectCostRollup {
    pub project_id: Uuid,
    pub budget_minor: Option<i64>,
    pub budget_currency: Option<String>,
    pub base_currency: String,
    pub po_bill_base_minor: i64,
    pub bills: Vec<RollupBill>,
}

pub async fn project_cost_rollup(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    project_id: Uuid,
) -> DbResult<ProjectCostRollup> {
    let project =
        sqlx::query("SELECT budget_minor, currency FROM projects WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(project_id)
            .fetch_optional(&mut *executor)
            .await?
            .ok_or(DbError::NotFound)?;
    let base_currency = crate::fx::base_currency(&mut *executor, org_id).await?;
    let rows = sqlx::query(
        r#"SELECT b.id AS bill_id, b.purchase_order_id, o.number AS order_number,
                  b.amount_minor, b.currency, b.base_amount_minor, b.posted_at
           FROM purchase_bills b
           JOIN purchase_orders o ON o.org_id = b.org_id AND o.id = b.purchase_order_id
           WHERE b.org_id = $1 AND o.project_id = $2 AND b.posted_at IS NOT NULL
           ORDER BY b.posted_at"#,
    )
    .bind(org_id)
    .bind(project_id)
    .fetch_all(&mut *executor)
    .await?;
    let mut bills = Vec::with_capacity(rows.len());
    let mut total: i64 = 0;
    for row in rows {
        let base_amount: Option<i64> = row.get("base_amount_minor");
        let amount: i64 = row.get("amount_minor");
        // `base_amount_minor` is the locked base conversion; a
        // base-currency bill has no snapshot and contributes its amount.
        total = total.saturating_add(base_amount.unwrap_or(amount));
        bills.push(RollupBill {
            bill_id: row.get("bill_id"),
            purchase_order_id: row.get("purchase_order_id"),
            purchase_order_number: row.get("order_number"),
            amount_minor: amount,
            currency: row.get("currency"),
            base_amount_minor: base_amount,
            posted_at: row.get("posted_at"),
        });
    }
    Ok(ProjectCostRollup {
        project_id,
        budget_minor: project.get("budget_minor"),
        budget_currency: project.get("currency"),
        base_currency,
        po_bill_base_minor: total,
        bills,
    })
}
