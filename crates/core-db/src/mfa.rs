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
///
/// A CONFIRMED row is never downgraded: the `WHERE` on the conflict arm means
/// the statement reports zero rows and the caller refuses. That is the SQL
/// half of "beginning enrolment cannot replace a live factor" — the handler's
/// step-up check is the other half, and neither alone survives a future
/// caller that forgets one of them.
pub async fn upsert_pending_totp<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    user_id: Uuid,
    secret_enc: &[u8],
) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        r#"INSERT INTO totp_secrets (id, org_id, user_id, secret_enc, confirmed)
           VALUES ($1, $2, $3, $4, false)
           ON CONFLICT (user_id) DO UPDATE
             SET secret_enc = EXCLUDED.secret_enc,
                 confirmed = false,
                 last_used_step = -1,
                 updated_at = now()
           WHERE totp_secrets.confirmed = false"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(user_id)
    .bind(secret_enc)
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Replaces a CONFIRMED TOTP secret with a pending one.
///
/// Only the re-enrolment path calls this, and only after a fresh step-up from
/// a method OTHER than TOTP (a live code cannot authorize replacing the
/// secret it belongs to). The delete and the insert are one statement, so the
/// account never commits a state where the old secret is gone and the new one
/// did not land; recovery codes survive until `enroll-verify`, which is what
/// keeps the account at one working factor through the swap.
///
/// Runtime `query` rather than `query!`: the statement is new in migration
/// 0161 and the offline cache is re-pinned on the lane's gate run, not while
/// the light checks run.
pub async fn replace_confirmed_totp_with_pending<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    user_id: Uuid,
    secret_enc: &[u8],
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"WITH gone AS (
               DELETE FROM totp_secrets WHERE user_id = $3 AND confirmed
           )
           INSERT INTO totp_secrets (id, org_id, user_id, secret_enc, confirmed)
           VALUES ($1, $2, $3, $4, false)
           ON CONFLICT (user_id) DO UPDATE
             SET secret_enc = EXCLUDED.secret_enc,
                 confirmed = false,
                 last_used_step = -1,
                 updated_at = now()"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(user_id)
    .bind(secret_enc)
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

// --- step-up records -------------------------------------------------------

/// One recorded factor verification on a signed-in refresh chain.
///
/// `fresh` is answered by the database clock at read time (`expires_at >
/// now()`), not by the process that asks, so a replica with a skewed clock
/// cannot widen its own window.
#[derive(Debug, Clone)]
pub struct StepUpRow {
    pub method: String,
    pub factor_ref: Option<String>,
    pub verified_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub fresh: bool,
}

/// Records that `session_id` verified a live factor, on the chain that session
/// belongs to (resolved in the same statement, so a caller cannot attach a
/// proof to the wrong lineage).
///
/// Expired rows for the same user are swept in the same statement: the table
/// only ever needs to answer "is there one live proof", and an unbounded
/// history of them is a liability rather than evidence (the audit trail
/// already records each verification).
///
/// Returns the rows written. Zero means the session row was not visible to
/// this transaction — the caller should treat that as "no step-up", never as
/// success.
pub struct StepUpRecord<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub session_id: Uuid,
    pub method: &'a str,
    pub factor_ref: Option<&'a str>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

pub async fn record_step_up<'e, E>(executor: E, step: StepUpRecord<'_>) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        r#"WITH swept AS (
               DELETE FROM mfa_step_ups WHERE user_id = $3 AND expires_at < now()
           )
           INSERT INTO mfa_step_ups
             (id, org_id, user_id, session_id, chain_id, verified_at, expires_at, method, factor_ref)
           SELECT $1, $2, $3, a.id, a.chain_id, now(), $5, $6, $7
           FROM auth_sessions a
           WHERE a.id = $4"#,
    )
    .bind(step.id)
    .bind(step.org_id)
    .bind(step.user_id)
    .bind(step.session_id)
    .bind(step.expires_at)
    .bind(step.method)
    .bind(step.factor_ref)
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// The newest step-up recorded for the chain `session_id` belongs to.
///
/// Resolving through `chain_id` rather than the session row is deliberate:
/// refresh rotation inserts a new session row on the same chain, and a proof
/// taken five minutes ago must not evaporate because the client refreshed.
/// A new login starts a new chain, so a different session can never borrow
/// this one's proof.
pub async fn latest_step_up<'e, E>(
    executor: E,
    session_id: Uuid,
    user_id: Uuid,
) -> DbResult<Option<StepUpRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            chrono::DateTime<chrono::Utc>,
            chrono::DateTime<chrono::Utc>,
            bool,
        ),
    >(
        r#"SELECT s.method, s.factor_ref, s.verified_at, s.expires_at,
                  (s.expires_at > now()) AS fresh
           FROM mfa_step_ups s
           JOIN auth_sessions a ON a.chain_id = s.chain_id
           WHERE a.id = $1 AND s.user_id = $2
           ORDER BY s.verified_at DESC
           LIMIT 1"#,
    )
    .bind(session_id)
    .bind(user_id)
    .fetch_optional(executor)
    .await?;
    Ok(row.map(
        |(method, factor_ref, verified_at, expires_at, fresh)| StepUpRow {
            method,
            factor_ref,
            verified_at,
            expires_at,
            fresh,
        },
    ))
}

// --- admin factor reset ----------------------------------------------------

/// Whether an admin reset left this account owing a factor.
pub async fn user_mfa_reenroll_required<'e, E>(executor: E, user_id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let flag: Option<bool> =
        sqlx::query_scalar("SELECT mfa_reenroll_required FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(executor)
            .await?;
    Ok(flag.unwrap_or(false))
}

/// Clears the re-enrolment requirement once a factor is confirmed again.
pub async fn clear_mfa_reenroll_required<'e, E>(executor: E, user_id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query("UPDATE users SET mfa_reenroll_required = false, updated_at = now() WHERE id = $1")
        .bind(user_id)
        .execute(executor)
        .await?;
    Ok(())
}

/// What an admin reset removed, for the audit row that records it.
#[derive(Debug, Clone, Copy)]
pub struct ResetCounts {
    pub totp_removed: i64,
    pub passkeys_removed: i64,
    pub recovery_codes_removed: i64,
}

/// Clears every factor, every in-flight ceremony and every step-up proof for
/// one user, and marks the account as owing a factor.
///
/// One statement, so the account is never observed half-reset: a reader sees
/// either the old factors or the flagged, factor-free state. Sessions are
/// revoked by the caller (`revoke_all_user_sessions`) because that is a
/// separate, auditable fact — this module owns factor storage, not the
/// session ledger.
pub async fn reset_user_factors<'e, E>(executor: E, user_id: Uuid) -> DbResult<ResetCounts>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as::<_, (i64, i64, i64, i64)>(
        r#"WITH totp AS (DELETE FROM totp_secrets WHERE user_id = $1 RETURNING 1),
                keys AS (DELETE FROM webauthn_credentials WHERE user_id = $1 RETURNING 1),
                codes AS (DELETE FROM recovery_codes WHERE user_id = $1 RETURNING 1),
                chals AS (DELETE FROM mfa_challenges WHERE user_id = $1 RETURNING 1),
                steps AS (DELETE FROM mfa_step_ups WHERE user_id = $1 RETURNING 1),
                flagged AS (
                    UPDATE users
                       SET mfa_reenroll_required = true, updated_at = now()
                     WHERE id = $1
                    RETURNING 1
                )
           SELECT (SELECT count(*) FROM totp)::bigint,
                  (SELECT count(*) FROM keys)::bigint,
                  (SELECT count(*) FROM codes)::bigint,
                  (SELECT count(*) FROM flagged)::bigint"#,
    )
    .bind(user_id)
    .fetch_one(executor)
    .await?;
    Ok(ResetCounts {
        totp_removed: row.0,
        passkeys_removed: row.1,
        recovery_codes_removed: row.2,
    })
}
