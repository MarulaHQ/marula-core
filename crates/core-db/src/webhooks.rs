//! core-db/webhooks — persistence for plugin runtime v1 (TASKLIST 2.15).
//!
//! Purpose: typed queries for the four extension-surface tables from
//! migration 0020 — webhook subscriptions (secrets sealed at rest), the
//! per-attempt delivery ledger, hashed API keys, and connector
//! installations. Delivery inserts are PK/unique-guarded and idempotent so
//! at-least-once workers can report the same attempt twice without ever
//! double-logging.
//!
//! Non-purpose: no signing or backoff math (core-domain), no HTTP, no
//! secret decryption decisions (callers own key material and never log it).

use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use uuid::Uuid;

// --- webhook subscriptions ---------------------------------------------------

#[derive(Debug, Clone)]
pub struct WebhookRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub url: String,
    /// AES-256-GCM ciphertext (`nonce || ct`). Never logged.
    pub secret_encrypted: Vec<u8>,
    /// Empty = wildcard (all event types).
    pub event_types: Vec<String>,
    pub enabled: bool,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// Inserts one subscription. `secret_encrypted` must already be sealed by
/// the caller; this module never sees plaintext.
#[allow(clippy::too_many_arguments)]
pub async fn insert_webhook<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    url: &str,
    secret_encrypted: &[u8],
    event_types: &[String],
    created_by: Uuid,
) -> DbResult<WebhookRow>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as!(
        WebhookRow,
        r#"INSERT INTO webhooks (id, org_id, url, secret_encrypted, event_types, created_by)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING id, org_id, url, secret_encrypted, event_types, enabled,
                     created_by, created_at"#,
        id,
        org_id,
        url,
        secret_encrypted,
        event_types,
        created_by,
    )
    .fetch_one(executor)
    .await?;
    Ok(row)
}

pub async fn get_webhook<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<WebhookRow>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as!(
        WebhookRow,
        r#"SELECT id, org_id, url, secret_encrypted, event_types, enabled,
                  created_by, created_at
           FROM webhooks WHERE id = $1 AND org_id = $2"#,
        id,
        org_id,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row)
}

pub async fn list_webhooks<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<WebhookRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query_as!(
        WebhookRow,
        r#"SELECT id, org_id, url, secret_encrypted, event_types, enabled,
                  created_by, created_at
           FROM webhooks WHERE org_id = $1 ORDER BY created_at DESC, id DESC"#,
        org_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// Patches mutable fields; NULL leaves a field untouched.
pub async fn update_webhook<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    url: Option<&str>,
    event_types: Option<&[String]>,
    enabled: Option<bool>,
) -> DbResult<WebhookRow>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as!(
        WebhookRow,
        r#"UPDATE webhooks SET
               url = COALESCE($3, url),
               event_types = COALESCE($4, event_types),
               enabled = COALESCE($5, enabled),
               updated_at = now()
           WHERE id = $1 AND org_id = $2
           RETURNING id, org_id, url, secret_encrypted, event_types, enabled,
                     created_by, created_at"#,
        id,
        org_id,
        url,
        event_types,
        enabled,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row)
}

pub async fn delete_webhook<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let res = sqlx::query!(
        "DELETE FROM webhooks WHERE id = $1 AND org_id = $2",
        id,
        org_id
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Enabled subscriptions of one org matching an event type. An empty
/// `event_types` array is a wildcard. Called by the worker with the owner
/// role (cross-org loop), so no RLS scope is set here by design.
pub async fn find_enabled_for_event<'e, E>(
    executor: E,
    org_id: Uuid,
    event_type: &str,
) -> DbResult<Vec<WebhookRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query_as!(
        WebhookRow,
        r#"SELECT id, org_id, url, secret_encrypted, event_types, enabled,
                  created_by, created_at
           FROM webhooks
           WHERE org_id = $1 AND enabled
             AND (event_types = '{}' OR $2 = ANY(event_types))
           ORDER BY created_at"#,
        org_id,
        event_type,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

// --- delivery ledger ----------------------------------------------------------

/// One recorded attempt. Exactly one row per (webhook, event, attempt):
/// conflicting replays are silently ignored (`applied == false`).
#[derive(Debug, Clone)]
pub struct NewDeliveryAttempt {
    pub webhook_id: Uuid,
    pub event_id: Uuid,
    pub event_type: String,
    pub attempt: i32,
    pub response_status: Option<i32>,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

pub struct DeliveryRow {
    pub id: Uuid,
    pub webhook_id: Uuid,
    pub event_id: Uuid,
    pub event_type: String,
    pub attempt: i32,
    pub response_status: Option<i32>,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Records one attempt inside an already-org-scoped transaction.
/// Returns false when this (webhook, event, attempt) was already logged.
pub async fn insert_delivery_attempt(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    org_id: Uuid,
    attempt: &NewDeliveryAttempt,
) -> DbResult<bool> {
    let id = Uuid::now_v7();
    let res = sqlx::query!(
        r#"INSERT INTO webhook_deliveries
               (id, org_id, webhook_id, event_id, event_type, attempt,
                response_status, next_retry_at, delivered_at, last_error)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           ON CONFLICT (webhook_id, event_id, attempt) DO NOTHING"#,
        id,
        org_id,
        attempt.webhook_id,
        attempt.event_id,
        attempt.event_type,
        attempt.attempt,
        attempt.response_status,
        attempt.next_retry_at,
        attempt.delivered_at,
        attempt.last_error,
    )
    .execute(&mut **tx)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Keyset page of the delivery ledger for one webhook, newest first.
/// `cursor` is `(created_at, id)` of the last row of the previous page.
pub async fn list_deliveries(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    org_id: Uuid,
    webhook_id: Uuid,
    cursor: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> DbResult<Vec<DeliveryRow>> {
    let rows = match cursor {
        Some((before_at, before_id)) => {
            sqlx::query_as!(
                DeliveryRow,
                r#"SELECT id, webhook_id, event_id, event_type, attempt,
                          response_status, next_retry_at, delivered_at, last_error, created_at
                   FROM webhook_deliveries
                   WHERE org_id = $1 AND webhook_id = $2
                     AND (created_at, id) < ($3, $4)
                   ORDER BY created_at DESC, id DESC
                   LIMIT $5"#,
                org_id,
                webhook_id,
                before_at,
                before_id,
                limit,
            )
            .fetch_all(&mut **tx)
            .await?
        }
        None => {
            sqlx::query_as!(
                DeliveryRow,
                r#"SELECT id, webhook_id, event_id, event_type, attempt,
                          response_status, next_retry_at, delivered_at, last_error, created_at
                   FROM webhook_deliveries
                   WHERE org_id = $1 AND webhook_id = $2
                   ORDER BY created_at DESC, id DESC
                   LIMIT $3"#,
                org_id,
                webhook_id,
                limit,
            )
            .fetch_all(&mut **tx)
            .await?
        }
    };
    Ok(rows)
}

// --- API keys -------------------------------------------------------------------
// NOTE: the `api_keys` table shipped in migration 0001 (RLS included) with
// `key_hash text UNIQUE`, a display `prefix`, and JSONB scopes. This module
// adapts to that shape: the hash is the hex of HMAC-SHA256(secret_key, raw)
// and raw material is never stored (§6).

#[derive(Debug, Clone)]
pub struct ApiKeyRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    /// hex(HMAC-SHA256(secret_key, raw)). Raw material is never stored.
    pub key_hash: String,
    /// Non-secret display identifier (e.g. "mrd_ab12cd34") for listings.
    pub prefix: String,
    /// Capability scope strings ("tickets:read", …).
    pub scopes: Vec<String>,
    pub created_by: Option<Uuid>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_api_key<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    name: &str,
    key_hash: &str,
    prefix: &str,
    scopes: &[String],
    created_by: Uuid,
) -> DbResult<ApiKeyRow>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as!(
        ApiKeyRecord,
        r#"INSERT INTO api_keys (id, org_id, name, key_hash, prefix, scopes, created_by)
           VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7)
           RETURNING id, org_id, name, key_hash, prefix, scopes, created_by,
                     last_used_at, revoked_at, created_at"#,
        id,
        org_id,
        name,
        key_hash,
        prefix,
        serde_json::to_value(scopes).unwrap_or_else(|_| serde_json::json!([])),
        created_by,
    )
    .fetch_one(executor)
    .await?;
    Ok(api_key_from(row))
}

/// Raw SELECT shape: `scopes` stays JSONB and is widened to Vec<String>
/// in [`api_key_from`].
struct ApiKeyRecord {
    id: Uuid,
    org_id: Uuid,
    name: String,
    key_hash: String,
    prefix: String,
    scopes: serde_json::Value,
    created_by: Option<Uuid>,
    last_used_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

fn api_key_from(r: ApiKeyRecord) -> ApiKeyRow {
    let scopes = r
        .scopes
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    ApiKeyRow {
        id: r.id,
        org_id: r.org_id,
        name: r.name,
        key_hash: r.key_hash,
        prefix: r.prefix,
        scopes,
        created_by: r.created_by,
        last_used_at: r.last_used_at,
        revoked_at: r.revoked_at,
        created_at: r.created_at,
    }
}

pub async fn list_api_keys<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<ApiKeyRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query_as!(
        ApiKeyRecord,
        r#"SELECT id, org_id, name, key_hash, prefix, scopes, created_by,
                  last_used_at, revoked_at, created_at
           FROM api_keys WHERE org_id = $1 ORDER BY created_at DESC, id DESC"#,
        org_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(api_key_from).collect())
}

/// Marks a key revoked (idempotent: revoking twice stays revoked).
pub async fn revoke_api_key<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let res = sqlx::query!(
        r#"UPDATE api_keys SET revoked_at = COALESCE(revoked_at, now())
           WHERE id = $1 AND org_id = $2 AND revoked_at IS NULL"#,
        id,
        org_id,
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Lookup by presented-key hash for request authentication. Cross-org BY
/// DESIGN — the key itself selects the org and no `app.org_id` scope can
/// exist yet — so this goes through the narrow SECURITY DEFINER function
/// from migration 0020 (works identically under the constrained
/// `meridian_app` role and the table owner). Only active keys resolve.
pub async fn find_active_api_key_by_hash<'e, E>(
    executor: E,
    key_hash: &str,
) -> DbResult<Option<ApiKeyRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as!(
        ApiKeyRecord,
        r#"SELECT id AS "id!", org_id AS "org_id!", name AS "name!",
                  key_hash AS "key_hash!", prefix AS "prefix!",
                  scopes AS "scopes!", created_by, last_used_at, revoked_at,
                  created_at AS "created_at!"
           FROM api_key_auth_lookup($1)"#,
        key_hash,
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(api_key_from))
}

/// Last-used tracking, throttled: only writes when the previous stamp is
/// older than `min_interval_secs` (or absent), keeping auth reads cheap on
/// hot paths while still surfacing recent usage.
pub async fn touch_api_key_last_used<'e, E>(
    executor: E,
    id: Uuid,
    min_interval_secs: i64,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    // Same seam as the lookup: the auth path may run before any org is
    // bound, so the throttled write rides the SECURITY DEFINER function.
    sqlx::query!(
        "SELECT api_key_touch_last_used($1, $2)",
        id,
        min_interval_secs as i32,
    )
    .execute(executor)
    .await?;
    Ok(())
}

// --- connector installations ------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ConnectorInstallationRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub connector_type: String,
    pub config_encrypted: Vec<u8>,
    pub capability_scopes: Vec<String>,
    pub consented_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Upserts the single installation per (org, connector type): re-consenting
/// replaces config + scopes atomically.
pub async fn upsert_connector_installation<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    connector_type: &str,
    config_encrypted: &[u8],
    capability_scopes: &[String],
    consented_by: Uuid,
) -> DbResult<ConnectorInstallationRow>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as!(
        ConnectorInstallationRow,
        r#"INSERT INTO connector_installations
               (id, org_id, connector_type, config_encrypted, capability_scopes, consented_by)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (org_id, connector_type) DO UPDATE SET
               config_encrypted = EXCLUDED.config_encrypted,
               capability_scopes = EXCLUDED.capability_scopes,
               consented_by = EXCLUDED.consented_by,
               updated_at = now()
           RETURNING id, org_id, connector_type, config_encrypted, capability_scopes,
                     consented_by, created_at, updated_at"#,
        id,
        org_id,
        connector_type,
        config_encrypted,
        capability_scopes,
        consented_by,
    )
    .fetch_one(executor)
    .await?;
    Ok(row)
}

pub async fn list_connector_installations<'e, E>(
    executor: E,
    org_id: Uuid,
) -> DbResult<Vec<ConnectorInstallationRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query_as!(
        ConnectorInstallationRow,
        r#"SELECT id, org_id, connector_type, config_encrypted, capability_scopes,
                  consented_by, created_at, updated_at
           FROM connector_installations WHERE org_id = $1 ORDER BY connector_type"#,
        org_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

pub async fn delete_connector_installation<'e, E>(
    executor: E,
    org_id: Uuid,
    connector_type: &str,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let res = sqlx::query!(
        "DELETE FROM connector_installations WHERE org_id = $1 AND connector_type = $2",
        org_id,
        connector_type,
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}
