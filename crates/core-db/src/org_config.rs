//! Org-scoped configuration stores that are entities, not settings.
//!
//! Purpose: persistence for org-wide configuration whose values are richer
//! than the three registry payload types (boolean/integer/text) — today
//! the ticket-workspace pod-visibility matrix (migration 0064). One row
//! per org; the JSONB config is shape-validated by core-api before it
//! lands here.
//!
//! Non-purpose: the typed settings registry (core-domain `settings` owns
//! declarations and core-db `settings` stores those overrides) and any
//! notion of who may write (policy lives in core-domain/core-api).

use crate::DbResult;
use sqlx::PgConnection;
use uuid::Uuid;

/// One org's stored config document.
#[derive(Debug, Clone, PartialEq)]
pub struct OrgConfigRow {
    pub org_id: Uuid,
    pub config: serde_json::Value,
    pub updated_by: Option<Uuid>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Reads the stored config document; `None` when the org never wrote one
/// (defaults-first: callers fall back to client registry defaults).
pub async fn get_pod_visibility<'e, E>(executor: E, org_id: Uuid) -> DbResult<Option<OrgConfigRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as!(
        OrgConfigRow,
        r#"SELECT org_id, config, updated_by, updated_at
           FROM org_pod_visibility WHERE org_id = $1"#,
        org_id
    )
    .fetch_optional(executor)
    .await?;
    Ok(row)
}

/// Upserts the org's pod-visibility matrix (one row per org). The caller
/// validates the JSONB shape and emits the domain event; this only stores.
pub async fn upsert_pod_visibility(
    conn: &mut PgConnection,
    org_id: Uuid,
    config: &serde_json::Value,
    updated_by: Uuid,
) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO org_pod_visibility (id, org_id, config, updated_by)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (org_id) DO UPDATE
             SET config = EXCLUDED.config,
                 updated_by = EXCLUDED.updated_by,
                 updated_at = now()"#,
        Uuid::now_v7(),
        org_id,
        config,
        updated_by,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}
