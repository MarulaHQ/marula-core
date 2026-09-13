//! Role directory + preset lifecycle (Wave 13 lane 6, migration 0071).
//!
//! RB1 (ADR 0004): the shipped preset roles exist as DATA — `roles` rows
//! with a `builtin` tag and `is_system`, on the same permission matrix as
//! custom roles. Orgs fork a preset into a custom copy (`forked_from`
//! records the origin) and restore a preset row to its canonical grant
//! set; they never edit a preset in place. This module carries the read
//! model and the two lifecycle writes; the canonical grant definitions
//! live in `core_domain::policy::BuiltinRole`.
//!
//! Like the rest of `core-db`, functions are generic over `Executor` and
//! rely on the caller's transaction carrying `app.org_id` (RLS).

use crate::{DbError, DbResult};
use serde_json::Value;
use uuid::Uuid;

/// One `roles` row: a preset (builtin + is_system) or an org-created
/// custom role. `permissions` is the raw matrix vocabulary (`{"matrix":
/// {"module","action"}}` entries plus bare special strings).
#[derive(Debug, Clone)]
pub struct RoleEntry {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub builtin: Option<String>,
    pub is_system: bool,
    pub forked_from: Option<String>,
    pub permissions: Value,
}

fn entry_from_row(row: sqlx::postgres::PgRow) -> DbResult<RoleEntry> {
    use sqlx::Row;
    let permissions: Value = row.get("permissions");
    Ok(RoleEntry {
        id: row.get("id"),
        org_id: row.get("org_id"),
        name: row.get("name"),
        builtin: row.get("builtin"),
        is_system: row.get("is_system"),
        forked_from: row.get("forked_from"),
        permissions,
    })
}

const ROLE_COLS: &str = "id, org_id, name, builtin, is_system, forked_from, permissions";

/// Every role of the org, presets first (precedence order, migration
/// 0071's `preset_builtin_precedence`), then custom roles by name.
pub async fn list_roles(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
) -> DbResult<Vec<RoleEntry>> {
    let sql = format!(
        "SELECT {ROLE_COLS} FROM roles WHERE org_id = $1 \
         ORDER BY preset_builtin_precedence(builtin), name"
    );
    let rows = sqlx::query(&sql).bind(org_id).fetch_all(executor).await?;
    rows.into_iter().map(entry_from_row).collect()
}

pub async fn get_role(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    role_id: Uuid,
) -> DbResult<RoleEntry> {
    let sql = format!("SELECT {ROLE_COLS} FROM roles WHERE org_id = $1 AND id = $2");
    sqlx::query(&sql)
        .bind(org_id)
        .bind(role_id)
        .fetch_optional(executor)
        .await?
        .map(entry_from_row)
        .ok_or(DbError::NotFound)?
}

/// Inserts a forked custom copy of `origin`. `builtin` is always NULL on
/// the copy (a fork is never itself a preset); `forked_from` records the
/// preset tag when the origin was one.
pub async fn insert_forked_role(
    conn: &mut sqlx::PgConnection,
    id: Uuid,
    org_id: Uuid,
    name: &str,
    forked_from: Option<&str>,
    permissions: &Value,
) -> DbResult<RoleEntry> {
    let row = sqlx::query(
        "INSERT INTO roles (id, org_id, name, builtin, permissions, is_system, forked_from) \
         VALUES ($1, $2, $3, NULL, $4, false, $5) \
         RETURNING id, org_id, name, builtin, is_system, forked_from, permissions",
    )
    .bind(id)
    .bind(org_id)
    .bind(name)
    .bind(permissions)
    .bind(forked_from)
    .fetch_one(&mut *conn)
    .await?;
    entry_from_row(row)
}

/// Restores a PRESET row's permission array to the canonical set. Only
/// `is_system` rows with a `builtin` tag qualify — custom roles and forks
/// are org-owned data, not restorable presets (RLS keeps the org scope).
pub async fn restore_preset_permissions(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    role_id: Uuid,
    canonical: &Value,
) -> DbResult<RoleEntry> {
    let row = sqlx::query(
        "UPDATE roles SET permissions = $3, forked_from = NULL \
         WHERE org_id = $1 AND id = $2 AND is_system AND builtin IS NOT NULL \
         RETURNING id, org_id, name, builtin, is_system, forked_from, permissions",
    )
    .bind(org_id)
    .bind(role_id)
    .bind(canonical)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    entry_from_row(row)
}

/// How many ACTIVE staff hold the given role (the roles surface shows
/// assignment counts; zero holders is the "safe to prune" signal).
pub async fn count_holders(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    role_id: Uuid,
) -> DbResult<i64> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM users u \
         JOIN user_roles ur ON ur.user_id = u.id \
         WHERE u.org_id = $1 AND u.is_active AND ur.role_id = $2",
    )
    .bind(org_id)
    .bind(role_id)
    .fetch_one(executor)
    .await?;
    Ok(count)
}
