//! Users, credentials, sessions, and role assignment.
//!
//! All functions are generic over `Executor`, so callers may run them
//! against the pool (autocommit) or inside an explicit transaction. RLS
//! applies either way: the executor's transaction must carry `app.org_id`.

use crate::{DbError, DbResult};
use core_domain::entities::{AuthSession, Role, User};
use sqlx::PgPool;
use uuid::Uuid;

/// Internal auth row — includes the password hash, never leaves the crate
/// except to the auth layer which verifies and discards it.
pub struct UserCredentials {
    pub user: User,
    pub password_hash: String,
}

fn user_from_row(row: sqlx::postgres::PgRow) -> DbResult<User> {
    use sqlx::Row;
    Ok(User {
        id: row.get("id"),
        org_id: row.get("org_id"),
        email: row.get("email"),
        display_name: row.get("display_name"),
        timezone: row.get("timezone"),
        is_active: row.get("is_active"),
        mfa_totp_secret: row.get("mfa_totp_secret"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

const USER_COLS: &str = "id, org_id, email, display_name, timezone, is_active, \
     mfa_totp_secret, created_at, updated_at";

pub async fn create_user<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    email: &str,
    display_name: &str,
    password_hash: &str,
    timezone: &str,
) -> DbResult<User>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO users (id, org_id, email, display_name, password_hash, timezone)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING id, org_id, email, display_name, timezone, is_active,
                     mfa_totp_secret, created_at, updated_at"#,
        id,
        org_id,
        email,
        display_name,
        password_hash,
        timezone,
    )
    .fetch_one(executor)
    .await?;
    Ok(User {
        id: row.id,
        org_id: row.org_id,
        email: row.email,
        display_name: row.display_name,
        timezone: row.timezone,
        is_active: row.is_active,
        mfa_totp_secret: row.mfa_totp_secret,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Credential lookup inside an org-bound transaction (RLS applies).
pub async fn find_credentials_by_email<'e, E>(executor: E, email: &str) -> DbResult<UserCredentials>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, email, display_name, timezone, is_active,
                  mfa_totp_secret, created_at, updated_at, password_hash
           FROM users WHERE lower(email) = lower($1) AND is_active"#,
        email
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(UserCredentials {
        user: User {
            id: row.id,
            org_id: row.org_id,
            email: row.email,
            display_name: row.display_name,
            timezone: row.timezone,
            is_active: row.is_active,
            mfa_totp_secret: row.mfa_totp_secret,
            created_at: row.created_at,
            updated_at: row.updated_at,
        },
        password_hash: row.password_hash,
    })
}

pub async fn get_user<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<User>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!("SELECT {USER_COLS} FROM users WHERE id = $1 AND org_id = $2");
    let row = sqlx::query(&sql)
        .bind(id)
        .bind(org_id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    user_from_row(row)
}

pub async fn list_users<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<User>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!("SELECT {USER_COLS} FROM users WHERE org_id = $1 ORDER BY display_name");
    let rows = sqlx::query(&sql).bind(org_id).fetch_all(executor).await?;
    rows.into_iter().map(user_from_row).collect()
}

/// Finds an active user by email within one org — requester resolution for
/// inbound email (email-in/out v1).
pub async fn find_by_email_in_org<'e, E>(
    executor: E,
    org_id: Uuid,
    email: &str,
) -> DbResult<Option<User>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "SELECT {USER_COLS} FROM users \
         WHERE org_id = $1 AND lower(email) = lower($2) AND is_active LIMIT 1"
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(email.trim())
        .fetch_optional(executor)
        .await?;
    match row {
        Some(row) => Ok(Some(user_from_row(row)?)),
        None => Ok(None),
    }
}

pub async fn assign_role<'e, E>(executor: E, user_id: Uuid, role_id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)
           ON CONFLICT DO NOTHING"#,
        user_id,
        role_id
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn roles_for_user<'e, E>(executor: E, user_id: Uuid, org_id: Uuid) -> DbResult<Vec<Role>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT r.id, r.org_id, r.name, r.builtin, r.permissions, r.is_system
           FROM roles r
           JOIN user_roles ur ON ur.role_id = r.id
           WHERE ur.user_id = $1 AND r.org_id = $2
           ORDER BY r.name"#,
        user_id,
        org_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Role {
            id: row.id,
            org_id: row.org_id,
            name: row.name,
            builtin: parse_builtin(row.builtin.as_deref()),
            permissions: serde_json::from_value(row.permissions).unwrap_or_default(),
            is_system: row.is_system,
        })
        .collect())
}

pub fn parse_builtin(s: Option<&str>) -> Option<core_domain::policy::BuiltinRole> {
    use core_domain::policy::BuiltinRole;
    match s? {
        "owner" => Some(BuiltinRole::Owner),
        "admin" => Some(BuiltinRole::Admin),
        "dispatcher" => Some(BuiltinRole::Dispatcher),
        "technician" => Some(BuiltinRole::Technician),
        "read_only" => Some(BuiltinRole::ReadOnly),
        _ => None,
    }
}

// --- refresh sessions -------------------------------------------------------

pub async fn create_session<'e, E>(
    executor: E,
    id: Uuid,
    user_id: Uuid,
    refresh_hash: &str,
    user_agent: Option<&str>,
    expires_at: chrono::DateTime<chrono::Utc>,
) -> DbResult<AuthSession>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO auth_sessions (id, user_id, refresh_token_hash, user_agent, expires_at)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id, user_id, refresh_token_hash, user_agent, expires_at, created_at, revoked_at"#,
        id,
        user_id,
        refresh_hash,
        user_agent,
        expires_at
    )
    .fetch_one(executor)
    .await?;
    Ok(AuthSession {
        id: row.id,
        user_id: row.user_id,
        refresh_token_hash: row.refresh_token_hash,
        user_agent: row.user_agent,
        expires_at: row.expires_at,
        created_at: row.created_at,
        revoked_at: row.revoked_at,
    })
}

/// Validates a presented refresh token against its stored hash. Callers
/// WITHOUT org scope must use `app_resolve_session` (SECURITY DEFINER)
/// first; this function runs under RLS.
pub async fn find_active_session<'e, E>(
    executor: E,
    session_id: Uuid,
    refresh_hash: &str,
) -> DbResult<AuthSession>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, user_id, refresh_token_hash, user_agent, expires_at, created_at, revoked_at
           FROM auth_sessions
           WHERE id = $1 AND refresh_token_hash = $2
             AND revoked_at IS NULL AND expires_at > now()"#,
        session_id,
        refresh_hash
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(AuthSession {
        id: row.id,
        user_id: row.user_id,
        refresh_token_hash: row.refresh_token_hash,
        user_agent: row.user_agent,
        expires_at: row.expires_at,
        created_at: row.created_at,
        revoked_at: row.revoked_at,
    })
}

/// Rotates the refresh secret for a session (refresh-token rotation).
pub async fn rotate_session<'e, E>(
    executor: E,
    session_id: Uuid,
    new_hash: &str,
    new_expiry: chrono::DateTime<chrono::Utc>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query!(
        r#"UPDATE auth_sessions SET refresh_token_hash = $2, expires_at = $3 WHERE id = $1"#,
        session_id,
        new_hash,
        new_expiry
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn revoke_session<'e, E>(executor: E, session_id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query!(
        r#"UPDATE auth_sessions SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL"#,
        session_id
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Resolves (user_id, org_id) via the SECURITY DEFINER bridge — the only
/// sanctioned way to find a session before org scope is known.
pub async fn resolve_session_org(
    pool: &PgPool,
    session_id: Uuid,
    token_hash: &str,
) -> DbResult<(Uuid, Uuid)> {
    sqlx::query_as("SELECT user_id, org_id FROM app_resolve_session($1, $2)")
        .bind(session_id)
        .bind(token_hash)
        .fetch_optional(pool)
        .await?
        .ok_or(DbError::NotFound)
}

/// Resolves (user_id, org_id) by email via the SECURITY DEFINER bridge.
pub async fn resolve_login_org(pool: &PgPool, email: &str) -> DbResult<(Uuid, Uuid)> {
    sqlx::query_as("SELECT user_id, org_id FROM app_resolve_login($1)")
        .bind(email)
        .fetch_optional(pool)
        .await?
        .ok_or(DbError::NotFound)
}
