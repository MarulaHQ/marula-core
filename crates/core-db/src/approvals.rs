//! Time-entry approval workflows (Wave 14 lane 1): submit → decide →
//! approve/reject over the `approval_*` columns on `time_entries`, with an
//! append-only decision trail in `time_entry_approvals` (per-entry
//! monotonic `seq`).
//!
//! Locked periods after invoicing: an entry referenced by
//! `invoice_time_entries` is frozen. Every mutation here carries the
//! `NOT EXISTS` guard against that link table, so the lock is enforced at
//! the row level even under a racing bulk request.

use crate::{DbError, DbResult};
use core_domain::entities::{
    ApprovalStatus, TimeApprovalAction, TimeEntry, TimeEntryApprovalRecord, TimeEntryQueueItem,
};
use sqlx::Postgres;
use uuid::Uuid;

/// Appends one audit row with the next per-entry `seq` and returns it.
#[allow(clippy::too_many_arguments)]
async fn append_audit<'e, E>(
    executor: E,
    org_id: Uuid,
    entry_id: Uuid,
    action: TimeApprovalAction,
    actor_id: Uuid,
    reason_code: Option<&str>,
    note: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> DbResult<TimeEntryApprovalRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO time_entry_approvals (id, org_id, time_entry_id, seq, action, actor_id, reason_code, note, created_at)
           SELECT $1, $2, $3, COALESCE(MAX(seq), 0) + 1, $4, $5, $6, $7, $8
           FROM time_entry_approvals WHERE time_entry_id = $3
           RETURNING id, org_id, time_entry_id, seq, action, actor_id, reason_code, note, created_at"#,
        uuid::Uuid::now_v7(),
        org_id,
        entry_id,
        action.as_str(),
        actor_id,
        reason_code,
        note,
        now,
    )
    .fetch_one(executor)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.constraint().is_some() => {
            tracing::warn!(constraint = db.constraint().unwrap_or(""), "approval seq collision");
            DbError::Conflict("approval decision sequence collision; retry".into())
        }
        _ => DbError::Sql(e),
    })?;
    Ok(TimeEntryApprovalRecord {
        id: row.id,
        org_id: row.org_id,
        time_entry_id: row.time_entry_id,
        seq: row.seq,
        action: TimeApprovalAction::parse(&row.action).unwrap_or(TimeApprovalAction::Submitted),
        actor_id: row.actor_id,
        reason_code: row.reason_code,
        note: row.note,
        created_at: row.created_at,
    })
}

/// Submits one entry into the approver queue. Refuses (NotFound) entries
/// that are not in a submittable state or are invoice-locked — the caller
/// classifies the refusal for a typed reason code.
pub async fn submit(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    entry_id: Uuid,
    actor_id: Uuid,
    note: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> DbResult<TimeEntry> {
    let row = sqlx::query!(
        r#"UPDATE time_entries
              SET approval_status = 'submitted',
                  approval_submitted_at = $3, approval_submitted_by = $4,
                  approval_reason_code = NULL
            WHERE org_id = $1 AND id = $2
              AND approval_status IN ('draft', 'rejected')
              AND NOT EXISTS (
                  SELECT 1 FROM invoice_time_entries l WHERE l.time_entry_id = time_entries.id)
           RETURNING id, org_id, ticket_id, user_id, started_at, ended_at, minutes, billable,
                     notes, work_type, timer_session_id, approval_status, approval_reason_code,
                     created_at"#,
        org_id,
        entry_id,
        now,
        actor_id,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DbError::NotFound)?;

    let entry = entry_from_parts(
        row.id,
        row.org_id,
        row.ticket_id,
        row.user_id,
        row.started_at,
        row.ended_at,
        row.minutes,
        row.billable,
        row.notes,
        row.work_type,
        row.timer_session_id,
        row.approval_status,
        row.approval_reason_code,
        row.created_at,
    );
    append_audit(
        &mut *tx,
        org_id,
        entry_id,
        TimeApprovalAction::Submitted,
        actor_id,
        None,
        note,
        now,
    )
    .await?;
    Ok(entry)
}

/// Bulk approval decision. Only entries currently `submitted` and not
/// invoice-locked are transitioned; the return value names exactly which
/// ids applied so the API layer can report per-item outcomes. Every
/// applied entry gets its own audit row.
#[allow(clippy::too_many_arguments)]
pub async fn decide(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    entry_ids: &[Uuid],
    approve: bool,
    actor_id: Uuid,
    reason_code: Option<&str>,
    note: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> DbResult<Vec<TimeEntry>> {
    if entry_ids.is_empty() {
        return Ok(vec![]);
    }
    let target = if approve { "approved" } else { "rejected" };
    let rows = sqlx::query!(
        r#"UPDATE time_entries
              SET approval_status = $3,
                  approval_decided_at = $4, approval_decided_by = $5,
                  approval_reason_code = $6
            WHERE org_id = $1 AND id = ANY($2) AND approval_status = 'submitted'
              AND NOT EXISTS (
                  SELECT 1 FROM invoice_time_entries l WHERE l.time_entry_id = time_entries.id)
           RETURNING id, org_id, ticket_id, user_id, started_at, ended_at, minutes, billable,
                     notes, work_type, timer_session_id, approval_status, approval_reason_code,
                     created_at"#,
        org_id,
        entry_ids,
        target,
        now,
        actor_id,
        reason_code,
    )
    .fetch_all(&mut *tx)
    .await?;
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let entry = entry_from_parts(
            row.id,
            row.org_id,
            row.ticket_id,
            row.user_id,
            row.started_at,
            row.ended_at,
            row.minutes,
            row.billable,
            row.notes,
            row.work_type,
            row.timer_session_id,
            row.approval_status,
            row.approval_reason_code,
            row.created_at,
        );
        append_audit(
            &mut *tx,
            org_id,
            entry.id,
            if approve {
                TimeApprovalAction::Approved
            } else {
                TimeApprovalAction::Rejected
            },
            actor_id,
            reason_code,
            note,
            now,
        )
        .await?;
        entries.push(entry);
    }
    Ok(entries)
}

/// Per-entry classification for bulk outcomes: current approval status and
/// whether the entry is invoice-locked. Absent from the result = not found
/// in scope (RLS-sparse).
pub async fn classify<'e, E>(
    executor: E,
    org_id: Uuid,
    entry_ids: &[Uuid],
) -> DbResult<Vec<(Uuid, Option<ApprovalStatus>, bool)>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    if entry_ids.is_empty() {
        return Ok(vec![]);
    }
    let rows = sqlx::query!(
        r#"SELECT te.id, te.approval_status,
                  EXISTS (SELECT 1 FROM invoice_time_entries l WHERE l.time_entry_id = te.id) AS "locked!"
           FROM time_entries te WHERE te.org_id = $1 AND te.id = ANY($2)"#,
        org_id,
        entry_ids,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.id,
                ApprovalStatus::parse(&row.approval_status),
                row.locked,
            )
        })
        .collect())
}

/// Reopens a rejected entry back to draft (explicit action or a content
/// edit). Only rejected entries reopen; locked ones refuse.
pub async fn reopen(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    entry_id: Uuid,
    actor_id: Uuid,
    now: chrono::DateTime<chrono::Utc>,
) -> DbResult<TimeEntry> {
    let row = sqlx::query!(
        r#"UPDATE time_entries
              SET approval_status = 'draft', approval_reason_code = NULL,
                  approval_submitted_at = NULL, approval_submitted_by = NULL,
                  approval_decided_at = NULL, approval_decided_by = NULL
            WHERE org_id = $1 AND id = $2 AND approval_status = 'rejected'
              AND NOT EXISTS (
                  SELECT 1 FROM invoice_time_entries l WHERE l.time_entry_id = time_entries.id)
           RETURNING id, org_id, ticket_id, user_id, started_at, ended_at, minutes, billable,
                     notes, work_type, timer_session_id, approval_status, approval_reason_code,
                     created_at"#,
        org_id,
        entry_id,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DbError::NotFound)?;
    let entry = entry_from_parts(
        row.id,
        row.org_id,
        row.ticket_id,
        row.user_id,
        row.started_at,
        row.ended_at,
        row.minutes,
        row.billable,
        row.notes,
        row.work_type,
        row.timer_session_id,
        row.approval_status,
        row.approval_reason_code,
        row.created_at,
    );
    append_audit(
        &mut *tx,
        org_id,
        entry_id,
        TimeApprovalAction::Reopened,
        actor_id,
        None,
        None,
        now,
    )
    .await?;
    Ok(entry)
}

/// The approver queue: entries in the given states (default: submitted),
/// oldest submission first. `user_id` narrows to one submitter;
/// `decided_by` narrows to entries that approver decided (the per-approver
/// decision history view).
#[allow(clippy::too_many_arguments)]
pub async fn queue<'e, E>(
    executor: E,
    org_id: Uuid,
    status: Option<ApprovalStatus>,
    user_id: Option<Uuid>,
    decided_by: Option<Uuid>,
    limit: i64,
) -> DbResult<Vec<TimeEntryQueueItem>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = "SELECT te.id, te.org_id, te.ticket_id, te.user_id, te.started_at, te.ended_at, \
         te.minutes, te.billable, te.notes, te.work_type, te.timer_session_id, \
         te.approval_status, te.approval_reason_code, te.created_at, \
         te.approval_submitted_by, te.approval_submitted_at, \
         te.approval_decided_by, te.approval_decided_at, \
         EXISTS (SELECT 1 FROM invoice_time_entries l WHERE l.time_entry_id = te.id) AS locked \
       FROM time_entries te \
       WHERE te.org_id = $1 \
         AND ($2::text IS NULL OR te.approval_status = $2) \
         AND ($3::uuid IS NULL OR te.user_id = $3) \
         AND ($4::uuid IS NULL OR te.approval_decided_by = $4) \
       ORDER BY te.approval_submitted_at ASC NULLS LAST, te.started_at DESC \
       LIMIT $5";
    let rows = sqlx::query(sql)
        .bind(org_id)
        .bind(status.map(|s| s.as_str().to_string()))
        .bind(user_id)
        .bind(decided_by)
        .bind(limit)
        .fetch_all(executor)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row: sqlx::postgres::PgRow| {
            use sqlx::Row;
            let submitted_by: Option<Uuid> = row.get("approval_submitted_by");
            let submitted_at: Option<chrono::DateTime<chrono::Utc>> =
                row.get("approval_submitted_at");
            let decided_by: Option<Uuid> = row.get("approval_decided_by");
            let decided_at: Option<chrono::DateTime<chrono::Utc>> = row.get("approval_decided_at");
            let locked: bool = row.get("locked");
            let entry = map_row(row);
            TimeEntryQueueItem {
                entry,
                submitted_by,
                submitted_at,
                decided_by,
                decided_at,
                locked,
            }
        })
        .collect())
}

/// The append-only decision trail for one entry, `seq` ascending.
pub async fn history<'e, E>(
    executor: E,
    org_id: Uuid,
    entry_id: Uuid,
) -> DbResult<Vec<TimeEntryApprovalRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, time_entry_id, seq, action, actor_id, reason_code, note, created_at
           FROM time_entry_approvals
           WHERE org_id = $1 AND time_entry_id = $2
           ORDER BY seq"#,
        org_id,
        entry_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| TimeEntryApprovalRecord {
            id: row.id,
            org_id: row.org_id,
            time_entry_id: row.time_entry_id,
            seq: row.seq,
            action: TimeApprovalAction::parse(&row.action).unwrap_or(TimeApprovalAction::Submitted),
            actor_id: row.actor_id,
            reason_code: row.reason_code,
            note: row.note,
            created_at: row.created_at,
        })
        .collect())
}

#[allow(clippy::too_many_arguments)]
fn entry_from_parts(
    id: Uuid,
    org_id: Uuid,
    ticket_id: Uuid,
    user_id: Uuid,
    started_at: chrono::DateTime<chrono::Utc>,
    ended_at: chrono::DateTime<chrono::Utc>,
    minutes: i32,
    billable: bool,
    notes: Option<String>,
    work_type: Option<String>,
    timer_session_id: Option<Uuid>,
    approval_status: String,
    approval_reason_code: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
) -> TimeEntry {
    TimeEntry {
        id,
        org_id,
        ticket_id,
        user_id,
        started_at,
        ended_at,
        minutes: minutes as i64,
        billable,
        notes,
        work_type,
        timer_session_id,
        approval_status: ApprovalStatus::parse(&approval_status),
        approval_reason_code,
        created_at,
    }
}

fn map_row(row: sqlx::postgres::PgRow) -> TimeEntry {
    use sqlx::Row;
    let status: Option<String> = row
        .try_get::<Option<String>, _>("approval_status")
        .unwrap_or(None);
    TimeEntry {
        id: row.get("id"),
        org_id: row.get("org_id"),
        ticket_id: row.get("ticket_id"),
        user_id: row.get("user_id"),
        started_at: row.get("started_at"),
        ended_at: row.get("ended_at"),
        minutes: row.get::<i32, _>("minutes") as i64,
        billable: row.get("billable"),
        notes: row.get("notes"),
        work_type: row.get("work_type"),
        timer_session_id: row.get("timer_session_id"),
        approval_status: status.as_deref().and_then(ApprovalStatus::parse),
        approval_reason_code: row.get("approval_reason_code"),
        created_at: row.get("created_at"),
    }
}
