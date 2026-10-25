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
           RETURNING id, org_id, ticket_id, phase_id, user_id, started_at, ended_at, minutes, billable,
                     notes, work_type, unbillable_reason, timer_session_id, approval_status,
                     approval_reason_code, created_at"#,
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
        row.phase_id,
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
        row.unbillable_reason,
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
           RETURNING id, org_id, ticket_id, phase_id, user_id, started_at, ended_at, minutes, billable,
                     notes, work_type, unbillable_reason, timer_session_id, approval_status,
                     approval_reason_code, created_at"#,
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
            row.phase_id,
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
            row.unbillable_reason,
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

/// Which of these entries were written by `user_id`? The decision gate
/// refuses self-approval for every role but owner/admin (write-gate symmetry
/// with `submit`); this names the refused ids in one read so the API can
/// answer `own_entry` instead of a bare not-in-queue.
pub async fn owned_entry_ids<'e, E>(
    executor: E,
    org_id: Uuid,
    entry_ids: &[Uuid],
    user_id: Uuid,
) -> DbResult<Vec<Uuid>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    if entry_ids.is_empty() {
        return Ok(vec![]);
    }
    let rows = sqlx::query(
        "SELECT te.id FROM time_entries te WHERE te.org_id = $1 AND te.id = ANY($2) AND te.user_id = $3",
    )
    .bind(org_id)
    .bind(entry_ids)
    .bind(user_id)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            use sqlx::Row;
            let id: Uuid = row.get("id");
            id
        })
        .collect())
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
           RETURNING id, org_id, ticket_id, phase_id, user_id, started_at, ended_at, minutes, billable,
                     notes, work_type, unbillable_reason, timer_session_id, approval_status,
                     approval_reason_code, created_at"#,
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
        row.phase_id,
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
        row.unbillable_reason,
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

/// What an approver's one-click fix changes. Grouped rather than passed as
/// loose arguments because "which of these two is it" is the caller's only real
/// decision, and a signature that takes both invites passing both by accident.
#[derive(Debug, Clone, Default)]
pub struct EntryFix<'a> {
    /// The description to write, when the verdict offered one.
    pub description: Option<&'a str>,
    /// Whole minutes to take OFF THE END of the window, when the verdict
    /// offered a trim. The duration follows, and the window is what an overlap
    /// is a fact about. Only ever a shortening, never a lengthening.
    pub trim_to_minutes: Option<i32>,
    /// What the trail records, in words.
    pub note: Option<&'a str>,
}

/// Applies an approver's one-click verdict fix to a queued entry: a better
/// description, a trim of the double-claimed minutes, or both.
///
/// The entry stays `submitted` — clearing the objection is not approving it,
/// and the human still presses Approve. Two guards live in the statement
/// itself so a racing request cannot slip past them: the entry must still be
/// in the queue, and it must not be locked by invoicing.
///
/// The caller has already validated the *direction* of a trim (an approver
/// may lower an entry's minutes, never raise them — raising is a billing
/// decision, not a tidy-up); this function refuses anything it is not handed.
pub async fn fix(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    entry_id: Uuid,
    actor_id: Uuid,
    change: &EntryFix<'_>,
    now: chrono::DateTime<chrono::Utc>,
) -> DbResult<TimeEntry> {
    // A trim moves the END of the window, and the duration follows it. Lowering
    // `minutes` on its own would be a lie: an overlap is a fact about when the
    // work happened, so shortening the minutes without moving `ended_at` would
    // leave the same overlap in place and the mark would never clear.
    let row = sqlx::query!(
        r#"UPDATE time_entries
              SET notes = COALESCE($3, notes),
                  ended_at = CASE WHEN $4::integer IS NULL THEN ended_at
                                  ELSE ended_at - make_interval(mins => $4::integer) END,
                  minutes = CASE WHEN $4::integer IS NULL THEN minutes
                                 ELSE GREATEST(1, minutes - $4::integer) END
            WHERE org_id = $1 AND id = $2 AND approval_status = 'submitted'
              AND ($4::integer IS NULL OR $4 > 0)
              AND ($4::integer IS NULL OR $4 < minutes)
              AND NOT EXISTS (
                  SELECT 1 FROM invoice_time_entries l WHERE l.time_entry_id = time_entries.id)
           RETURNING id, org_id, ticket_id, phase_id, user_id, started_at, ended_at, minutes, billable,
                     notes, work_type, unbillable_reason, timer_session_id, approval_status,
                     approval_reason_code, created_at"#,
        org_id,
        entry_id,
        change.description,
        change.trim_to_minutes,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DbError::NotFound)?;
    let entry = entry_from_parts(
        row.id,
        row.org_id,
        row.ticket_id,
        row.phase_id,
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
        row.unbillable_reason,
        row.created_at,
    );
    append_audit(
        &mut *tx,
        org_id,
        entry_id,
        TimeApprovalAction::Fixed,
        actor_id,
        None,
        change.note,
        now,
    )
    .await?;
    Ok(entry)
}

/// Everything the billing verdicts need about a set of entries, read in one
/// pass so a hundred queue rows do not cost a hundred queries.
///
/// For each requested entry: itself, the same person's other entries in the
/// same week (the `usual` comparison and the overlap check), and the
/// ticket's human number and title (the factual suggested description).
#[derive(Debug, Clone)]
pub struct VerdictEntry {
    pub entry: TimeEntry,
    /// The entry's ticket's human number, when ticket-scoped.
    pub ticket_number: Option<String>,
    /// The entry's ticket's title, when ticket-scoped.
    pub ticket_title: Option<String>,
    /// The same person's other entries' minutes within ±7 days, ascending.
    pub peer_minutes: Vec<i64>,
    /// Entries of the same person that overlap this one, with their ticket
    /// number and the overlap in whole minutes.
    pub overlaps: Vec<(Uuid, Option<String>, i64)>,
}

/// The overlap scan: for every entry in `ids`, the other entries of the same
/// person that share any of its window, in one query.
pub async fn verdict_inputs(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    ids: &[Uuid],
) -> DbResult<Vec<VerdictEntry>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let rows = sqlx::query!(
        r#"SELECT e.id, e.org_id, e.ticket_id, e.phase_id, e.user_id, e.started_at, e.ended_at,
                  e.minutes, e.billable, e.notes, e.work_type, e.unbillable_reason,
                  e.timer_session_id, e.approval_status, e.approval_reason_code, e.created_at
             FROM time_entries e
            WHERE e.org_id = $1 AND e.id = ANY($2)"#,
        org_id,
        ids,
    )
    .fetch_all(&mut *tx)
    .await?;

    let mut out: Vec<VerdictEntry> = rows
        .into_iter()
        .map(|row| VerdictEntry {
            entry: entry_from_parts(
                row.id,
                row.org_id,
                row.ticket_id,
                row.phase_id,
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
                row.unbillable_reason,
                row.created_at,
            ),
            ticket_number: None,
            ticket_title: None,
            peer_minutes: vec![],
            overlaps: vec![],
        })
        .collect();

    // The tickets in play, in their own read. Joined inline this would be one
    // query, but a left-joined `t.number` is inferred from the column's own
    // NOT NULL and would hand the macro a non-nullable String for a
    // project-scoped entry that has no ticket at all — so the lookup is
    // explicit here and the absence is represented honestly.
    let ticket_ids: Vec<Uuid> = {
        let mut tids: Vec<Uuid> = out.iter().filter_map(|v| v.entry.ticket_id).collect();
        tids.sort_unstable();
        tids.dedup();
        tids
    };
    let mut tickets: std::collections::HashMap<Uuid, (String, String)> =
        std::collections::HashMap::new();
    if !ticket_ids.is_empty() {
        let trows = sqlx::query!(
            r#"SELECT id, number, title FROM tickets WHERE org_id = $1 AND id = ANY($2)"#,
            org_id,
            &ticket_ids,
        )
        .fetch_all(&mut *tx)
        .await?;
        for row in trows {
            tickets.insert(row.id, (row.number, row.title));
        }
    }
    for target in &mut out {
        if let Some((number, title)) = target
            .entry
            .ticket_id
            .and_then(|tid| tickets.get(&tid).cloned())
        {
            target.ticket_number = Some(number);
            target.ticket_title = Some(title);
        }
    }

    // One window query for the whole set: every entry of the same people,
    // inside the bounding box that covers all the target windows (±7 days).
    // A bounding box over-fetches and the per-entry comparison below narrows
    // it, which is honest and one round trip — an OR over per-entry windows
    // would need a row per pair.
    let lo = out
        .iter()
        .map(|v| v.entry.started_at)
        .min()
        .map(|t| t - chrono::Duration::days(7))
        .unwrap_or_else(chrono::Utc::now);
    let hi = out
        .iter()
        .map(|v| v.entry.ended_at)
        .max()
        .map(|t| t + chrono::Duration::days(7))
        .unwrap_or_else(chrono::Utc::now);
    let user_ids: Vec<Uuid> = {
        let mut ids: Vec<Uuid> = out.iter().map(|v| v.entry.user_id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    if user_ids.is_empty() {
        return Ok(out);
    }
    let peers = sqlx::query!(
        r#"SELECT e.id, e.ticket_id, e.user_id, e.started_at, e.ended_at, e.minutes
             FROM time_entries e
            WHERE e.org_id = $1
              AND e.user_id = ANY($2)
              AND e.started_at < $3
              AND e.ended_at > $4"#,
        org_id,
        &user_ids,
        hi,
        lo,
    )
    .fetch_all(&mut *tx)
    .await?;

    for target in &mut out {
        for peer in &peers {
            if peer.id == target.entry.id || peer.user_id != target.entry.user_id {
                continue;
            }
            let overlap = core_domain::entities::overlap_minutes(
                target.entry.started_at,
                target.entry.ended_at,
                peer.started_at,
                peer.ended_at,
            );
            if overlap > 0 {
                let number = peer
                    .ticket_id
                    .and_then(|tid| tickets.get(&tid).cloned())
                    .map(|(number, _)| number);
                target.overlaps.push((peer.id, number, overlap));
            } else if (peer.started_at - target.entry.started_at).num_days().abs() <= 7 {
                target.peer_minutes.push(peer.minutes as i64);
            }
        }
        target.peer_minutes.sort_unstable();
    }
    Ok(out)
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
    let sql = "SELECT te.id, te.org_id, te.ticket_id, te.phase_id, te.user_id, te.started_at, te.ended_at, \
         te.minutes, te.billable, te.notes, te.work_type, te.unbillable_reason, \
         te.timer_session_id, te.approval_status, te.approval_reason_code, te.created_at, \
         te.approval_submitted_by, te.approval_submitted_at, \
         te.approval_decided_by, te.approval_decided_at, \
         EXISTS (SELECT 1 FROM invoice_time_entries l WHERE l.time_entry_id = te.id) AS locked, \
         t.number AS ticket_number, t.title AS ticket_title, c.name AS client_name, \
         u.timezone AS user_timezone \
       FROM time_entries te \
       LEFT JOIN tickets t ON t.id = te.ticket_id \
       LEFT JOIN clients c ON c.id = t.client_id \
       LEFT JOIN users u ON u.id = te.user_id \
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
            let ticket_number: Option<String> = row.get("ticket_number");
            let ticket_title: Option<String> = row.get("ticket_title");
            let client_name: Option<String> = row.get("client_name");
            let user_timezone: Option<String> = row.get("user_timezone");
            let ticket_id: Option<Uuid> = row.get("ticket_id");
            let entry = map_row(row);
            let ticket = match (ticket_id, ticket_number, ticket_title) {
                (Some(id), Some(number), Some(title)) => {
                    Some(core_domain::entities::TimeEntryTicketRef {
                        id,
                        number,
                        title,
                        client_name,
                    })
                }
                _ => None,
            };
            TimeEntryQueueItem {
                entry,
                ticket,
                submitted_by,
                submitted_at,
                decided_by,
                decided_at,
                locked,
                user_timezone,
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
    ticket_id: Option<Uuid>,
    phase_id: Option<Uuid>,
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
    unbillable_reason: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
) -> TimeEntry {
    TimeEntry {
        id,
        org_id,
        ticket_id,
        phase_id,
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
        unbillable_reason,
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
        phase_id: row.get("phase_id"),
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
        unbillable_reason: row
            .try_get::<Option<String>, _>("unbillable_reason")
            .unwrap_or(None),
        created_at: row.get("created_at"),
    }
}
