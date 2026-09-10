//! Credit notes persistence (TASKLIST Money+docs backend gap; migration
//! 0058): the credit document, its application ledger, and the
//! allocation counters.
//!
//! Every read and write carries an explicit `org_id` predicate on top of
//! the migration's RLS policies (same posture as the rest of the crate).
//! Allocation counters move through compare-and-set UPDATEs
//! (`remaining >= amount`) so concurrent applies/refunds can never
//! over-allocate a credit — the `None` return is the typed
//! "not enough credit left" signal the API maps to a 409.

use crate::{DbError, DbResult};
use core_domain::entities::{CreditNote, CreditNoteApplication, CreditNoteStatus};
use sqlx::Postgres;
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
struct CreditNoteRow {
    id: Uuid,
    org_id: Uuid,
    number: String,
    client_id: Uuid,
    issued_against_invoice_id: Option<Uuid>,
    status: String,
    currency: String,
    total_minor: i64,
    applied_minor: i64,
    refunded_minor: i64,
    reason: Option<String>,
    created_by: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn credit_note_row(r: CreditNoteRow) -> CreditNote {
    CreditNote {
        id: r.id,
        org_id: r.org_id,
        number: r.number,
        client_id: r.client_id,
        issued_against_invoice_id: r.issued_against_invoice_id,
        status: CreditNoteStatus::from_str(&r.status).unwrap_or(CreditNoteStatus::Issued),
        currency: r.currency,
        total_minor: r.total_minor,
        applied_minor: r.applied_minor,
        refunded_minor: r.refunded_minor,
        reason: r.reason,
        created_by: r.created_by,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

const COLUMNS: &str = "id, org_id, number, client_id, issued_against_invoice_id, status, currency,
                        total_minor, applied_minor, refunded_minor, reason, created_by,
                        created_at, updated_at";

pub async fn insert_credit_note<'e, E>(executor: E, note: &CreditNote) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO credit_notes (id, org_id, number, client_id, issued_against_invoice_id,
                                     status, currency, total_minor, applied_minor,
                                     refunded_minor, reason, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 0, 0, $9, $10)"#,
        note.id,
        note.org_id,
        note.number,
        note.client_id,
        note.issued_against_invoice_id,
        note.status.as_str(),
        note.currency,
        note.total_minor,
        note.reason,
        note.created_by,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_credit_note<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<CreditNote>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {COLUMNS} FROM credit_notes WHERE org_id = $1 AND id = $2");
    let row = sqlx::query_as::<_, CreditNoteRow>(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(credit_note_row(row))
}

pub struct CreditNoteFilters {
    pub client_id: Option<Uuid>,
    /// Exact status: `issued` | `void`.
    pub status: Option<String>,
}

pub async fn list_credit_notes<'e, E>(
    executor: E,
    org_id: Uuid,
    filters: &CreditNoteFilters,
    limit: i64,
) -> DbResult<Vec<CreditNote>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {COLUMNS} FROM credit_notes
         WHERE org_id = $1
           AND ($2::uuid IS NULL OR client_id = $2)
           AND ($3::text IS NULL OR status = $3)
         ORDER BY created_at DESC, id DESC
         LIMIT $4"
    );
    let rows = sqlx::query_as::<_, CreditNoteRow>(&sql)
        .bind(org_id)
        .bind(filters.client_id)
        .bind(filters.status.clone())
        .bind(limit)
        .fetch_all(executor)
        .await?;
    Ok(rows.into_iter().map(credit_note_row).collect())
}

/// Moves `amount` from remaining onto `applied_minor`. `None` means the
/// note is voided or does not have enough remaining credit.
pub async fn apply_credit<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    amount_minor: i64,
) -> DbResult<Option<CreditNote>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE credit_notes SET applied_minor = applied_minor + $3, updated_at = now()
         WHERE org_id = $1 AND id = $2 AND status = 'issued'
           AND total_minor - applied_minor - refunded_minor >= $3
         RETURNING {COLUMNS}"
    );
    let row = sqlx::query_as::<_, CreditNoteRow>(&sql)
        .bind(org_id)
        .bind(id)
        .bind(amount_minor)
        .fetch_optional(executor)
        .await?;
    Ok(row.map(credit_note_row))
}

/// Moves `amount` from remaining onto `refunded_minor`. `None` means the
/// note is voided or does not have enough remaining credit.
pub async fn refund_credit<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    amount_minor: i64,
) -> DbResult<Option<CreditNote>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE credit_notes SET refunded_minor = refunded_minor + $3, updated_at = now()
         WHERE org_id = $1 AND id = $2 AND status = 'issued'
           AND total_minor - applied_minor - refunded_minor >= $3
         RETURNING {COLUMNS}"
    );
    let row = sqlx::query_as::<_, CreditNoteRow>(&sql)
        .bind(org_id)
        .bind(id)
        .bind(amount_minor)
        .fetch_optional(executor)
        .await?;
    Ok(row.map(credit_note_row))
}

/// Voids a note that has no allocations. `None` means the note does not
/// exist, is already void, or has moved value (applied/refunded > 0).
pub async fn void_credit_note<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<Option<CreditNote>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE credit_notes SET status = 'void', updated_at = now()
         WHERE org_id = $1 AND id = $2 AND status = 'issued'
           AND applied_minor = 0 AND refunded_minor = 0
         RETURNING {COLUMNS}"
    );
    let row = sqlx::query_as::<_, CreditNoteRow>(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?;
    Ok(row.map(credit_note_row))
}

// --- application ledger --------------------------------------------------------

pub async fn insert_application<'e, E>(
    executor: E,
    application: &CreditNoteApplication,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO credit_note_applications (id, org_id, credit_note_id, invoice_id,
                                                 amount_minor, payment_id, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        application.id,
        application.org_id,
        application.credit_note_id,
        application.invoice_id,
        application.amount_minor,
        application.payment_id,
        application.created_by,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn list_applications<'e, E>(
    executor: E,
    org_id: Uuid,
    credit_note_id: Uuid,
) -> DbResult<Vec<CreditNoteApplication>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as!(
        CreditNoteApplication,
        r#"SELECT id, org_id, credit_note_id, invoice_id, amount_minor, payment_id,
                  created_by, created_at
           FROM credit_note_applications
           WHERE org_id = $1 AND credit_note_id = $2
           ORDER BY created_at DESC, id DESC"#,
        org_id,
        credit_note_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}
