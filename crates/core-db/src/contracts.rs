//! Contracts + rate cards + fixed-fee schedules (TASKLIST Phase 3,
//! Wave 8b billing lane).
//!
//! Typed queries over migration 0029's tables. Every read carries an
//! explicit `org_id` predicate (same posture as the rest of the crate);
//! billing-rule evaluation reads billable time entries for one client.

use crate::{DbError, DbResult};
use core_domain::entities::{Contract, ContractFixedFee, ContractRate};
use sqlx::Postgres;
use uuid::Uuid;

/// Filters for [`list_contracts`]; `None` fields are omitted.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ContractFilters {
    pub client_id: Option<Uuid>,
    /// Exact status match (`draft|active|ended|cancelled`).
    pub status: Option<String>,
    /// Effective dating: contracts whose `[effective_from, effective_until]`
    /// window contains this date (open-ended contracts always match).
    pub effective_on: Option<chrono::NaiveDate>,
}

/// Contracts excluded from billing-rule evaluation by [`list_billable_contracts`].
pub const CANCELLED: &str = "cancelled";

#[allow(clippy::too_many_arguments)]
pub async fn create_contract<'e, E>(executor: E, contract: &Contract) -> DbResult<Contract>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as!(
        ContractRow,
        r#"INSERT INTO contracts (id, org_id, client_id, number, name, contract_type,
                                   status, currency, effective_from, effective_until,
                                   billing_frequency, billing_anchor, amount_minor,
                                   hourly_rate_minor, included_minutes, overage_rate_minor, notes)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
           RETURNING id, org_id, client_id, number, name, contract_type, status, currency,
                     effective_from, effective_until, billing_frequency, billing_anchor,
                     amount_minor, hourly_rate_minor, included_minutes, overage_rate_minor,
                     notes, created_at, updated_at"#,
        contract.id,
        contract.org_id,
        contract.client_id,
        contract.number,
        contract.name,
        contract.contract_type.as_str(),
        contract.status.as_str(),
        contract.currency,
        contract.effective_from,
        contract.effective_until,
        contract.billing_frequency.map(|f| f.as_str()),
        contract.billing_anchor,
        contract.amount_minor,
        contract.hourly_rate_minor,
        contract.included_minutes,
        contract.overage_rate_minor,
        contract.notes,
    )
    .fetch_one(executor)
    .await?;
    Ok(contract_row(row))
}

pub async fn get_contract<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Contract>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as!(
        ContractRow,
        r#"SELECT id, org_id, client_id, number, name, contract_type, status, currency,
                  effective_from, effective_until, billing_frequency, billing_anchor,
                  amount_minor, hourly_rate_minor, included_minutes, overage_rate_minor,
                  notes, created_at, updated_at
           FROM contracts WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(contract_row(row))
}

pub async fn list_contracts<'e, E>(
    executor: E,
    org_id: Uuid,
    filters: &ContractFilters,
) -> DbResult<Vec<Contract>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as!(
        ContractRow,
        r#"SELECT id, org_id, client_id, number, name, contract_type, status, currency,
                  effective_from, effective_until, billing_frequency, billing_anchor,
                  amount_minor, hourly_rate_minor, included_minutes, overage_rate_minor,
                  notes, created_at, updated_at
           FROM contracts
           WHERE org_id = $1
             AND ($2::uuid IS NULL OR client_id = $2)
             AND ($3::text IS NULL OR status = $3)
             AND ($4::date IS NULL OR (effective_from <= $4
                  AND (effective_until IS NULL OR effective_until >= $4)))
           ORDER BY number"#,
        org_id,
        filters.client_id,
        filters.status.as_deref(),
        filters.effective_on,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(contract_row).collect())
}

/// Contracts whose effective window intersects the evaluation window and
/// which are not cancelled — the population an invoice run (Wave 9) or a
/// preview would evaluate.
pub async fn list_billable_contracts<'e, E>(
    executor: E,
    org_id: Uuid,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
) -> DbResult<Vec<Contract>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as!(
        ContractRow,
        r#"SELECT id, org_id, client_id, number, name, contract_type, status, currency,
                  effective_from, effective_until, billing_frequency, billing_anchor,
                  amount_minor, hourly_rate_minor, included_minutes, overage_rate_minor,
                  notes, created_at, updated_at
           FROM contracts
           WHERE org_id = $1
             AND status <> $2
             AND effective_from <= $4
             AND (effective_until IS NULL OR effective_until >= $3)
           ORDER BY number"#,
        org_id,
        CANCELLED,
        from,
        to,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(contract_row).collect())
}

/// Whole-row update of the resolved contract (instant-first: only a fully
/// resolved entity persists). NotFound = foreign org or missing id.
#[allow(clippy::too_many_arguments)]
pub async fn update_contract<'e, E>(executor: E, contract: &Contract) -> DbResult<Contract>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as!(
        ContractRow,
        r#"UPDATE contracts SET client_id = $3, number = $4, name = $5, contract_type = $6,
                                status = $7, currency = $8, effective_from = $9,
                                effective_until = $10, billing_frequency = $11,
                                billing_anchor = $12, amount_minor = $13,
                                hourly_rate_minor = $14, included_minutes = $15,
                                overage_rate_minor = $16, notes = $17, updated_at = now()
           WHERE org_id = $1 AND id = $2
           RETURNING id, org_id, client_id, number, name, contract_type, status, currency,
                     effective_from, effective_until, billing_frequency, billing_anchor,
                     amount_minor, hourly_rate_minor, included_minutes, overage_rate_minor,
                     notes, created_at, updated_at"#,
        contract.org_id,
        contract.id,
        contract.client_id,
        contract.number,
        contract.name,
        contract.contract_type.as_str(),
        contract.status.as_str(),
        contract.currency,
        contract.effective_from,
        contract.effective_until,
        contract.billing_frequency.map(|f| f.as_str()),
        contract.billing_anchor,
        contract.amount_minor,
        contract.hourly_rate_minor,
        contract.included_minutes,
        contract.overage_rate_minor,
        contract.notes,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(contract_row(row))
}

pub async fn delete_contract<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        "DELETE FROM contracts WHERE org_id = $1 AND id = $2",
        org_id,
        id
    )
    .execute(executor)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

// --- rate card + fee schedule (managed as part of the contract aggregate,
// replaced wholesale by the API layer inside the mutation transaction) ----

pub async fn replace_rates(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    contract_id: Uuid,
    rates: &[ContractRate],
) -> DbResult<()> {
    sqlx::query!(
        "DELETE FROM contract_rates WHERE org_id = $1 AND contract_id = $2",
        org_id,
        contract_id
    )
    .execute(&mut *executor)
    .await?;
    // One bulk insert (empty slices insert nothing): rates replace
    // atomically with the contract mutation.
    let ids: Vec<Uuid> = rates.iter().map(|r| r.id).collect();
    let orgs: Vec<Uuid> = rates.iter().map(|_| org_id).collect();
    let contracts: Vec<Uuid> = rates.iter().map(|_| contract_id).collect();
    let work_types: Vec<String> = rates.iter().map(|r| r.work_type.clone()).collect();
    let rate_minors: Vec<i64> = rates.iter().map(|r| r.rate_minor).collect();
    sqlx::query!(
        r#"INSERT INTO contract_rates (id, org_id, contract_id, work_type, rate_minor)
           SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::uuid[], $4::text[], $5::bigint[])"#,
        &ids,
        &orgs,
        &contracts,
        &work_types,
        &rate_minors,
    )
    .execute(&mut *executor)
    .await?;
    Ok(())
}

pub async fn list_rates<'e, E>(
    executor: E,
    org_id: Uuid,
    contract_id: Uuid,
) -> DbResult<Vec<ContractRate>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, contract_id, work_type, rate_minor, created_at
           FROM contract_rates WHERE org_id = $1 AND contract_id = $2
           ORDER BY work_type"#,
        org_id,
        contract_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ContractRate {
            id: r.id,
            org_id: r.org_id,
            contract_id: r.contract_id,
            work_type: r.work_type,
            rate_minor: r.rate_minor,
            created_at: r.created_at,
        })
        .collect())
}

pub async fn replace_fees(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    contract_id: Uuid,
    fees: &[ContractFixedFee],
) -> DbResult<()> {
    sqlx::query!(
        "DELETE FROM contract_fixed_fees WHERE org_id = $1 AND contract_id = $2",
        org_id,
        contract_id
    )
    .execute(&mut *executor)
    .await?;
    // One bulk insert (empty slices insert nothing): the schedule
    // replaces atomically with the contract mutation.
    let ids: Vec<Uuid> = fees.iter().map(|f| f.id).collect();
    let orgs: Vec<Uuid> = fees.iter().map(|_| org_id).collect();
    let contracts: Vec<Uuid> = fees.iter().map(|_| contract_id).collect();
    let labels: Vec<String> = fees.iter().map(|f| f.label.clone()).collect();
    let amounts: Vec<i64> = fees.iter().map(|f| f.amount_minor).collect();
    let due_ons: Vec<chrono::NaiveDate> = fees.iter().map(|f| f.due_on).collect();
    let notes: Vec<Option<String>> = fees.iter().map(|f| f.notes.clone()).collect();
    sqlx::query!(
        r#"INSERT INTO contract_fixed_fees (id, org_id, contract_id, label, amount_minor, due_on, notes)
           SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::uuid[], $4::text[], $5::bigint[], $6::date[], $7::text[])"#,
        &ids,
        &orgs,
        &contracts,
        &labels,
        &amounts,
        &due_ons,
        &notes as &[Option<String>],
    )
    .execute(&mut *executor)
    .await?;
    Ok(())
}

pub async fn list_fees<'e, E>(
    executor: E,
    org_id: Uuid,
    contract_id: Uuid,
) -> DbResult<Vec<ContractFixedFee>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, contract_id, label, amount_minor, due_on, notes, created_at
           FROM contract_fixed_fees WHERE org_id = $1 AND contract_id = $2
           ORDER BY due_on, label"#,
        org_id,
        contract_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ContractFixedFee {
            id: r.id,
            org_id: r.org_id,
            contract_id: r.contract_id,
            label: r.label,
            amount_minor: r.amount_minor,
            due_on: r.due_on,
            notes: r.notes,
            created_at: r.created_at,
        })
        .collect())
}

// --- billing-rule inputs -------------------------------------------------

/// One billable time entry row for rule evaluation (joined to the
/// contract's client through its ticket).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BillableEntryRow {
    pub time_entry_id: Uuid,
    pub minutes: i32,
    pub work_type: Option<String>,
    pub entry_date: chrono::NaiveDate,
}

/// Billable time entries for one client within the inclusive date window,
/// ascending by start time (the engine re-buckets deterministically).
pub async fn list_billable_entries<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
) -> DbResult<Vec<BillableEntryRow>>
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
        .map(|r| BillableEntryRow {
            time_entry_id: r.id,
            minutes: r.minutes,
            work_type: r.work_type,
            entry_date: r.entry_date,
        })
        .collect())
}

/// The `contracts` row shape as `query_as!` sees it; mapped to the domain
/// entity once, in one place.
struct ContractRow {
    id: Uuid,
    org_id: Uuid,
    client_id: Uuid,
    number: String,
    name: String,
    contract_type: String,
    status: String,
    currency: String,
    effective_from: chrono::NaiveDate,
    effective_until: Option<chrono::NaiveDate>,
    billing_frequency: Option<String>,
    billing_anchor: Option<chrono::NaiveDate>,
    amount_minor: Option<i64>,
    hourly_rate_minor: Option<i64>,
    included_minutes: Option<i32>,
    overage_rate_minor: Option<i64>,
    notes: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn contract_row(row: ContractRow) -> Contract {
    use core_domain::entities::{BillingFrequency, ContractStatus, ContractType};
    use std::str::FromStr;
    let contract_type = ContractType::from_str(&row.contract_type)
        .expect("CHECK constraint guarantees a valid contract_type");
    let status =
        ContractStatus::from_str(&row.status).expect("CHECK constraint guarantees a valid status");
    let billing_frequency = row.billing_frequency.map(|f| {
        BillingFrequency::from_str(&f).expect("CHECK constraint guarantees a valid frequency")
    });
    Contract {
        id: row.id,
        org_id: row.org_id,
        client_id: row.client_id,
        number: row.number,
        name: row.name,
        contract_type,
        status,
        currency: row.currency,
        effective_from: row.effective_from,
        effective_until: row.effective_until,
        billing_frequency,
        billing_anchor: row.billing_anchor,
        amount_minor: row.amount_minor,
        hourly_rate_minor: row.hourly_rate_minor,
        included_minutes: row.included_minutes,
        overage_rate_minor: row.overage_rate_minor,
        notes: row.notes,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}
