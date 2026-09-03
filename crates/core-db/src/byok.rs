//! BYOK (Bring-Your-Own-Key) sealed secrets persistence (migration 0044).
//!
//! Purpose: store and retrieve encrypted API keys per (org_id, provider)
//! for the `CompletionProvider` seam and the QBO/Xero export-sync
//! connectors. Ciphertexts are AES-256-GCM encrypted bytes
//! (`secret_encrypted`) and are never logged or returned to user-facing
//! APIs.

use crate::DbResult;
use uuid::Uuid;

/// Summary of a stored BYOK secret (without key material).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByokSecretSummary {
    pub provider: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Upserts a sealed secret for a provider and resolves the stable row id
/// (first insert mints a fresh id; updates keep the existing row's id so
/// write-trail seqs and idempotency keys stay anchored to one entity).
/// Returns `(id, inserted)`.
pub async fn upsert_secret(
    tx: &mut sqlx::PgConnection,
    id: Uuid,
    org_id: Uuid,
    provider: &str,
    secret_encrypted: &[u8],
    created_by: Option<Uuid>,
) -> DbResult<(Uuid, bool)> {
    let existing: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM byok_secrets WHERE org_id = $1 AND provider = $2")
            .bind(org_id)
            .bind(provider)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(row_id) = existing {
        sqlx::query(
            "UPDATE byok_secrets SET secret_encrypted = $3, updated_at = now() \
             WHERE org_id = $1 AND provider = $2",
        )
        .bind(org_id)
        .bind(provider)
        .bind(secret_encrypted)
        .execute(&mut *tx)
        .await?;
        return Ok((row_id, false));
    }
    sqlx::query(
        r#"INSERT INTO byok_secrets (id, org_id, provider, secret_encrypted, created_by)
           VALUES ($1, $2, $3, $4, $5)"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(provider)
    .bind(secret_encrypted)
    .bind(created_by)
    .execute(&mut *tx)
    .await?;
    Ok((id, true))
}

/// Loads the row id + last-update stamp for one provider without any
/// secret bytes (envelope gate + delete flow need the entity anchor).
pub async fn get_secret_meta(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    provider: &str,
) -> DbResult<Option<(Uuid, chrono::DateTime<chrono::Utc>)>> {
    let row: Option<(Uuid, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, updated_at FROM byok_secrets WHERE org_id = $1 AND provider = $2",
    )
    .bind(org_id)
    .bind(provider)
    .fetch_optional(&mut *tx)
    .await?;
    Ok(row)
}

/// Retrieves the raw encrypted ciphertext for provider execution.
pub async fn get_secret<'e, E>(
    executor: E,
    org_id: Uuid,
    provider: &str,
) -> DbResult<Option<Vec<u8>>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row: Option<(Vec<u8>,)> = sqlx::query_as(
        "SELECT secret_encrypted FROM byok_secrets WHERE org_id = $1 AND provider = $2",
    )
    .bind(org_id)
    .bind(provider)
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|r| r.0))
}

/// Deletes a stored secret for a provider.
pub async fn delete_secret(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    provider: &str,
) -> DbResult<()> {
    sqlx::query("DELETE FROM byok_secrets WHERE org_id = $1 AND provider = $2")
        .bind(org_id)
        .bind(provider)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Lists available BYOK configurations for an org (summaries only, zero
/// secret bytes).
pub async fn list_secret_summaries<'e, E>(
    executor: E,
    org_id: Uuid,
) -> DbResult<Vec<ByokSecretSummary>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT provider, created_at, updated_at
           FROM byok_secrets
           WHERE org_id = $1
           ORDER BY provider"#,
        org_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ByokSecretSummary {
            provider: r.provider,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect())
}
