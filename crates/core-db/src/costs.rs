//! Ticket costs & expenses (Wave 11 lane 9, migration 0056): per-ticket
//! money lines and the invoicing-candidate feed.
//!
//! Purpose: persistence for the cost construct — cost-line CRUD (parts /
//! flat / other), expense entries, the billable flag, the consumption
//! marker (`invoice_line_id` = "billed once, ever"), and the candidate
//! feed the invoicing layer queries.
//!
//! Non-purpose: no invoice-line writing (Wave 9 invoicing owns its
//! schema; consumption marks through the twin columns), no currency
//! conversion, no tax math.

use crate::{DbError, DbResult};
use core_domain::entities::{CostLineKind, TicketCostLine, TicketExpense};
use sqlx::Postgres;
use uuid::Uuid;

const COST_COLS: &str = "id, org_id, ticket_id, kind, description, quantity, unit_cost_minor, currency, billable, fx_rate_used::text AS fx_rate_used, fx_rate_date, fx_source, base_amount_minor, invoice_line_id, created_by, created_at, updated_at";
const EXPENSE_COLS: &str = "id, org_id, ticket_id, description, amount_minor, currency, billable, fx_rate_used::text AS fx_rate_used, fx_rate_date, fx_source, base_amount_minor, invoice_line_id, incurred_by, incurred_at, created_at, updated_at";

fn cost_line_from(row: sqlx::postgres::PgRow) -> DbResult<TicketCostLine> {
    use sqlx::Row;
    let kind: String = row.get("kind");
    Ok(TicketCostLine {
        id: row.get("id"),
        org_id: row.get("org_id"),
        ticket_id: row.get("ticket_id"),
        kind: CostLineKind::parse(&kind).ok_or(DbError::NotFound)?,
        description: row.get("description"),
        quantity: row.get("quantity"),
        unit_cost_minor: row.get("unit_cost_minor"),
        currency: row.get("currency"),
        billable: row.get("billable"),
        fx_rate_used: row.get("fx_rate_used"),
        fx_rate_date: row.get("fx_rate_date"),
        fx_source: row.get("fx_source"),
        base_amount_minor: row.get("base_amount_minor"),
        invoice_line_id: row.get("invoice_line_id"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn expense_from(row: sqlx::postgres::PgRow) -> DbResult<TicketExpense> {
    use sqlx::Row;
    Ok(TicketExpense {
        id: row.get("id"),
        org_id: row.get("org_id"),
        ticket_id: row.get("ticket_id"),
        description: row.get("description"),
        amount_minor: row.get("amount_minor"),
        currency: row.get("currency"),
        billable: row.get("billable"),
        fx_rate_used: row.get("fx_rate_used"),
        fx_rate_date: row.get("fx_rate_date"),
        fx_source: row.get("fx_source"),
        base_amount_minor: row.get("base_amount_minor"),
        invoice_line_id: row.get("invoice_line_id"),
        incurred_by: row.get("incurred_by"),
        incurred_at: row.get("incurred_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub async fn list_cost_lines<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<Vec<TicketCostLine>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {COST_COLS} FROM ticket_cost_lines \
         WHERE org_id = $1 AND ticket_id = $2 ORDER BY created_at"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(ticket_id)
        .fetch_all(executor)
        .await?;
    rows.into_iter().map(cost_line_from).collect()
}

pub async fn insert_cost_line<'e, E>(executor: E, line: &TicketCostLine) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO ticket_cost_lines
             (id, org_id, ticket_id, kind, description, quantity, unit_cost_minor,
              currency, billable, fx_rate_used, fx_rate_date, fx_source,
              base_amount_minor, created_by)
           VALUES ($1, $2, $3, $4, $5, $6::double precision, $7, $8, $9, $10::text::numeric, $11, $12, $13, $14)"#,
        line.id,
        line.org_id,
        line.ticket_id,
        line.kind.as_str(),
        line.description,
        line.quantity,
        line.unit_cost_minor,
        line.currency,
        line.billable,
        line.fx_rate_used,
        line.fx_rate_date,
        line.fx_source,
        line.base_amount_minor,
        line.created_by
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Patch editable fields; `quantity`/`unit_cost`/`billable` ride
/// Option<Option>. Billed lines (`invoice_line_id` set) refuse any
/// mutation — "billed once, ever" is enforced at the API surface too, but
/// the DB guard keeps a stale client honest. Returns NotFound when the
/// line is not in scope, `DbError::Conflict` when it is already billed.
pub async fn update_cost_line(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    description: Option<&str>,
    quantity: Option<f64>,
    unit_cost_minor: Option<i64>,
    billable: Option<bool>,
) -> DbResult<()> {
    let billed = sqlx::query_scalar!(
        r#"SELECT (invoice_line_id IS NOT NULL) AS billed FROM ticket_cost_lines
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(&mut *executor)
    .await?
    .ok_or(DbError::NotFound)?;
    if billed.unwrap_or(false) {
        return Err(DbError::Conflict("cost line already billed".into()));
    }
    let res = sqlx::query!(
        r#"UPDATE ticket_cost_lines SET
             description = COALESCE($3, description),
             quantity = COALESCE($4::double precision, quantity),
             unit_cost_minor = COALESCE($5, unit_cost_minor),
             billable = COALESCE($6, billable),
             updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
        description,
        quantity,
        unit_cost_minor,
        billable
    )
    .execute(&mut *executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

pub async fn remove_cost_line(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<bool> {
    let billed = sqlx::query_scalar!(
        r#"SELECT (invoice_line_id IS NOT NULL) AS billed FROM ticket_cost_lines
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(&mut *executor)
    .await?
    .ok_or(DbError::NotFound)?;
    if billed.unwrap_or(false) {
        return Err(DbError::Conflict("cost line already billed".into()));
    }
    let res = sqlx::query!(
        "DELETE FROM ticket_cost_lines WHERE org_id = $1 AND id = $2",
        org_id,
        id
    )
    .execute(&mut *executor)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn list_expenses<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<Vec<TicketExpense>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {EXPENSE_COLS} FROM ticket_expenses \
         WHERE org_id = $1 AND ticket_id = $2 ORDER BY incurred_at"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(ticket_id)
        .fetch_all(executor)
        .await?;
    rows.into_iter().map(expense_from).collect()
}

/// Wave 32 lane 7: reconciliation reads linked expenses by id in one
/// round trip (draft.expense_id → the reimbursement amount).
pub async fn list_expenses_by_ids<'e, E>(
    executor: E,
    org_id: Uuid,
    ids: &[Uuid],
) -> DbResult<Vec<TicketExpense>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT {EXPENSE_COLS} FROM ticket_expenses \
         WHERE org_id = $1 AND id = ANY($2) ORDER BY incurred_at"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(ids)
        .fetch_all(executor)
        .await?;
    rows.into_iter().map(expense_from).collect()
}

pub async fn insert_expense<'e, E>(executor: E, e: &TicketExpense) -> DbResult<()>
where    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO ticket_expenses
             (id, org_id, ticket_id, description, amount_minor, currency,
              billable, fx_rate_used, fx_rate_date, fx_source, base_amount_minor,
              incurred_by, incurred_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8::text::numeric, $9, $10, $11, $12, $13)"#,
        e.id,
        e.org_id,
        e.ticket_id,
        e.description,
        e.amount_minor,
        e.currency,
        e.billable,
        e.fx_rate_used,
        e.fx_rate_date,
        e.fx_source,
        e.base_amount_minor,
        e.incurred_by,
        e.incurred_at
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn update_expense(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    description: Option<&str>,
    amount_minor: Option<i64>,
    billable: Option<bool>,
) -> DbResult<()> {
    let billed = sqlx::query_scalar!(
        r#"SELECT (invoice_line_id IS NOT NULL) AS billed FROM ticket_expenses
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(&mut *executor)
    .await?
    .ok_or(DbError::NotFound)?;
    if billed.unwrap_or(false) {
        return Err(DbError::Conflict("expense already billed".into()));
    }
    let res = sqlx::query!(
        r#"UPDATE ticket_expenses SET
             description = COALESCE($3, description),
             amount_minor = COALESCE($4, amount_minor),
             billable = COALESCE($5, billable),
             updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
        description,
        amount_minor,
        billable
    )
    .execute(&mut *executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

pub async fn remove_expense(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<bool> {
    let billed = sqlx::query_scalar!(
        r#"SELECT (invoice_line_id IS NOT NULL) AS billed FROM ticket_expenses
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(&mut *executor)
    .await?
    .ok_or(DbError::NotFound)?;
    if billed.unwrap_or(false) {
        return Err(DbError::Conflict("expense already billed".into()));
    }
    let res = sqlx::query!(
        "DELETE FROM ticket_expenses WHERE org_id = $1 AND id = $2",
        org_id,
        id
    )
    .execute(&mut *executor)
    .await?;
    Ok(res.rows_affected() > 0)
}

// --- invoicing candidate feed ---------------------------------------------------------

/// One billable-unbilled candidate line for an invoice run's T&M-style
/// sweep. `source` distinguishes the construct that produced it.
#[derive(Debug, Clone)]
pub struct InvoiceCandidateLine {
    pub ticket_id: Uuid,
    pub client_id: Option<Uuid>,
    pub source: &'static str,
    pub line_id: Uuid,
    pub description: String,
    pub amount_minor: i64,
    pub currency: String,
}

/// Every billable, not-yet-billed cost line + expense in the org,
/// oldest first — the invoicing candidate feed. Consumption writes
/// `invoice_line_id` via [`mark_candidates_billed`].
pub async fn list_invoice_candidates(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<InvoiceCandidateLine>> {
    // Two straight selects instead of one UNION ALL: sqlx's compile-time
    // checker conservatively NULLs every UNION output column, and the
    // candidate contract is non-nullable by construction.
    let cost_rows = sqlx::query!(
        r#"SELECT t.id AS ticket_id, t.client_id, c.id AS line_id, c.description,
                  round(c.quantity * c.unit_cost_minor)::bigint AS amount_minor, c.currency
           FROM ticket_cost_lines c JOIN tickets t ON t.org_id = c.org_id AND t.id = c.ticket_id
           WHERE c.org_id = $1 AND c.billable AND c.invoice_line_id IS NULL"#,
        org_id
    )
    .fetch_all(&mut *executor)
    .await?;
    let expense_rows = sqlx::query!(
        r#"SELECT t.id AS ticket_id, t.client_id, e.id AS line_id, e.description,
                  e.amount_minor, e.currency
           FROM ticket_expenses e JOIN tickets t ON t.org_id = e.org_id AND t.id = e.ticket_id
           WHERE e.org_id = $1 AND e.billable AND e.invoice_line_id IS NULL"#,
        org_id
    )
    .fetch_all(&mut *executor)
    .await?;
    let mut out: Vec<InvoiceCandidateLine> = cost_rows
        .into_iter()
        .map(|r| InvoiceCandidateLine {
            ticket_id: r.ticket_id,
            client_id: r.client_id,
            source: "cost",
            line_id: r.line_id,
            description: r.description,
            amount_minor: r.amount_minor.unwrap_or(0),
            currency: r.currency,
        })
        .collect();
    out.extend(expense_rows.into_iter().map(|r| InvoiceCandidateLine {
        ticket_id: r.ticket_id,
        client_id: r.client_id,
        source: "expense",
        line_id: r.line_id,
        description: r.description,
        amount_minor: r.amount_minor,
        currency: r.currency,
    }));
    Ok(out)
}

/// Marks the named candidate lines consumed by one invoice line id
/// (cost lines + expenses share the marker shape). Only unbilled rows
/// update — re-runs are inert, not double-marked.
pub async fn mark_candidates_billed(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    cost_line_ids: &[Uuid],
    expense_ids: &[Uuid],
    invoice_line_id: Uuid,
) -> DbResult<()> {
    if !cost_line_ids.is_empty() {
        sqlx::query!(
            r#"UPDATE ticket_cost_lines SET invoice_line_id = $3, updated_at = now()
               WHERE org_id = $1 AND invoice_line_id IS NULL AND id = ANY($2)"#,
            org_id,
            cost_line_ids,
            invoice_line_id
        )
        .execute(&mut *executor)
        .await?;
    }
    if !expense_ids.is_empty() {
        sqlx::query!(
            r#"UPDATE ticket_expenses SET invoice_line_id = $3, updated_at = now()
               WHERE org_id = $1 AND invoice_line_id IS NULL AND id = ANY($2)"#,
            org_id,
            expense_ids,
            invoice_line_id
        )
        .execute(&mut *executor)
        .await?;
    }
    Ok(())
}
