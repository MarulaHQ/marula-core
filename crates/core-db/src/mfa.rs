//! MFA persistence: TOTP secrets (encrypted at rest), single-use hashed
//! recovery codes, WebAuthn credentials, and short-lived ceremony state.
//!
//! Purpose: storage only — crypto and ceremony logic live in core-api.
//! Non-purpose: code generation or verification; this module never sees
//! plaintext TOTP secrets or recovery codes except where the caller passes
//! an already-built hash/ciphertext.

use crate::DbResult;
use sqlx::Postgres;
use uuid::Uuid;

// --- capabilities ----------------------------------------------------------

/// Which second factors could satisfy a login right now. Drives whether
/// `/api/auth/login` issues tokens directly or a short-lived MFA bridge.
#[derive(Debug, Clone, Copy)]
pub struct MfaCapabilities {
    pub totp_confirmed: bool,
    pub webauthn_credentials: i64,
    pub unused_recovery_codes: i64,
}

impl MfaCapabilities {
    pub fn any(&self) -> bool {
        self.totp_confirmed || self.webauthn_credentials > 0 || self.unused_recovery_codes > 0
    }

    pub fn methods(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.totp_confirmed {
            out.push("totp");
        }
        if self.webauthn_credentials > 0 {
            out.push("webauthn");
        }
        if self.unused_recovery_codes > 0 {
            out.push("recovery_code");
        }
        out
    }
}

pub async fn mfa_capabilities<'e, E>(executor: E, user_id: Uuid) -> DbResult<MfaCapabilities>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as::<_, (bool, i64, i64)>(
        r#"SELECT
             EXISTS(SELECT 1 FROM totp_secrets WHERE user_id = $1 AND confirmed),
             (SELECT count(*) FROM webauthn_credentials WHERE user_id = $1),
             (SELECT count(*) FROM recovery_codes WHERE user_id = $1 AND used_at IS NULL)"#,
    )
    .bind(user_id)
    .fetch_one(executor)
    .await?;
    Ok(MfaCapabilities {
        totp_confirmed: row.0,
        webauthn_credentials: row.1,
        unused_recovery_codes: row.2,
    })
}

// --- totp ------------------------------------------------------------------

pub struct TotpSecretRow {
    pub user_id: Uuid,
    pub secret_enc: Vec<u8>,
    pub confirmed: bool,
    pub last_used_step: i64,
}

/// Inserts (or resets during a re-enrollment window) an unconfirmed TOTP
/// secret for the user. At most one row per user.
pub async fn upsert_pending_totp<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    user_id: Uuid,
    secret_enc: &[u8],
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO totp_secrets (id, org_id, user_id, secret_enc, confirmed)
           VALUES ($1, $2, $3, $4, false)
           ON CONFLICT (user_id) DO UPDATE
             SET secret_enc = EXCLUDED.secret_enc,
                 confirmed = false,
                 last_used_step = -1,
                 updated_at = now()"#,
        id,
        org_id,
        user_id,
        secret_enc
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Fetches the user's TOTP row, confirmed or pending.
pub async fn find_totp<'e, E>(executor: E, user_id: Uuid) -> DbResult<Option<TotpSecretRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT user_id, secret_enc, confirmed, last_used_step
           FROM totp_secrets WHERE user_id = $1"#,
        user_id
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|r| TotpSecretRow {
        user_id: r.user_id,
        secret_enc: r.secret_enc,
        confirmed: r.confirmed,
        last_used_step: r.last_used_step,
    }))
}

/// Confirms enrollment and records the first consumed timestep atomically:
/// the enabling code itself must never be reusable.
pub async fn confirm_totp<'e, E>(executor: E, user_id: Uuid, used_step: i64) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        r#"UPDATE totp_secrets SET confirmed = true, last_used_step = $2, updated_at = now()
               WHERE user_id = $1 AND confirmed = false"#,
        user_id,
        used_step
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Advances the replay watermark; returns 0 rows when `step` is not strictly
/// newer than the stored one — callers treat that as REPLAY REJECTED.
pub async fn consume_totp_step<'e, E>(executor: E, user_id: Uuid, step: i64) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        r#"UPDATE totp_secrets SET last_used_step = $2, updated_at = now()
           WHERE user_id = $1 AND confirmed AND last_used_step < $2"#,
        user_id,
        step
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Removes the user's TOTP enrollment entirely (disable flow).
pub async fn delete_totp<'e, E>(executor: E, user_id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!("DELETE FROM totp_secrets WHERE user_id = $1", user_id)
        .execute(executor)
        .await?;
    Ok(())
}

// --- recovery codes --------------------------------------------------------

/// Replaces all recovery codes with a fresh batch (enable + regenerate).
/// Single statement: old codes die exactly when the new batch lands.
pub async fn replace_recovery_codes<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
    hashes: &[String],
    ids: &[Uuid],
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let orgs = vec![org_id; ids.len()];
    let users = vec![user_id; ids.len()];
    sqlx::query!(
        r#"WITH gone AS (
               DELETE FROM recovery_codes WHERE user_id = $1
           )
           INSERT INTO recovery_codes (id, org_id, user_id, code_hash)
           SELECT * FROM UNNEST($2::uuid[], $3::uuid[], $4::uuid[], $5::text[])"#,
        user_id,
        ids as &[Uuid],
        orgs.as_slice(),
        users.as_slice(),
        hashes
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Counts unused recovery codes (MFA-capable check on login).
pub async fn count_unused_recovery_codes<'e, E>(executor: E, user_id: Uuid) -> DbResult<i64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let (count,) = sqlx::query_as::<_, (i64,)>(
        "SELECT count(*) FROM recovery_codes WHERE user_id = $1 AND used_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(executor)
    .await?;
    Ok(count)
}

/// Lists unused hashes so the verifier can attempt matches in order.
pub async fn unused_recovery_hashes<'e, E>(
    executor: E,
    user_id: Uuid,
) -> DbResult<Vec<(Uuid, String)>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, code_hash FROM recovery_codes \
         WHERE user_id = $1 AND used_at IS NULL ORDER BY created_at",
    )
    .bind(user_id)
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// Marks one code consumed. Single-use is enforced IN SQL (`used_at IS
/// NULL` guard): two concurrent verifications of the same code yield
/// exactly one success.
pub async fn consume_recovery_code<'e, E>(executor: E, code_id: Uuid) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        r#"UPDATE recovery_codes SET used_at = now()
           WHERE id = $1 AND used_at IS NULL"#,
        code_id
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

// --- webauthn --------------------------------------------------------------

pub struct WebauthnCredentialRow {
    pub id: Uuid,
    pub cred_id: String,
    pub credential: serde_json::Value,
    pub label: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn insert_webauthn_credential<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    user_id: Uuid,
    cred_id: &str,
    credential: &serde_json::Value,
    label: &str,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO webauthn_credentials (id, org_id, user_id, cred_id, credential, label)
           VALUES ($1, $2, $3, $4, $5, $6)"#,
        id,
        org_id,
        user_id,
        cred_id,
        credential,
        label
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn list_webauthn_credentials<'e, E>(
    executor: E,
    user_id: Uuid,
) -> DbResult<Vec<WebauthnCredentialRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, cred_id, credential, label, created_at, last_used_at
           FROM webauthn_credentials WHERE user_id = $1 ORDER BY created_at"#,
        user_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| WebauthnCredentialRow {
            id: r.id,
            cred_id: r.cred_id,
            credential: r.credential,
            label: r.label,
            created_at: r.created_at,
            last_used_at: r.last_used_at,
        })
        .collect())
}

pub async fn touch_webauthn_credential<'e, E>(executor: E, cred_id: &str) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        "UPDATE webauthn_credentials SET last_used_at = now() WHERE cred_id = $1",
        cred_id
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn delete_webauthn_credential<'e, E>(
    executor: E,
    user_id: Uuid,
    cred_row_id: Uuid,
) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        "DELETE FROM webauthn_credentials WHERE id = $1 AND user_id = $2",
        cred_row_id,
        user_id
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

// --- ceremony challenges ---------------------------------------------------

/// Persists one ceremony state. Expired rows (and any superseded ceremony
/// of the same kind for this user) are swept in the same statement.
pub async fn insert_challenge<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    user_id: Uuid,
    kind: &str,
    state: &serde_json::Value,
    expires_at: chrono::DateTime<chrono::Utc>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"WITH swept AS (
               DELETE FROM mfa_challenges
               WHERE expires_at < now()
                  OR (user_id = $3 AND kind = $4)
           )
           INSERT INTO mfa_challenges (id, org_id, user_id, kind, state, expires_at)
           VALUES ($1, $2, $3, $4, $5, $6)"#,
        id,
        org_id,
        user_id,
        kind,
        state,
        expires_at
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Takes (loads + deletes) a challenge — ceremonies are strictly one-shot;
/// replaying a finish call can never re-validate against stale state.
pub async fn take_challenge<'e, E>(
    executor: E,
    id: Uuid,
    user_id: Uuid,
    kind: &str,
) -> DbResult<Option<serde_json::Value>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"DELETE FROM mfa_challenges
           WHERE id = $1 AND user_id = $2 AND kind = $3 AND expires_at > now()
           RETURNING state"#,
        id,
        user_id,
        kind
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|r| r.state))
}

/// Takes an SSO handoff code by id alone (the caller resolved its org via
/// the SECURITY DEFINER bridge first). Single-use like every ceremony.
pub async fn take_sso_code<'e, E>(executor: E, id: Uuid) -> DbResult<Option<serde_json::Value>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"DELETE FROM mfa_challenges
           WHERE id = $1 AND kind = 'sso_code' AND expires_at > now()
           RETURNING state"#,
        id
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|r| r.state))
}
