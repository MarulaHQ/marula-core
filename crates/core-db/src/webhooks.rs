//! core-db/webhooks — persistence for plugin runtime v1 (the private build tracker 2.15).
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
use sqlx::Row;
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
    /// Wave 43 lane 4 (A): the install whose declared endpoints seeded
    /// this row, or NULL for a human-created subscription. An
    /// install-attributed row is torn down by the install's lifecycle —
    /// `ON DELETE CASCADE` on uninstall, and an explicit delete beside the
    /// status flip in `revoke_catalog_entry` on revoke — so a connector's
    /// outbound posture cannot outlive the consent that created it.
    pub owner_install_id: Option<Uuid>,
}

/// The SELECT shape of a subscription row, in the order the map helper
/// expects. Kept as one const because four queries share it and a
/// disagreement between them is a silent type error.
const WEBHOOK_COLS: &str = "id, org_id, url, secret_encrypted, event_types, enabled, \
                            created_by, created_at, owner_install_id";

fn webhook_from(r: &sqlx::postgres::PgRow) -> DbResult<WebhookRow> {
    Ok(WebhookRow {
        id: r.get("id"),
        org_id: r.get("org_id"),
        url: r.get("url"),
        secret_encrypted: r.get("secret_encrypted"),
        event_types: r.get("event_types"),
        enabled: r.get("enabled"),
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
        owner_install_id: r.get("owner_install_id"),
    })
}

/// Inserts one subscription. `secret_encrypted` must already be sealed by
/// the caller; this module never sees plaintext. `owner_install_id` is
/// NULL for a human-created subscription and set when a connector's
/// declared endpoints seed it.
#[allow(clippy::too_many_arguments)]
pub async fn insert_webhook<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    url: &str,
    secret_encrypted: &[u8],
    event_types: &[String],
    created_by: Uuid,
    owner_install_id: Option<Uuid>,
) -> DbResult<WebhookRow>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query(&format!(
        "INSERT INTO webhooks (id, org_id, url, secret_encrypted, event_types, created_by, \
         owner_install_id) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING {WEBHOOK_COLS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(url)
    .bind(secret_encrypted)
    .bind(event_types)
    .bind(created_by)
    .bind(owner_install_id)
    .fetch_one(executor)
    .await?;
    webhook_from(&row)
}

/// Seeds the outbound subscriptions a consented connector's declared
/// `connector_profile.webhook_endpoints` promise (Wave 43 lane 4 A).
///
/// The consent screen has always said "Declared webhook endpoints seed the
/// connector's outbound posture"; this is the code that makes that true.
/// The rows are attributed to the install so an uninstall or a revoke
/// tears them down, which is what turns a declaration into a teardown-able
/// posture rather than decoration.
///
/// Idempotent and non-destructive to an operator's choices, in two halves:
/// a row the manifest no longer declares is deleted (the deny-by-default
/// residue is deletion, the same posture the surface sync keeps), while a
/// still-declared row is left alone so an admin who disabled one endpoint
/// keeps it disabled. Returns how many rows this call created.
pub async fn seed_install_webhooks<'e, E>(
    executor: E,
    org_id: Uuid,
    install_id: Uuid,
    urls: &[String],
    secret_encrypted: &[u8],
    created_by: Uuid,
) -> DbResult<usize>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    if urls.is_empty() {
        // Nothing declared: the connector's outbound posture is empty, and
        // any earlier seeding is stale rather than retained.
        remove_install_webhooks(executor, org_id, install_id).await?;
        return Ok(0);
    }

    // One statement, three halves. They touch disjoint row sets, so the
    // single snapshot they share makes the order irrelevant: the insert
    // only ever adds a URL that is in the declared list, and the delete
    // only ever removes a URL that is not. `ON CONFLICT DO NOTHING` is what
    // preserves an operator's choices on a re-seed — a row they disabled
    // stays disabled, and one they deleted stays deleted.
    let ids: Vec<Uuid> = urls.iter().map(|_| Uuid::now_v7()).collect();
    let seeded: i64 = sqlx::query_scalar(
        "WITH incoming AS (
             SELECT * FROM UNNEST($3::uuid[], $4::text[]) AS t(id, url)
         ),
         ins AS (
             INSERT INTO webhooks
                 (id, org_id, url, secret_encrypted, event_types, created_by,
                  owner_install_id)
             SELECT t.id, $1, t.url, $5, $6::text[], $7, $2
             FROM incoming t
             ON CONFLICT DO NOTHING
             RETURNING id
         ),
         del AS (
             DELETE FROM webhooks
             WHERE org_id = $1 AND owner_install_id = $2
               AND url NOT IN (SELECT url FROM incoming)
             RETURNING id
         )
         SELECT count(*)::int8 FROM ins",
    )
    .bind(org_id)
    .bind(install_id)
    .bind(&ids)
    .bind(urls)
    .bind(secret_encrypted)
    .bind(Vec::<String>::new())
    .bind(created_by)
    .fetch_one(executor)
    .await?;
    Ok(seeded.try_into().unwrap_or(0))
}

/// Deletes every subscription this install seeded. Called on uninstall
/// (belt-and-braces beside the FK's `ON DELETE CASCADE`) and on revoke,
/// where a status flip is not a delete so the FK never fires.
pub async fn remove_install_webhooks<'e, E>(
    executor: E,
    org_id: Uuid,
    install_id: Uuid,
) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let res = sqlx::query("DELETE FROM webhooks WHERE org_id = $1 AND owner_install_id = $2")
        .bind(org_id)
        .bind(install_id)
        .execute(executor)
        .await?;
    Ok(res.rows_affected())
}

pub async fn get_webhook<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<WebhookRow>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query(&format!(
        "SELECT {WEBHOOK_COLS} FROM webhooks WHERE id = $1 AND org_id = $2"
    ))
    .bind(id)
    .bind(org_id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    webhook_from(&row)
}

pub async fn list_webhooks<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<WebhookRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query(&format!(
        "SELECT {WEBHOOK_COLS} FROM webhooks WHERE org_id = $1 ORDER BY created_at DESC, id DESC"
    ))
    .bind(org_id)
    .fetch_all(executor)
    .await?;
    rows.iter().map(webhook_from).collect()
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
    let row = sqlx::query(&format!(
        "UPDATE webhooks SET
            url = COALESCE($3, url),
            event_types = COALESCE($4, event_types),
            enabled = COALESCE($5, enabled),
            updated_at = now()
          WHERE id = $1 AND org_id = $2
          RETURNING {WEBHOOK_COLS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(url)
    .bind(event_types)
    .bind(enabled)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    webhook_from(&row)
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
///
/// Wave 43 lane 4 (A): an install-attributed row is live only while its
/// install is ENABLED and not revoked. This is the gate that keeps a
/// connector's outbound posture honest through the install lifecycle —
/// a fresh install is created DISABLED (`webhooks.enabled` defaults true),
/// so without it a seeded row would live-deliver before anyone enabled
/// the connector. It is a `LEFT JOIN` precisely so a human-created row
/// (`owner_install_id IS NULL`) passes through the `OR` untouched, exactly
/// as before. Revocation gets this for free, which is defence in depth
/// beside `revoke_catalog_entry` deleting the rows outright.
///
/// Safe with the cross-org caller: the owner role bypasses
/// `plugin_installs_org`, so the join resolves across orgs rather than
/// silently returning nothing for every other org.
pub async fn find_enabled_for_event<'e, E>(
    executor: E,
    org_id: Uuid,
    event_type: &str,
) -> DbResult<Vec<WebhookRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query(
        "SELECT w.id, w.org_id, w.url, w.secret_encrypted, w.event_types, w.enabled,
                w.created_by, w.created_at, w.owner_install_id
         FROM webhooks w
         LEFT JOIN plugin_installs i
                ON i.id = w.owner_install_id AND i.org_id = w.org_id
         WHERE w.org_id = $1 AND w.enabled
           AND (w.event_types = '{}' OR $2 = ANY(w.event_types))
           AND (w.owner_install_id IS NULL
                OR (i.status = 'enabled' AND i.revoked_at IS NULL))
         ORDER BY w.created_at",
    )
    .bind(org_id)
    .bind(event_type)
    .fetch_all(executor)
    .await?;
    rows.iter().map(webhook_from).collect()
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
    /// Non-secret display identifier (e.g. "mrl_ab12cd34") for listings.
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
/// `marula_app` role and the table owner). Only active keys resolve.
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
