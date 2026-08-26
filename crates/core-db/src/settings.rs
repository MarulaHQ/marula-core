//! Typed settings registry persistence (docs/10-agent-platform.md §2).
//!
//! Purpose: store and retrieve validated setting overrides keyed by
//! (org, scope, key, owner). Only overrides live here — defaults come from
//! the declarations and are overlaid at read time (defaults-first).
//!
//! Non-purpose: validation of values against kinds (core-domain owns the
//! registry; callers validate BEFORE persisting) and authorization.

use crate::DbResult;
use sqlx::Postgres;
use uuid::Uuid;

/// One stored override: the registry key plus its JSONB payload.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingOverride {
    pub key: String,
    pub value: serde_json::Value,
}

/// Inserts or updates one org-scope setting override.
pub async fn upsert_org_setting(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    key: &str,
    value: &serde_json::Value,
    updated_by: Uuid,
) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO settings (id, org_id, scope, key, owner_id, value, updated_by)
           VALUES ($1, $2, 'org', $3, $2, $4, $5)
           ON CONFLICT (org_id, scope, key,
                        COALESCE(owner_id, '00000000-0000-0000-0000-000000000000'))
           DO UPDATE SET value = EXCLUDED.value,
                         updated_by = EXCLUDED.updated_by,
                         updated_at = now()"#,
        Uuid::now_v7(),
        org_id,
        key,
        value,
        updated_by,
    )
    .execute(tx)
    .await?;
    Ok(())
}

/// Inserts or updates one user-scope setting override for `user_id`.
pub async fn upsert_user_setting(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    key: &str,
    value: &serde_json::Value,
) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO settings (id, org_id, scope, key, owner_id, value, updated_by)
           VALUES ($1, $2, 'user', $3, $4, $5, $4)
           ON CONFLICT (org_id, scope, key,
                        COALESCE(owner_id, '00000000-0000-0000-0000-000000000000'))
           DO UPDATE SET value = EXCLUDED.value,
                         updated_by = EXCLUDED.updated_by,
                         updated_at = now()"#,
        Uuid::now_v7(),
        org_id,
        key,
        user_id,
        value,
    )
    .execute(tx)
    .await?;
    Ok(())
}

/// All org-scope overrides for the tenant.
pub async fn list_org_overrides(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
) -> DbResult<Vec<SettingOverride>> {
    let rows = sqlx::query!(
        r#"SELECT key, value FROM settings
           WHERE org_id = $1 AND scope = 'org'"#,
        org_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| SettingOverride {
            key: r.key,
            value: r.value,
        })
        .collect())
}

/// The caller's own user-scope overrides. RLS additionally hides other
/// users' rows; the explicit predicate keeps reads correct regardless of
/// role exemption.
pub async fn list_user_overrides(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    user_id: Uuid,
) -> DbResult<Vec<SettingOverride>> {
    let rows = sqlx::query!(
        r#"SELECT key, value FROM settings
           WHERE org_id = $1 AND scope = 'user' AND owner_id = $2"#,
        org_id,
        user_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| SettingOverride {
            key: r.key,
            value: r.value,
        })
        .collect())
}
