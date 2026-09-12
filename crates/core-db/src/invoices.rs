//! Native invoicing v1 (TASKLIST Phase 3, Wave 9): invoice runs,
//! invoices + lines, time-entry consumption links, and payments over
//! migration 0036's tables.
//!
//! Every read and write carries an explicit `org_id` predicate (same
//! posture as the rest of the crate) on top of the migration's RLS
//! policies. The "billed once, ever" invariant is enforced here plus the
//! partial unique indexes from the migration:
//!
//! * [`list_unbilled_entries`] excludes time entries already linked to
//!   an invoice — the invoice run's engine input.
//! * [`list_billed_keys`] surfaces the (contract, kind, period_start,
//!   description) tuples already billed so the run skips them; the
//!   migration's partial unique indexes are the concurrency backstop.

use crate::{DbError, DbResult};
use core_domain::entities::{
    BillingLineKind, DunningState, Invoice, InvoiceLine, InvoicePayment, InvoiceRun, InvoiceStatus,
};
use sqlx::Postgres;
use std::str::FromStr;
use uuid::Uuid;

/// Allocates the next per-org invoice/run sequence and formats it with
/// the given prefix (`INV-`/`RUN-`). Row-locked increment, safe under
/// concurrency; numbers are unique per (org, prefix).
pub async fn next_number<'e, E>(executor: E, org_id: Uuid, prefix: &str) -> DbResult<String>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let seq: i64 = sqlx::query_scalar(
        "UPDATE orgs SET invoice_sequence = invoice_sequence + 1
         WHERE id = $1 RETURNING invoice_sequence",
    )
    .bind(org_id)
    .fetch_one(executor)
    .await?;
    Ok(format!("{prefix}{seq:06}"))
}

// --- runs ---------------------------------------------------------------------

pub async fn create_run<'e, E>(executor: E, run: &InvoiceRun) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO invoice_runs (id, org_id, number, period_from, period_to,
                                     invoice_count, total_minor, currency, due_in_days, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
        run.id,
        run.org_id,
        run.number,
        run.period_from,
        run.period_to,
        run.invoice_count,
        run.total_minor,
        run.currency,
        run.due_in_days,
        run.created_by,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Fills in the run's roll-up counters after its invoices committed
/// (the row is created up front so invoices can reference it).
pub async fn finalize_run<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    invoice_count: i32,
    total_minor: i64,
    currency: Option<String>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"UPDATE invoice_runs SET invoice_count = $3, total_minor = $4, currency = $5
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
        invoice_count,
        total_minor,
        currency,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// The `invoice_runs` row shape as `query_as!` sees it; mapped to the
/// domain entity once, in one place.
struct RunRow {
    id: Uuid,
    org_id: Uuid,
    number: String,
    period_from: chrono::NaiveDate,
    period_to: chrono::NaiveDate,
    invoice_count: i32,
    total_minor: i64,
    currency: Option<String>,
    due_in_days: i32,
    created_by: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
}

fn run_row(row: RunRow) -> InvoiceRun {
    InvoiceRun {
        id: row.id,
        org_id: row.org_id,
        number: row.number,
        period_from: row.period_from,
        period_to: row.period_to,
        invoice_count: row.invoice_count,
        total_minor: row.total_minor,
        currency: row.currency,
        due_in_days: row.due_in_days,
        created_by: row.created_by,
        created_at: row.created_at,
    }
}

pub async fn get_run<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<InvoiceRun>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as!(
        RunRow,
        r#"SELECT id, org_id, number, period_from, period_to, invoice_count,
                  total_minor, currency, due_in_days, created_by, created_at
           FROM invoice_runs WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(run_row(row))
}

pub async fn list_runs<'e, E>(executor: E, org_id: Uuid, limit: i64) -> DbResult<Vec<InvoiceRun>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as!(
        RunRow,
        r#"SELECT id, org_id, number, period_from, period_to, invoice_count,
                  total_minor, currency, due_in_days, created_by, created_at
           FROM invoice_runs WHERE org_id = $1
           ORDER BY created_at DESC, number DESC
           LIMIT $2"#,
        org_id,
        limit
    )
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(run_row).collect())
}

// --- invoices -----------------------------------------------------------------

pub async fn insert_invoice<'e, E>(executor: E, invoice: &Invoice) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO invoices (id, org_id, run_id, client_id, contract_id, number, status,
                                 currency, period_from, period_to, issued_at, due_on,
                                 total_minor, total_paid_minor, dunning_state,
                                 fx_rate_used, fx_rate_date, fx_source, base_total_minor,
                                 created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
                   $16::text::numeric, $17, $18, $19, $20)"#,
        invoice.id,
        invoice.org_id,
        invoice.run_id,
        invoice.client_id,
        invoice.contract_id,
        invoice.number,
        invoice.status.as_str(),
        invoice.currency,
        invoice.period_from,
        invoice.period_to,
        invoice.issued_at,
        invoice.due_on,
        invoice.total_minor,
        invoice.total_paid_minor,
        invoice.dunning_state.as_str(),
        invoice.fx_rate_used,
        invoice.fx_rate_date,
        invoice.fx_source,
        invoice.base_total_minor,
        invoice.created_by,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Bulk-inserts the invoice's lines in `position` order (one UNNEST).
pub async fn insert_lines<'e, E>(
    executor: E,
    org_id: Uuid,
    invoice_id: Uuid,
    lines: &[InvoiceLine],
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let ids: Vec<Uuid> = lines.iter().map(|l| l.id).collect();
    let orgs: Vec<Uuid> = lines.iter().map(|_| org_id).collect();
    let invoices: Vec<Uuid> = lines.iter().map(|_| invoice_id).collect();
    let contracts: Vec<Uuid> = lines.iter().map(|l| l.contract_id).collect();
    let kinds: Vec<String> = lines.iter().map(|l| l.kind.as_str().to_string()).collect();
    let descriptions: Vec<String> = lines.iter().map(|l| l.description.clone()).collect();
    let starts: Vec<chrono::NaiveDate> = lines.iter().map(|l| l.period_start).collect();
    let ends: Vec<chrono::NaiveDate> = lines.iter().map(|l| l.period_end).collect();
    let minutes: Vec<i64> = lines.iter().map(|l| l.quantity_minutes).collect();
    let units: Vec<i64> = lines.iter().map(|l| l.unit_amount_minor).collect();
    let amounts: Vec<i64> = lines.iter().map(|l| l.amount_minor).collect();
    // Optional snapshot columns ride as text with "" ⇒ NULL (NULLIF) so
    // the UNNEST arrays stay plain-typed; NULLIF maps the sentinel back.
    let fx_rates: Vec<String> = lines
        .iter()
        .map(|l| l.fx_rate_used.clone().unwrap_or_default())
        .collect();
    let fx_dates: Vec<String> = lines
        .iter()
        .map(|l| l.fx_rate_date.map(|d| d.to_string()).unwrap_or_default())
        .collect();
    let fx_sources: Vec<String> = lines
        .iter()
        .map(|l| l.fx_source.clone().unwrap_or_default())
        .collect();
    let base_amounts: Vec<String> = lines
        .iter()
        .map(|l| {
            l.base_amount_minor
                .map(|v| v.to_string())
                .unwrap_or_default()
        })
        .collect();
    let positions: Vec<i32> = lines.iter().enumerate().map(|(i, _)| i as i32).collect();
    // Dynamic (not macro) on purpose: the optional FX snapshot columns
    // ride as text with "" ⇒ NULL sentinels, and the same optional-array
    // shape is what costs.rs/assets.rs already do dynamically.
    sqlx::query(
        r#"INSERT INTO invoice_lines (id, org_id, invoice_id, contract_id, kind, description,
                                      period_start, period_end, quantity_minutes,
                                      unit_amount_minor, amount_minor,
                                      fx_rate_used, fx_rate_date, fx_source, base_amount_minor,
                                      position)
           SELECT id, org_id, invoice_id, contract_id, kind, description,
                  period_start, period_end, quantity_minutes,
                  unit_amount_minor, amount_minor,
                  NULLIF(fx_rate_used, '')::numeric, NULLIF(fx_rate_date, '')::date,
                  NULLIF(fx_source, ''), NULLIF(base_amount_minor, '')::bigint,
                  position
           FROM UNNEST($1::uuid[], $2::uuid[], $3::uuid[], $4::uuid[], $5::text[],
                       $6::text[], $7::date[], $8::date[], $9::bigint[],
                       $10::bigint[], $11::bigint[], $12::text[], $13::text[],
                       $14::text[], $15::text[], $16::int[])
                AS t(id, org_id, invoice_id, contract_id, kind, description,
                     period_start, period_end, quantity_minutes,
                     unit_amount_minor, amount_minor,
                     fx_rate_used, fx_rate_date, fx_source, base_amount_minor, position)"#,
    )
    .bind(&ids)
    .bind(&orgs)
    .bind(&invoices)
    .bind(&contracts)
    .bind(&kinds)
    .bind(&descriptions)
    .bind(&starts)
    .bind(&ends)
    .bind(&minutes)
    .bind(&units)
    .bind(&amounts)
    .bind(&fx_rates)
    .bind(&fx_dates)
    .bind(&fx_sources)
    .bind(&base_amounts)
    .bind(&positions)
    .execute(executor)
    .await?;
    Ok(())
}

/// Links the time entries this invoice consumed. Each entry can be
/// linked at most once EVER (UNIQUE on time_entry_id); a violation
/// surfaces as a typed conflict at the API layer.
pub async fn insert_entry_links<'e, E>(
    executor: E,
    org_id: Uuid,
    invoice_id: Uuid,
    time_entry_ids: &[Uuid],
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let ids: Vec<Uuid> = time_entry_ids.iter().map(|_| Uuid::now_v7()).collect();
    let orgs: Vec<Uuid> = time_entry_ids.iter().map(|_| org_id).collect();
    let invoices: Vec<Uuid> = time_entry_ids.iter().map(|_| invoice_id).collect();
    sqlx::query!(
        r#"INSERT INTO invoice_time_entries (id, org_id, invoice_id, time_entry_id)
           SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::uuid[], $4::uuid[])"#,
        &ids,
        &orgs,
        &invoices,
        time_entry_ids,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// The `invoices` row shape as `query_as!` sees it; mapped to the domain
/// entity once, in one place.
struct InvoiceRow {
    id: Uuid,
    org_id: Uuid,
    run_id: Option<Uuid>,
    client_id: Uuid,
    contract_id: Uuid,
    number: String,
    status: String,
    currency: String,
    period_from: Option<chrono::NaiveDate>,
    period_to: Option<chrono::NaiveDate>,
    issued_at: chrono::NaiveDate,
    due_on: chrono::NaiveDate,
    total_minor: i64,
    total_paid_minor: i64,
    dunning_state: String,
    dunning_updated_at: Option<chrono::DateTime<chrono::Utc>>,
    dunning_note: Option<String>,
    notes: Option<String>,
    fx_rate_used: Option<String>,
    fx_rate_date: Option<chrono::NaiveDate>,
    fx_source: Option<String>,
    base_total_minor: Option<i64>,
    created_by: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn invoice_row(row: InvoiceRow) -> Invoice {
    Invoice {
        id: row.id,
        org_id: row.org_id,
        run_id: row.run_id,
        client_id: row.client_id,
        contract_id: row.contract_id,
        number: row.number,
        status: InvoiceStatus::from_str(&row.status)
            .expect("CHECK constraint guarantees a valid invoice status"),
        currency: row.currency,
        period_from: row.period_from,
        period_to: row.period_to,
        issued_at: row.issued_at,
        due_on: row.due_on,
        total_minor: row.total_minor,
        total_paid_minor: row.total_paid_minor,
        dunning_state: DunningState::from_str(&row.dunning_state)
            .expect("CHECK constraint guarantees a valid dunning state"),
        dunning_updated_at: row.dunning_updated_at,
        dunning_note: row.dunning_note,
        notes: row.notes,
        fx_rate_used: row.fx_rate_used,
        fx_rate_date: row.fx_rate_date,
        fx_source: row.fx_source,
        base_total_minor: row.base_total_minor,
        created_by: row.created_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InvoiceFilters {
    pub client_id: Option<Uuid>,
    pub contract_id: Option<Uuid>,
    /// Exact status (`draft|issued|paid|void`).
    pub status: Option<String>,
    /// Exact dunning state
    /// (`none|reminder_sent|overdue|final_notice|collections`).
    pub dunning_state: Option<String>,
}

pub async fn get_invoice<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Invoice>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as!(
        InvoiceRow,
        r#"SELECT id, org_id, run_id, client_id, contract_id, number, status, currency,
                  period_from, period_to, issued_at, due_on, total_minor, total_paid_minor,
                  dunning_state, dunning_updated_at, dunning_note, notes, created_by,
                  fx_rate_used::text AS fx_rate_used, fx_rate_date, fx_source, base_total_minor,
                  created_at, updated_at
           FROM invoices WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(invoice_row(row))
}

pub async fn list_invoices<'e, E>(
    executor: E,
    org_id: Uuid,
    filters: &InvoiceFilters,
    limit: i64,
) -> DbResult<Vec<Invoice>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as!(
        InvoiceRow,
        r#"SELECT id, org_id, run_id, client_id, contract_id, number, status, currency,
                  period_from, period_to, issued_at, due_on, total_minor, total_paid_minor,
                  dunning_state, dunning_updated_at, dunning_note, notes, created_by,
                  fx_rate_used::text AS fx_rate_used, fx_rate_date, fx_source, base_total_minor,
                  created_at, updated_at
           FROM invoices
           WHERE org_id = $1
             AND ($2::uuid IS NULL OR client_id = $2)
             AND ($3::uuid IS NULL OR contract_id = $3)
             AND ($4::text IS NULL OR status = $4)
             AND ($5::text IS NULL OR dunning_state = $5)
           ORDER BY number DESC
           LIMIT $6"#,
        org_id,
        filters.client_id,
        filters.contract_id,
        filters.status.as_deref(),
        filters.dunning_state.as_deref(),
        limit,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(invoice_row).collect())
}

/// Portal v2 client-scoped invoice reads. Doctrine mirrors portal v1
/// tickets: client scoping happens IN the query on top of org RLS, so a
/// correctly-org-scoped caller still cannot see another client's
/// documents. `void` and `draft` invoices stay staff-internal (a voided
/// invoice was never valid billing; `draft` does not exist yet) — the
/// portal sees exactly `issued` and `paid`.
pub async fn list_client_invoices<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
    limit: i64,
) -> DbResult<Vec<Invoice>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as!(
        InvoiceRow,
        r#"SELECT id, org_id, run_id, client_id, contract_id, number, status, currency,
                  period_from, period_to, issued_at, due_on, total_minor, total_paid_minor,
                  dunning_state, dunning_updated_at, dunning_note, notes, created_by,
                  fx_rate_used::text AS fx_rate_used, fx_rate_date, fx_source, base_total_minor,
                  created_at, updated_at
           FROM invoices
           WHERE org_id = $1 AND client_id = $2 AND status IN ('issued', 'paid')
           ORDER BY issued_at DESC, number DESC
           LIMIT $3"#,
        org_id,
        client_id,
        limit,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(invoice_row).collect())
}

/// One invoice under the same client-scope contract as
/// [`list_client_invoices`]; `DbError::NotFound` when the document
/// belongs to another client or is void/draft — indistinguishable from
/// missing by design (no existence leak).
pub async fn get_client_invoice<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
    invoice_id: Uuid,
) -> DbResult<Invoice>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as!(
        InvoiceRow,
        r#"SELECT id, org_id, run_id, client_id, contract_id, number, status, currency,
                  period_from, period_to, issued_at, due_on, total_minor, total_paid_minor,
                  dunning_state, dunning_updated_at, dunning_note, notes, created_by,
                  fx_rate_used::text AS fx_rate_used, fx_rate_date, fx_source, base_total_minor,
                  created_at, updated_at
           FROM invoices
           WHERE org_id = $1 AND id = $2 AND client_id = $3
             AND status IN ('issued', 'paid')"#,
        org_id,
        invoice_id,
        client_id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(invoice_row(row))
}

pub async fn list_lines<'e, E>(
    executor: E,
    org_id: Uuid,
    invoice_id: Uuid,
) -> DbResult<Vec<InvoiceLine>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, invoice_id, contract_id, kind, description, period_start,
                  period_end, quantity_minutes, unit_amount_minor, amount_minor,
                  fx_rate_used::text AS fx_rate_used, fx_rate_date, fx_source, base_amount_minor, position
           FROM invoice_lines WHERE org_id = $1 AND invoice_id = $2
           ORDER BY position, id"#,
        org_id,
        invoice_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| InvoiceLine {
            id: r.id,
            org_id: r.org_id,
            invoice_id: r.invoice_id,
            contract_id: r.contract_id,
            kind: BillingLineKind::from_str(&r.kind)
                .expect("CHECK constraint guarantees a valid line kind"),
            description: r.description,
            period_start: r.period_start,
            period_end: r.period_end,
            quantity_minutes: r.quantity_minutes,
            unit_amount_minor: r.unit_amount_minor,
            amount_minor: r.amount_minor,
            fx_rate_used: r.fx_rate_used,
            fx_rate_date: r.fx_rate_date,
            fx_source: r.fx_source,
            base_amount_minor: r.base_amount_minor,
            position: r.position,
        })
        .collect())
}

/// One already-billed line identity (the run's dedupe key). `kind` stays
/// a string here; the API layer compares against the wire kind strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BilledLineKey {
    pub contract_id: Uuid,
    pub kind: String,
    pub period_start: chrono::NaiveDate,
    pub description: String,
}

/// The billed (contract, kind, period_start, description) tuples for the
/// given contracts — the run's dedupe key set.
pub async fn list_billed_keys<'e, E>(
    executor: E,
    org_id: Uuid,
    contract_ids: &[Uuid],
) -> DbResult<Vec<BilledLineKey>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    if contract_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query!(
        r#"SELECT contract_id, kind, period_start, description
           FROM invoice_lines
           WHERE org_id = $1 AND contract_id = ANY($2)"#,
        org_id,
        contract_ids
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BilledLineKey {
            contract_id: r.contract_id,
            kind: r.kind,
            period_start: r.period_start,
            description: r.description,
        })
        .collect())
}

/// Whether any invoice references the contract — the billing-lock guard
/// that stops contract deletes once invoices exist.
pub async fn contract_has_invoices<'e, E>(
    executor: E,
    org_id: Uuid,
    contract_id: Uuid,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM invoices WHERE org_id = $1 AND contract_id = $2")
            .bind(org_id)
            .bind(contract_id)
            .fetch_one(executor)
            .await?;
    Ok(count > 0)
}

// --- mutations: void / dunning / payments -------------------------------------

/// Voids an ISSUED invoice; `None` return means the invoice was not in a
/// voidable state (paid/void/draft invoices are terminal or immaterial).
pub async fn void_invoice<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Option<Invoice>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as!(
        InvoiceRow,
        r#"UPDATE invoices SET status = 'void', updated_at = now()
           WHERE org_id = $1 AND id = $2 AND status = 'issued'
           RETURNING id, org_id, run_id, client_id, contract_id, number, status, currency,
                     period_from, period_to, issued_at, due_on, total_minor, total_paid_minor,
                     dunning_state, dunning_updated_at, dunning_note, notes,
                     fx_rate_used::text AS fx_rate_used, fx_rate_date, fx_source, base_total_minor,
                     created_by, created_at, updated_at"#,
        org_id,
        id
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(invoice_row))
}

/// Moves an ISSUED invoice's dunning state (the caller validates the
/// forward-only rule). `None` return means the invoice is not in an
/// issued state (paid/void/draft invoices have no dunning posture).
pub async fn set_dunning<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    state: DunningState,
    note: Option<String>,
) -> DbResult<Option<Invoice>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as!(
        InvoiceRow,
        r#"UPDATE invoices SET dunning_state = $3, dunning_updated_at = now(),
                               dunning_note = $4, updated_at = now()
           WHERE org_id = $1 AND id = $2 AND status = 'issued'
           RETURNING id, org_id, run_id, client_id, contract_id, number, status, currency,
                     period_from, period_to, issued_at, due_on, total_minor, total_paid_minor,
                     dunning_state, dunning_updated_at, dunning_note, notes,
                     fx_rate_used::text AS fx_rate_used, fx_rate_date, fx_source, base_total_minor,
                     created_by, created_at, updated_at"#,
        org_id,
        id,
        state.as_str(),
        note,
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(invoice_row))
}

pub async fn insert_payment<'e, E>(executor: E, payment: &InvoicePayment) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO invoice_payments (id, org_id, invoice_id, amount_minor, currency,
                                         fx_rate_used, fx_rate_date, fx_source, base_amount_minor,
                                         provider, provider_ref, notes, received_at, created_by)
           VALUES ($1, $2, $3, $4, $5, $6::text::numeric, $7, $8, $9, $10, $11, $12, $13, $14)"#,
        payment.id,
        payment.org_id,
        payment.invoice_id,
        payment.amount_minor,
        payment.currency,
        payment.fx_rate_used,
        payment.fx_rate_date,
        payment.fx_source,
        payment.base_amount_minor,
        payment.provider,
        payment.provider_ref,
        payment.notes,
        payment.received_at,
        payment.created_by,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Applies a recorded payment to the invoice's paid total, flipping the
/// status to `paid` on settlement. Returns `None` when the invoice is
/// not in an issued state (payments only land on issued invoices).
pub async fn apply_payment_total<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    amount_minor: i64,
) -> DbResult<Option<Invoice>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as!(
        InvoiceRow,
        r#"UPDATE invoices SET total_paid_minor = total_paid_minor + $3,
                               status = CASE
                                   WHEN total_paid_minor + $3 >= total_minor THEN 'paid'
                                   ELSE status END,
                               updated_at = now()
           WHERE org_id = $1 AND id = $2 AND status = 'issued'
           RETURNING id, org_id, run_id, client_id, contract_id, number, status, currency,
                     period_from, period_to, issued_at, due_on, total_minor, total_paid_minor,
                     dunning_state, dunning_updated_at, dunning_note, notes,
                     fx_rate_used::text AS fx_rate_used, fx_rate_date, fx_source, base_total_minor,
                     created_by, created_at, updated_at"#,
        org_id,
        id,
        amount_minor,
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(invoice_row))
}

pub async fn list_payments<'e, E>(
    executor: E,
    org_id: Uuid,
    invoice_id: Uuid,
) -> DbResult<Vec<InvoicePayment>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, invoice_id, amount_minor, currency, provider, provider_ref,
                  notes, received_at, created_by,
                  fx_rate_used::text AS fx_rate_used, fx_rate_date, fx_source, base_amount_minor
           FROM invoice_payments WHERE org_id = $1 AND invoice_id = $2
           ORDER BY received_at, id"#,
        org_id,
        invoice_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| InvoicePayment {
            id: r.id,
            org_id: r.org_id,
            invoice_id: r.invoice_id,
            amount_minor: r.amount_minor,
            currency: r.currency,
            fx_rate_used: r.fx_rate_used,
            fx_rate_date: r.fx_rate_date,
            fx_source: r.fx_source,
            base_amount_minor: r.base_amount_minor,
            provider: r.provider,
            provider_ref: r.provider_ref,
            notes: r.notes,
            received_at: r.received_at,
            created_by: r.created_by,
        })
        .collect())
}

// --- billing-rule inputs ------------------------------------------------------

/// One billable time entry row for rule evaluation (same shape as
/// `contracts::BillableEntryRow`, restricted to entries no invoice has
/// consumed yet — the invoice run's engine input).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnbilledEntryRow {
    pub time_entry_id: Uuid,
    pub minutes: i32,
    pub work_type: Option<String>,
    pub entry_date: chrono::NaiveDate,
}

/// Billable, NOT-YET-BILLED time entries for one client within the
/// inclusive date window, ascending by start time. The `NOT EXISTS`
/// against the consumption links makes double-billing an entry
/// impossible even across overlapping runs and across contracts sharing
/// a client.
pub async fn list_unbilled_entries<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
) -> DbResult<Vec<UnbilledEntryRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT te.id, te.minutes, te.work_type, te.started_at::date AS "entry_date!"
           FROM time_entries te
           JOIN tickets t ON t.id = te.ticket_id
           WHERE te.org_id = $1 AND t.client_id = $2
             AND te.billable
             AND te.started_at::date >= $3 AND te.started_at::date <= $4
             AND NOT EXISTS (
                 SELECT 1 FROM invoice_time_entries l WHERE l.time_entry_id = te.id)
           ORDER BY te.started_at, te.id"#,
        org_id,
        client_id,
        from,
        to,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| UnbilledEntryRow {
            time_entry_id: r.id,
            minutes: r.minutes,
            work_type: r.work_type,
            entry_date: r.entry_date,
        })
        .collect())
}
