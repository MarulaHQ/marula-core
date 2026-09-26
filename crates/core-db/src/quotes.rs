//! Quotes persistence (TASKLIST Phase 3, Wave 12 lane 2; migration
//! 0060): the quote document, its line items, and the per-org number
//! sequence.
//!
//! Every read and write carries an explicit `org_id` predicate on top of
//! the migration's RLS policies (same posture as the rest of the crate).
//! Status transitions are guarded UPDATEs (`WHERE status = $from`) so a
//! concurrent lifecycle leg can never double-fire — the `None` return is
//! the typed "left its editable/expected state" signal the API maps to a
//! 409. Draft-only edits are enforced the same way: every content UPDATE
//! carries `AND status = 'draft'` in SQL, not just in Rust.

use crate::{DbError, DbResult};
use core_domain::entities::{Quote, QuoteLine, QuoteStatus};
use sqlx::Postgres;
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
struct QuoteRow {
    id: Uuid,
    org_id: Uuid,
    number: String,
    client_id: Uuid,
    status: String,
    currency: String,
    issued_on: chrono::NaiveDate,
    valid_until: chrono::NaiveDate,
    notes: Option<String>,
    terms: Option<String>,
    subtotal_minor: i64,
    tax_rate_bps: i32,
    tax_minor: i64,
    total_minor: i64,
    sent_at: Option<chrono::DateTime<chrono::Utc>>,
    decided_at: Option<chrono::DateTime<chrono::Utc>>,
    converted_contract_id: Option<Uuid>,
    converted_invoice_id: Option<Uuid>,
    converted_at: Option<chrono::DateTime<chrono::Utc>>,
    created_by: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn quote_row(r: QuoteRow) -> Quote {
    Quote {
        id: r.id,
        org_id: r.org_id,
        number: r.number,
        client_id: r.client_id,
        status: QuoteStatus::from_str(&r.status).unwrap_or(QuoteStatus::Draft),
        currency: r.currency,
        issued_on: r.issued_on,
        valid_until: r.valid_until,
        notes: r.notes,
        terms: r.terms,
        subtotal_minor: r.subtotal_minor,
        tax_rate_bps: r.tax_rate_bps,
        tax_minor: r.tax_minor,
        total_minor: r.total_minor,
        sent_at: r.sent_at,
        decided_at: r.decided_at,
        converted_contract_id: r.converted_contract_id,
        converted_invoice_id: r.converted_invoice_id,
        converted_at: r.converted_at,
        created_by: r.created_by,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

const COLUMNS: &str = "id, org_id, number, client_id, status, currency, issued_on, valid_until,
                        notes, terms, subtotal_minor, tax_rate_bps, tax_minor, total_minor,
                        sent_at, decided_at, converted_contract_id, converted_invoice_id,
                        converted_at, created_by, created_at, updated_at";

/// Allocates the next per-org quote number (`Q-000012`). Row-locked
/// increment on the org's own counter, safe under concurrency.
pub async fn next_quote_number<'e, E>(executor: E, org_id: Uuid) -> DbResult<String>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let seq: i64 = sqlx::query_scalar(
        "UPDATE orgs SET quote_sequence = quote_sequence + 1
         WHERE id = $1 RETURNING quote_sequence",
    )
    .bind(org_id)
    .fetch_one(executor)
    .await?;
    Ok(format!("Q-{seq:06}"))
}

pub async fn insert_quote<'e, E>(executor: E, quote: &Quote) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"INSERT INTO quotes (id, org_id, number, client_id, status, currency, issued_on,
                               valid_until, notes, terms, subtotal_minor, tax_rate_bps,
                               tax_minor, total_minor, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)"#,
    )
    .bind(quote.id)
    .bind(quote.org_id)
    .bind(quote.number.trim())
    .bind(quote.client_id)
    .bind(quote.status.as_str())
    .bind(&quote.currency)
    .bind(quote.issued_on)
    .bind(quote.valid_until)
    .bind(quote.notes.as_deref())
    .bind(quote.terms.as_deref())
    .bind(quote.subtotal_minor)
    .bind(quote.tax_rate_bps)
    .bind(quote.tax_minor)
    .bind(quote.total_minor)
    .bind(quote.created_by)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_quote<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Quote>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {COLUMNS} FROM quotes WHERE org_id = $1 AND id = $2");
    let row = sqlx::query_as::<_, QuoteRow>(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(quote_row(row))
}

pub struct QuoteFilters {
    pub client_id: Option<Uuid>,
    /// Exact status: `draft|sent|accepted|declined|expired`.
    pub status: Option<String>,
}

pub async fn list_quotes<'e, E>(
    executor: E,
    org_id: Uuid,
    filters: &QuoteFilters,
    limit: i64,
) -> DbResult<Vec<Quote>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {COLUMNS} FROM quotes
         WHERE org_id = $1
           AND ($2::uuid IS NULL OR client_id = $2)
           AND ($3::text IS NULL OR status = $3)
         ORDER BY created_at DESC, id DESC
         LIMIT $4"
    );
    let rows = sqlx::query_as::<_, QuoteRow>(&sql)
        .bind(org_id)
        .bind(filters.client_id)
        .bind(filters.status.as_deref())
        .bind(limit)
        .fetch_all(executor)
        .await?;
    Ok(rows.into_iter().map(quote_row).collect())
}

/// Content update (header + roll-ups). Guarded to draft quotes IN SQL:
/// `None` means the quote does not exist OR is no longer editable.
pub async fn update_draft_quote<'e, E>(executor: E, quote: &Quote) -> DbResult<Option<Quote>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE quotes SET valid_until = $3, notes = $4, terms = $5,
                            tax_rate_bps = $6, subtotal_minor = $7, tax_minor = $8,
                            total_minor = $9, updated_at = now()
         WHERE org_id = $1 AND id = $2 AND status = 'draft'
         RETURNING {COLUMNS}"
    );
    let row = sqlx::query_as::<_, QuoteRow>(&sql)
        .bind(quote.org_id)
        .bind(quote.id)
        .bind(quote.valid_until)
        .bind(quote.notes.as_deref())
        .bind(quote.terms.as_deref())
        .bind(quote.tax_rate_bps)
        .bind(quote.subtotal_minor)
        .bind(quote.tax_minor)
        .bind(quote.total_minor)
        .fetch_optional(executor)
        .await?;
    Ok(row.map(quote_row))
}

/// One lifecycle leg (`from` → `to`), stamped server-side. Guarded in
/// SQL: `None` means the quote was not in the expected source state.
pub async fn transition_status<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    from: &str,
    to: &str,
    stamp_sent: bool,
    stamp_decided: bool,
) -> DbResult<Option<Quote>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE quotes SET status = $3,
                            sent_at = CASE WHEN $4 THEN now() ELSE sent_at END,
                            decided_at = CASE WHEN $5 THEN now() ELSE decided_at END,
                            updated_at = now()
         WHERE org_id = $1 AND id = $2 AND status = $6
         RETURNING {COLUMNS}"
    );
    let row = sqlx::query_as::<_, QuoteRow>(&sql)
        .bind(org_id)
        .bind(id)
        .bind(to)
        .bind(stamp_sent)
        .bind(stamp_decided)
        .bind(from)
        .fetch_optional(executor)
        .await?;
    Ok(row.map(quote_row))
}

/// Records the convert-to-invoice seam's targets. Guarded in SQL: only an
/// ACCEPTED quote that has not been converted yet converts.
pub async fn mark_converted<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    contract_id: Uuid,
    invoice_id: Uuid,
) -> DbResult<Option<Quote>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE quotes SET status = 'accepted', converted_contract_id = $3,
                            converted_invoice_id = $4, converted_at = now(), updated_at = now()
         WHERE org_id = $1 AND id = $2 AND status = 'accepted'
           AND converted_invoice_id IS NULL
         RETURNING {COLUMNS}"
    );
    let row = sqlx::query_as::<_, QuoteRow>(&sql)
        .bind(org_id)
        .bind(id)
        .bind(contract_id)
        .bind(invoice_id)
        .fetch_optional(executor)
        .await?;
    Ok(row.map(quote_row))
}

// --- lines ---------------------------------------------------------------------

pub async fn insert_lines<'e, E>(executor: E, lines: &[QuoteLine]) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    if lines.is_empty() {
        return Ok(());
    }
    let ids: Vec<Uuid> = lines.iter().map(|l| l.id).collect();
    let orgs: Vec<Uuid> = lines.iter().map(|l| l.org_id).collect();
    let quotes: Vec<Uuid> = lines.iter().map(|l| l.quote_id).collect();
    let items: Vec<Option<Uuid>> = lines.iter().map(|l| l.catalog_item_id).collect();
    let descriptions: Vec<String> = lines.iter().map(|l| l.description.clone()).collect();
    let units: Vec<String> = lines.iter().map(|l| l.unit.clone()).collect();
    let quantities: Vec<i64> = lines.iter().map(|l| l.quantity_milli).collect();
    let units_amounts: Vec<i64> = lines.iter().map(|l| l.unit_amount_minor).collect();
    let taxables: Vec<bool> = lines.iter().map(|l| l.taxable).collect();
    let amounts: Vec<i64> = lines.iter().map(|l| l.amount_minor).collect();
    let positions: Vec<i32> = lines.iter().enumerate().map(|(i, _)| i as i32).collect();
    sqlx::query(
        r#"INSERT INTO quote_lines (id, org_id, quote_id, catalog_item_id, description, unit,
                                    quantity_milli, unit_amount_minor, taxable, amount_minor,
                                    position)
           SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::uuid[], $4::uuid[], $5::text[],
                                $6::text[], $7::bigint[], $8::bigint[], $9::boolean[],
                                $10::bigint[], $11::int[])"#,
    )
    .bind(&ids)
    .bind(&orgs)
    .bind(&quotes)
    .bind(&items)
    .bind(&descriptions)
    .bind(&units)
    .bind(&quantities)
    .bind(&units_amounts)
    .bind(&taxables)
    .bind(&amounts)
    .bind(&positions)
    .execute(executor)
    .await?;
    Ok(())
}

/// Replaces a draft quote's lines wholesale (the edit model: PATCH
/// carries the full line list). The DELETE is guarded to draft quotes IN
/// SQL, so a line never vanishes from an immutable (sent+) quote.
pub async fn replace_lines(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    quote_id: Uuid,
    lines: &[QuoteLine],
) -> DbResult<()> {
    sqlx::query(
        "DELETE FROM quote_lines
         WHERE org_id = $1 AND quote_id = $2
           AND EXISTS (SELECT 1 FROM quotes q
                       WHERE q.org_id = $1 AND q.id = $2 AND q.status = 'draft')",
    )
    .bind(org_id)
    .bind(quote_id)
    .execute(&mut *executor)
    .await?;
    insert_lines(&mut *executor, lines).await
}

pub async fn list_lines<'e, E>(
    executor: E,
    org_id: Uuid,
    quote_id: Uuid,
) -> DbResult<Vec<QuoteLine>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    #[derive(Debug, sqlx::FromRow)]
    struct LineRow {
        id: Uuid,
        org_id: Uuid,
        quote_id: Uuid,
        catalog_item_id: Option<Uuid>,
        description: String,
        unit: String,
        quantity_milli: i64,
        unit_amount_minor: i64,
        taxable: bool,
        amount_minor: i64,
        position: i32,
    }
    let rows = sqlx::query_as::<_, LineRow>(
        r#"SELECT id, org_id, quote_id, catalog_item_id, description, unit, quantity_milli,
                  unit_amount_minor, taxable, amount_minor, position
           FROM quote_lines
           WHERE org_id = $1 AND quote_id = $2
           ORDER BY position, id"#,
    )
    .bind(org_id)
    .bind(quote_id)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| QuoteLine {
            id: r.id,
            org_id: r.org_id,
            quote_id: r.quote_id,
            catalog_item_id: r.catalog_item_id,
            description: r.description,
            unit: r.unit,
            quantity_milli: r.quantity_milli,
            unit_amount_minor: r.unit_amount_minor,
            taxable: r.taxable,
            amount_minor: r.amount_minor,
            position: r.position,
        })
        .collect())
}

// --- client (portal) scope -----------------------------------------------------
//
// Portal-visible quotes: the caller's client's non-draft documents only.
// A quote belonging to another client, or still in draft, is
// indistinguishable from missing by design (no existence leak).

/// All non-draft quotes of one client, newest first (portal list).
pub async fn list_client_quotes<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
    limit: i64,
) -> DbResult<Vec<Quote>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {COLUMNS} FROM quotes
         WHERE org_id = $1 AND client_id = $2 AND status <> 'draft'
         ORDER BY created_at DESC, id DESC
         LIMIT $3"
    );
    let rows = sqlx::query_as::<_, QuoteRow>(&sql)
        .bind(org_id)
        .bind(client_id)
        .bind(limit)
        .fetch_all(executor)
        .await?;
    Ok(rows.into_iter().map(quote_row).collect())
}

/// One quote under the same client-scope contract as
/// [`list_client_quotes`]; `DbError::NotFound` when the document belongs
/// to another client or is still a draft.
pub async fn get_client_quote<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
    quote_id: Uuid,
) -> DbResult<Quote>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {COLUMNS} FROM quotes
         WHERE org_id = $1 AND client_id = $2 AND id = $3 AND status <> 'draft'"
    );
    let row = sqlx::query_as::<_, QuoteRow>(&sql)
        .bind(org_id)
        .bind(client_id)
        .bind(quote_id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(quote_row(row))
}
