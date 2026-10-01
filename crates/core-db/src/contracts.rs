//! Contracts + rate cards + fixed-fee schedules (TASKLIST Phase 3,
//! Wave 8b billing lane).
//!
//! Typed queries over migration 0029's tables. Every read carries an
//! explicit `org_id` predicate (same posture as the rest of the crate);
//! billing-rule evaluation reads billable time entries for one client.

use crate::{DbError, DbResult};
use core_domain::entities::{
    Contract, ContractBlock, ContractCap, ContractChangeOrder, ContractEscalator, ContractFixedFee,
    ContractRate, ContractSlaCredit, ContractType, UnitBasis,
};
use sqlx::Postgres;
use std::str::FromStr;
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
                                   hourly_rate_minor, included_minutes, overage_rate_minor,
                                   unit_price_minor, unit_basis, incident_price_minor,
                                   incident_priority, po_number, notes)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16,
                   $17, $18, $19, $20, $21, $22)
           RETURNING id, org_id, client_id, number, name, contract_type, status, currency,
                      effective_from, effective_until, billing_frequency, billing_anchor,
                      amount_minor, hourly_rate_minor, included_minutes, overage_rate_minor,
                      unit_price_minor, unit_basis, incident_price_minor, incident_priority,
                      po_number, notes, created_at, updated_at"#,
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
        contract.unit_price_minor,
        contract.unit_basis.map(|b| b.as_str()),
        contract.incident_price_minor,
        contract.incident_priority,
        contract.po_number,
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
                  unit_price_minor, unit_basis, incident_price_minor, incident_priority,
                  po_number, notes, created_at, updated_at
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
                  unit_price_minor, unit_basis, incident_price_minor, incident_priority,
                  po_number, notes, created_at, updated_at
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
                  unit_price_minor, unit_basis, incident_price_minor, incident_priority,
                  po_number, notes, created_at, updated_at
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
                                overage_rate_minor = $16, unit_price_minor = $17,
                                unit_basis = $18, incident_price_minor = $19,
                                incident_priority = $20, po_number = $21,
                                notes = $22, updated_at = now()
           WHERE org_id = $1 AND id = $2
           RETURNING id, org_id, client_id, number, name, contract_type, status, currency,
                      effective_from, effective_until, billing_frequency, billing_anchor,
                      amount_minor, hourly_rate_minor, included_minutes, overage_rate_minor,
                      unit_price_minor, unit_basis, incident_price_minor, incident_priority,
                      po_number, notes, created_at, updated_at"#,
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
        contract.unit_price_minor,
        contract.unit_basis.map(|b| b.as_str()),
        contract.incident_price_minor,
        contract.incident_priority,
        contract.po_number,
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
    // Wave 39: the card keys `work_type` onto the same category slugs
    // entries now carry, so normalize each card row through the org's
    // kept mapping (exact slug, then alias). A value with no category
    // keeps its own trimmed key — an admin's custom bucket still
    // applies, and it simply matches entries carrying the same text.
    let mut work_types: Vec<String> = Vec::with_capacity(rates.len());
    for r in rates {
        let normalized =
            crate::time_categories::resolve_work_type_slug(&mut *executor, org_id, &r.work_type)
                .await?;
        work_types.push(normalized.unwrap_or_else(|| r.work_type.trim().to_string()));
    }
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

/// Billable, APPROVED time entries for one client within the inclusive
/// date window, ascending by start time (the engine re-buckets
/// deterministically). Approval state is carried into billing reads (Wave
/// 14 lane 1): unapproved work is never priced, so previews match what an
/// invoice run will actually bill. Verification adjustments (Wave 39 lane
/// 2, report V2) price the effective minutes — the raw minutes less the
/// signed adjustment ledger.
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
    // Wave 39 lane 2 (report V2): verification adjustments price the
    // EFFECTIVE minutes, not the raw ones. The adjustment ledger is
    // additive, so the original `te.minutes` is never rewritten —
    // priced minutes are `te.minutes + Σ delta` clamped at zero (the
    // domain rule already bars an overshoot). A fully-zeroed entry
    // drops out of pricing entirely: a zero-minute line is a line
    // nobody wants on an invoice.
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
    unit_price_minor: Option<i64>,
    unit_basis: Option<String>,
    incident_price_minor: Option<i64>,
    incident_priority: Option<String>,
    po_number: Option<String>,
    notes: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn contract_row(row: ContractRow) -> Contract {
    use core_domain::entities::{BillingFrequency, ContractStatus};
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
        unit_price_minor: row.unit_price_minor,
        unit_basis: row
            .unit_basis
            .map(|b| UnitBasis::from_str(&b).expect("CHECK constraint guarantees a valid basis")),
        incident_price_minor: row.incident_price_minor,
        incident_priority: row.incident_priority,
        po_number: row.po_number,
        notes: row.notes,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

// --- Wave 39 aggregate tables (blocks / caps / minimums / escalators /
// change orders / SLA credits): managed as part of the contract
// aggregate like rates and fees — replace semantics inside the mutation
// transaction, no standalone CRUD surfaces. Change orders and SLA
// credits are the exception: they carry a LIFECYCLE, so they get
// insert + status-transition reads/writes instead of replace.

pub async fn replace_blocks(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    contract_id: Uuid,
    blocks: &[ContractBlock],
) -> DbResult<()> {
    sqlx::query!(
        "DELETE FROM contract_blocks WHERE org_id = $1 AND contract_id = $2",
        org_id,
        contract_id
    )
    .execute(&mut *executor)
    .await?;
    let ids: Vec<Uuid> = blocks.iter().map(|b| b.id).collect();
    let orgs: Vec<Uuid> = blocks.iter().map(|_| org_id).collect();
    let contracts: Vec<Uuid> = blocks.iter().map(|_| contract_id).collect();
    let labels: Vec<String> = blocks.iter().map(|b| b.label.clone()).collect();
    let totals: Vec<i32> = blocks.iter().map(|b| b.total_minutes).collect();
    let prices: Vec<i64> = blocks.iter().map(|b| b.sale_price_minor).collect();
    let rates: Vec<Option<i64>> = blocks.iter().map(|b| b.block_rate_minor).collect();
    let expiries: Vec<Option<chrono::NaiveDate>> = blocks.iter().map(|b| b.expires_on).collect();
    let policies: Vec<String> = blocks.iter().map(|b| b.rollover_policy.clone()).collect();
    sqlx::query!(
        r#"INSERT INTO contract_blocks (id, org_id, contract_id, label, total_minutes,
                                        sale_price_minor, block_rate_minor, expires_on,
                                        rollover_policy)
           SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::uuid[], $4::text[], $5::int[],
                                $6::bigint[], $7::bigint[], $8::date[], $9::text[])"#,
        &ids,
        &orgs,
        &contracts,
        &labels,
        &totals,
        &prices,
        &rates as &[Option<i64>],
        &expiries as &[Option<chrono::NaiveDate>],
        &policies,
    )
    .execute(&mut *executor)
    .await?;
    Ok(())
}

pub async fn list_blocks<'e, E>(
    executor: E,
    org_id: Uuid,
    contract_id: Uuid,
) -> DbResult<Vec<ContractBlock>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, contract_id, label, total_minutes, sale_price_minor,
                  block_rate_minor, expires_on, rollover_policy, created_at
           FROM contract_blocks WHERE org_id = $1 AND contract_id = $2
           ORDER BY created_at, label"#,
        org_id,
        contract_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ContractBlock {
            id: r.id,
            org_id: r.org_id,
            contract_id: r.contract_id,
            label: r.label,
            total_minutes: r.total_minutes,
            sale_price_minor: r.sale_price_minor,
            block_rate_minor: r.block_rate_minor,
            expires_on: r.expires_on,
            rollover_policy: r.rollover_policy,
            created_at: r.created_at,
        })
        .collect())
}

/// Persisted draw-downs for one contract's blocks (the verification
/// surface's remaining-balance source of truth).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDrawRow {
    pub block_id: Uuid,
    pub time_entry_id: Uuid,
    pub invoice_id: Option<Uuid>,
    pub minutes: i32,
    pub drawn_on: chrono::NaiveDate,
}

pub async fn list_block_draws<'e, E>(
    executor: E,
    org_id: Uuid,
    contract_id: Uuid,
) -> DbResult<Vec<BlockDrawRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT d.block_id, d.time_entry_id, d.invoice_id, d.minutes, d.drawn_on
           FROM contract_block_draws d
           JOIN contract_blocks b ON b.id = d.block_id
           WHERE d.org_id = $1 AND b.contract_id = $2
           ORDER BY d.drawn_on, d.id"#,
        org_id,
        contract_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BlockDrawRow {
            block_id: r.block_id,
            time_entry_id: r.time_entry_id,
            invoice_id: r.invoice_id,
            minutes: r.minutes,
            drawn_on: r.drawn_on,
        })
        .collect())
}

pub async fn replace_caps(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    contract_id: Uuid,
    caps: &[ContractCap],
) -> DbResult<()> {
    sqlx::query!(
        "DELETE FROM contract_caps WHERE org_id = $1 AND contract_id = $2",
        org_id,
        contract_id
    )
    .execute(&mut *executor)
    .await?;
    let ids: Vec<Uuid> = caps.iter().map(|c| c.id).collect();
    let orgs: Vec<Uuid> = caps.iter().map(|_| org_id).collect();
    let contracts: Vec<Uuid> = caps.iter().map(|_| contract_id).collect();
    let scopes: Vec<String> = caps.iter().map(|c| c.scope.clone()).collect();
    let cap_minors: Vec<i64> = caps.iter().map(|c| c.cap_minor).collect();
    let behaviors: Vec<String> = caps
        .iter()
        .map(|c| c.reached_behavior.as_str().to_string())
        .collect();
    let overages: Vec<Option<i64>> = caps.iter().map(|c| c.overage_rate_minor).collect();
    let work_types: Vec<Option<String>> = caps.iter().map(|c| c.work_type.clone()).collect();
    sqlx::query!(
        r#"INSERT INTO contract_caps (id, org_id, contract_id, scope, cap_minor,
                                      reached_behavior, overage_rate_minor, work_type)
           SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::uuid[], $4::text[], $5::bigint[],
                                $6::text[], $7::bigint[], $8::text[])"#,
        &ids,
        &orgs,
        &contracts,
        &scopes,
        &cap_minors,
        &behaviors,
        &overages as &[Option<i64>],
        &work_types as &[Option<String>],
    )
    .execute(&mut *executor)
    .await?;
    Ok(())
}

pub async fn list_caps<'e, E>(
    executor: E,
    org_id: Uuid,
    contract_id: Uuid,
) -> DbResult<Vec<ContractCap>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, contract_id, scope, cap_minor, reached_behavior,
                  overage_rate_minor, work_type, created_at
           FROM contract_caps WHERE org_id = $1 AND contract_id = $2
           ORDER BY scope, work_type NULLS FIRST, created_at"#,
        org_id,
        contract_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ContractCap {
            id: r.id,
            org_id: r.org_id,
            contract_id: r.contract_id,
            scope: r.scope,
            cap_minor: r.cap_minor,
            reached_behavior: core_domain::entities::CapReachedBehavior::from_str(
                &r.reached_behavior,
            )
            .expect("CHECK constraint guarantees a valid cap behavior"),
            overage_rate_minor: r.overage_rate_minor,
            work_type: r.work_type,
            created_at: r.created_at,
        })
        .collect())
}

/// One contract's minimum commitment, if any.
pub async fn get_minimum<'e, E>(
    executor: E,
    org_id: Uuid,
    contract_id: Uuid,
) -> DbResult<Option<i64>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT minimum_commitment_minor FROM contract_minimums
         WHERE org_id = $1 AND contract_id = $2",
    )
    .bind(org_id)
    .bind(contract_id)
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|r| r.0))
}

pub async fn set_minimum(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    contract_id: Uuid,
    minimum_commitment_minor: Option<i64>,
) -> DbResult<()> {
    if minimum_commitment_minor.is_none() {
        sqlx::query!(
            "DELETE FROM contract_minimums WHERE org_id = $1 AND contract_id = $2",
            org_id,
            contract_id
        )
        .execute(&mut *executor)
        .await?;
        return Ok(());
    }
    sqlx::query!(
        r#"INSERT INTO contract_minimums (id, org_id, contract_id, minimum_commitment_minor)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (contract_id) DO UPDATE
             SET minimum_commitment_minor = EXCLUDED.minimum_commitment_minor"#,
        Uuid::now_v7(),
        org_id,
        contract_id,
        minimum_commitment_minor,
    )
    .execute(&mut *executor)
    .await?;
    Ok(())
}

pub async fn get_escalator<'e, E>(
    executor: E,
    org_id: Uuid,
    contract_id: Uuid,
) -> DbResult<Option<ContractEscalator>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, contract_id, uplift_bps, first_uplift_on, created_at
           FROM contract_escalators WHERE org_id = $1 AND contract_id = $2"#,
        org_id,
        contract_id
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|r| ContractEscalator {
        id: r.id,
        org_id: r.org_id,
        contract_id: r.contract_id,
        uplift_bps: r.uplift_bps,
        first_uplift_on: r.first_uplift_on,
        created_at: r.created_at,
    }))
}

pub async fn set_escalator(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    contract_id: Uuid,
    escalator: Option<&ContractEscalator>,
) -> DbResult<()> {
    if escalator.is_none() {
        sqlx::query!(
            "DELETE FROM contract_escalators WHERE org_id = $1 AND contract_id = $2",
            org_id,
            contract_id
        )
        .execute(&mut *executor)
        .await?;
        return Ok(());
    }
    let e = escalator.expect("checked above");
    sqlx::query!(
        r#"INSERT INTO contract_escalators (id, org_id, contract_id, uplift_bps, first_uplift_on)
           VALUES ($1, $2, $3, $4, $5)
           ON CONFLICT (contract_id) DO UPDATE
             SET uplift_bps = EXCLUDED.uplift_bps,
                 first_uplift_on = EXCLUDED.first_uplift_on"#,
        e.id,
        org_id,
        contract_id,
        e.uplift_bps,
        e.first_uplift_on,
    )
    .execute(&mut *executor)
    .await?;
    Ok(())
}

/// Change orders: lifecycle entities, insert + transition — never
/// replaced wholesale.
pub async fn insert_change_order<'e, E>(executor: E, order: &ContractChangeOrder) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO contract_change_orders (id, org_id, contract_id, number, title,
                                               description, amount_minor, status,
                                               approved_by, approved_at, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
        order.id,
        order.org_id,
        order.contract_id,
        order.number,
        order.title,
        order.description,
        order.amount_minor,
        order.status,
        order.approved_by,
        order.approved_at,
        order.created_by,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn list_change_orders<'e, E>(
    executor: E,
    org_id: Uuid,
    contract_id: Uuid,
) -> DbResult<Vec<ContractChangeOrder>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, contract_id, number, title, description, amount_minor, status,
                  approved_by, approved_at, billed_invoice_id, created_by, created_at, updated_at
           FROM contract_change_orders WHERE org_id = $1 AND contract_id = $2
           ORDER BY number"#,
        org_id,
        contract_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ContractChangeOrder {
            id: r.id,
            org_id: r.org_id,
            contract_id: r.contract_id,
            number: r.number,
            title: r.title,
            description: r.description,
            amount_minor: r.amount_minor,
            status: r.status,
            approved_by: r.approved_by,
            approved_at: r.approved_at,
            billed_invoice_id: r.billed_invoice_id,
            created_by: r.created_by,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect())
}

/// Transition a change order's status. Typed `DbError::NotFound` when
/// the row is not visible in scope; a stale transition is the API
/// layer's typed conflict.
/// A change-order transition's stamped fields (who/when approved, which
/// invoice billed). `None` fields leave the stored value untouched.
#[derive(Debug, Clone, Default)]
pub struct ChangeOrderTransition<'a> {
    pub status: &'a str,
    pub approved_by: Option<Uuid>,
    pub approved_at: Option<chrono::DateTime<chrono::Utc>>,
    pub billed_invoice_id: Option<Uuid>,
}

#[allow(clippy::too_many_arguments)]
pub async fn update_change_order_status<'e, E>(
    executor: E,
    org_id: Uuid,
    contract_id: Uuid,
    id: Uuid,
    status: &str,
    approved_by: Option<Uuid>,
    approved_at: Option<chrono::DateTime<chrono::Utc>>,
    billed_invoice_id: Option<Uuid>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        r#"UPDATE contract_change_orders SET status = $4,
              approved_by = COALESCE($5, approved_by),
              approved_at = COALESCE($6, approved_at),
              billed_invoice_id = COALESCE($7, billed_invoice_id),
              updated_at = now()
           WHERE org_id = $1 AND contract_id = $2 AND id = $3"#,
        org_id,
        contract_id,
        id,
        status,
        approved_by,
        approved_at,
        billed_invoice_id,
    )
    .execute(executor)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

pub async fn insert_sla_credit<'e, E>(executor: E, credit: &ContractSlaCredit) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO contract_sla_credits (id, org_id, contract_id, ticket_id, reason,
                                             amount_minor, currency, applies_on, status,
                                             created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
        credit.id,
        credit.org_id,
        credit.contract_id,
        credit.ticket_id,
        credit.reason,
        credit.amount_minor,
        credit.currency,
        credit.applies_on,
        credit.status,
        credit.created_by,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn list_sla_credits<'e, E>(
    executor: E,
    org_id: Uuid,
    contract_id: Uuid,
) -> DbResult<Vec<ContractSlaCredit>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, contract_id, ticket_id, reason, amount_minor, currency,
                  applies_on, status, applied_invoice_id, created_by, created_at, updated_at
           FROM contract_sla_credits WHERE org_id = $1 AND contract_id = $2
           ORDER BY applies_on, created_at"#,
        org_id,
        contract_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ContractSlaCredit {
            id: r.id,
            org_id: r.org_id,
            contract_id: r.contract_id,
            ticket_id: r.ticket_id,
            reason: r.reason,
            amount_minor: r.amount_minor,
            currency: r.currency,
            applies_on: r.applies_on,
            status: r.status,
            applied_invoice_id: r.applied_invoice_id,
            created_by: r.created_by,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect())
}

/// Marks the credits a run applied (status pending → applied with the
/// consuming invoice). Driving the ids from the run's evaluation keeps
/// this honest: a credit another window consumed is invisible here.
pub async fn mark_sla_credits_applied<'e, E>(
    executor: E,
    org_id: Uuid,
    credit_ids: &[Uuid],
    invoice_id: Uuid,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    if credit_ids.is_empty() {
        return Ok(());
    }
    sqlx::query!(
        r#"UPDATE contract_sla_credits SET status = 'applied',
              applied_invoice_id = $3, updated_at = now()
           WHERE org_id = $1 AND id = ANY($2)"#,
        org_id,
        credit_ids,
        invoice_id,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Per-unit evaluation inputs: the client's active device-asset count
/// (basis = devices) or the org user count (basis = users), read at
/// evaluation time as the contract (C) requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitCountRow {
    pub devices: i64,
    pub users: i64,
}

pub async fn count_units(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<UnitCountRow> {
    let devices: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM device_assets WHERE org_id = $1 AND client_id = $2
         AND status = 'active'",
    )
    .bind(org_id)
    .bind(client_id)
    .fetch_one(&mut *executor)
    .await?;
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE org_id = $1")
        .bind(org_id)
        .fetch_one(&mut *executor)
        .await?;
    Ok(UnitCountRow { devices, users })
}

/// Per-incident evaluation inputs: closed tickets for the contract's
/// client whose resolution date falls in the window, optionally banded
/// to one priority. Ordered deterministically (resolved_at, id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedTicketRow {
    pub ticket_id: Uuid,
    pub number: String,
    pub priority: String,
    pub resolved_on: chrono::NaiveDate,
}

pub async fn list_closed_tickets<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
    priority: Option<&str>,
) -> DbResult<Vec<ClosedTicketRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, number, priority, resolved_at::date AS "resolved_on!"
           FROM tickets
           WHERE org_id = $1 AND client_id = $2 AND category = 'closed'
             AND resolved_at IS NOT NULL
             AND resolved_at::date >= $3 AND resolved_at::date <= $4
             AND ($5::text IS NULL OR priority = $5)
           ORDER BY resolved_at, id"#,
        org_id,
        client_id,
        from,
        to,
        priority,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ClosedTicketRow {
            ticket_id: r.id,
            number: r.number,
            priority: r.priority,
            resolved_on: r.resolved_on,
        })
        .collect())
}

/// Persists one block draw (run-time, additive ledger). The
/// (block, time_entry) uniqueness keeps a re-run from double-drawing the
/// same entry; the invoice link points at the consuming document.
pub async fn insert_block_draw(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    block_id: Uuid,
    time_entry_id: Uuid,
    invoice_id: Uuid,
    minutes: i32,
) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO contract_block_draws (id, org_id, block_id, time_entry_id,
                                             invoice_id, minutes, drawn_on)
           VALUES ($1, $2, $3, $4, $5, $6, now())"#,
        Uuid::now_v7(),
        org_id,
        block_id,
        time_entry_id,
        invoice_id,
        minutes,
    )
    .execute(&mut *executor)
    .await?;
    Ok(())
}

/// Withdraws a PENDING credit (cancelled). Applied credits are
/// immutable — the caller refuses them before this runs.
pub async fn cancel_sla_credit(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    credit_id: Uuid,
) -> DbResult<()> {
    sqlx::query!(
        "UPDATE contract_sla_credits SET status = 'cancelled', updated_at = now()
         WHERE org_id = $1 AND id = $2",
        org_id,
        credit_id
    )
    .execute(&mut *executor)
    .await?;
    Ok(())
}
