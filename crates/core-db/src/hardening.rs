//! Auth hardening persistence (Wave 19 lane 5, migration 0088):
//! durable rate-limit/lockout gates and single-use hashed tokens.
//!
//! All pre-auth access rides the SECURITY DEFINER bridges created by the
//! migration — the application role never reads the table pre-org-scope.
//! Runtime queries (not `query!` macros) keep this lane free of offline
//! cache maintenance; every SQL string here lives beside its migration.

use sqlx::PgPool;
use uuid::Uuid;

use crate::DbResult;

/// Throttle surface names (mirrors the CHECK in migration 0088 prose).
pub mod gate_kind {
    pub const PASSWORD_LOGIN: &str = "password_login";
    pub const MFA_VERIFY: &str = "mfa_verify";
    pub const WEBAUTHN_LOGIN: &str = "webauthn_login";
    pub const WEBAUTHN_BEGIN: &str = "webauthn_begin";
    pub const REFRESH: &str = "refresh";
    pub const PASSWORD_RESET: &str = "password_reset";
    pub const EMAIL_VERIFY: &str = "email_verify";
    pub const MAGIC_LINK: &str = "magic_link";
    /// The client portal's own emailed sign-in link (migration 0119). A
    /// SEPARATE kind from `MAGIC_LINK` on purpose: the two throttle and lock
    /// independently, so hammering the portal never locks a staff member out
    /// of their own sign-in, and vice versa. The column is free text (the
    /// `kind` in `auth_lockouts` carries no CHECK), so this needs no schema
    /// change.
    pub const PORTAL_MAGIC_LINK: &str = "portal_magic_link";
}

/// Seconds until the strictest lockout for these keys expires (0 = allow).
pub async fn gate_check(pool: &PgPool, kind: &str, lock_keys: &[String]) -> DbResult<i64> {
    let remaining: Option<i64> = sqlx::query_scalar("SELECT app_auth_gate_check($1, $2)")
        .bind(kind)
        .bind(lock_keys)
        .fetch_one(pool)
        .await?;
    Ok(remaining.unwrap_or(0))
}

/// Records one failure on every key; returns the lockout seconds that now
/// apply per the exponential-backoff curve (0 = counting, not locked).
pub async fn gate_fail(
    pool: &PgPool,
    kind: &str,
    lock_keys: &[String],
    org_id: Option<Uuid>,
    subject_key: Option<&str>,
    ip_key: Option<&str>,
) -> DbResult<i64> {
    let secs: i64 = sqlx::query_scalar("SELECT app_auth_gate_fail($1, $2, $3, $4, $5)")
        .bind(kind)
        .bind(lock_keys)
        .bind(org_id)
        .bind(subject_key)
        .bind(ip_key)
        .fetch_one(pool)
        .await?;
    Ok(secs)
}

/// Clears the counters after a successful authentication.
pub async fn gate_clear(pool: &PgPool, kind: &str, lock_keys: &[String]) -> DbResult<()> {
    sqlx::query("SELECT app_auth_gate_clear($1, $2)")
        .bind(kind)
        .bind(lock_keys)
        .execute(pool)
        .await?;
    Ok(())
}

pub struct PutToken<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub kind: &'a str,
    pub token_hash: &'a str,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// Stores one hashed single-use token (the raw token never persists).
pub async fn put_one_use_token<'e, E>(executor: E, token: PutToken<'_>) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        "INSERT INTO auth_one_use_tokens \
         (id, org_id, user_id, kind, token_hash, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(token.id)
    .bind(token.org_id)
    .bind(token.user_id)
    .bind(token.kind)
    .bind(token.token_hash)
    .bind(token.expires_at)
    .execute(executor)
    .await?;
    Ok(())
}

pub struct TokenResolution {
    pub user_id: Uuid,
    pub org_id: Uuid,
}

/// Consumes (validates + marks) a one-use token atomically — the used_at
/// guard makes two concurrent redemptions yield exactly one winner.
/// Unknown/expired/already-used all collapse to None (no enumeration).
pub async fn consume_one_use_token(
    pool: &PgPool,
    kind: &str,
    token_hash: &str,
) -> DbResult<Option<TokenResolution>> {
    let row: Option<(Uuid, Uuid)> =
        sqlx::query_as("SELECT user_id, org_id FROM app_consume_one_use_token($1, $2)")
            .bind(kind)
            .bind(token_hash)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(user_id, org_id)| TokenResolution { user_id, org_id }))
}

/// Narrow pre-auth (user, org, email_verified) resolution — SECURITY
/// DEFINER via `app_resolve_auth_email`; None for unknown/inactive joints.
/// `magic_link` kind additionally requires the org opt-in.
pub async fn resolve_auth_email(
    pool: &PgPool,
    email: &str,
    magic_link: bool,
) -> DbResult<Option<(Uuid, Uuid, bool)>> {
    let row: Option<(Uuid, Uuid, bool)> = sqlx::query_as(
        "SELECT user_id, org_id, email_verified \
             FROM app_resolve_auth_email($1, $2)",
    )
    .bind(email)
    .bind(if magic_link {
        "magic_link"
    } else {
        "password_reset"
    })
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Deletes every one-use token of one (user, kind) family — reissued
/// tokens supersede older outstanding ones (single active link).
pub async fn invalidate_one_use_tokens<'e, E>(
    executor: E,
    user_id: Uuid,
    kind: &str,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        "DELETE FROM auth_one_use_tokens WHERE user_id = $1 AND kind = $2 AND used_at IS NULL",
    )
    .bind(user_id)
    .bind(kind)
    .execute(executor)
    .await?;
    Ok(())
}
/// Under-org-scope read of one user's email-verification flag
/// (migration 0088 column; default false). The login gate and the
/// admin-override surface read it here.
pub async fn user_email_verified<'e, E>(executor: E, user_id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let verified: Option<bool> =
        sqlx::query_scalar("SELECT email_verified FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(executor)
            .await?
            .flatten();
    Ok(verified.unwrap_or(false))
}
