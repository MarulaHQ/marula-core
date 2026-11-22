//! Native invoicing v1 (the private build tracker Phase 3, Wave 9): invoice runs,
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
                                 po_number, notes, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
                   $16::text::numeric, $17, $18, $19, $20, $21, $22)"#,
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
        invoice.po_number,
        invoice.notes,
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
    let source_kinds: Vec<String> = lines.iter().map(|l| l.source_kind.clone()).collect();
    let source_ids: Vec<Uuid> = lines.iter().map(|l| l.source_id).collect();
    let positions: Vec<i32> = lines.iter().enumerate().map(|(i, _)| i as i32).collect();
    // The optional FX snapshot columns ride as text with "" => NULL
    // sentinels, which the macro types as text and the statement casts.
    sqlx::query!(
        r#"INSERT INTO invoice_lines (id, org_id, invoice_id, contract_id, kind, description,
                                      period_start, period_end, quantity_minutes,
                                      unit_amount_minor, amount_minor,
                                      fx_rate_used, fx_rate_date, fx_source, base_amount_minor,
                                      source_kind, source_id, position)
           SELECT id, org_id, invoice_id, contract_id, kind, description,
                  period_start, period_end, quantity_minutes,
                  unit_amount_minor, amount_minor,
                  NULLIF(fx_rate_used, '')::numeric, NULLIF(fx_rate_date, '')::date,
                  NULLIF(fx_source, ''), NULLIF(base_amount_minor, '')::bigint,
                  source_kind, source_id, position
           FROM UNNEST($1::uuid[], $2::uuid[], $3::uuid[], $4::uuid[], $5::text[],
                       $6::text[], $7::date[], $8::date[], $9::bigint[],
                       $10::bigint[], $11::bigint[], $12::text[], $13::text[],
                       $14::text[], $15::text[], $16::text[], $17::uuid[], $18::int[])
                AS t(id, org_id, invoice_id, contract_id, kind, description,
                     period_start, period_end, quantity_minutes,
                     unit_amount_minor, amount_minor,
                     fx_rate_used, fx_rate_date, fx_source, base_amount_minor,
                     source_kind, source_id, position)"#,
        &ids,
        &orgs,
        &invoices,
        &contracts,
        &kinds,
        &descriptions,
        &starts,
        &ends,
        &minutes,
        &units,
        &amounts,
        &fx_rates,
        &fx_dates,
        &fx_sources,
        &base_amounts,
        &source_kinds,
        &source_ids,
        &positions,
    )
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
/// entity once, in one place. `FromRow` is what the runtime `query_as`
/// queries in this module read into (the CAS paths whose SQL is built at
/// runtime, e.g. `apply_payment_total_expecting`).
#[derive(sqlx::FromRow)]
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
    po_number: Option<String>,
    notes: Option<String>,
    fx_rate_used: Option<String>,
    fx_rate_date: Option<chrono::NaiveDate>,
    fx_source: Option<String>,
    base_total_minor: Option<i64>,
    created_by: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

/// Every column `InvoiceRow` reads, spelled for the runtime `query_as`
/// paths (a `query_as!` macro spells them inline). Kept beside the struct
/// so a new column is added in one place, not two.
const INVOICE_COLUMNS: &str =
    "id, org_id, run_id, client_id, contract_id, number, status, currency, \
     period_from, period_to, issued_at, due_on, total_minor, total_paid_minor, dunning_state, \
     dunning_updated_at, dunning_note, po_number, notes, \
     fx_rate_used::text AS fx_rate_used, fx_rate_date, fx_source, base_total_minor, \
     created_by, created_at, updated_at";

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
        po_number: row.po_number,
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
                  dunning_state, dunning_updated_at, dunning_note, po_number, notes, created_by,
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
                  dunning_state, dunning_updated_at, dunning_note, po_number, notes, created_by,
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
                  dunning_state, dunning_updated_at, dunning_note, po_number, notes, created_by,
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
                  dunning_state, dunning_updated_at, dunning_note, po_number, notes, created_by,
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
                  fx_rate_used::text AS fx_rate_used, fx_rate_date, fx_source, base_amount_minor,
                  source_kind, source_id, position
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
            // The insert trigger refuses a line without a source, so a read
            // cannot meet a null here except on a pre-0260 row the backfill
            // missed; fall back to the contract rather than lie.
            source_kind: r.source_kind.unwrap_or_else(|| "contract_line".to_string()),
            source_id: r.source_id.unwrap_or(r.contract_id),
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
                     dunning_state, dunning_updated_at, dunning_note, po_number, notes,
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
                     dunning_state, dunning_updated_at, dunning_note, po_number, notes,
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

/// The unique index migration 0165 puts on a receipt's provider reference.
///
/// Named here because both insert paths below answer for the same constraint:
/// [`insert_payment_once`] reads zero affected rows as "already applied", and
/// [`insert_payment`] types the violation for the API writers.
const PROVIDER_REF_INDEX: &str = "idx_invoice_payments_provider_ref";

const INSERT_PAYMENT_SQL: &str = r#"INSERT INTO invoice_payments (id, org_id, invoice_id, amount_minor, currency,
                                         fx_rate_used, fx_rate_date, fx_source, base_amount_minor,
                                         provider, provider_ref, notes, received_at, created_by)
           VALUES ($1, $2, $3, $4, $5, $6::text::numeric, $7, $8, $9, $10, $11, $12, $13, $14)"#;

/// One receipt insert, one statement. `once` appends `ON CONFLICT DO NOTHING`
/// so the caller learns from the affected-row count whether the unique index
/// (0165) already held this provider reference on this invoice.
async fn insert_payment_rows<'e, E>(
    executor: E,
    payment: &InvoicePayment,
    once: bool,
) -> Result<u64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = if once {
        format!("{INSERT_PAYMENT_SQL} ON CONFLICT DO NOTHING")
    } else {
        INSERT_PAYMENT_SQL.to_string()
    };
    let done = sqlx::query(&sql)
        .bind(payment.id)
        .bind(payment.org_id)
        .bind(payment.invoice_id)
        .bind(payment.amount_minor)
        .bind(payment.currency.as_str())
        .bind(payment.fx_rate_used.as_deref())
        .bind(payment.fx_rate_date)
        .bind(payment.fx_source.as_deref())
        .bind(payment.base_amount_minor)
        .bind(payment.provider.as_str())
        .bind(payment.provider_ref.as_deref())
        .bind(payment.notes.as_deref())
        .bind(payment.received_at)
        .bind(payment.created_by)
        .execute(executor)
        .await?;
    Ok(done.rows_affected())
}

/// Records one receipt, refusing a provider reference the invoice already
/// carries.
///
/// The refusal used to be a read in the caller, which two concurrent callers
/// could both pass. It is the unique index now (migration 0165), so this
/// statement cannot write the double credit at all: a violation becomes
/// `DbError::Conflict` naming the reference (409 at the wire) rather than the
/// sanitized generic conflict an unmapped unique violation produces.
///
/// The webhook does not use this form — a duplicate delivery there is an
/// acknowledgement, not an error. It calls [`insert_payment_once`].
pub async fn insert_payment<'e, E>(executor: E, payment: &InvoicePayment) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    match insert_payment_rows(executor, payment, false).await {
        Ok(_) => Ok(()),
        Err(sqlx::Error::Database(db)) if db.constraint() == Some(PROVIDER_REF_INDEX) => {
            Err(DbError::Conflict(format!(
                "provider reference {} is already recorded on this invoice",
                payment.provider_ref.as_deref().unwrap_or_default()
            )))
        }
        Err(e) => Err(DbError::Sql(e)),
    }
}

/// Records one receipt UNLESS the invoice already carries this provider
/// reference, and says which happened: `true` when the row landed, `false`
/// when the unique index (0165) refused it.
///
/// This is the settlement path's fence. `ON CONFLICT DO NOTHING` turns the
/// index's answer into a row count instead of an error, so two deliveries of
/// one event that raced past the advisory read in `record_payment` end with
/// one receipt and one `false` — the same answer a later delivery gets, not a
/// 500 and not a second credit. A losing insert writes nothing, so the caller
/// can commit the transaction it opened and answer normally.
pub async fn insert_payment_once<'e, E>(executor: E, payment: &InvoicePayment) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = insert_payment_rows(executor, payment, true).await?;
    Ok(rows == 1)
}

/// Whether ONE payment already carries `provider_ref` on this invoice.
///
/// ADVISORY, not the fence. The Stripe webhook calls this first so a
/// redelivery replies before it pays FX resolution, but two deliveries can
/// both read `false` here — the read takes no lock. The decision belongs to
/// [`insert_payment_once`], which the database cannot let both of them pass.
/// Never treat a `false` answer as permission to insert twice.
pub async fn payment_exists_for_ref<'e, E>(
    executor: E,
    org_id: Uuid,
    invoice_id: Uuid,
    provider_ref: &str,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_scalar!(
        r#"SELECT EXISTS(
             SELECT 1 FROM invoice_payments
             WHERE org_id = $1 AND invoice_id = $2 AND provider_ref = $3
           ) AS "exists!""#,
        org_id,
        invoice_id,
        provider_ref,
    )
    .fetch_one(executor)
    .await?;
    Ok(row)
}

/// Applies a recorded payment to the invoice's paid total, flipping the
/// status to `paid` on settlement. Returns `None` when the invoice is
/// not in an issued state (payments only land on issued invoices).
///
/// This is the no-expectation form: the caller has no earlier read to
/// compare the invoice against, so it applies whatever the row holds now.
/// Callers that already priced the receipt on a total they read —
/// every API payment path — use `apply_payment_total_expecting` instead,
/// which makes that read part of the predicate.
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
                     dunning_state, dunning_updated_at, dunning_note, po_number, notes,
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

/// Applies a recorded payment against the basis the caller priced it on,
/// with every part of that basis in the UPDATE's predicate:
///
/// * `status = 'issued'` — a paid or void invoice takes no more receipts;
/// * `total_minor = $4` — the invoice still totals what the caller read
///   before it validated the amount, so a receipt cannot settle against a
///   total that moved underneath it;
/// * `total_paid_minor + $3 <= total_minor` — the receipt still fits the
///   outstanding balance, so two receipts each inside the balance cannot
///   both land and push the paid total past the total.
///
/// Postgres re-evaluates the whole predicate against the latest committed
/// row once it holds the row lock, which is what makes this atomic under
/// READ COMMITTED. The loser gets a typed `DbError::Conflict` (409 at the
/// API) naming which part of the basis moved, and its caller's transaction
/// rolls the payment row back with it — no 500, no silent write.
pub async fn apply_payment_total_expecting(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    amount_minor: i64,
    expected_total_minor: i64,
) -> DbResult<Invoice> {
    let sql = format!(
        "UPDATE invoices SET total_paid_minor = total_paid_minor + $3,
                status = CASE
                    WHEN total_paid_minor + $3 >= total_minor THEN 'paid'
                    ELSE status END,
                updated_at = now()
          WHERE org_id = $1 AND id = $2 AND status = 'issued'
            AND total_minor = $4
            AND total_paid_minor + $3 <= total_minor
          RETURNING {INVOICE_COLUMNS}"
    );
    let row = sqlx::query_as::<_, InvoiceRow>(&sql)
        .bind(org_id)
        .bind(id)
        .bind(amount_minor)
        .bind(expected_total_minor)
        .fetch_optional(&mut *conn)
        .await?;
    let Some(row) = row else {
        // Lost the predicate. Read the row back (same transaction) to say
        // WHICH part refused, so the refusal is actionable rather than a
        // generic "retry".
        let current: Option<(String, i64, i64)> = sqlx::query_as(
            "SELECT status, total_minor, total_paid_minor FROM invoices \
             WHERE org_id = $1 AND id = $2",
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
        let Some((status, total_minor, total_paid_minor)) = current else {
            return Err(DbError::NotFound);
        };
        if status != "issued" {
            return Err(DbError::Conflict(format!(
                "invoice left its payable state ({status}); the payment was not applied"
            )));
        }
        if total_minor != expected_total_minor {
            return Err(DbError::Conflict(format!(
                "invoice total changed from {expected_total_minor} to {total_minor} \
                 while the payment was recorded; the payment was not applied"
            )));
        }
        return Err(DbError::Conflict(format!(
            "payment exceeds the outstanding balance ({} minor units); \
             the payment was not applied",
            total_minor.saturating_sub(total_paid_minor)
        )));
    };
    Ok(invoice_row(row))
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

/// Billable, APPROVED, NOT-YET-BILLED time entries for one client within
/// the inclusive date window, ascending by start time. The `NOT EXISTS`
/// against the consumption links makes double-billing an entry
/// impossible even across overlapping runs and across contracts sharing
/// a client; the approval filter means only approved
/// work reaches an invoice. Minutes are the EFFECTIVE minutes — the raw
/// minutes less the verification adjustment ledger (Wave 39 lane 2), so
/// this candidate panel agrees with what the engine will price.
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
        r#"SELECT te.id,
                  GREATEST(te.minutes + COALESCE((
                      SELECT SUM(a.delta_minutes) FROM time_entry_adjustments a
                       WHERE a.org_id = te.org_id AND a.time_entry_id = te.id
                  ), 0), 0)::integer AS "minutes!",
                  te.work_type, te.started_at::date AS "entry_date!"
           FROM time_entries te
           JOIN tickets t ON t.id = te.ticket_id
           WHERE te.org_id = $1 AND t.client_id = $2
             AND te.billable
             AND te.approval_status = 'approved'
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

// --- billing a ticket's candidates onto an invoice ----------------------------

/// What the caller wants billed, and under which contract.
pub struct BillCandidatesRequest {
    pub contract_id: Uuid,
    /// The rate the invoice and its lines lock, when the contract's currency
    /// is not the organisation's base. Resolved by the caller in the same
    /// transaction, the way the invoice run resolves it.
    pub fx: Option<CandidateFx>,
    pub cost_line_ids: Vec<Uuid>,
    pub expense_ids: Vec<Uuid>,
    pub period_from: Option<chrono::NaiveDate>,
    pub period_to: Option<chrono::NaiveDate>,
    pub due_on: chrono::NaiveDate,
    pub notes: Option<String>,
    pub created_by: Uuid,
}

/// The conversion a candidate invoice locks: the rate itself for per-line
/// base amounts, and the snapshot columns the header stores.
pub struct CandidateFx {
    pub rate: core_domain::fx::FxRate,
    pub rate_used: String,
    pub rate_date: chrono::NaiveDate,
    pub source: String,
}

/// Bills the named ticket cost lines and expenses onto one new issued invoice,
/// under one contract, in the caller's transaction. Each line keeps its source,
/// each candidate gets its `invoice_line_id` in the same transaction, and a
/// candidate already billed refuses the whole request. Issuing a document is
/// the only way invoice totals move, and this writes exactly one.
pub async fn bill_candidates(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    req: &BillCandidatesRequest,
) -> DbResult<(Invoice, Vec<InvoiceLine>)> {
    use core_domain::entities::{BillingLineKind, DunningState, InvoiceStatus};

    if req.cost_line_ids.is_empty() && req.expense_ids.is_empty() {
        return Err(DbError::Conflict(
            "nothing to bill: name at least one candidate line".into(),
        ));
    }
    let contract = crate::contracts::get_contract(&mut *conn, org_id, req.contract_id).await?;
    let client_id: Uuid = contract.client_id;
    let currency = contract.currency.clone();
    let issued_at = chrono::Utc::now();
    let invoice_id = Uuid::now_v7();
    let number = next_number(&mut *conn, org_id, "INV-").await?;

    struct Candidate {
        line_id: Uuid,
        description: String,
        amount_minor: i64,
        kind: &'static str,
    }
    let mut candidates: Vec<Candidate> = Vec::new();
    for id in &req.cost_line_ids {
        let row = sqlx::query!(
            r#"SELECT c.description, c.currency,
                      round(c.quantity * c.unit_cost_minor)::bigint AS "amount_minor!",
                      t.client_id,
                      (c.invoice_line_id IS NOT NULL) AS "billed!"
                 FROM ticket_cost_lines c
                 JOIN tickets t ON t.org_id = c.org_id AND t.id = c.ticket_id
                WHERE c.org_id = $1 AND c.id = $2 AND c.billable
                FOR UPDATE OF c"#,
            org_id,
            id,
        )
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(DbError::NotFound)?;
        if row.client_id != Some(client_id) {
            return Err(DbError::Conflict(
                "a cost line belongs to another client than the contract".into(),
            ));
        }
        if row.billed {
            return Err(DbError::Conflict(format!(
                "cost line {id} is already billed"
            )));
        }
        if !row.currency.eq_ignore_ascii_case(&currency) {
            return Err(DbError::Conflict(format!(
                "cost line {id} is {} but contract {} bills in {}",
                row.currency, contract.number, currency
            )));
        }
        candidates.push(Candidate {
            line_id: *id,
            description: row.description,
            amount_minor: row.amount_minor,
            kind: "ticket_cost_line",
        });
    }
    for id in &req.expense_ids {
        let row = sqlx::query!(
            r#"SELECT e.description, e.currency, e.amount_minor, t.client_id,
                      (e.invoice_line_id IS NOT NULL) AS "billed!"
                 FROM ticket_expenses e
                 JOIN tickets t ON t.org_id = e.org_id AND t.id = e.ticket_id
                WHERE e.org_id = $1 AND e.id = $2 AND e.billable
                FOR UPDATE OF e"#,
            org_id,
            id,
        )
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(DbError::NotFound)?;
        if row.client_id != Some(client_id) {
            return Err(DbError::Conflict(
                "an expense belongs to another client than the contract".into(),
            ));
        }
        if row.billed {
            return Err(DbError::Conflict(format!("expense {id} is already billed")));
        }
        if !row.currency.eq_ignore_ascii_case(&currency) {
            return Err(DbError::Conflict(format!(
                "expense {id} is {} but contract {} bills in {}",
                row.currency, contract.number, currency
            )));
        }
        candidates.push(Candidate {
            line_id: *id,
            description: row.description,
            amount_minor: row.amount_minor,
            kind: "ticket_expense",
        });
    }

    let total_minor = candidates.iter().try_fold(0i64, |acc, candidate| {
        acc.checked_add(candidate.amount_minor)
            .ok_or_else(|| DbError::Conflict("the invoice total exceeds the money range".into()))
    })?;
    if total_minor <= 0 {
        return Err(DbError::Conflict(
            "the selected candidates total nothing to bill".into(),
        ));
    }

    // One canonical conversion point: every line's base amount applies the
    // same rate to its own amount, and the header's base total is their sum,
    // so the reconciliation read model cross-foots exactly.
    let base_of = |minor: i64| -> DbResult<Option<i64>> {
        match &req.fx {
            Some(fx) => {
                let code = core_domain::money::CurrencyCode::new(&currency)
                    .map_err(|e| DbError::Conflict(e.to_string()))?;
                let converted = core_domain::money::Money::new(minor, code)
                    .convert(&fx.rate)
                    .map_err(|e| DbError::Conflict(e.to_string()))?;
                Ok(Some(converted.minor))
            }
            None => Ok(None),
        }
    };
    let line_bases: Vec<Option<i64>> = candidates
        .iter()
        .map(|candidate| base_of(candidate.amount_minor))
        .collect::<DbResult<_>>()?;
    let base_total = match &req.fx {
        Some(_) => Some(line_bases.iter().flatten().try_fold(0i64, |acc, base| {
            acc.checked_add(*base)
                .ok_or_else(|| DbError::Conflict("the base total exceeds the money range".into()))
        })?),
        None => None,
    };

    let invoice = Invoice {
        id: invoice_id,
        org_id,
        run_id: None,
        client_id,
        contract_id: req.contract_id,
        number,
        status: InvoiceStatus::Issued,
        currency: currency.clone(),
        period_from: req.period_from,
        period_to: req.period_to,
        issued_at: issued_at.date_naive(),
        due_on: req.due_on,
        total_minor,
        total_paid_minor: 0,
        dunning_state: DunningState::None,
        dunning_updated_at: None,
        dunning_note: None,
        fx_rate_used: req.fx.as_ref().map(|fx| fx.rate_used.clone()),
        fx_rate_date: req.fx.as_ref().map(|fx| fx.rate_date),
        fx_source: req.fx.as_ref().map(|fx| fx.source.clone()),
        base_total_minor: base_total,
        po_number: None,
        notes: req.notes.clone(),
        created_by: Some(req.created_by),
        created_at: issued_at,
        updated_at: issued_at,
    };
    invoice
        .validate()
        .map_err(|e| DbError::Conflict(e.to_string()))?;
    insert_invoice(&mut *conn, &invoice).await?;

    let mut lines = Vec::with_capacity(candidates.len());
    for (position, candidate) in candidates.iter().enumerate() {
        // An expense is its own kind: the fixed-fee unique index keys on
        // (contract, kind, period_start, description), so two expenses that
        // share a description are two lines rather than a collision.
        let kind = if candidate.kind == "ticket_expense" {
            BillingLineKind::Expense
        } else {
            BillingLineKind::TimeMaterials
        };
        let line = InvoiceLine {
            id: Uuid::now_v7(),
            org_id,
            invoice_id,
            contract_id: req.contract_id,
            kind,
            description: candidate.description.clone(),
            period_start: req.period_from.unwrap_or(issued_at.date_naive()),
            period_end: req.period_to.unwrap_or(issued_at.date_naive()),
            quantity_minutes: 0,
            unit_amount_minor: candidate.amount_minor,
            amount_minor: candidate.amount_minor,
            base_amount_minor: line_bases[position],
            fx_rate_used: req.fx.as_ref().map(|fx| fx.rate_used.clone()),
            fx_rate_date: req.fx.as_ref().map(|fx| fx.rate_date),
            fx_source: req.fx.as_ref().map(|fx| fx.source.clone()),
            source_kind: candidate.kind.to_string(),
            source_id: candidate.line_id,
            position: position as i32,
        };
        if candidate.kind == "ticket_expense" {
            crate::costs::mark_candidates_billed(
                &mut *conn,
                org_id,
                &[],
                std::slice::from_ref(&candidate.line_id),
                line.id,
            )
            .await?;
        } else {
            crate::costs::mark_candidates_billed(
                &mut *conn,
                org_id,
                std::slice::from_ref(&candidate.line_id),
                &[],
                line.id,
            )
            .await?;
        }
        lines.push(line);
    }
    insert_lines(&mut *conn, org_id, invoice_id, &lines).await?;
    Ok((invoice, lines))
}
