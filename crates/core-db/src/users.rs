//! Users, credentials, sessions, and role assignment.

use crate::{DbError, DbResult};
use core_domain::entities::{AuthSession, Role, User};
use sqlx::PgPool;
use uuid::Uuid;

/// Internal auth row — includes the password hash, never leaves the crate
/// except to the auth module in core-api which verifies and discards it.
pub struct UserCredentials {
    pub user: User,
    pub password_hash: String,
}

pub async fn create_user(
    pool: &PgPool,
    id: Uuid,
    org_id: Uuid,
    email: &str,
    display_name: &str,
    password_hash: &str,
    timezone: &str,
) -> DbResult<User> {
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
    .fetch_one(pool)
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

pub async fn find_credentials_by_email(pool: &PgPool, email: &str) -> DbResult<UserCredentials> {
    let row = sqlx::query!(
        r#"SELECT id, org_id, email, display_name, timezone, is_active,
                  mfa_totp_secret, created_at, updated_at, password_hash
           FROM users WHERE lower(email) = lower($1) AND is_active"#,
        email
    )
    .fetch_optional(pool)
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

pub async fn get_user(pool: &PgPool, org_id: Uuid, id: Uuid) -> DbResult<User> {
    let row = sqlx::query!(
        r#"SELECT id, org_id, email, display_name, timezone, is_active,
                  mfa_totp_secret, created_at, updated_at
           FROM users WHERE id = $1 AND org_id = $2"#,
        id,
        org_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;
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

pub async fn list_users(pool: &PgPool, org_id: Uuid) -> DbResult<Vec<User>> {
    let rows = sqlx::query!(
        r#"SELECT id, org_id, email, display_name, timezone, is_active,
                  mfa_totp_secret, created_at, updated_at
           FROM users WHERE org_id = $1 ORDER BY display_name"#,
        org_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| User {
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
        .collect())
}

pub async fn assign_role(pool: &PgPool, user_id: Uuid, role_id: Uuid) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)
           ON CONFLICT DO NOTHING"#,
        user_id,
        role_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn roles_for_user(pool: &PgPool, user_id: Uuid, org_id: Uuid) -> DbResult<Vec<Role>> {
    let rows = sqlx::query!(
        r#"SELECT r.id, r.org_id, r.name, r.builtin, r.permissions, r.is_system
           FROM roles r
           JOIN user_roles ur ON ur.role_id = r.id
           WHERE ur.user_id = $1 AND r.org_id = $2
           ORDER BY r.name"#,
        user_id,
        org_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Role {
            id: row.id,
            org_id: row.org_id,
            name: row.name,
            builtin: row
                .builtin
                .as_deref()
                .and_then(|b| {
                    use core_domain::policy::BuiltinRole;
                    match b {
                        "owner" => Some(BuiltinRole::Owner),
                        "admin" => Some(BuiltinRole::Admin),
                        "dispatcher" => Some(BuiltinRole::Dispatcher),
                        "technician" => Some(BuiltinRole::Technician),
                        "read_only" => Some(BuiltinRole::ReadOnly),
                        _ => None,
                    }
                }),
            permissions: serde_json::from_value(row.permissions).unwrap_or_default(),
            is_system: row.is_system,
        })
        .collect())
}

pub async fn create_session(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    refresh_hash: &str,
    user_agent: Option<&str>,
    expires_at: chrono::DateTime<chrono::Utc>,
) -> DbResult<AuthSession> {
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
    .fetch_one(pool)
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

pub async fn find_active_session(
    pool: &PgPool,
    session_id: Uuid,
    refresh_hash: &str,
) -> DbResult<AuthSession> {
    let row = sqlx::query!(
        r#"SELECT id, user_id, refresh_token_hash, user_agent, expires_at, created_at, revoked_at
           FROM auth_sessions
           WHERE id = $1 AND refresh_token_hash = $2
             AND revoked_at IS NULL AND expires_at > now()"#,
        session_id,
        refresh_hash
    )
    .fetch_optional(pool)
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
pub async fn rotate_session(
    pool: &PgPool,
    session_id: Uuid,
    new_hash: &str,
    new_expiry: chrono::DateTime<chrono::Utc>,
) -> DbResult<()> {
    sqlx::query!(
        r#"UPDATE auth_sessions SET refresh_token_hash = $2, expires_at = $3 WHERE id = $1"#,
        session_id,
        new_hash,
        new_expiry
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn revoke_session(pool: &PgPool, session_id: Uuid) -> DbResult<()> {
    sqlx::query!(
        r#"UPDATE auth_sessions SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL"#,
        session_id
    )
    .execute(pool)
    .await?;
    Ok(())
}
