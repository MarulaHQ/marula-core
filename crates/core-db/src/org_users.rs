//! Org user directory + lifecycle (Wave 11 lane 2, migration 0047).
//!
//! Staff users are org-scoped rows in `users`; this module adds the
//! management read model (directory with resolved org role), the invite
//! flow (`user_invites` one-time setup tokens), and the activate /
//! deactivate / org-role mutations. Users are never deleted here —
//! deactivation (`is_active = false`) is the terminal state and the login
//! path already gates on it.
//!
//! Like the rest of `core-db`, functions are generic over `Executor` and
//! rely on the caller's transaction carrying `app.org_id` (RLS). The one
//! exception is `resolve_invite_by_token`, which runs the narrow SECURITY
//! DEFINER bridge from 0047 against the raw pool (redemption happens
//! before the invitee has any session to scope with).

use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use rand::RngCore;
use sqlx::PgPool;
use uuid::Uuid;

/// One directory row: a staff user plus its resolved builtin org role
/// (highest-precedence builtin role when several are held; `None` when the
/// user carries only custom roles).
#[derive(Debug, Clone, serde::Serialize)]
pub struct OrgUserEntry {
    pub id: Uuid,
    pub org_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub org_role: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

const DIRECTORY_COLS: &str = "u.id, u.org_id, u.email, u.display_name, u.is_active, u.created_at, \
     (SELECT r.builtin FROM user_roles ur JOIN roles r ON r.id = ur.role_id \
      WHERE ur.user_id = u.id AND r.builtin IS NOT NULL \
      ORDER BY preset_builtin_precedence(r.builtin) \
      LIMIT 1) AS org_role";

fn entry_from_row(row: sqlx::postgres::PgRow) -> DbResult<OrgUserEntry> {
    use sqlx::Row;
    Ok(OrgUserEntry {
        id: row.get("id"),
        org_id: row.get("org_id"),
        email: row.get("email"),
        display_name: row.get("display_name"),
        org_role: row.get("org_role"),
        is_active: row.get("is_active"),
        created_at: row.get("created_at"),
    })
}

/// Offset-paged directory listing, newest first.
pub async fn list_directory(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    limit: i64,
    offset: i64,
) -> DbResult<Vec<OrgUserEntry>> {
    let sql = format!(
        "SELECT {DIRECTORY_COLS} FROM users u WHERE u.org_id = $1 \
         ORDER BY u.created_at DESC, u.id DESC \
         LIMIT $2 OFFSET $3"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(executor)
        .await?;
    rows.into_iter().map(entry_from_row).collect()
}

pub async fn count_directory(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
) -> DbResult<i64> {
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM users WHERE org_id = $1")
        .bind(org_id)
        .fetch_one(executor)
        .await?;
    Ok(count)
}

/// Creates an INVITED user: `is_active = false` with an unusable random
/// password hash until the invitee redeems the setup token.
pub async fn create_invited_user(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    id: Uuid,
    org_id: Uuid,
    email: &str,
    display_name: &str,
) -> DbResult<OrgUserEntry> {
    // Random per-user hash; nobody ever presents a password against it.
    let mut seed = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    let unusable_hash = format!("$invited$${}", hex::encode(seed));
    let row = sqlx::query(
        "INSERT INTO users (id, org_id, email, display_name, password_hash, is_active) \
         VALUES ($1, $2, $3, $4, $5, false) \
         RETURNING id, org_id, email, display_name, is_active, created_at, NULL::text AS org_role",
    )
    .bind(id)
    .bind(org_id)
    .bind(email)
    .bind(display_name)
    .bind(&unusable_hash)
    .fetch_one(executor)
    .await?;
    entry_from_row(row)
}

pub async fn get_entry(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    user_id: Uuid,
) -> DbResult<OrgUserEntry> {
    let sql = format!("SELECT {DIRECTORY_COLS} FROM users u WHERE u.id = $1 AND u.org_id = $2");
    let row = sqlx::query(&sql)
        .bind(user_id)
        .bind(org_id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    entry_from_row(row)
}

pub async fn insert_invite(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    id: Uuid,
    org_id: Uuid,
    user_id: Uuid,
    token_hash: &str,
    created_by: Uuid,
    expires_at: DateTime<Utc>,
) -> DbResult<()> {
    sqlx::query(
        "INSERT INTO user_invites (id, org_id, user_id, token_hash, created_by, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(org_id)
    .bind(user_id)
    .bind(token_hash)
    .bind(created_by)
    .bind(expires_at)
    .execute(executor)
    .await?;
    Ok(())
}

/// Single-use token redemption under the caller's org scope: marks the
/// invite used and returns its user. Expired or already-used tokens are
/// indistinguishable misses (`NotFound`).
pub async fn take_invite(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    token_hash: &str,
) -> DbResult<Uuid> {
    let (user_id,): (Uuid,) = sqlx::query_as(
        "UPDATE user_invites SET used_at = now() \
         WHERE org_id = $1 AND token_hash = $2 AND used_at IS NULL AND expires_at > now() \
         RETURNING user_id",
    )
    .bind(org_id)
    .bind(token_hash)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(user_id)
}

/// Raw-pool resolution for redemption (SECURITY DEFINER bridge, 0047).
pub async fn resolve_invite_by_token(pool: &PgPool, token_hash: &str) -> DbResult<(Uuid, Uuid)> {
    sqlx::query_as("SELECT org_id, user_id FROM app_resolve_user_invite($1)")
        .bind(token_hash)
        .fetch_optional(pool)
        .await?
        .ok_or(DbError::NotFound)
}

/// Sets the first password and activates the account (invite completion).
pub async fn complete_invite(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    user_id: Uuid,
    password_hash: &str,
) -> DbResult<OrgUserEntry> {
    let row = sqlx::query(
        "UPDATE users SET password_hash = $3, is_active = true, updated_at = now() \
         WHERE id = $1 AND org_id = $2 \
         RETURNING id, org_id, email, display_name, is_active, created_at, NULL::text AS org_role",
    )
    .bind(user_id)
    .bind(org_id)
    .bind(password_hash)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    entry_from_row(row)
}

/// Serializes one org's owner-count decision for the rest of the caller's
/// transaction.
///
/// Every path that can take an org's last active owner away (`set_active`
/// off, or `set_builtin_role` away from `owner`) takes this lock BEFORE it
/// reads the state it decides on, and holds it until commit. Without it two
/// overlapping removals both count two owners, both pass the guard, and the
/// org commits with none — the count and the write are one decision, so
/// they run under one lock. A second caller queues here and then sees the
/// first caller's committed removal, which is exactly what makes its
/// refusal true. Released automatically at commit or rollback.
pub async fn lock_owner_guard(conn: &mut sqlx::PgConnection, org_id: Uuid) -> DbResult<()> {
    let key = format!("org.owner_guard:{org_id}");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 42))")
        .bind(key)
        .execute(conn)
        .await?;
    Ok(())
}

/// Activate / deactivate. Never deletes: `is_active = false` is terminal.
///
/// Deactivating an owner must be preceded by `lock_owner_guard` in this
/// transaction — the count that decides "last active owner" is only true
/// while nothing else can take the other owners away.
pub async fn set_active(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    user_id: Uuid,
    active: bool,
) -> DbResult<OrgUserEntry> {
    let row = sqlx::query(
        "UPDATE users SET is_active = $3, updated_at = now() \
         WHERE id = $1 AND org_id = $2 \
         RETURNING id, org_id, email, display_name, is_active, created_at, NULL::text AS org_role",
    )
    .bind(user_id)
    .bind(org_id)
    .bind(active)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    entry_from_row(row)
}

/// Revokes every live auth session for one user (deactivation side
/// effect). Returns the number of sessions revoked.
pub async fn revoke_all_sessions(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    user_id: Uuid,
) -> DbResult<u64> {
    let result = sqlx::query(
        "UPDATE auth_sessions SET revoked_at = now() \
         WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// How many ACTIVE users currently hold the given builtin role (the
/// last-active-owner guard feeds on this).
pub async fn count_active_with_builtin_role(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    builtin: &str,
) -> DbResult<i64> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM users u \
         JOIN user_roles ur ON ur.user_id = u.id \
         JOIN roles r ON r.id = ur.role_id \
         WHERE u.org_id = $1 AND u.is_active AND r.builtin = $2",
    )
    .bind(org_id)
    .bind(builtin)
    .fetch_one(executor)
    .await?;
    Ok(count)
}

/// Replaces the user's BUILTIN org role: strips every builtin role row and
/// grants exactly `role_id`. Custom (non-builtin) role grants pass through
/// untouched. Returns the previous builtin role, when one was held.
pub async fn set_builtin_role(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    role_id: Uuid,
) -> DbResult<Option<String>> {
    let previous: Option<String> = sqlx::query_scalar(
        "SELECT r.builtin FROM user_roles ur JOIN roles r ON r.id = ur.role_id \
          WHERE ur.user_id = $1 AND r.org_id = $2 AND r.builtin IS NOT NULL \
          ORDER BY preset_builtin_precedence(r.builtin) \
         LIMIT 1",
    )
    .bind(user_id)
    .bind(org_id)
    .fetch_optional(&mut *conn)
    .await?
    .flatten();
    sqlx::query(
        "DELETE FROM user_roles ur USING roles r \
         WHERE r.id = ur.role_id AND ur.user_id = $1 AND r.org_id = $2 AND r.builtin IS NOT NULL",
    )
    .bind(user_id)
    .bind(org_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(user_id)
        .bind(role_id)
        .execute(&mut *conn)
        .await?;
    Ok(previous)
}

/// Role lookup for one org by builtin name (`owner`, `admin`, …).
pub async fn find_builtin_role_id(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    builtin: &str,
) -> DbResult<Uuid> {
    let (id,): (Uuid,) =
        sqlx::query_as("SELECT id FROM roles WHERE org_id = $1 AND builtin = $2 AND is_system")
            .bind(org_id)
            .bind(builtin)
            .fetch_optional(executor)
            .await?
            .ok_or(DbError::NotFound)?;
    Ok(id)
}

/// Duplicate check for invites: matches ANY state (active, deactivated,
/// invited-inactive), since `(org_id, email)` is unique on `users`.
pub async fn find_any_by_email(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    email: &str,
) -> DbResult<Option<Uuid>> {
    let row: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM users WHERE org_id = $1 AND lower(email) = lower($2)")
            .bind(org_id)
            .bind(email)
            .fetch_optional(executor)
            .await?;
    Ok(row.map(|(id,)| id))
}
