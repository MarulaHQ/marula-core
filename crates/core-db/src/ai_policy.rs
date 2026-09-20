//! AI availability matrix persistence (ADR 0008, migration 0081).
//!
//! Purpose: surface registry (as data), admin-curated model catalog,
//! per-surface per-role/user grants (deny-by-default), the sealed
//! ModelCredential slot (org AND personal credentials), the append-only
//! usage ledger, soft/hard token caps, and sticky per-surface model
//! picks. Query style follows `byok::` — plain string queries so the
//! module stays readable against the 0081 schema and independent of the
//! offline query-cache state.
//!
//! Non-purpose: NO enforcement decision logic (precedence, fallback,
//! refusal typing — core-api owns the completion edge), no HTTP, no
//! provider transports. Secret bytes are only ever stored or fetched as
//! sealed ciphertext here — never logged, never returned to clients.

use crate::DbResult;
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Surface registry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiSurface {
    pub id: Uuid,
    pub org_id: Option<Uuid>,
    pub key: String,
    pub label: String,
    pub kind: String,
    pub description: String,
    pub enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Lists the visible surface registry for an org: globally seeded rows
/// (org_id NULL) plus the org's own plugin registrations.
pub async fn list_surfaces<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<AiSurface>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT id, org_id, key, label, kind, description, enabled, created_at, updated_at
           FROM ai_surfaces
           WHERE org_id IS NULL OR org_id = $1
           ORDER BY kind, key"#,
    )
    .bind(org_id)
    .fetch_all(executor)
    .await?;
    rows.into_iter().map(surface_from).collect()
}

/// Resolves one surface by key: an org-scoped plugin registration wins
/// over the global seed when both exist.
pub async fn get_surface_by_key<'e, E>(
    executor: E,
    org_id: Uuid,
    key: &str,
) -> DbResult<Option<AiSurface>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query(
        r#"SELECT id, org_id, key, label, kind, description, enabled, created_at, updated_at
           FROM ai_surfaces
           WHERE key = $2 AND (org_id IS NULL OR org_id = $1)
           ORDER BY (org_id IS NOT NULL) DESC
           LIMIT 1"#,
    )
    .bind(org_id)
    .bind(key)
    .fetch_optional(executor)
    .await?;
    row.map(surface_from).transpose()
}

/// Registers (or updates) an org-scoped plugin surface. Native/internal
/// kinds stay global seed data — this path only mints `plugin` rows.
pub async fn upsert_org_plugin_surface(
    tx: &mut PgConnection,
    id: Uuid,
    org_id: Uuid,
    key: &str,
    label: &str,
    description: &str,
    created_by: Option<Uuid>,
) -> DbResult<(Uuid, bool)> {
    let existing: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM ai_surfaces WHERE key = $2 AND org_id = $1")
            .bind(org_id)
            .bind(key)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(row_id) = existing {
        sqlx::query(
            r#"UPDATE ai_surfaces
               SET label = $2, description = $3, enabled = true, updated_at = now()
                WHERE id = $4 AND org_id = $1"#,
        )
        .bind(org_id)
        .bind(label)
        .bind(description)
        .bind(row_id)
        .execute(&mut *tx)
        .await?;
        return Ok((row_id, false));
    }
    sqlx::query(
        r#"INSERT INTO ai_surfaces (id, org_id, key, label, kind, description, created_by)
           VALUES ($1, $2, $3, $4, 'plugin', $5, $6)"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(key)
    .bind(label)
    .bind(description)
    .bind(created_by)
    .execute(&mut *tx)
    .await?;
    Ok((id, true))
}

/// Every org-scoped plugin surface registration (kind = 'plugin'),
/// newest first — the admin list behind the plugin-surface HTTP routes.
pub async fn list_org_plugin_surfaces<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<AiSurface>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT id, org_id, key, label, kind, description, enabled, created_at, updated_at
           FROM ai_surfaces WHERE org_id = $1 AND kind = 'plugin'
           ORDER BY created_at DESC"#,
    )
    .bind(org_id)
    .fetch_all(executor)
    .await?;
    rows.into_iter().map(surface_from).collect()
}

/// Enable/disable one org-scoped surface. Returns the post-update row,
/// or None when the id is not an org-scoped surface of this org (the
/// globally seeded native/plugin keys are NOT administrable here).
pub async fn set_surface_enabled(
    tx: &mut PgConnection,
    org_id: Uuid,
    surface_id: Uuid,
    enabled: bool,
) -> DbResult<Option<AiSurface>> {
    let updated = sqlx::query(
        r#"UPDATE ai_surfaces SET enabled = $3, updated_at = now()
           WHERE id = $2 AND org_id = $1"#,
    )
    .bind(org_id)
    .bind(surface_id)
    .bind(enabled)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() == 0 {
        return Ok(None);
    }
    let row = sqlx::query(
        r#"SELECT id, org_id, key, label, kind, description, enabled, created_at, updated_at
           FROM ai_surfaces WHERE id = $1 AND org_id = $2"#,
    )
    .bind(surface_id)
    .bind(org_id)
    .fetch_one(&mut *tx)
    .await?;
    surface_from(row).map(Some)
}

fn surface_from(row: sqlx::postgres::PgRow) -> DbResult<AiSurface> {
    use sqlx::Row;
    Ok(AiSurface {
        id: row.get("id"),
        org_id: row.get("org_id"),
        key: row.get("key"),
        label: row.get("label"),
        kind: row.get("kind"),
        description: row.get("description"),
        enabled: row.get("enabled"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// One surface by id within the org (global rows surface through their
/// org visibility; org rows only their own).
pub async fn get_surface_by_id<'e, E>(
    executor: E,
    org_id: Uuid,
    surface_id: Uuid,
) -> DbResult<Option<AiSurface>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query(
        r#"SELECT id, org_id, key, label, kind, description, enabled, created_at, updated_at
           FROM ai_surfaces WHERE id = $2 AND (org_id = $1 OR org_id IS NULL)"#,
    )
    .bind(org_id)
    .bind(surface_id)
    .fetch_optional(executor)
    .await?;
    row.map(surface_from).transpose()
}

// ---------------------------------------------------------------------------
// Model catalog (admin-curated)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct AiModelCatalogEntry {
    pub id: Uuid,
    pub org_id: Uuid,
    pub provider: String,
    pub model: String,
    pub label: String,
    pub capabilities: Value,
    pub cost_weight: i32,
    pub enabled: bool,
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_catalog_entry(
    tx: &mut PgConnection,
    id: Uuid,
    org_id: Uuid,
    provider: &str,
    model: &str,
    label: &str,
    capabilities: &Value,
    cost_weight: i32,
    enabled: bool,
    created_by: Option<Uuid>,
) -> DbResult<(Uuid, bool)> {
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM ai_model_catalog WHERE org_id = $1 AND provider = $2 AND model = $3",
    )
    .bind(org_id)
    .bind(provider)
    .bind(model)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(row_id) = existing {
        sqlx::query(
            r#"UPDATE ai_model_catalog
               SET label = $1, capabilities = $2, cost_weight = $3, enabled = $4,
                   updated_at = now()
               WHERE id = $5"#,
        )
        .bind(label)
        .bind(capabilities)
        .bind(cost_weight)
        .bind(enabled)
        .bind(row_id)
        .execute(&mut *tx)
        .await?;
        return Ok((row_id, false));
    }
    sqlx::query(
        r#"INSERT INTO ai_model_catalog
             (id, org_id, provider, model, label, capabilities, cost_weight, enabled, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(provider)
    .bind(model)
    .bind(label)
    .bind(capabilities)
    .bind(cost_weight)
    .bind(enabled)
    .bind(created_by)
    .execute(&mut *tx)
    .await?;
    Ok((id, true))
}

pub async fn list_catalog<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<AiModelCatalogEntry>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT id, org_id, provider, model, label, capabilities, cost_weight, enabled
           FROM ai_model_catalog
           WHERE org_id = $1
           ORDER BY provider, model"#,
    )
    .bind(org_id)
    .fetch_all(executor)
    .await?;
    rows.into_iter().map(catalog_from).collect()
}

pub async fn get_catalog_entry<'e, E>(
    executor: E,
    org_id: Uuid,
    provider: &str,
    model: &str,
) -> DbResult<Option<AiModelCatalogEntry>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query(
        r#"SELECT id, org_id, provider, model, label, capabilities, cost_weight, enabled
           FROM ai_model_catalog
           WHERE org_id = $1 AND provider = $2 AND model = $3"#,
    )
    .bind(org_id)
    .bind(provider)
    .bind(model)
    .fetch_optional(executor)
    .await?;
    row.map(catalog_from).transpose()
}

pub async fn get_catalog_entry_by_id<'e, E>(
    executor: E,
    org_id: Uuid,
    entry_id: Uuid,
) -> DbResult<Option<AiModelCatalogEntry>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query(
        r#"SELECT id, org_id, provider, model, label, capabilities, cost_weight, enabled
           FROM ai_model_catalog
           WHERE id = $2 AND org_id = $1"#,
    )
    .bind(org_id)
    .bind(entry_id)
    .fetch_optional(executor)
    .await?;
    row.map(catalog_from).transpose()
}

pub async fn delete_catalog_entry(tx: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<bool> {
    let result = sqlx::query("DELETE FROM ai_model_catalog WHERE id = $2 AND org_id = $1")
        .bind(org_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    Ok(result.rows_affected() > 0)
}

fn catalog_from(row: sqlx::postgres::PgRow) -> DbResult<AiModelCatalogEntry> {
    use sqlx::Row;
    Ok(AiModelCatalogEntry {
        id: row.get("id"),
        org_id: row.get("org_id"),
        provider: row.get("provider"),
        model: row.get("model"),
        label: row.get("label"),
        capabilities: row.get("capabilities"),
        cost_weight: row.get("cost_weight"),
        enabled: row.get("enabled"),
    })
}

// ---------------------------------------------------------------------------
// Surface grants (deny-by-default; every mutation audited by the API layer)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiSurfaceGrant {
    pub id: Uuid,
    pub org_id: Uuid,
    pub surface_id: Uuid,
    pub surface_key: String,
    pub grantee_kind: String,
    pub role_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_grant(
    tx: &mut PgConnection,
    id: Uuid,
    org_id: Uuid,
    surface_id: Uuid,
    grantee_kind: &str,
    role_id: Option<Uuid>,
    user_id: Option<Uuid>,
    created_by: Uuid,
) -> DbResult<(Uuid, bool)> {
    let existing: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT g.id FROM ai_surface_grants g
           WHERE g.org_id = $1 AND g.surface_id = $2 AND g.grantee_kind = $3
             AND g.role_id IS NOT DISTINCT FROM $4
             AND g.user_id IS NOT DISTINCT FROM $5"#,
    )
    .bind(org_id)
    .bind(surface_id)
    .bind(grantee_kind)
    .bind(role_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(row_id) = existing {
        sqlx::query("UPDATE ai_surface_grants SET updated_at = now() WHERE id = $1")
            .bind(row_id)
            .execute(&mut *tx)
            .await?;
        return Ok((row_id, false));
    }
    sqlx::query(
        r#"INSERT INTO ai_surface_grants
             (id, org_id, surface_id, grantee_kind, role_id, user_id, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(surface_id)
    .bind(grantee_kind)
    .bind(role_id)
    .bind(user_id)
    .bind(created_by)
    .execute(&mut *tx)
    .await?;
    Ok((id, true))
}

/// Removes one grant row. Returns false when the target was not visible
/// in the caller's org scope (RlsSparse semantics at the API layer).
pub async fn revoke_grant(tx: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<bool> {
    let result = sqlx::query("DELETE FROM ai_surface_grants WHERE id = $2 AND org_id = $1")
        .bind(org_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn list_grants<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<AiSurfaceGrant>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT g.id, g.org_id, g.surface_id, s.key AS surface_key, g.grantee_kind,
                  g.role_id, g.user_id, g.created_at, g.updated_at
           FROM ai_surface_grants g
           JOIN ai_surfaces s ON s.id = g.surface_id
           WHERE g.org_id = $1
           ORDER BY s.key, g.grantee_kind, g.user_id NULLS FIRST, g.role_id NULLS FIRST"#,
    )
    .bind(org_id)
    .fetch_all(executor)
    .await?;
    rows.into_iter().map(grant_from).collect()
}

/// Deny-by-default availability check: the user holds the surface when a
/// user grant names them OR a role grant names one of their roles, and
/// the surface row itself is enabled (disabled registry rows deny).
pub async fn user_has_surface_grant<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
    surface_id: Uuid,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let granted: Option<bool> = sqlx::query_scalar(
        r#"SELECT EXISTS (
             SELECT 1
             FROM ai_surface_grants g
             JOIN ai_surfaces s ON s.id = g.surface_id
             WHERE g.org_id = $1 AND g.surface_id = $2 AND s.enabled
               AND (
                 (g.grantee_kind = 'user' AND g.user_id = $3)
                 OR (g.grantee_kind = 'role' AND g.role_id IN (
                       SELECT ur.role_id FROM user_roles ur WHERE ur.user_id = $3))
               )
           )"#,
    )
    .bind(org_id)
    .bind(surface_id)
    .bind(user_id)
    .fetch_optional(executor)
    .await?;
    Ok(granted.unwrap_or(false))
}

fn grant_from(row: sqlx::postgres::PgRow) -> DbResult<AiSurfaceGrant> {
    use sqlx::Row;
    Ok(AiSurfaceGrant {
        id: row.get("id"),
        org_id: row.get("org_id"),
        surface_id: row.get("surface_id"),
        surface_key: row.get("surface_key"),
        grantee_kind: row.get("grantee_kind"),
        role_id: row.get("role_id"),
        user_id: row.get("user_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

// ---------------------------------------------------------------------------
// ModelCredential slot (sealed, never-returned)
// ---------------------------------------------------------------------------

/// Owner filter for summary listings (never secret bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialOwnerFilter {
    /// Org credentials only (owner NULL) — the admin org view.
    OrgOnly,
    /// One user's personal credentials.
    PersonalOf(Uuid),
    /// Every owner in the org (admin matrix view).
    AllOwners,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiModelCredentialSummary {
    pub id: Uuid,
    pub org_id: Uuid,
    pub owner_user_id: Option<Uuid>,
    pub provider: String,
    pub kind: String,
    pub label: String,
    pub credential_meta: Value,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Inserts or replaces a sealed credential for its owner anchor (org
/// credentials anchor owner NULL; personal ones the user id). The row id
/// stays stable across re-seals so write-trail seqs stay anchored.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_credential(
    tx: &mut PgConnection,
    id: Uuid,
    org_id: Uuid,
    owner_user_id: Option<Uuid>,
    provider: &str,
    kind: &str,
    label: &str,
    secret_encrypted: &[u8],
    credential_meta: &Value,
    created_by: Uuid,
) -> DbResult<(Uuid, bool)> {
    let existing: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM ai_model_credentials
           WHERE org_id = $1 AND provider = $2 AND kind = $3
             AND owner_user_id IS NOT DISTINCT FROM $4"#,
    )
    .bind(org_id)
    .bind(provider)
    .bind(kind)
    .bind(owner_user_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(row_id) = existing {
        sqlx::query(
            r#"UPDATE ai_model_credentials
               SET label = $2, secret_encrypted = $3, credential_meta = $4,
                    status = 'active', updated_at = now()
                WHERE id = $5 AND org_id = $1"#,
        )
        .bind(org_id)
        .bind(label)
        .bind(secret_encrypted)
        .bind(credential_meta)
        .bind(row_id)
        .execute(&mut *tx)
        .await?;
        return Ok((row_id, false));
    }
    sqlx::query(
        r#"INSERT INTO ai_model_credentials
             (id, org_id, owner_user_id, provider, kind, label,
              secret_encrypted, credential_meta, status, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'active', $9)"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(owner_user_id)
    .bind(provider)
    .bind(kind)
    .bind(label)
    .bind(secret_encrypted)
    .bind(credential_meta)
    .bind(created_by)
    .execute(&mut *tx)
    .await?;
    Ok((id, true))
}

/// Lists credential summaries (zero secret bytes) by owner filter.
pub async fn list_credential_summaries<'e, E>(
    executor: E,
    org_id: Uuid,
    filter: CredentialOwnerFilter,
) -> DbResult<Vec<AiModelCredentialSummary>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let (sql, bind_user): (&str, Option<Uuid>) = match filter {
        CredentialOwnerFilter::OrgOnly => (
            "SELECT id, org_id, owner_user_id, provider, kind, label, credential_meta,
                    status, created_at, updated_at
             FROM ai_model_credentials
             WHERE org_id = $1 AND owner_user_id IS NULL
             ORDER BY provider, kind",
            None,
        ),
        CredentialOwnerFilter::PersonalOf(user_id) => (
            "SELECT id, org_id, owner_user_id, provider, kind, label, credential_meta,
                    status, created_at, updated_at
             FROM ai_model_credentials
             WHERE org_id = $1 AND owner_user_id = $2
             ORDER BY provider, kind",
            Some(user_id),
        ),
        CredentialOwnerFilter::AllOwners => (
            "SELECT id, org_id, owner_user_id, provider, kind, label, credential_meta,
                    status, created_at, updated_at
             FROM ai_model_credentials
             WHERE org_id = $1
             ORDER BY owner_user_id NULLS FIRST, provider, kind",
            None,
        ),
    };
    let mut query = sqlx::query(sql).bind(org_id);
    if let Some(user) = bind_user {
        query = query.bind(user);
    }
    let rows = query.fetch_all(executor).await?;
    rows.into_iter().map(credential_from).collect()
}

/// Marks a credential revoked (rows keep their anchor for the write
/// trail; resolution ignores revoked rows). `owner_lock` MUST match the
/// row's owner anchor — personal credentials are only revocable by their
/// owner context; returns false when nothing matched.
pub async fn revoke_credential(
    tx: &mut PgConnection,
    org_id: Uuid,
    credential_id: Uuid,
    owner_lock: Option<Uuid>,
) -> DbResult<bool> {
    let result = sqlx::query(
        r#"UPDATE ai_model_credentials
           SET status = 'revoked', updated_at = now()
           WHERE id = $2 AND org_id = $1
             AND owner_user_id IS NOT DISTINCT FROM $3"#,
    )
    .bind(org_id)
    .bind(credential_id)
    .bind(owner_lock)
    .execute(&mut *tx)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Resolution for the completion edge: personal (owner = user, active)
/// first when the org policy flag allows personal credentials, else the
/// org credential (owner NULL, active). Returns the row id + sealed
/// ciphertext; the CALLER opens it (`crypto::open`) — plaintext never
/// lives in this module.
pub async fn resolve_credential_secret(
    executor: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    provider: &str,
    personal_allowed: bool,
) -> DbResult<Option<(Uuid, Vec<u8>)>> {
    if personal_allowed {
        let row: Option<(Uuid, Vec<u8>)> = sqlx::query_as(
            r#"SELECT id, secret_encrypted FROM ai_model_credentials
               WHERE org_id = $1 AND owner_user_id = $2 AND provider = $3 AND status = 'active'
               ORDER BY updated_at DESC LIMIT 1"#,
        )
        .bind(org_id)
        .bind(user_id)
        .bind(provider)
        .fetch_optional(&mut *executor)
        .await?;
        if row.is_some() {
            return Ok(row);
        }
    }
    let row: Option<(Uuid, Vec<u8>)> = sqlx::query_as(
        r#"SELECT id, secret_encrypted FROM ai_model_credentials
           WHERE org_id = $1 AND owner_user_id IS NULL AND provider = $2 AND status = 'active'
           ORDER BY updated_at DESC LIMIT 1"#,
    )
    .bind(org_id)
    .bind(provider)
    .fetch_optional(&mut *executor)
    .await?;
    Ok(row)
}

fn credential_from(row: sqlx::postgres::PgRow) -> DbResult<AiModelCredentialSummary> {
    use sqlx::Row;
    Ok(AiModelCredentialSummary {
        id: row.get("id"),
        org_id: row.get("org_id"),
        owner_user_id: row.get("owner_user_id"),
        provider: row.get("provider"),
        kind: row.get("kind"),
        label: row.get("label"),
        credential_meta: row.get("credential_meta"),
        status: row.get("status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// Revocation/audit meta for one credential (never secret bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiCredentialMeta {
    pub provider: String,
    pub kind: String,
    pub owner_user_id: Option<Uuid>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

pub async fn get_credential_meta<'e, E>(
    executor: E,
    org_id: Uuid,
    credential_id: Uuid,
) -> DbResult<Option<AiCredentialMeta>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row: Option<(String, String, Option<Uuid>, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as(
            r#"SELECT provider, kind, owner_user_id, updated_at
               FROM ai_model_credentials
               WHERE id = $2 AND org_id = $1"#,
        )
        .bind(org_id)
        .bind(credential_id)
        .fetch_optional(executor)
        .await?;
    Ok(row.map(
        |(provider, kind, owner_user_id, updated_at)| AiCredentialMeta {
            provider,
            kind,
            owner_user_id,
            updated_at,
        },
    ))
}

// ---------------------------------------------------------------------------
// Usage ledger (append-only — UPDATE/DELETE are rejected by trigger)
// ---------------------------------------------------------------------------

/// New-ledger-row input (the INSERT shape accepted by [`record_usage`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiUsageNew {
    pub id: Uuid,
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub surface: String,
    pub provider: String,
    pub model: String,
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    /// The catalog `cost_weight` snapshotted at call time (1 when the
    /// resolved pair is outside the catalog — legacy fallback). Caps and
    /// progress read models consume the WEIGHTED total; the raw token
    /// columns stay authoritative for reconstruction.
    pub cost_weight: i32,
    pub request_id: Option<String>,
}

/// The org master switch (ADR 0010 §4, Wave 23 lane 2): `ai.features_enabled`,
/// default OFF. When OFF the org sees exactly the pre-AI product — the
/// shared admission pipeline refuses every call typed BEFORE touching
/// the surface/catalog, and the embeddings legs degrade to their
/// no-vector posture. One switch for all AI edges.
pub async fn features_enabled(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
) -> DbResult<bool> {
    let row: Option<(Value,)> = sqlx::query_as(
        r#"SELECT value FROM settings
           WHERE org_id = $1 AND scope = 'org' AND key = 'ai.features_enabled'"#,
    )
    .bind(org_id)
    .fetch_optional(executor)
    .await?;
    Ok(row.and_then(|(v,)| v.as_bool()).unwrap_or(false))
}

/// Append one usage row. Idempotent at the edge (Wave 18 lane 2b): a
/// repeated write carrying the SAME (org, surface, request_id) hits the
/// `uq_ai_usage_ledger_edge_request` partial unique index and is silently
/// dropped — dedupe lives in the database, not in runner discipline.
pub async fn record_usage(tx: &mut PgConnection, usage: &AiUsageNew) -> DbResult<()> {
    sqlx::query(
        r#"INSERT INTO ai_usage_ledger
             (id, org_id, user_id, surface, provider, model,
              prompt_tokens, completion_tokens, cost_weight, request_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           ON CONFLICT (org_id, surface, request_id) WHERE request_id IS NOT NULL
           DO NOTHING"#,
    )
    .bind(usage.id)
    .bind(usage.org_id)
    .bind(usage.user_id)
    .bind(&usage.surface)
    .bind(&usage.provider)
    .bind(&usage.model)
    .bind(usage.prompt_tokens)
    .bind(usage.completion_tokens)
    .bind(usage.cost_weight)
    .bind(&usage.request_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Total COST-WEIGHTED tokens for one user since a timestamp (Wave 18
/// lane 2b: caps consume `weighted_tokens`, not raw tokens; raw columns
/// remain the reconstruction source of truth).
pub async fn user_usage_since<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
    since: chrono::DateTime<chrono::Utc>,
) -> DbResult<i64>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let total: Option<i64> = sqlx::query_scalar(
        r#"SELECT COALESCE(SUM(weighted_tokens), 0)::bigint
           FROM ai_usage_ledger
           WHERE org_id = $1 AND user_id = $2 AND created_at >= $3"#,
    )
    .bind(org_id)
    .bind(user_id)
    .bind(since)
    .fetch_one(executor)
    .await?;
    Ok(total.unwrap_or(0))
}

/// Total COST-WEIGHTED tokens for the whole org since a timestamp.
pub async fn org_usage_since<'e, E>(
    executor: E,
    org_id: Uuid,
    since: chrono::DateTime<chrono::Utc>,
) -> DbResult<i64>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let total: Option<i64> = sqlx::query_scalar(
        r#"SELECT COALESCE(SUM(weighted_tokens), 0)::bigint
           FROM ai_usage_ledger
           WHERE org_id = $1 AND created_at >= $2"#,
    )
    .bind(org_id)
    .bind(since)
    .fetch_one(executor)
    .await?;
    Ok(total.unwrap_or(0))
}

/// The cap periods the schema admits (migration 0091 widened the CHECK
/// from 'month'-only to this superset).
pub const VALID_PERIODS: [&str; 4] = ["week", "month", "quarter", "year"];

/// One cap binding, with the usage ALREADY measured over THAT cap's own
/// period window (a 'week' cap is checked against week-to-date usage, not
/// month-to-date). Per-user rows shadow the org-wide row of the same
/// period.
#[derive(Debug, Clone)]
pub struct BindingCap {
    pub period: String,
    pub soft_token_limit: Option<i64>,
    pub hard_token_limit: Option<i64>,
    pub used_weighted: i64,
}

type CapRow = (String, Option<i64>, Option<i64>, Option<Uuid>, i64);

/// Every cap that binds this (org, user) right now — one entry per
/// configured period, per-user row winning over the org-wide row. Caps of
/// ALL periods bind at admission (Wave 18 lane 2b item 3): a weekly hard
/// cap is enforced exactly like the monthly one.
pub async fn binding_caps(
    executor: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
) -> DbResult<Vec<BindingCap>> {
    let rows: Vec<CapRow> = sqlx::query_as(
        r#"SELECT c.period, c.soft_token_limit, c.hard_token_limit,
                  c.user_id,
                  COALESCE((
                    SELECT SUM(l.weighted_tokens)
                    FROM ai_usage_ledger l
                    WHERE l.org_id = c.org_id
                      AND (c.user_id IS NULL OR l.user_id = c.user_id)
                      AND l.created_at >= date_trunc(c.period, now())
                  ), 0)::bigint AS used_weighted
           FROM ai_caps c
           WHERE c.org_id = $1 AND (c.user_id = $2 OR c.user_id IS NULL)
           ORDER BY c.period, (c.user_id IS NULL)"#,
    )
    .bind(org_id)
    .bind(user_id)
    .fetch_all(&mut *executor)
    .await?;
    // Per period: keep the first row (per-user sorts before org-wide).
    let mut out: Vec<BindingCap> = Vec::new();
    for (period, soft, hard, _user, used) in rows {
        if out.iter().any(|c| c.period == period) {
            continue;
        }
        out.push(BindingCap {
            period,
            soft_token_limit: soft,
            hard_token_limit: hard,
            used_weighted: used,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Caps
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiCap {
    pub id: Uuid,
    pub org_id: Uuid,
    pub user_id: Option<Uuid>,
    pub period: String,
    pub soft_token_limit: Option<i64>,
    pub hard_token_limit: Option<i64>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_cap(
    tx: &mut PgConnection,
    id: Uuid,
    org_id: Uuid,
    user_id: Option<Uuid>,
    period: &str,
    soft_token_limit: Option<i64>,
    hard_token_limit: Option<i64>,
    created_by: Uuid,
) -> DbResult<(Uuid, bool)> {
    let existing: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM ai_caps
           WHERE org_id = $1 AND period = $2
             AND user_id IS NOT DISTINCT FROM $3"#,
    )
    .bind(org_id)
    .bind(period)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(row_id) = existing {
        sqlx::query(
            r#"UPDATE ai_caps
               SET soft_token_limit = $3, hard_token_limit = $4, updated_at = now()
               WHERE id = $5 AND org_id = $1 AND period = $2"#,
        )
        .bind(org_id)
        .bind(period)
        .bind(soft_token_limit)
        .bind(hard_token_limit)
        .bind(row_id)
        .execute(&mut *tx)
        .await?;
        return Ok((row_id, false));
    }
    sqlx::query(
        r#"INSERT INTO ai_caps
             (id, org_id, user_id, period, soft_token_limit, hard_token_limit, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(user_id)
    .bind(period)
    .bind(soft_token_limit)
    .bind(hard_token_limit)
    .bind(created_by)
    .execute(&mut *tx)
    .await?;
    Ok((id, true))
}

/// Deletes a cap row entirely (clearing a limit = removing the row — a
/// cap row with neither limit set is rejected by the schema check).
pub async fn delete_cap(tx: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<bool> {
    let result = sqlx::query("DELETE FROM ai_caps WHERE id = $2 AND org_id = $1")
        .bind(org_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn list_caps<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<AiCap>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT id, org_id, user_id, period, soft_token_limit, hard_token_limit,
                  created_at, updated_at
           FROM ai_caps
           WHERE org_id = $1
           ORDER BY period, user_id NULLS FIRST"#,
    )
    .bind(org_id)
    .fetch_all(executor)
    .await?;
    rows.into_iter().map(cap_from).collect()
}

/// Effective limits for a user: their per-user cap when present, else the
/// org-wide cap (same period).
pub async fn effective_cap(
    executor: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    period: &str,
) -> DbResult<Option<(Option<i64>, Option<i64>)>> {
    let row: Option<(Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT soft_token_limit, hard_token_limit FROM ai_caps
         WHERE org_id = $1 AND period = $2 AND user_id = $3",
    )
    .bind(org_id)
    .bind(period)
    .bind(user_id)
    .fetch_optional(&mut *executor)
    .await?;
    if row.is_some() {
        return Ok(row);
    }
    let org_row: Option<(Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT soft_token_limit, hard_token_limit FROM ai_caps
         WHERE org_id = $1 AND period = $2 AND user_id IS NULL",
    )
    .bind(org_id)
    .bind(period)
    .fetch_optional(&mut *executor)
    .await?;
    Ok(org_row)
}

fn cap_from(row: sqlx::postgres::PgRow) -> DbResult<AiCap> {
    use sqlx::Row;
    Ok(AiCap {
        id: row.get("id"),
        org_id: row.get("org_id"),
        user_id: row.get("user_id"),
        period: row.get("period"),
        soft_token_limit: row.get("soft_token_limit"),
        hard_token_limit: row.get("hard_token_limit"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

// ---------------------------------------------------------------------------
// Sticky model picks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiModelPick {
    pub id: Uuid,
    pub org_id: Uuid,
    pub user_id: Option<Uuid>,
    pub surface_id: Uuid,
    pub provider: String,
    pub model: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_pick(
    tx: &mut PgConnection,
    id: Uuid,
    org_id: Uuid,
    user_id: Option<Uuid>,
    surface_id: Uuid,
    provider: &str,
    model: &str,
    updated_by: Uuid,
) -> DbResult<(Uuid, bool)> {
    let existing: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM ai_model_picks
           WHERE org_id = $1 AND surface_id = $2
             AND user_id IS NOT DISTINCT FROM $3"#,
    )
    .bind(org_id)
    .bind(surface_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(row_id) = existing {
        sqlx::query(
            r#"UPDATE ai_model_picks
               SET provider = $1, model = $2, updated_by = $3, updated_at = now()
               WHERE id = $4"#,
        )
        .bind(provider)
        .bind(model)
        .bind(updated_by)
        .bind(row_id)
        .execute(&mut *tx)
        .await?;
        return Ok((row_id, false));
    }
    sqlx::query(
        r#"INSERT INTO ai_model_picks
             (id, org_id, user_id, surface_id, provider, model, updated_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(user_id)
    .bind(surface_id)
    .bind(provider)
    .bind(model)
    .bind(updated_by)
    .execute(&mut *tx)
    .await?;
    Ok((id, true))
}

/// One user's sticky pick for a surface (NOT the org default).
pub async fn get_user_pick<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
    surface_id: Uuid,
) -> DbResult<Option<AiModelPick>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query(
        r#"SELECT id, org_id, user_id, surface_id, provider, model, updated_at
           FROM ai_model_picks
           WHERE org_id = $1 AND user_id = $2 AND surface_id = $3"#,
    )
    .bind(org_id)
    .bind(user_id)
    .bind(surface_id)
    .fetch_optional(executor)
    .await?;
    row.map(pick_from).transpose()
}

/// The org default pick for a surface.
pub async fn get_org_pick<'e, E>(
    executor: E,
    org_id: Uuid,
    surface_id: Uuid,
) -> DbResult<Option<AiModelPick>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query(
        r#"SELECT id, org_id, user_id, surface_id, provider, model, updated_at
           FROM ai_model_picks
           WHERE org_id = $1 AND user_id IS NULL AND surface_id = $2"#,
    )
    .bind(org_id)
    .bind(surface_id)
    .fetch_optional(executor)
    .await?;
    row.map(pick_from).transpose()
}

/// Removes a user's sticky pick (resolution then falls back to the org
/// default pick — the honest-fallback posture).
pub async fn delete_user_pick(
    tx: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    surface_id: Uuid,
) -> DbResult<bool> {
    let result = sqlx::query(
        "DELETE FROM ai_model_picks WHERE org_id = $1 AND user_id = $2 AND surface_id = $3",
    )
    .bind(org_id)
    .bind(user_id)
    .bind(surface_id)
    .execute(&mut *tx)
    .await?;
    Ok(result.rows_affected() > 0)
}

fn pick_from(row: sqlx::postgres::PgRow) -> DbResult<AiModelPick> {
    use sqlx::Row;
    Ok(AiModelPick {
        id: row.get("id"),
        org_id: row.get("org_id"),
        user_id: row.get("user_id"),
        surface_id: row.get("surface_id"),
        provider: row.get("provider"),
        model: row.get("model"),
        updated_at: row.get("updated_at"),
    })
}
