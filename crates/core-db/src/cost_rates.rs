//! Labour cost rates: the read and write surface for the two columns
//! migration 0188 added (`users.cost_rate_minor`, `orgs.default_cost_rate_minor`).
//!
//! Why these live outside the `User` and `Org` entities: both entities are
//! constructed as struct literals in dozens of places (the auth layer alone
//! builds ~44 `User { … }` values), so adding a field to either turns one
//! nullable column into a workspace-wide edit. The rate is payroll data that
//! only the settings screens and the margin read model touch, so it gets its
//! own narrow reads and writes and the entities stay as they are.
//!
//! Units: minor units of the org's base currency, per HOUR (ADR 0002, the
//! same unit as `contracts.hourly_rate_minor`). The columns are `integer`,
//! so this module speaks `i32`; the wire speaks `i64` and converts at the
//! route boundary after range validation.
//!
//! `None` means UNSET, never zero:
//!   * a user's `None` = "no personal rate, fall back to the org default";
//!   * an org's `None`  = "labour cost unknown".
//!
//! `Some(0)` is a real and different answer ("this work costs nothing"), and
//! `ops.v1_client_margin` costs it at zero rather than reporting it uncosted.

use crate::{DbError, DbResult};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

/// One user's stored rate, read back as written.
///
/// The `user_id` travels with the value because "no rate" and "no such user"
/// are different answers and both would otherwise be a bare `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserCostRate {
    pub user_id: Uuid,
    /// `None` = unset (fall back to the org default); `Some(0)` = free.
    pub cost_rate_minor: Option<i32>,
}

fn rate_from_row(row: &PgRow) -> DbResult<UserCostRate> {
    Ok(UserCostRate {
        user_id: row.try_get("id")?,
        cost_rate_minor: row.try_get("cost_rate_minor")?,
    })
}

/// Read one user's rate. The org filter is part of the statement, so a user
/// from another org is `NotFound`, not a cross-org read.
pub async fn user_cost_rate<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
) -> DbResult<UserCostRate>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query("SELECT id, cost_rate_minor FROM users WHERE id = $1 AND org_id = $2")
        .bind(user_id)
        .bind(org_id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    rate_from_row(&row)
}

/// Set or clear one user's rate. `None` clears it; `Some(0)` stores zero.
///
/// `updated_at` is deliberately NOT touched: the rate is payroll data, not a
/// profile change, and directory-sync code reads `updated_at` when deciding
/// what to push.
pub async fn set_user_cost_rate<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
    cost_rate_minor: Option<i32>,
) -> DbResult<UserCostRate>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query(
        "UPDATE users SET cost_rate_minor = $3
          WHERE id = $1 AND org_id = $2
          RETURNING id, cost_rate_minor",
    )
    .bind(user_id)
    .bind(org_id)
    .bind(cost_rate_minor)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    rate_from_row(&row)
}

/// Read the org-wide fallback. `None` = cost unknown, not zero.
pub async fn org_default_cost_rate<'e, E>(executor: E, org_id: Uuid) -> DbResult<Option<i32>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query("SELECT default_cost_rate_minor FROM orgs WHERE id = $1")
        .bind(org_id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row.try_get("default_cost_rate_minor")?)
}

/// Set or clear the org-wide fallback, returning what was stored.
pub async fn set_org_default_cost_rate<'e, E>(
    executor: E,
    org_id: Uuid,
    cost_rate_minor: Option<i32>,
) -> DbResult<Option<i32>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query(
        "UPDATE orgs SET default_cost_rate_minor = $2
          WHERE id = $1
          RETURNING default_cost_rate_minor",
    )
    .bind(org_id)
    .bind(cost_rate_minor)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row.try_get("default_cost_rate_minor")?)
}
