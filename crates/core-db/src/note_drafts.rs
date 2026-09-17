//! Note drafts — the IF2 autosave store for the Notes & Tasks workbar
//! composer (Wave 17 lane 11, migration 0080; docs/notes-tasks-workbar.md
//! §5.2).
//!
//! Purpose: one autosaved draft per (org, ticket, user), keyed entity in
//! the instant-first sense — idempotency key + timestamps, degraded hour
//! of retention (30 days). Drafts are transient work product: they never
//! render on any ledger/timeline, and they are deleted in the SAME
//! transaction as the comment submit whose idempotency key matches
//! (delete-on-submit), so reload either restores the draft or has already
//! seen it committed.
//!
//! Non-purpose: no versioning vector (autosave overwrite is last-writer-
//! wins per the debounced 800ms cadence; no concurrent-editor semantics).

use sqlx::Postgres;
use crate::DbResult;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Draft retention window (§5.2): older rows are stale (assuming never
/// finished) and may be purged.
pub const DRAFT_RETENTION: chrono::Duration = chrono::Duration::days(30);

pub struct NoteDraftRow {
    pub body: String,
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Upserts the draft (IF2 autosave: debounced overwrite, retry-safe).
pub async fn upsert_draft<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    ticket_id: Uuid,
    user_id: Uuid,
    body: &str,
    idempotency_key: &str,
) -> DbResult<NoteDraftRow>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as!(
        NoteDraftRow,
        r#"INSERT INTO note_drafts (id, org_id, ticket_id, user_id, body, idempotency_key)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (org_id, ticket_id, user_id) DO UPDATE
             SET body = $5, idempotency_key = $6, updated_at = now()
           RETURNING body, idempotency_key, created_at, updated_at"#,
        id,
        org_id,
        ticket_id,
        user_id,
        body,
        idempotency_key
    )
    .fetch_one(executor)
    .await?;
    Ok(row)
}

/// Reads the caller's draft for the ticket (None = no draft).
pub async fn get_draft<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
    user_id: Uuid,
) -> DbResult<Option<NoteDraftRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as!(
        NoteDraftRow,
        r#"SELECT body, idempotency_key, created_at, updated_at
           FROM note_drafts
           WHERE org_id = $1 AND ticket_id = $2 AND user_id = $3"#,
        org_id,
        ticket_id,
        user_id
    )
    .fetch_optional(executor)
    .await?;
    Ok(row)
}

/// Deletes the draft whose idempotency key CLAIMS the submit — called in
/// the SAME transaction as the successful comment insert (delete-on-
/// submit). Rows are scoped to (org, ticket, user) plus the matching key.
pub async fn delete_submitted<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
    user_id: Uuid,
    idempotency_key: &str,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"DELETE FROM note_drafts
           WHERE org_id = $1 AND ticket_id = $2 AND user_id = $3
             AND idempotency_key = $4"#,
        org_id,
        ticket_id,
        user_id,
        idempotency_key
    )
    .execute(executor)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Explicit discard (panel closed with cleared composer): removes the
/// caller's row outright.
pub async fn delete<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
    user_id: Uuid,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"DELETE FROM note_drafts
           WHERE org_id = $1 AND ticket_id = $2 AND user_id = $3"#,
        org_id,
        ticket_id,
        user_id
    )
    .execute(executor)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Best-effort retention purge of drafts older than 30 days. Returned
/// count is informational; failures on the autosave path are logged, not
/// fatal (housekeeping must never block a write).
pub async fn purge_stale<'e, E>(executor: E) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        "DELETE FROM note_drafts WHERE updated_at < $1",
        Utc::now() - DRAFT_RETENTION
    )
    .execute(executor)
    .await?;
    Ok(res.rows_affected())
}
