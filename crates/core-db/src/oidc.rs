//! Generic OIDC provider configuration and conservative account links.
//!
//! Purpose: org-level SSO config storage. Client secrets are stored as
//! AES-256-GCM ciphertext (`client_secret_enc`) and are never returned
//! after write — read paths expose only metadata.
//! Non-purpose: the protocol flow itself (discovery, token exchange,
//! validation) lives in core-api.

use crate::DbResult;
use sqlx::Postgres;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct OidcProviderRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub display_name: String,
    pub issuer: String,
    pub discovery_url: String,
    pub client_id: String,
    pub client_secret_enc: Vec<u8>,
    pub jit_provisioning: bool,
    pub enabled: bool,
    pub preset: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Provider configuration payload for [`upsert_provider`].
pub struct NewOidcProvider<'a> {
    pub name: &'a str,
    pub display_name: &'a str,
    pub issuer: &'a str,
    pub discovery_url: &'a str,
    pub client_id: &'a str,
    pub client_secret_enc: &'a [u8],
    pub jit_provisioning: bool,
    /// Name of the declarative first-party preset this provider was
    /// created from (`google` | `entra` | `okta`), when any.
    pub preset: Option<&'a str>,
}

/// Creates or updates a provider by (org, name). The secret is write-only:
/// no query in this module ever selects `client_secret_enc` back except
/// the login flow's internal resolution.
pub async fn upsert_provider<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    provider: NewOidcProvider<'_>,
    created_by: Uuid,
) -> DbResult<Uuid>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let NewOidcProvider {
        name,
        display_name,
        issuer,
        discovery_url,
        client_id,
        client_secret_enc,
        jit_provisioning,
        preset,
    } = provider;
    let row: (Uuid,) = sqlx::query_as(
        r#"INSERT INTO oidc_providers
             (id, org_id, name, display_name, issuer, discovery_url,
              client_id, client_secret_enc, jit_provisioning, enabled, created_by, preset)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, true, $10, $11)
           ON CONFLICT (org_id, name) DO UPDATE SET
             display_name = EXCLUDED.display_name,
             issuer = EXCLUDED.issuer,
             discovery_url = EXCLUDED.discovery_url,
             client_id = EXCLUDED.client_id,
             client_secret_enc = EXCLUDED.client_secret_enc,
             jit_provisioning = EXCLUDED.jit_provisioning,
             preset = EXCLUDED.preset,
             updated_at = now()
           RETURNING id"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(name)
    .bind(display_name)
    .bind(issuer)
    .bind(discovery_url)
    .bind(client_id)
    .bind(client_secret_enc)
    .bind(jit_provisioning)
    .bind(created_by)
    .bind(preset)
    .fetch_one(executor)
    .await?;
    Ok(row.0)
}

pub struct ProviderMeta {
    pub id: Uuid,
    pub name: String,
    pub display_name: String,
    pub issuer: String,
    pub discovery_url: String,
    pub client_id: String,
    pub jit_provisioning: bool,
    pub enabled: bool,
    pub preset: Option<String>,
}

/// Metadata listing for admins — secrets excluded by construction.
pub async fn list_providers<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<ProviderMeta>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as!(
        ProviderMeta,
        r#"SELECT id, name, display_name, issuer, discovery_url,
                  client_id, jit_provisioning, enabled, preset
           FROM oidc_providers WHERE org_id = $1 ORDER BY created_at"#,
        org_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

pub async fn get_provider_meta<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<ProviderMeta>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_as!(
        ProviderMeta,
        r#"SELECT id, name, display_name, issuer, discovery_url,
                  client_id, jit_provisioning, enabled, preset
           FROM oidc_providers WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(crate::DbError::NotFound)
}

/// Internal: full row including the encrypted secret (login flow only).
pub async fn find_provider_with_secret<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<OidcProviderRow>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_as!(
        OidcProviderRow,
        r#"SELECT id, org_id, name, display_name, issuer, discovery_url,
                  client_id, client_secret_enc, jit_provisioning, enabled, preset, created_at
           FROM oidc_providers WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(crate::DbError::NotFound)
}

pub async fn set_provider_enabled<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    enabled: bool,
) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        "UPDATE oidc_providers SET enabled = $3, updated_at = now() WHERE id = $2 AND org_id = $1",
        org_id,
        id,
        enabled
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

pub async fn delete_provider<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        "DELETE FROM oidc_providers WHERE id = $2 AND org_id = $1",
        org_id,
        id
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

// --- account links ---------------------------------------------------------

#[derive(Debug, Clone)]
pub struct OidcLink {
    pub user_id: Uuid,
    pub subject: String,
    pub email: String,
}

pub async fn find_link<'e, E>(
    executor: E,
    provider_id: Uuid,
    subject: &str,
) -> DbResult<Option<OidcLink>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as!(
        OidcLink,
        "SELECT user_id, subject, email FROM oidc_accounts WHERE provider_id = $1 AND subject = $2",
        provider_id,
        subject
    )
    .fetch_optional(executor)
    .await?;
    Ok(row)
}

/// Identity asserted by the IdP for a link row.
pub struct IdpIdentity<'a> {
    pub subject: &'a str,
    pub email: &'a str,
}

/// Explicit link: binds an IdP subject to an existing local user. `linked_by`
/// is None only for JIT-provisioned accounts (no human performed the link).
pub async fn create_link<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    provider_id: Uuid,
    user_id: Uuid,
    identity: IdpIdentity<'_>,
    linked_by: Option<Uuid>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let IdpIdentity { subject, email } = identity;
    sqlx::query!(
        r#"INSERT INTO oidc_accounts (id, org_id, provider_id, user_id, subject, email, linked_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        id,
        org_id,
        provider_id,
        user_id,
        subject,
        email,
        linked_by
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn touch_link_login<'e, E>(executor: E, provider_id: Uuid, subject: &str) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        "UPDATE oidc_accounts SET last_login_at = now() WHERE provider_id = $1 AND subject = $2",
        provider_id,
        subject
    )
    .execute(executor)
    .await?;
    Ok(())
}
