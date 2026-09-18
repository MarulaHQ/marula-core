//! Users, credentials, sessions, and role assignment.
//!
//! Refresh sessions form immutable chains (`chain_id` + `rotated_from`):
//! rotation revokes the parent and inserts a child, which is what makes
//! replay of an old refresh token detectable and chain-revocable.
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
//
// Sessions form immutable CHAINS: rotation never mutates a presented row,
// it revokes the parent and inserts a child (`rotated_from`), so every
// refresh token maps to exactly one historical row. Presenting any
// non-active row therefore proves reuse and revokes the whole chain.

/// Chain-aware session row (superset of [`AuthSession`]).
#[derive(Debug, Clone)]
pub struct RefreshSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub refresh_token_hash: String,
    pub user_agent: Option<String>,
    /// Chain root — constant across all rotations of one login lineage.
    pub chain_id: Uuid,
    /// Parent session this row was rotated from (None for chain roots).
    pub rotated_from: Option<Uuid>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

const REFRESH_COLS: &str = "id, user_id, refresh_token_hash, user_agent, chain_id, rotated_from, \
     expires_at, created_at, revoked_at";

fn refresh_from_row(row: sqlx::postgres::PgRow) -> DbResult<RefreshSession> {
    use sqlx::Row;
    Ok(RefreshSession {
        id: row.get("id"),
        user_id: row.get("user_id"),
        refresh_token_hash: row.get("refresh_token_hash"),
        user_agent: row.get("user_agent"),
        chain_id: row.get("chain_id"),
        rotated_from: row.get("rotated_from"),
        expires_at: row.get("expires_at"),
        created_at: row.get("created_at"),
        revoked_at: row.get("revoked_at"),
    })
}

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
        r#"SELECT * FROM app_create_session($1, $2, $3, $4, $5, $6)"#,
        id,
        user_id,
        refresh_hash,
        user_agent,
        expires_at,
        id
    )
    .fetch_one(executor)
    .await?;
    Ok(AuthSession {
        id: row.id.expect("app_create_session returns id"),
        user_id: row.user_id.expect("app_create_session returns user_id"),
        refresh_token_hash: row
            .refresh_token_hash
            .expect("app_create_session returns refresh_token_hash"),
        user_agent: row.user_agent,
        expires_at: row
            .expires_at
            .expect("app_create_session returns expires_at"),
        created_at: row
            .created_at
            .expect("app_create_session returns created_at"),
        revoked_at: row.revoked_at,
    })
}

/// Validates a presented refresh token against its stored hash. Callers
/// WITHOUT org scope must use `resolve_refresh` (SECURITY DEFINER) first;
/// this function runs under RLS.
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

/// Any session row matching (id, hash) regardless of revocation/expiry —
/// the lookup reuse detection needs to classify a presented token.
pub async fn find_session_any<'e, E>(
    executor: E,
    session_id: Uuid,
    refresh_hash: &str,
) -> DbResult<RefreshSession>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "SELECT {REFRESH_COLS} FROM auth_sessions WHERE id = $1 AND refresh_token_hash = $2"
    );
    let row = sqlx::query(&sql)
        .bind(session_id)
        .bind(refresh_hash)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    refresh_from_row(row)
}

/// Rotates to a NEW chain node atomically: the parent is revoked (guarded:
/// only when still active — concurrent replays lose the race and fall
/// through to reuse handling) and the child carrying the fresh secret is
/// inserted in the same statement.
pub async fn rotate_session_chain<'e, E>(
    executor: E,
    parent: &RefreshSession,
    new_id: Uuid,
    new_hash: &str,
    new_expiry: chrono::DateTime<chrono::Utc>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query!(
        r#"WITH retired AS (
               UPDATE auth_sessions SET revoked_at = now()
               WHERE id = $1 AND revoked_at IS NULL
           )
           INSERT INTO auth_sessions (id, user_id, refresh_token_hash, user_agent,
                                      chain_id, rotated_from, expires_at)
           VALUES ($2, $3, $4, $5, $6, $1, $7)"#,
        parent.id,
        new_id,
        parent.user_id,
        new_hash,
        parent.user_agent,
        parent.chain_id,
        new_expiry
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Revokes every still-live session in a chain (reuse-detection response).
/// Returns how many rows died.
pub async fn revoke_chain<'e, E>(executor: E, chain_id: Uuid) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let result = sqlx::query!(
        r#"UPDATE auth_sessions SET revoked_at = now()
           WHERE chain_id = $1 AND revoked_at IS NULL"#,
        chain_id
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Lists a user's live sessions, newest first (security settings page).
pub async fn list_user_sessions<'e, E>(executor: E, user_id: Uuid) -> DbResult<Vec<RefreshSession>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "SELECT {REFRESH_COLS} FROM auth_sessions \
         WHERE user_id = $1 AND revoked_at IS NULL ORDER BY created_at DESC"
    );
    let rows = sqlx::query(&sql).bind(user_id).fetch_all(executor).await?;
    rows.into_iter().map(refresh_from_row).collect()
}

/// Resolves a presented refresh token via the SECURITY DEFINER bridge —
/// the only sanctioned pre-org-scope lookup. Returns None for unknown
/// (id, hash) pairs; Some with `active=false` marks reuse of a rotated,
/// revoked, or expired token.
pub struct RefreshResolution {
    pub user_id: Uuid,
    pub org_id: Uuid,
    pub chain_id: Uuid,
    pub active: bool,
}

pub async fn resolve_refresh(
    pool: &PgPool,
    session_id: Uuid,
    token_hash: &str,
) -> DbResult<Option<RefreshResolution>> {
    let row: Option<(Uuid, Uuid, Uuid, bool)> =
        sqlx::query_as("SELECT user_id, org_id, chain_id, active FROM app_resolve_refresh($1, $2)")
            .bind(session_id)
            .bind(token_hash)
            .fetch_optional(pool)
            .await?;
    Ok(
        row.map(|(user_id, org_id, chain_id, active)| RefreshResolution {
            user_id,
            org_id,
            chain_id,
            active,
        }),
    )
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

/// Resolves (user_id, org_id) by email via the SECURITY DEFINER bridge.
pub async fn resolve_login_org(pool: &PgPool, email: &str) -> DbResult<(Uuid, Uuid)> {
    sqlx::query_as("SELECT user_id, org_id FROM app_resolve_login($1)")
        .bind(email)
        .fetch_optional(pool)
        .await?
        .ok_or(DbError::NotFound)
}
/// Wave 19 lane 5: revokes EVERY live refresh session of one user
/// (password-reset / email-takeover response). Returns rows killed.
/// RLS applies — the caller must carry the org scope.
pub async fn revoke_all_user_sessions<'e, E>(executor: E, user_id: Uuid) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let result = sqlx::query(
        "UPDATE auth_sessions SET revoked_at = now() \
         WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Wave 19 lane 5: marks a user's email verified (verification link or
/// admin override). Idempotent: the first stamp wins the timestamp.
pub async fn mark_email_verified<'e, E>(executor: E, user_id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        "UPDATE users SET email_verified = true, \
                email_verified_at = COALESCE(email_verified_at, now()) \
         WHERE id = $1",
    )
    .bind(user_id)
    .execute(executor)
    .await?;
    Ok(())
}

/// Wave 19 lane 5: admin override — flips an unverified email back to
/// unverified (the override is also retractable) and clears the stamp.
pub async fn clear_email_verified<'e, E>(executor: E, user_id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        "UPDATE users SET email_verified = false, email_verified_at = NULL WHERE id = $1",
    )
    .bind(user_id)
    .execute(executor)
    .await?;
    Ok(())
}
/// Wave 19 lane 5: replaces one user's password hash (password-reset
/// completion and admin resets). RLS applies; callers audit + revoke
/// sessions themselves. Plain runner — no offline-cache churn.
pub async fn set_password_hash<'e, E>(
    executor: E,
    user_id: Uuid,
    password_hash: &str,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        "UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1",
    )
    .bind(user_id)
    .bind(password_hash)
    .execute(executor)
    .await?;
    Ok(())
}
