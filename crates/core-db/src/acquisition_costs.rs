//! Acquisition cost layers and their allocations (ADR 0035 decision 3, plan
//! §6; migration 0258).
//!
//! A receipt is the cost fact. Freight, duty, brokerage, insurance and
//! handling arrive later, as documents, and become layers that are split over
//! the receipt's lines. Nothing here edits a receipt or another layer: a wrong
//! layer is reversed by a new layer that names it.
//!
//! What this module owns:
//!
//!   * the layer row — vendor, reference, document date, currency, amount in
//!     integer minor units, the allocation method and the rule that produced
//!     the split, the org mode that was in force and the decision it produced,
//!     the reporting class, and the FX snapshot locked at the document date;
//!   * the split — one row per receipt line, in the layer's currency and in
//!     the org base currency, each side summing to the layer exactly;
//!   * the read models — a receipt's valuation (receipt cost plus allocated
//!     capitalised layers) and the reporting-class rollup.
//!
//! What this module does not do: no tax, no journal, no period close, and no
//! re-conversion. The valuation read answers from stored base amounts, so
//! running it twice on the same receipt produces the same money even after the
//! rate book gains a newer rate.
//!
//! Queries are runtime `sqlx::query`/`query_as` with explicit row mapping. This
//! follows `ewaste`, `assets` and `document_links`: the valuation and rollup
//! reads join five tables and carry optional filters, so their SQL text is
//! dynamic and has no single compile-time cache entry to prepare. Migration
//! 0258 is the authority on every guard; a macro would check column names and
//! prove nothing about the statement trigger that makes a split sum.

use crate::{DbError, DbResult};
use chrono::{DateTime, NaiveDate, Utc};
use core_domain::entities::TicketExpense;
use sqlx::{PgConnection, Row};
use std::collections::HashMap;
use uuid::Uuid;

/// The five charge kinds plan §6.2 names.
pub const COST_KINDS: [&str; 5] = ["freight", "duty", "brokerage", "insurance", "handling"];

/// How a layer is split over the receipt lines.
pub const ALLOCATION_METHODS: [&str; 3] = ["extended_value", "quantity", "weight"];

/// The rules a recorded split can name. `largest_remainder` is a fresh split;
/// `reversal_copy` means the rows were copied from the reversed layer.
pub const ALLOCATION_RULES: [&str; 2] = ["largest_remainder", "reversal_copy"];

pub fn is_valid_kind(value: &str) -> bool {
    COST_KINDS.contains(&value.trim())
}

pub fn is_valid_allocation_method(value: &str) -> bool {
    ALLOCATION_METHODS.contains(&value.trim())
}

/// The expense class a charge reports under when the operator names none
/// (plan §6.4): freight and duty have their own classes, and the other three
/// are services. A capitalised layer is `inventory` instead, which the API
/// decides from the mode and the flag before it reaches here.
pub fn default_reporting_class(kind: &str) -> &'static str {
    match kind {
        "freight" => "freight_in",
        "duty" => "duty",
        _ => "service_cost",
    }
}

fn guard_error(e: sqlx::Error) -> DbError {
    if let sqlx::Error::Database(db) = &e {
        if db.code().as_deref() == Some("P0001") {
            return DbError::Conflict(db.message().to_string());
        }
    }
    DbError::Sql(e)
}

// --- row shapes --------------------------------------------------------------

/// One charge document: a layer over one receipt.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AcquisitionCost {
    pub id: Uuid,
    pub org_id: Uuid,
    pub purchase_order_id: Uuid,
    pub receipt_id: Uuid,
    pub kind: String,
    pub vendor_id: Option<Uuid>,
    pub vendor_reference: Option<String>,
    pub reference: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub document_date: NaiveDate,
    pub currency: String,
    /// Signed: a charge is positive and a reversal is negative.
    pub amount_minor: i64,
    pub allocation_method: String,
    pub allocation_rule: String,
    /// The org's landed-cost mode when this layer was recorded.
    pub policy_mode: String,
    /// Whether the charge entered inventory cost. Derived once, from the mode
    /// and the operator's flag, and stored so a later switch cannot rewrite it.
    pub capitalised: bool,
    pub reporting_class: String,
    pub project_id: Option<Uuid>,
    pub ticket_id: Option<Uuid>,
    /// The client the purchase serves, copied from the order header. A
    /// citation, like the project and ticket: it gates a restricted client's
    /// money on every read.
    pub client_id: Option<Uuid>,
    pub posted_ticket_expense_id: Option<Uuid>,
    pub reverses_acquisition_cost_id: Option<Uuid>,
    pub fx_rate_used: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub fx_rate_date: Option<NaiveDate>,
    pub fx_source: Option<String>,
    pub base_amount_minor: Option<i64>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

impl AcquisitionCost {
    /// The layer's amount in the org base currency, which is what every
    /// valuation and rollup sums. A base-currency fact carries no snapshot and
    /// its amount IS its base amount — the 0065 rule.
    pub fn base_amount(&self) -> i64 {
        self.base_amount_minor.unwrap_or(self.amount_minor)
    }
}

/// One line's share of a layer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AcquisitionCostAllocation {
    pub id: Uuid,
    pub acquisition_cost_id: Uuid,
    pub receipt_line_id: Uuid,
    pub allocated_minor: i64,
    pub allocated_base_minor: Option<i64>,
    /// Extended value (order currency), quantity (milli-units) or weight
    /// (grams), depending on the layer's `allocation_method`.
    pub basis_value: i64,
}

impl AcquisitionCostAllocation {
    pub fn base_amount(&self) -> i64 {
        self.allocated_base_minor.unwrap_or(self.allocated_minor)
    }
}

/// A layer with its split, which is what every endpoint answers with.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AcquisitionCostDetail {
    pub cost: AcquisitionCost,
    pub allocations: Vec<AcquisitionCostAllocation>,
}

// --- the receipt a layer is split over ---------------------------------------

/// One receipt line, with everything a split and a valuation need. Read as one
/// set for the whole receipt: the allocation loop never issues a query per
/// line.
#[derive(Debug, Clone)]
pub struct ReceiptLineBasis {
    pub receipt_line_id: Uuid,
    pub po_line_id: Uuid,
    pub description: String,
    pub qty_milli: i64,
    pub destination_class: Option<String>,
    /// The PO line's unit cost, in the order currency.
    pub unit_cost_minor: i64,
    /// Per-unit weight in grams: the receipt line's own value, falling back to
    /// the stock item's. `None` means weight allocation cannot use this line.
    pub weight_g: Option<i32>,
    pub movement_id: Option<Uuid>,
    pub movement_cost_minor: Option<i64>,
    pub movement_base_cost_minor: Option<i64>,
    pub device_asset_id: Option<Uuid>,
    pub purchase_cost_minor: Option<i64>,
    pub purchase_base_cost_minor: Option<i64>,
}

/// The receipt header plus its lines.
#[derive(Debug, Clone)]
pub struct ReceiptContext {
    pub receipt_id: Uuid,
    pub purchase_order_id: Uuid,
    pub order_currency: String,
    pub project_id: Option<Uuid>,
    pub ticket_id: Option<Uuid>,
    /// The client the order serves, when it names one.
    pub client_id: Option<Uuid>,
    pub lines: Vec<ReceiptLineBasis>,
}

/// Loads one receipt's header and lines inside the org. A receipt outside the
/// caller's org is [`DbError::NotFound`], the same answer as one that does not
/// exist.
pub async fn receipt_context(
    conn: &mut PgConnection,
    org_id: Uuid,
    receipt_id: Uuid,
) -> DbResult<ReceiptContext> {
    let header = sqlx::query(
        r#"SELECT r.purchase_order_id, o.currency AS order_currency,
                  o.project_id, o.ticket_id, o.client_id
           FROM purchase_receipts r
           JOIN purchase_orders o ON o.org_id = r.org_id AND o.id = r.purchase_order_id
           WHERE r.org_id = $1 AND r.id = $2"#,
    )
    .bind(org_id)
    .bind(receipt_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;

    let rows = sqlx::query(
        r#"SELECT rl.id AS receipt_line_id, rl.po_line_id, rl.qty_milli,
                  rl.destination_class,
                  pol.description, pol.unit_cost_minor,
                  COALESCE(rl.weight_g, si.weight_g) AS weight_g,
                  rl.movement_id, sm.cost_minor AS movement_cost_minor,
                  sm.base_cost_minor AS movement_base_cost_minor,
                  rl.device_asset_id, da.purchase_cost_minor,
                  da.base_purchase_cost_minor
           FROM purchase_receipt_lines rl
           JOIN purchase_order_lines pol
             ON pol.org_id = rl.org_id AND pol.id = rl.po_line_id
           LEFT JOIN stock_items si
             ON si.org_id = pol.org_id AND si.id = pol.stock_item_id
           LEFT JOIN stock_movements sm
             ON sm.org_id = rl.org_id AND sm.id = rl.movement_id
           LEFT JOIN device_assets da
             ON da.org_id = rl.org_id AND da.id = rl.device_asset_id
           WHERE rl.org_id = $1 AND rl.receipt_id = $2
           ORDER BY rl.created_at, rl.id"#,
    )
    .bind(org_id)
    .bind(receipt_id)
    .fetch_all(&mut *conn)
    .await?;

    let lines = rows
        .into_iter()
        .map(|row| ReceiptLineBasis {
            receipt_line_id: row.get("receipt_line_id"),
            po_line_id: row.get("po_line_id"),
            description: row.get("description"),
            qty_milli: row.get("qty_milli"),
            destination_class: row.get("destination_class"),
            unit_cost_minor: row.get("unit_cost_minor"),
            weight_g: row.get("weight_g"),
            movement_id: row.get("movement_id"),
            movement_cost_minor: row.get("movement_cost_minor"),
            movement_base_cost_minor: row.get("movement_base_cost_minor"),
            device_asset_id: row.get("device_asset_id"),
            purchase_cost_minor: row.get("purchase_cost_minor"),
            purchase_base_cost_minor: row.get("purchase_base_cost_minor"),
        })
        .collect();

    Ok(ReceiptContext {
        receipt_id,
        purchase_order_id: header.get("purchase_order_id"),
        order_currency: header.get("order_currency"),
        project_id: header.get("project_id"),
        ticket_id: header.get("ticket_id"),
        client_id: header.get("client_id"),
        lines,
    })
}

// --- the split ---------------------------------------------------------------

/// Why a set of lines cannot be split under a method. Each arm names the line,
/// because "weight is missing" without a line is not actionable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitError {
    /// No line carries a positive basis under the method.
    EmptyBasis(&'static str),
    /// Weight was chosen and this line has no per-unit weight.
    MissingWeight {
        receipt_line_id: Uuid,
        description: String,
    },
    /// The receipt has no lines at all, so there is nothing to split over.
    NoLines,
    /// The apportionment arithmetic left a residual no line could absorb. It
    /// is unreachable when the inputs are non-negative and the total is
    /// non-negative; it exists so a silent loss is impossible.
    Unbalanced,
}

impl std::fmt::Display for SplitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SplitError::EmptyBasis(method) => write!(
                f,
                "the receipt lines carry no {method} to allocate by; record that basis first"
            ),
            SplitError::MissingWeight {
                receipt_line_id,
                description,
            } => write!(
                f,
                "line \"{description}\" ({receipt_line_id}) has no weight_g, and weight allocation needs one"
            ),
            SplitError::NoLines => write!(f, "the receipt has no lines to allocate over"),
            SplitError::Unbalanced => write!(f, "the allocation did not sum to its total"),
        }
    }
}

/// The unit cost the receipt recorded, in the order currency: the movement's
/// cost, the asset's purchase cost, or the line's own cost when the leg kept no
/// record of its own (a lot or a service). A later edit to the purchase line
/// cannot move a split that was computed after the receipt.
pub fn receipt_unit_cost(line: &ReceiptLineBasis) -> i64 {
    line.movement_cost_minor
        .or(line.purchase_cost_minor)
        .unwrap_or(line.unit_cost_minor)
}

/// The basis each line carries under the chosen method, in the method's own
/// unit: order-currency minor units, milli-units, or grams.
pub fn allocation_basis(
    lines: &[ReceiptLineBasis],
    method: &str,
) -> Result<Vec<(Uuid, i128)>, SplitError> {
    if lines.is_empty() {
        return Err(SplitError::NoLines);
    }
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        let basis = match method {
            "extended_value" => i128::from(
                core_domain::entities::line_amount_minor(line.qty_milli, receipt_unit_cost(line))
                    .map_err(|_| SplitError::EmptyBasis("extended value"))?,
            ),
            "quantity" => i128::from(line.qty_milli),
            "weight" => {
                let weight = line.weight_g.ok_or_else(|| SplitError::MissingWeight {
                    receipt_line_id: line.receipt_line_id,
                    description: line.description.clone(),
                })?;
                i128::from(
                    core_domain::entities::line_amount_minor(line.qty_milli, i64::from(weight))
                        .map_err(|_| SplitError::EmptyBasis("weight"))?,
                )
            }
            _ => return Err(SplitError::EmptyBasis("basis")),
        };
        out.push((line.receipt_line_id, basis.max(0)));
    }
    if out.iter().map(|(_, b)| b).sum::<i128>() <= 0 {
        return Err(SplitError::EmptyBasis(match method {
            "extended_value" => "extended value",
            "quantity" => "quantity",
            "weight" => "weight",
            _ => "basis",
        }));
    }
    Ok(out)
}

/// Apportions `total` over `shares` with the largest-remainder rule: every line
/// gets the floor of its exact share, and the residue goes one minor unit at a
/// time to the largest fractional remainders, ties broken by the line id so the
/// answer does not depend on the order the rows arrived in. The result sums to
/// `total` exactly, which is the property the migration's statement guard
/// re-checks.
pub fn largest_remainder(total: i64, shares: &[(Uuid, i128)]) -> Result<Vec<i64>, SplitError> {
    if shares.is_empty() {
        return Err(SplitError::NoLines);
    }
    let sum: i128 = shares.iter().map(|(_, share)| *share).sum();
    if sum <= 0 {
        return Err(SplitError::EmptyBasis("basis"));
    }
    if total < 0 {
        // Reversals copy their target's rows instead of splitting again, so a
        // negative total never reaches here; refusing is honest.
        return Err(SplitError::Unbalanced);
    }
    let total_i = i128::from(total);
    let mut allocated = Vec::with_capacity(shares.len());
    let mut remainders: Vec<(i128, Uuid, usize)> = Vec::with_capacity(shares.len());
    let mut assigned: i128 = 0;
    for (index, (id, share)) in shares.iter().enumerate() {
        let numerator = total_i * share;
        let floor = numerator / sum;
        let remainder = numerator % sum;
        assigned += floor;
        allocated.push(i64::try_from(floor).map_err(|_| SplitError::Unbalanced)?);
        remainders.push((remainder, *id, index));
    }
    let mut residue = total_i - assigned;
    if residue < 0 {
        return Err(SplitError::Unbalanced);
    }
    if residue > 0 {
        remainders.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        for (_, _, index) in remainders
            .into_iter()
            .take(usize::try_from(residue).unwrap_or(0))
        {
            allocated[index] += 1;
            residue -= 1;
        }
    }
    if residue != 0 {
        return Err(SplitError::Unbalanced);
    }
    Ok(allocated)
}

/// Splits one layer over a receipt's lines, in the layer currency and in the
/// base currency. `base_total` is the locked base amount; the same weights are
/// apportioned twice so both sides sum exactly instead of one drifting by a
/// cent when shares are converted individually.
pub fn split_layer(
    cost_id: Uuid,
    currency: &str,
    amount_minor: i64,
    base_total: i64,
    base_currency: &str,
    lines: &[ReceiptLineBasis],
    method: &str,
) -> Result<Vec<AcquisitionCostAllocation>, SplitError> {
    let shares = allocation_basis(lines, method)?;
    let amounts = largest_remainder(amount_minor, &shares)?;
    let in_base = currency == base_currency;
    let bases = if in_base {
        amounts.clone()
    } else {
        largest_remainder(base_total, &shares)?
    };
    let mut allocations = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        allocations.push(AcquisitionCostAllocation {
            id: Uuid::now_v7(),
            acquisition_cost_id: cost_id,
            receipt_line_id: line.receipt_line_id,
            allocated_minor: amounts[index],
            allocated_base_minor: if in_base { None } else { Some(bases[index]) },
            // Every basis comes from an i64 (a line amount, a quantity or a
            // weight), so the cast cannot lose a unit; the refusal keeps that a
            // fact rather than a silent saturation.
            basis_value: i64::try_from(shares[index].1).map_err(|_| SplitError::Unbalanced)?,
        });
    }
    Ok(allocations)
}

// --- writes ------------------------------------------------------------------

const COST_COLS: &str = "id, org_id, purchase_order_id, receipt_id, kind, vendor_id, \
     vendor_reference, reference, document_date, currency, amount_minor, \
     allocation_method, allocation_rule, policy_mode, capitalised, reporting_class, \
     project_id, ticket_id, client_id, posted_ticket_expense_id, \
     reverses_acquisition_cost_id, \
     fx_rate_used::text AS fx_rate_used, fx_rate_date, fx_source, base_amount_minor, \
     created_by, created_at";

fn cost_row(row: sqlx::postgres::PgRow) -> AcquisitionCost {
    AcquisitionCost {
        id: row.get("id"),
        org_id: row.get("org_id"),
        purchase_order_id: row.get("purchase_order_id"),
        receipt_id: row.get("receipt_id"),
        kind: row.get("kind"),
        vendor_id: row.get("vendor_id"),
        vendor_reference: row.get("vendor_reference"),
        reference: row.get("reference"),
        document_date: row.get("document_date"),
        currency: row.get("currency"),
        amount_minor: row.get("amount_minor"),
        allocation_method: row.get("allocation_method"),
        allocation_rule: row.get("allocation_rule"),
        policy_mode: row.get("policy_mode"),
        capitalised: row.get("capitalised"),
        reporting_class: row.get("reporting_class"),
        project_id: row.get("project_id"),
        ticket_id: row.get("ticket_id"),
        client_id: row.get("client_id"),
        posted_ticket_expense_id: row.get("posted_ticket_expense_id"),
        reverses_acquisition_cost_id: row.get("reverses_acquisition_cost_id"),
        fx_rate_used: row.get("fx_rate_used"),
        fx_rate_date: row.get("fx_rate_date"),
        fx_source: row.get("fx_source"),
        base_amount_minor: row.get("base_amount_minor"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
    }
}

pub async fn insert_cost(conn: &mut PgConnection, cost: &AcquisitionCost) -> DbResult<()> {
    sqlx::query(
        r#"INSERT INTO acquisition_costs
             (id, org_id, purchase_order_id, receipt_id, kind, vendor_id,
              vendor_reference, reference, document_date, currency, amount_minor,
              allocation_method, allocation_rule, policy_mode, capitalised,
              reporting_class, project_id, ticket_id, client_id,
              posted_ticket_expense_id, reverses_acquisition_cost_id,
              fx_rate_used, fx_rate_date, fx_source, base_amount_minor, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                   $15, $16, $17, $18, $19, $20, $21::text::numeric, $22, $23, $24,
                   $25, $26)"#,
    )
    .bind(cost.id)
    .bind(cost.org_id)
    .bind(cost.purchase_order_id)
    .bind(cost.receipt_id)
    .bind(&cost.kind)
    .bind(cost.vendor_id)
    .bind(cost.vendor_reference.as_deref())
    .bind(&cost.reference)
    .bind(cost.document_date)
    .bind(&cost.currency)
    .bind(cost.amount_minor)
    .bind(&cost.allocation_method)
    .bind(&cost.allocation_rule)
    .bind(&cost.policy_mode)
    .bind(cost.capitalised)
    .bind(&cost.reporting_class)
    .bind(cost.project_id)
    .bind(cost.ticket_id)
    .bind(cost.client_id)
    .bind(cost.posted_ticket_expense_id)
    .bind(cost.reverses_acquisition_cost_id)
    .bind(cost.fx_rate_used.as_deref())
    .bind(cost.fx_rate_date)
    .bind(cost.fx_source.as_deref())
    .bind(cost.base_amount_minor)
    .bind(cost.created_by)
    .execute(&mut *conn)
    .await
    .map_err(guard_error)?;
    Ok(())
}

/// Inserts a whole split in one statement, which is what the migration's
/// statement-level balance guard expects: a partial set would be refused.
pub async fn insert_allocations(
    conn: &mut PgConnection,
    org_id: Uuid,
    cost_id: Uuid,
    allocations: &[AcquisitionCostAllocation],
) -> DbResult<()> {
    if allocations.is_empty() {
        return Err(DbError::Conflict(
            "a layer needs a split; an empty allocation set is a writer bug".into(),
        ));
    }
    let ids: Vec<Uuid> = allocations.iter().map(|a| a.id).collect();
    let line_ids: Vec<Uuid> = allocations.iter().map(|a| a.receipt_line_id).collect();
    let amounts: Vec<i64> = allocations.iter().map(|a| a.allocated_minor).collect();
    let bases: Vec<Option<i64>> = allocations.iter().map(|a| a.allocated_base_minor).collect();
    let basis_values: Vec<i64> = allocations.iter().map(|a| a.basis_value).collect();
    sqlx::query(
        r#"INSERT INTO acquisition_cost_allocations
             (id, org_id, acquisition_cost_id, receipt_line_id, allocated_minor,
              allocated_base_minor, basis_value)
           SELECT share.id, $2, $3, share.receipt_line_id, share.allocated_minor,
                  share.allocated_base_minor, share.basis_value
           FROM UNNEST($1::uuid[], $4::uuid[], $5::bigint[], $6::bigint[], $7::bigint[])
             AS share(id, receipt_line_id, allocated_minor, allocated_base_minor,
                      basis_value)"#,
    )
    .bind(&ids)
    .bind(org_id)
    .bind(cost_id)
    .bind(&line_ids)
    .bind(&amounts)
    .bind(&bases)
    .bind(&basis_values)
    .execute(&mut *conn)
    .await
    .map_err(guard_error)?;
    Ok(())
}

/// Posts a classed ticket expense. The class is the layer's; `billable` is
/// false, because a cost we incurred is not a recharge the client agreed to
/// pay — a recharge is a separate, priced document (plan §5).
pub async fn insert_classed_expense(
    conn: &mut PgConnection,
    expense: &TicketExpense,
    reporting_class: &str,
) -> DbResult<()> {
    sqlx::query(
        r#"INSERT INTO ticket_expenses
             (id, org_id, ticket_id, description, amount_minor, currency,
              billable, fx_rate_used, fx_rate_date, fx_source, base_amount_minor,
              incurred_by, incurred_at, reporting_class)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8::text::numeric, $9, $10, $11,
                   $12, $13, $14)"#,
    )
    .bind(expense.id)
    .bind(expense.org_id)
    .bind(expense.ticket_id)
    .bind(&expense.description)
    .bind(expense.amount_minor)
    .bind(&expense.currency)
    .bind(expense.billable)
    .bind(expense.fx_rate_used.as_deref())
    .bind(expense.fx_rate_date)
    .bind(expense.fx_source.as_deref())
    .bind(expense.base_amount_minor)
    .bind(expense.incurred_by)
    .bind(expense.incurred_at)
    .bind(reporting_class)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

// --- reads -------------------------------------------------------------------

pub async fn get_cost(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<AcquisitionCost> {
    let sql = format!("SELECT {COST_COLS} FROM acquisition_costs WHERE org_id = $1 AND id = $2");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(cost_row(row))
}

pub async fn list_costs_for_receipt(
    conn: &mut PgConnection,
    org_id: Uuid,
    receipt_id: Uuid,
) -> DbResult<Vec<AcquisitionCost>> {
    let sql = format!(
        "SELECT {COST_COLS} FROM acquisition_costs \
         WHERE org_id = $1 AND receipt_id = $2 ORDER BY created_at, id"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(receipt_id)
        .fetch_all(&mut *conn)
        .await?;
    Ok(rows.into_iter().map(cost_row).collect())
}

pub async fn list_allocations(
    conn: &mut PgConnection,
    org_id: Uuid,
    cost_id: Uuid,
) -> DbResult<Vec<AcquisitionCostAllocation>> {
    let rows = sqlx::query(
        r#"SELECT id, acquisition_cost_id, receipt_line_id, allocated_minor,
                  allocated_base_minor, basis_value
           FROM acquisition_cost_allocations
           WHERE org_id = $1 AND acquisition_cost_id = $2
           ORDER BY allocated_minor DESC, receipt_line_id"#,
    )
    .bind(org_id)
    .bind(cost_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| AcquisitionCostAllocation {
            id: row.get("id"),
            acquisition_cost_id: row.get("acquisition_cost_id"),
            receipt_line_id: row.get("receipt_line_id"),
            allocated_minor: row.get("allocated_minor"),
            allocated_base_minor: row.get("allocated_base_minor"),
            basis_value: row.get("basis_value"),
        })
        .collect())
}

pub async fn detail(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<AcquisitionCostDetail> {
    let cost = get_cost(&mut *conn, org_id, id).await?;
    let allocations = list_allocations(&mut *conn, org_id, id).await?;
    Ok(AcquisitionCostDetail { cost, allocations })
}

/// Every layer on a receipt with its split, in two reads rather than one per
/// layer.
pub async fn details_for_receipt(
    conn: &mut PgConnection,
    org_id: Uuid,
    receipt_id: Uuid,
) -> DbResult<Vec<AcquisitionCostDetail>> {
    let costs = list_costs_for_receipt(&mut *conn, org_id, receipt_id).await?;
    if costs.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        r#"SELECT a.id, a.acquisition_cost_id, a.receipt_line_id, a.allocated_minor,
                  a.allocated_base_minor, a.basis_value
           FROM acquisition_cost_allocations a
           JOIN acquisition_costs c
             ON c.org_id = a.org_id AND c.id = a.acquisition_cost_id
           WHERE a.org_id = $1 AND c.receipt_id = $2
           ORDER BY a.allocated_minor DESC, a.receipt_line_id"#,
    )
    .bind(org_id)
    .bind(receipt_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut by_cost: HashMap<Uuid, Vec<AcquisitionCostAllocation>> = HashMap::new();
    for row in rows {
        by_cost
            .entry(row.get("acquisition_cost_id"))
            .or_default()
            .push(AcquisitionCostAllocation {
                id: row.get("id"),
                acquisition_cost_id: row.get("acquisition_cost_id"),
                receipt_line_id: row.get("receipt_line_id"),
                allocated_minor: row.get("allocated_minor"),
                allocated_base_minor: row.get("allocated_base_minor"),
                basis_value: row.get("basis_value"),
            });
    }
    Ok(costs
        .into_iter()
        .map(|cost| AcquisitionCostDetail {
            allocations: by_cost.remove(&cost.id).unwrap_or_default(),
            cost,
        })
        .collect())
}

/// Whether this layer has already been reversed. A second reversal is a typed
/// conflict, and the unique index is what holds under a race.
pub async fn has_been_reversed(conn: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<bool> {
    let exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
             SELECT 1 FROM acquisition_costs
             WHERE org_id = $1 AND reverses_acquisition_cost_id = $2
           )"#,
    )
    .bind(org_id)
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(exists)
}

/// Whether a vendor belongs to the org. The layer cites the vendor without a
/// foreign key, so the write path proves the id is in scope before it stores it
/// — a foreign vendor is the same 404 as a missing one.
pub async fn vendor_exists(
    conn: &mut PgConnection,
    org_id: Uuid,
    vendor_id: Uuid,
) -> DbResult<bool> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM vendors WHERE org_id = $1 AND id = $2)")
            .bind(org_id)
            .bind(vendor_id)
            .fetch_one(&mut *conn)
            .await?;
    Ok(exists)
}

// --- the valuation read model ------------------------------------------------

/// One receipt line's cost, and the layers that moved it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReceiptLineValuation {
    pub receipt_line_id: Uuid,
    pub po_line_id: Uuid,
    pub description: String,
    pub qty_milli: i64,
    pub destination_class: Option<String>,
    /// The receipt cost in the currency it was recorded in.
    pub receipt_unit_cost_minor: i64,
    pub currency: String,
    /// The line's receipt cost in base currency. `None` for a lot or service
    /// line recorded in a foreign currency, where no receipt leg locked a rate:
    /// the cost is known in the order currency and unvalued in base until a
    /// layer or a receipt leg gives it one. Missing cost is never zero.
    pub receipt_cost_base_minor: Option<i64>,
    /// Allocated capitalised layers, which are part of the basis.
    pub capitalised_base_minor: i64,
    /// Allocated layers that were expensed under their class. Reported so the
    /// total layer money on a receipt reconciles; never added to the basis.
    pub expensed_base_minor: i64,
    /// Receipt cost plus capitalised layers. `None` when the receipt cost is
    /// unknown in base currency.
    pub basis_total_base_minor: Option<i64>,
    pub allocated_layer_count: i64,
}

/// A receipt's valuation: every line's basis and the totals that prove the two
/// halves agree.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReceiptValuation {
    pub receipt_id: Uuid,
    pub purchase_order_id: Uuid,
    pub currency: String,
    pub base_currency: String,
    pub lines: Vec<ReceiptLineValuation>,
    /// Sum of the lines' receipt cost in base currency.
    pub receipt_cost_base_minor: i64,
    /// Sum of the capitalised allocations.
    pub capitalised_base_minor: i64,
    /// Sum of the expensed allocations.
    pub expensed_base_minor: i64,
    /// Receipt cost plus capitalised layers: the value of the units held.
    pub basis_total_base_minor: i64,
    /// Sum of the capitalised LAYER amounts on this receipt. Equal to
    /// `capitalised_base_minor`; both are carried so a reader (and the
    /// acceptance query) can see the two halves meet instead of trusting one.
    pub capitalised_layer_total_base_minor: i64,
    /// Sum of the expensed layer amounts on this receipt.
    pub expensed_layer_total_base_minor: i64,
    /// Lines whose receipt cost is not known in base currency. Never silently
    /// zeroed.
    pub uncosted_line_count: i64,
}

/// The line's receipt cost per unit in base currency, when the receipt leg
/// recorded one.
fn receipt_unit_base(line: &ReceiptLineBasis) -> Option<i64> {
    if line.movement_id.is_some() {
        return line.movement_base_cost_minor.or(line.movement_cost_minor);
    }
    if line.device_asset_id.is_some() {
        return line.purchase_base_cost_minor.or(line.purchase_cost_minor);
    }
    None
}

/// Reads one receipt's valuation inside the org. Every amount is taken from a
/// stored snapshot: the FX rate book is never consulted, so the answer does not
/// move when a newer rate is recorded.
pub async fn receipt_valuation(
    conn: &mut PgConnection,
    org_id: Uuid,
    receipt_id: Uuid,
) -> DbResult<ReceiptValuation> {
    let context = receipt_context(&mut *conn, org_id, receipt_id).await?;
    let base_currency = crate::fx::base_currency(&mut *conn, org_id).await?;

    // One read for the whole receipt's allocations, one for the layer totals.
    // No per-line round trip.
    let allocation_rows = sqlx::query(
        r#"SELECT a.receipt_line_id, a.allocated_minor, a.allocated_base_minor,
                  c.capitalised
           FROM acquisition_cost_allocations a
           JOIN acquisition_costs c
             ON c.org_id = a.org_id AND c.id = a.acquisition_cost_id
           WHERE a.org_id = $1 AND c.receipt_id = $2"#,
    )
    .bind(org_id)
    .bind(receipt_id)
    .fetch_all(&mut *conn)
    .await?;

    struct Share {
        capitalised: i64,
        expensed: i64,
        count: i64,
    }
    let mut shares: HashMap<Uuid, Share> = HashMap::new();
    for row in allocation_rows {
        let base: i64 = match row.get::<Option<i64>, _>("allocated_base_minor") {
            Some(value) => value,
            None => row.get("allocated_minor"),
        };
        let capitalised: bool = row.get("capitalised");
        let entry = shares.entry(row.get("receipt_line_id")).or_insert(Share {
            capitalised: 0,
            expensed: 0,
            count: 0,
        });
        if capitalised {
            entry.capitalised += base;
        } else {
            entry.expensed += base;
        }
        entry.count += 1;
    }

    let totals = sqlx::query(
        r#"SELECT
             COALESCE(SUM(COALESCE(base_amount_minor, amount_minor))
                      FILTER (WHERE capitalised), 0)::bigint AS capitalised_total,
             COALESCE(SUM(COALESCE(base_amount_minor, amount_minor))
                      FILTER (WHERE NOT capitalised), 0)::bigint AS expensed_total
           FROM acquisition_costs
           WHERE org_id = $1 AND receipt_id = $2"#,
    )
    .bind(org_id)
    .bind(receipt_id)
    .fetch_one(&mut *conn)
    .await?;

    let mut lines = Vec::with_capacity(context.lines.len());
    let mut receipt_cost_total: i64 = 0;
    let mut capitalised_total: i64 = 0;
    let mut expensed_total: i64 = 0;
    let mut basis_total: i64 = 0;
    let mut uncosted = 0i64;

    for line in &context.lines {
        let unit_base = receipt_unit_base(line);
        let currency = context.order_currency.clone();
        let receipt_cost_base = match unit_base {
            Some(unit) => Some(
                core_domain::entities::line_amount_minor(line.qty_milli, unit)
                    .map_err(|e| DbError::Corrupt(format!("receipt line cost: {e}")))?,
            ),
            None if context.order_currency == base_currency => Some(
                core_domain::entities::line_amount_minor(line.qty_milli, receipt_unit_cost(line))
                    .map_err(|e| DbError::Corrupt(format!("receipt line cost: {e}")))?,
            ),
            None => None,
        };
        let share = shares.get(&line.receipt_line_id);
        let capitalised_line = share.map(|s| s.capitalised).unwrap_or(0);
        let expensed_line = share.map(|s| s.expensed).unwrap_or(0);
        let basis = match receipt_cost_base {
            Some(cost) => Some(cost + capitalised_line),
            None => {
                uncosted += 1;
                None
            }
        };
        if let Some(cost) = receipt_cost_base {
            receipt_cost_total += cost;
        }
        if let Some(total) = basis {
            basis_total += total;
        }
        capitalised_total += capitalised_line;
        expensed_total += expensed_line;
        lines.push(ReceiptLineValuation {
            receipt_line_id: line.receipt_line_id,
            po_line_id: line.po_line_id,
            description: line.description.clone(),
            qty_milli: line.qty_milli,
            destination_class: line.destination_class.clone(),
            receipt_unit_cost_minor: receipt_unit_cost(line),
            currency,
            receipt_cost_base_minor: receipt_cost_base,
            capitalised_base_minor: capitalised_line,
            expensed_base_minor: expensed_line,
            basis_total_base_minor: basis,
            allocated_layer_count: share.map(|s| s.count).unwrap_or(0),
        });
    }

    Ok(ReceiptValuation {
        receipt_id,
        purchase_order_id: context.purchase_order_id,
        currency: context.order_currency,
        base_currency,
        lines,
        receipt_cost_base_minor: receipt_cost_total,
        capitalised_base_minor: capitalised_total,
        expensed_base_minor: expensed_total,
        basis_total_base_minor: basis_total,
        capitalised_layer_total_base_minor: totals.get("capitalised_total"),
        expensed_layer_total_base_minor: totals.get("expensed_total"),
        uncosted_line_count: uncosted,
    })
}

// --- the reporting-class rollup ----------------------------------------------

/// Which layers a rollup counts, and for whom.
pub struct RollupFilter {
    pub purchase_order_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub ticket_id: Option<Uuid>,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    /// The caller. A layer whose client is restricted and does not name them is
    /// left out of the totals, the same rule the ticket lists apply in SQL.
    pub viewer_id: Uuid,
    pub viewer_is_admin: bool,
}

/// One reporting class's total on documents dated inside the window.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CostClassTotal {
    pub reporting_class: String,
    pub layer_count: i64,
    pub capitalised_count: i64,
    /// Base-currency total, signed: a reversal reduces its class.
    pub base_amount_minor: i64,
}

/// Sums layers by reporting class. A reversal is its own layer with a negative
/// amount, so the class total is the net of the charges and the corrections
/// without either being rewritten.
pub async fn class_rollup(
    conn: &mut PgConnection,
    org_id: Uuid,
    filter: &RollupFilter,
) -> DbResult<Vec<CostClassTotal>> {
    let rows = sqlx::query(
        r#"SELECT a.reporting_class,
                  COUNT(*)::bigint AS layer_count,
                  COUNT(*) FILTER (WHERE a.capitalised)::bigint AS capitalised_count,
                  COALESCE(SUM(COALESCE(a.base_amount_minor, a.amount_minor)), 0)::bigint
                    AS base_amount_minor
           FROM acquisition_costs a
           WHERE a.org_id = $1
             AND ($2::uuid IS NULL OR a.purchase_order_id = $2)
             AND ($3::uuid IS NULL OR a.project_id = $3)
             AND ($4::uuid IS NULL OR a.ticket_id = $4)
             AND ($5::date IS NULL OR a.document_date >= $5)
             AND ($6::date IS NULL OR a.document_date <= $6)
             AND NOT EXISTS (
               SELECT 1 FROM clients c
                WHERE c.org_id = a.org_id AND c.id = a.client_id
                  AND jsonb_array_length(c.restricted_to) > 0
                  AND NOT (c.restricted_to ? $7::text)
                  AND NOT $8)
           GROUP BY a.reporting_class
           ORDER BY a.reporting_class"#,
    )
    .bind(org_id)
    .bind(filter.purchase_order_id)
    .bind(filter.project_id)
    .bind(filter.ticket_id)
    .bind(filter.from)
    .bind(filter.to)
    .bind(filter.viewer_id)
    .bind(filter.viewer_is_admin)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| CostClassTotal {
            reporting_class: row.get("reporting_class"),
            layer_count: row.get("layer_count"),
            capitalised_count: row.get("capitalised_count"),
            base_amount_minor: row.get("base_amount_minor"),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(
        id: Uuid,
        qty_milli: i64,
        unit_cost_minor: i64,
        weight_g: Option<i32>,
    ) -> ReceiptLineBasis {
        ReceiptLineBasis {
            receipt_line_id: id,
            po_line_id: Uuid::now_v7(),
            description: "part".into(),
            qty_milli,
            destination_class: Some("sale_stock".into()),
            unit_cost_minor,
            weight_g,
            movement_id: Some(Uuid::now_v7()),
            movement_cost_minor: Some(unit_cost_minor),
            movement_base_cost_minor: None,
            device_asset_id: None,
            purchase_cost_minor: None,
            purchase_base_cost_minor: None,
        }
    }

    #[test]
    fn largest_remainder_sums_exactly_and_breaks_ties_by_id() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let c = Uuid::from_u128(3);
        let out = largest_remainder(100, &[(a, 1), (b, 1), (c, 1)]).expect("splits");
        assert_eq!(out.iter().sum::<i64>(), 100);
        // 33.33 each: the residue goes to the two lowest ids.
        assert_eq!(out, vec![34, 33, 33]);

        // Reversing the input order must not move the residue.
        let out_reversed = largest_remainder(100, &[(c, 1), (b, 1), (a, 1)]).expect("splits");
        assert_eq!(out_reversed, vec![33, 33, 34]);
    }

    #[test]
    fn largest_remainder_handles_a_total_smaller_than_the_line_count() {
        let shares: Vec<(Uuid, i128)> = (0..5).map(|i| (Uuid::from_u128(i + 1), 1)).collect();
        let out = largest_remainder(3, &shares).expect("splits");
        assert_eq!(out.iter().sum::<i64>(), 3);
        assert_eq!(out.iter().filter(|v| **v == 1).count(), 3);
    }

    #[test]
    fn largest_remainder_refuses_a_zero_basis() {
        let a = Uuid::now_v7();
        assert_eq!(
            largest_remainder(100, &[(a, 0)]),
            Err(SplitError::EmptyBasis("basis"))
        );
    }

    #[test]
    fn weight_allocation_multiplies_grams_by_quantity() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        // 1 unit at 900g vs 3 units at 100g: 900 vs 300, so 750/250.
        let lines = vec![line(a, 1000, 100, Some(900)), line(b, 3000, 100, Some(100))];
        let shares = allocation_basis(&lines, "weight").expect("basis");
        assert_eq!(shares, vec![(a, 900), (b, 300)]);
        let split =
            split_layer(Uuid::now_v7(), "USD", 1000, 1000, "USD", &lines, "weight").expect("split");
        assert_eq!(split[0].allocated_minor, 750);
        assert_eq!(split[1].allocated_minor, 250);
    }

    #[test]
    fn weight_allocation_names_the_line_without_weight() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let lines = vec![line(a, 1000, 100, Some(900)), line(b, 1000, 100, None)];
        match allocation_basis(&lines, "weight") {
            Err(SplitError::MissingWeight {
                receipt_line_id, ..
            }) => {
                assert_eq!(receipt_line_id, b)
            }
            other => panic!("expected MissingWeight, got {other:?}"),
        }
    }

    #[test]
    fn extended_value_is_the_line_amount() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let lines = vec![line(a, 2000, 1500, None), line(b, 1000, 1000, None)];
        let shares = allocation_basis(&lines, "extended_value").expect("basis");
        assert_eq!(shares, vec![(a, 3000), (b, 1000)]);
        let split = split_layer(
            Uuid::now_v7(),
            "USD",
            401,
            401,
            "USD",
            &lines,
            "extended_value",
        )
        .expect("split");
        assert_eq!(split[0].allocated_minor, 301);
        assert_eq!(split[1].allocated_minor, 100);
        assert_eq!(split.iter().map(|s| s.allocated_minor).sum::<i64>(), 401);
    }

    #[test]
    fn cross_currency_split_sums_both_sides() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let lines = vec![line(a, 1000, 1000, None), line(b, 1000, 2000, None)];
        let split = split_layer(
            Uuid::now_v7(),
            "EUR",
            100,
            87,
            "USD",
            &lines,
            "extended_value",
        )
        .expect("split");
        assert_eq!(split.iter().map(|s| s.allocated_minor).sum::<i64>(), 100);
        assert_eq!(
            split
                .iter()
                .map(|s| s.allocated_base_minor.expect("base"))
                .sum::<i64>(),
            87
        );
    }

    #[test]
    fn class_defaults_follow_the_kind() {
        assert_eq!(default_reporting_class("freight"), "freight_in");
        assert_eq!(default_reporting_class("duty"), "duty");
        assert_eq!(default_reporting_class("brokerage"), "service_cost");
        assert_eq!(default_reporting_class("insurance"), "service_cost");
        assert_eq!(default_reporting_class("handling"), "service_cost");
        assert!(is_valid_kind("freight"));
        assert!(!is_valid_kind("rent"));
        assert!(is_valid_allocation_method("weight"));
        assert!(!is_valid_allocation_method("manual"));
    }
}
