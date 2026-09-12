//! FX persistence (ADR 0002, Wave 12 lane 10; migrations 0066/0067): the
//! append-only rate book, the effective-rate resolver, and the
//! reconciliation read model.
//!
//! Doctrine (ADR 0002):
//! * `fx_rates` is append-only — the migration's triggers refuse
//!   UPDATE/DELETE, and this module only ever INSERTs. Corrections are
//!   new rows; the effective rate for a lookup is the newest
//!   `captured_at` among rows for the pair whose `as_of_date` is the
//!   latest date ≤ the requested date.
//! * Rate rows are org-private (RLS org predicate + parameterized
//!   `org_id` on every query, like the rest of the crate).
//! * The read model sums base amounts from snapshots only; it never
//!   re-converts. Missing rates are a typed [`DbError::NotFound`]-free
//!   signal: [`resolve_rate`] returns `Ok(None)` and the API layer maps
//!   it to a typed 422 naming the missing pair (fail-closed manual
//!   mode — never an implicit 1:1).

use crate::{DbError, DbResult};
use core_domain::entities::{FxRateEntry, FxReconciliationRow};
use sqlx::Postgres;
use uuid::Uuid;

fn entry_from(row: FxRateRow) -> FxRateEntry {
    FxRateEntry {
        id: row.id,
        org_id: row.org_id,
        base_currency: row.base_currency,
        quote_currency: row.quote_currency,
        as_of_date: row.as_of_date,
        rate: row.rate,
        source: row.source,
        captured_at: row.captured_at,
        created_by: row.created_by,
    }
}

#[derive(Debug, sqlx::FromRow)]
struct FxRateRow {
    id: Uuid,
    org_id: Uuid,
    base_currency: String,
    quote_currency: String,
    as_of_date: chrono::NaiveDate,
    rate: String,
    source: String,
    captured_at: chrono::DateTime<chrono::Utc>,
    created_by: Option<Uuid>,
}

/// Appends one rate row (manual entry or a managed feed). The pair must
/// differ; shape/positivity are DB-checked. Returns the persisted entry.
/// (Eight parameters is the uniform append shape: executor, org, pair,
/// date, rate, source, author — the API layer wraps them.)
#[allow(clippy::too_many_arguments)]
pub async fn insert_rate<'e, E>(
    executor: E,
    org_id: Uuid,
    base_currency: &str,
    quote_currency: &str,
    as_of_date: chrono::NaiveDate,
    rate: &str,
    source: &str,
    created_by: Option<Uuid>,
) -> DbResult<FxRateEntry>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let id = Uuid::now_v7();
    let row = sqlx::query_as!(
        FxRateRow,
        r#"INSERT INTO fx_rates (id, org_id, base_currency, quote_currency, as_of_date, rate, source, created_by)
           VALUES ($1, $2, $3, $4, $5, $6::text::numeric, $7, $8)
           RETURNING id, org_id,
             base_currency AS "base_currency!", quote_currency AS "quote_currency!",
             as_of_date AS "as_of_date!", rate::text AS "rate!",
             source AS "source!", captured_at AS "captured_at!", created_by"#,
        id,
        org_id,
        base_currency,
        quote_currency,
        as_of_date,
        rate,
        source,
        created_by,
    )
    .fetch_one(executor)
    .await?;
    Ok(entry_from(row))
}

/// Lists the org's rate book, newest captures first, optionally filtered
/// to one directed pair.
pub async fn list_rates<'e, E>(
    executor: E,
    org_id: Uuid,
    base_currency: Option<&str>,
    quote_currency: Option<&str>,
    limit: i64,
) -> DbResult<Vec<FxRateEntry>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as!(
        FxRateRow,
        r#"SELECT id, org_id, base_currency, quote_currency, as_of_date, rate::text AS "rate!",
                  source AS "source!", captured_at AS "captured_at!", created_by
           FROM fx_rates
           WHERE org_id = $1
             AND ($2::text IS NULL OR base_currency = $2)
             AND ($3::text IS NULL OR quote_currency = $3)
           ORDER BY captured_at DESC, as_of_date DESC
           LIMIT $4"#,
        org_id,
        base_currency,
        quote_currency,
        limit,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(entry_from).collect())
}

/// Resolves the EFFECTIVE rate for one direction on one date:
/// (base, quote) stored directly, or the inverse pair (the resolver
/// inverts through the typed direction, never a UI-side 1/x). Latest
/// `as_of_date` ≤ requested wins; among equal dates, newest `captured_at`.
/// `Ok(None)` = no coverage — the caller fails the write with a typed
/// error naming the missing pair.
pub async fn resolve_rate(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    from_currency: &str,
    to_currency: &str,
    as_of: chrono::NaiveDate,
) -> DbResult<Option<FxRateEntry>> {
    // Directed coverage: stored pair matches the requested direction.
    let row = sqlx::query_as!(
        FxRateRow,
        r#"SELECT id, org_id, base_currency AS "base_currency!", quote_currency AS "quote_currency!",
                  as_of_date AS "as_of_date!", rate::text AS "rate!",
                  source AS "source!", captured_at AS "captured_at!", created_by
           FROM fx_rates
           WHERE org_id = $1 AND base_currency = $2 AND quote_currency = $3
             AND as_of_date <= $4
           ORDER BY as_of_date DESC, captured_at DESC
           LIMIT 1"#,
        org_id,
        from_currency,
        to_currency,
        as_of,
    )
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(r) = row {
        return Ok(Some(entry_from(r)));
    }
    // Inverse coverage: stored pair is (to, from); caller inverts.
    let row = sqlx::query_as!(
        FxRateRow,
        r#"SELECT id, org_id, base_currency AS "base_currency!", quote_currency AS "quote_currency!",
                  as_of_date AS "as_of_date!", rate::text AS "rate!",
                  source AS "source!", captured_at AS "captured_at!", created_by
           FROM fx_rates
           WHERE org_id = $1 AND base_currency = $2 AND quote_currency = $3
             AND as_of_date <= $4
           ORDER BY as_of_date DESC, captured_at DESC
           LIMIT 1"#,
        org_id,
        to_currency,
        from_currency,
        as_of,
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(entry_from))
}

/// The reconciliation read model (FX7): every cross-currency fact with
/// its immutable snapshot, newest first. Reads `ops.v1_fx_reconciliation`
/// (0067); org scoping rides the underlying tables' RLS plus the
/// explicit org predicate the view carries.
pub async fn reconciliation<'e, E>(
    executor: E,
    org_id: Uuid,
    limit: i64,
) -> DbResult<Vec<FxReconciliationRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows: Vec<FxReconciliationRow> = sqlx::query_as(
        r#"SELECT kind, fact_id, org_id, ticket_id, invoice_id,
                  original_currency, original_amount_minor,
                  fx_rate_used::text AS fx_rate_used, fx_rate_date, fx_source,
                  base_amount_minor, sync_target, fact_at
           FROM ops.v1_fx_reconciliation
           WHERE org_id = $1
           ORDER BY fact_at DESC, fact_id
           LIMIT $2"#,
    )
    .bind(org_id)
    .bind(limit)
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// The org base currency (FX1) — the join key every snapshot computation
/// needs. NotFound when the org row is gone.
pub async fn base_currency<'e, E>(executor: E, org_id: Uuid) -> DbResult<String>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_scalar!(r#"SELECT base_currency FROM orgs WHERE id = $1"#, org_id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)
}
