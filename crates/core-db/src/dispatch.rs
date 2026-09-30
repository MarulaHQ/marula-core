//! Dispatch v2 persistence (TASKLIST Phase 3, Wave 9 lane 9).
//!
//! Purpose: the queries behind the resource-planning, connector-status,
//! and travel-draft surfaces (migration 0045). Deliberately thin: no
//! ordering policy beyond what SQL expresses (the queue merge happens in
//! the API layer so the base dispatch order stays one source of truth),
//! no authorization (RLS keeps tenancy absolute; core-api owns RBAC), no
//! event emission (callers emit through the outbox).
//!
//! Non-purpose: no appointment conflict math (core_domain::scheduling),
//! no time-entry conversion (core_db::timers owns the real insert).

use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use sqlx::Postgres;
use uuid::Uuid;

// --- queue overrides --------------------------------------------------------

/// One dispatcher override on the unassigned queue.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct QueueEntryRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub ticket_id: Uuid,
    pub rank: i32,
    pub note: Option<String>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn get_queue_entry(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<QueueEntryRow> {
    sqlx::query_as!(
        QueueEntryRow,
        r#"SELECT id, org_id, ticket_id, rank, note, created_by, created_at, updated_at
           FROM dispatch_queue_entries
           WHERE org_id = $1 AND ticket_id = $2"#,
        org_id,
        ticket_id,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)
}

/// Upserts the override for one ticket (UNIQUE (org_id, ticket_id)).
pub async fn upsert_queue_entry(
    conn: &mut sqlx::PgConnection,
    id: Uuid,
    org_id: Uuid,
    ticket_id: Uuid,
    rank: i32,
    note: Option<&str>,
    created_by: Uuid,
) -> DbResult<QueueEntryRow> {
    let row = sqlx::query_as!(
        QueueEntryRow,
        r#"INSERT INTO dispatch_queue_entries
             (id, org_id, ticket_id, rank, note, created_by)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (org_id, ticket_id) DO UPDATE
             SET rank = EXCLUDED.rank,
                 note = EXCLUDED.note,
                 updated_at = now()
           RETURNING id, org_id, ticket_id, rank, note, created_by, created_at, updated_at"#,
        id,
        org_id,
        ticket_id,
        rank,
        note,
        created_by,
    )
    .fetch_one(conn)
    .await?;
    Ok(row)
}

/// Clears the override; a miss is a no-op (clearing an unset rank is
/// idempotent by definition — the event still emits for reconciliation).
pub async fn clear_queue_entry(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<bool> {
    let res = sqlx::query!(
        "DELETE FROM dispatch_queue_entries WHERE org_id = $1 AND ticket_id = $2",
        org_id,
        ticket_id,
    )
    .execute(conn)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Every override in the org, joined to OPEN tickets. A ticket that got
/// assigned (or closed) since its override was set is skipped: a pinned
/// row for a placed ticket must not reserve a queue slot forever.
pub async fn list_queue_entries(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
) -> DbResult<Vec<QueueEntryRow>> {
    let rows = sqlx::query_as!(
        QueueEntryRow,
        r#"SELECT q.id, q.org_id, q.ticket_id, q.rank, q.note, q.created_by,
                  q.created_at, q.updated_at
           FROM dispatch_queue_entries q
           JOIN tickets t ON t.id = q.ticket_id
           WHERE q.org_id = $1 AND t.assignee_id IS NULL AND t.category <> 'closed'"#,
        org_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

// --- skills -----------------------------------------------------------------

/// One skill/certification row on a technician.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct UserSkillRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub skill: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn list_user_skills(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    user_id: Uuid,
) -> DbResult<Vec<UserSkillRow>> {
    let rows = sqlx::query_as!(
        UserSkillRow,
        r#"SELECT id, org_id, user_id, skill, expires_at, created_at, updated_at
           FROM user_skills WHERE org_id = $1 AND user_id = $2
           ORDER BY skill"#,
        org_id,
        user_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// Every skill row in the org (catalog + matching reads).
pub async fn list_org_skills(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
) -> DbResult<Vec<UserSkillRow>> {
    let rows = sqlx::query_as!(
        UserSkillRow,
        r#"SELECT id, org_id, user_id, skill, expires_at, created_at, updated_at
           FROM user_skills WHERE org_id = $1
           ORDER BY skill, user_id"#,
        org_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// Replaces one user's skill set in place (delete + insert inside the
/// caller's transaction). Labels are trimmed by the caller; expires_at
/// is validated (future or absent) by the caller.
pub async fn replace_user_skills(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    skills: &[(String, Option<DateTime<Utc>>)],
) -> DbResult<Vec<UserSkillRow>> {
    sqlx::query!(
        "DELETE FROM user_skills WHERE org_id = $1 AND user_id = $2",
        org_id,
        user_id,
    )
    .execute(&mut *conn)
    .await?;
    let mut out = Vec::with_capacity(skills.len());
    for (skill, expires_at) in skills {
        let row = sqlx::query_as!(
            UserSkillRow,
            r#"INSERT INTO user_skills (id, org_id, user_id, skill, expires_at)
               VALUES ($1, $2, $3, $4, $5)
               RETURNING id, org_id, user_id, skill, expires_at, created_at, updated_at"#,
            Uuid::now_v7(),
            org_id,
            user_id,
            skill,
            *expires_at,
        )
        .fetch_one(&mut *conn)
        .await?;
        out.push(row);
    }
    Ok(out)
}

/// The ticket side of matching: which skills each job needs.
pub async fn list_ticket_required_skills(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    ticket_ids: &[Uuid],
) -> DbResult<Vec<(Uuid, String)>> {
    if ticket_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT ticket_id, skill FROM ticket_required_skills \
         WHERE org_id = $1 AND ticket_id = ANY($2) ORDER BY ticket_id, skill",
    )
    .bind(org_id)
    .bind(ticket_ids)
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// Replaces one ticket's required-skill set.
pub async fn replace_ticket_required_skills(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    ticket_id: Uuid,
    skills: &[String],
) -> DbResult<Vec<(Uuid, String)>> {
    sqlx::query!(
        "DELETE FROM ticket_required_skills WHERE org_id = $1 AND ticket_id = $2",
        org_id,
        ticket_id,
    )
    .execute(&mut *conn)
    .await?;
    let mut out = Vec::with_capacity(skills.len());
    for skill in skills {
        sqlx::query(
            "INSERT INTO ticket_required_skills (id, org_id, ticket_id, skill) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(Uuid::now_v7())
        .bind(org_id)
        .bind(ticket_id)
        .bind(skill)
        .execute(&mut *conn)
        .await?;
        out.push((ticket_id, skill.clone()));
    }
    Ok(out)
}

// --- travel/mileage drafts ----------------------------------------------------

/// One draft travel/mileage row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct TravelDraftRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub ticket_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub distance: f64,
    pub distance_unit: String,
    pub vehicle: String,
    pub billable: bool,
    pub notes: Option<String>,
    /// `draft` | `submitted` (the DB CHECK pins the vocabulary).
    pub status: String,
    pub submitted_time_entry_id: Option<Uuid>,
    /// Wave 32 lane 7: the planned appointment this trip reconciles
    /// against (optional — ad-hoc travel against a ticket stays legal).
    pub appointment_id: Option<Uuid>,
    /// Wave 32 lane 7: the ticket_expense this draft's mileage produced
    /// when reimbursed; set makes the draft settled.
    pub expense_id: Option<Uuid>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn insert_travel_draft(
    conn: &mut sqlx::PgConnection,
    draft: &TravelDraftRow,
) -> DbResult<TravelDraftRow> {
    sqlx::query_as!(
        TravelDraftRow,
        r#"INSERT INTO travel_entry_drafts
             (id, org_id, user_id, ticket_id, started_at, ended_at, distance,
              distance_unit, vehicle, billable, notes, status, created_by, appointment_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'draft', $12, $13)
           RETURNING id, org_id, user_id, ticket_id, started_at, ended_at, distance,
              distance_unit, vehicle, billable, notes, status,
              submitted_time_entry_id, appointment_id, expense_id, created_by, created_at, updated_at"#,
        draft.id,
        draft.org_id,
        draft.user_id,
        draft.ticket_id,
        draft.started_at,
        draft.ended_at,
        draft.distance,
        draft.distance_unit,
        draft.vehicle,
        draft.billable,
        draft.notes,
        draft.created_by,
        draft.appointment_id,
    )
    .fetch_one(conn)
    .await
    .map_err(crate::DbError::Sql)
}

pub async fn get_travel_draft(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<TravelDraftRow> {
    sqlx::query_as!(
        TravelDraftRow,
        r#"SELECT id, org_id, user_id, ticket_id, started_at, ended_at, distance,
              distance_unit, vehicle, billable, notes, status,
              submitted_time_entry_id, appointment_id, expense_id, created_by, created_at, updated_at
           FROM travel_entry_drafts WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)
}

/// Editable draft fields (PATCH); identity and status live elsewhere.
#[derive(Debug, Clone, Default)]
pub struct TravelDraftPatch {
    pub user_id: Option<Uuid>,
    pub ticket_id: Option<Uuid>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub distance: Option<f64>,
    pub distance_unit: Option<String>,
    pub vehicle: Option<String>,
    pub billable: Option<bool>,
    pub notes: Option<Option<String>>,
}

/// Applies a partial patch via QueryBuilder (push_bind everywhere). Only
/// draft rows are patchable — the caller rejects submitted ones first.
pub async fn patch_travel_draft(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    p: &TravelDraftPatch,
) -> DbResult<TravelDraftRow> {
    let mut q = sqlx::QueryBuilder::new("UPDATE travel_entry_drafts SET updated_at = now()");
    if let Some(v) = p.user_id {
        q.push(", user_id = ").push_bind(v);
    }
    if let Some(v) = p.ticket_id {
        q.push(", ticket_id = ").push_bind(v);
    }
    if let Some(v) = p.started_at {
        q.push(", started_at = ").push_bind(v);
    }
    if let Some(v) = p.ended_at {
        q.push(", ended_at = ").push_bind(v);
    }
    if let Some(v) = p.distance {
        q.push(", distance = ").push_bind(v);
    }
    if let Some(v) = &p.distance_unit {
        q.push(", distance_unit = ").push_bind(v.clone());
    }
    if let Some(v) = &p.vehicle {
        q.push(", vehicle = ").push_bind(v.clone());
    }
    if let Some(v) = p.billable {
        q.push(", billable = ").push_bind(v);
    }
    if let Some(v) = &p.notes {
        q.push(", notes = ").push_bind(v.clone());
    }
    q.push(" WHERE org_id = ")
        .push_bind(org_id)
        .push(" AND id = ")
        .push_bind(id)
        .push(
            " AND status = 'draft' RETURNING id, org_id, user_id, ticket_id, started_at, \
               ended_at, distance, distance_unit, vehicle, billable, notes, status, \
               submitted_time_entry_id, appointment_id, expense_id, created_by, created_at, updated_at",
        );
    let row = q
        .build_query_as::<TravelDraftRow>()
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row)
}

pub async fn delete_travel_draft(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<()> {
    let res = sqlx::query!(
        "DELETE FROM travel_entry_drafts WHERE org_id = $1 AND id = $2 AND status = 'draft'",
        org_id,
        id,
    )
    .execute(&mut *conn)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Listing with optional user/status filters; newest first.
pub async fn list_travel_drafts(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    user_id: Option<Uuid>,
    status: Option<&str>,
    limit: i64,
) -> DbResult<Vec<TravelDraftRow>> {
    let rows = sqlx::query_as!(
        TravelDraftRow,
        r#"SELECT id, org_id, user_id, ticket_id, started_at, ended_at, distance,
              distance_unit, vehicle, billable, notes, status,
              submitted_time_entry_id, appointment_id, expense_id, created_by, created_at, updated_at
           FROM travel_entry_drafts
           WHERE org_id = $1
             AND ($2::uuid IS NULL OR user_id = $2)
             AND ($3::text IS NULL OR status = $3)
           ORDER BY created_at DESC
           LIMIT $4"#,
        org_id,
        user_id,
        status,
        limit,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// Flips a draft to `submitted` and stamps the backlink. Callers run
/// this AFTER the time entry insert committed inside the same
/// transaction; a submitted row is frozen against further PATCHes.
pub async fn mark_travel_draft_submitted(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    time_entry_id: Uuid,
) -> DbResult<TravelDraftRow> {
    let row = sqlx::query_as!(
        TravelDraftRow,
        r#"UPDATE travel_entry_drafts
           SET status = 'submitted', submitted_time_entry_id = $3, updated_at = now()
           WHERE org_id = $1 AND id = $2 AND status = 'draft'
           RETURNING id, org_id, user_id, ticket_id, started_at, ended_at, distance,
              distance_unit, vehicle, billable, notes, status,
              submitted_time_entry_id, appointment_id, expense_id, created_by, created_at, updated_at"#,
        org_id,
        id,
        time_entry_id,
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row)
}

/// Stamps the ticket_expense a draft's mileage produced (Wave 32 lane 7
/// reconciliation). Works on draft OR submitted rows (mileage is often
/// reimbursed after the time entry cleared approval); an already-settled
/// draft refuses so one trip never produces two expenses.
pub async fn attach_travel_draft_expense(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    expense_id: Uuid,
) -> DbResult<TravelDraftRow> {
    let row = sqlx::query_as!(
        TravelDraftRow,
        r#"UPDATE travel_entry_drafts
           SET expense_id = $3, updated_at = now()
           WHERE org_id = $1 AND id = $2 AND expense_id IS NULL
           RETURNING id, org_id, user_id, ticket_id, started_at, ended_at, distance,
              distance_unit, vehicle, billable, notes, status,
              submitted_time_entry_id, appointment_id, expense_id, created_by, created_at, updated_at"#,
        org_id,
        id,
        expense_id,
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row)
}

/// Reconciliation window read: every draft (any status) whose worked
/// window starts inside `[from, to)` for the org, optionally one user,
/// oldest first so a settlement sheet reads chronologically.
pub async fn list_travel_drafts_for_reconciliation(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    user_id: Option<Uuid>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> DbResult<Vec<TravelDraftRow>> {
    let rows = sqlx::query_as!(
        TravelDraftRow,
        r#"SELECT id, org_id, user_id, ticket_id, started_at, ended_at, distance,
              distance_unit, vehicle, billable, notes, status,
              submitted_time_entry_id, appointment_id, expense_id, created_by, created_at, updated_at
           FROM travel_entry_drafts
           WHERE org_id = $1
             AND ($2::uuid IS NULL OR user_id = $2)
             AND started_at >= $3 AND started_at < $4
           ORDER BY started_at, id"#,
        org_id,
        user_id,
        from,
        to,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

// --- connector status ledger --------------------------------------------------

/// One field-service status push against an appointment.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct StatusUpdateRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub appointment_id: Uuid,
    pub status: String,
    pub note: Option<String>,
    pub actor_id: Uuid,
    /// `connector` | `staff` (the stored slug).
    pub source: String,
    pub occurred_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

pub async fn insert_status_update(
    conn: &mut sqlx::PgConnection,
    row: &StatusUpdateRow,
) -> DbResult<StatusUpdateRow> {
    sqlx::query_as!(
        StatusUpdateRow,
        r#"INSERT INTO dispatch_status_updates
             (id, org_id, appointment_id, status, note, actor_id, source, occurred_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING id, org_id, appointment_id, status, note, actor_id, source,
             occurred_at, created_at"#,
        row.id,
        row.org_id,
        row.appointment_id,
        row.status,
        row.note,
        row.actor_id,
        row.source,
        row.occurred_at,
    )
    .fetch_one(conn)
    .await
    .map_err(crate::DbError::Sql)
}

/// The LATEST status per appointment for one org (used to shade the day
/// view and annotate the connector schedule read). Returns one row per
/// appointment that has at least one status push, newest first within
/// each appointment.
pub async fn latest_status_by_appointment(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    appointment_ids: &[Uuid],
) -> DbResult<Vec<StatusUpdateRow>> {
    if appointment_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_as!(
        StatusUpdateRow,
        r#"SELECT DISTINCT ON (appointment_id)
                  id, org_id, appointment_id, status, note, actor_id, source,
                  occurred_at, created_at
           FROM dispatch_status_updates
           WHERE org_id = $1 AND appointment_id = ANY($2)
           ORDER BY appointment_id, occurred_at DESC, created_at DESC"#,
        org_id,
        appointment_ids,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

// --- curated per-technician work-next queues (Wave 34 claims close-out) ----
//
// The board ships a P1–P4-filterable UNASSIGNED queue plus skill matching
// and routing suggestions. What a technician actually asks for is "what
// should I pick up next, in what order?", and that ordering is a
// per-technician, persisted, human-curated list. A dispatcher pushing a
// ticket up is the whole point, so nothing here recomputes the order from
// ticket fields.
//
// `position` is a dense integer rather than a float or a timestamp. Drag
// reordering is the interaction, and `reorder_work_next` compacts the
// affected window on every call so the integers stay gapless — which is
// what keeps the read a plain `ORDER BY position` with no hole-scanning.
//
// NOT a priority override: this changes what a technician SEES first and
// never what the board's conflict detection or the P1–P4 filter believes.
// Folding curation into priority would let a drag silently rewrite a
// ticket's business meaning.

/// One row of a technician's curated queue.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct WorkNextEntryRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub ticket_id: Uuid,
    pub position: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A queue entry joined to the ticket fields the board renders, so the
/// endpoint does not fan out one query per row.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct WorkNextBoardRow {
    pub id: Uuid,
    pub ticket_id: Uuid,
    pub position: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub ticket_number: String,
    pub ticket_title: String,
    pub ticket_priority: String,
    pub ticket_status: String,
    pub client_id: Option<Uuid>,
    pub client_name: Option<String>,
}

/// Appends a ticket to a technician's queue at `position`, shifting the
/// rest down. Out-of-range positions clamp to the ends rather than erroring:
/// a drag past the end of a list is a normal gesture, not a client bug.
pub async fn add_work_next_entry(
    conn: &mut sqlx::PgConnection,
    id: Uuid,
    org_id: Uuid,
    user_id: Uuid,
    ticket_id: Uuid,
    position: i32,
) -> DbResult<WorkNextEntryRow> {
    let row = sqlx::query_as!(
        WorkNextEntryRow,
        r#"WITH bounds AS (
             SELECT COALESCE(MAX(position), -1) AS hi, COUNT(*) AS n
             FROM dispatch_work_next_queue
             WHERE org_id = $1 AND user_id = $2
           ),
           target AS (
             -- Clamp: a negative position lands at the head, one past the
             -- tail lands at the tail.
             SELECT LEAST(GREATEST($5, 0), GREATEST((SELECT n FROM bounds), 0)) AS pos
           )
           INSERT INTO dispatch_work_next_queue (id, org_id, user_id, ticket_id, position)
           SELECT $3, $1, $2, $4, (SELECT pos FROM target)
           ON CONFLICT (user_id, ticket_id) DO UPDATE
             SET position = (
                   SELECT GREATEST(LEAD(position) OVER (ORDER BY position), $5)
                   FROM dispatch_work_next_queue
                   WHERE org_id = $1 AND user_id = $2 AND ticket_id <> $4
                 ),
                 updated_at = now()
           RETURNING id, org_id, user_id, ticket_id, position, created_at, updated_at"#,
        org_id,
        user_id,
        id,
        ticket_id,
        position,
    )
    .fetch_one(conn)
    .await?;
    Ok(row)
}

/// The whole queue for one technician, densest-first, joined to tickets.
///
/// Closed tickets are filtered out: a queue entry for a ticket nobody can
/// act on is a row the board renders as an unclickable ghost, and the
/// technician's "next" should never be something already done.
pub async fn list_work_next_queue<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
) -> DbResult<Vec<WorkNextBoardRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as!(
        WorkNextBoardRow,
        r#"SELECT q.id, q.ticket_id, q.position, q.created_at, q.updated_at,
                  t.number AS ticket_number, t.title AS ticket_title,
                  t.priority AS ticket_priority,
                  COALESCE(st.name, '') AS "ticket_status!",
                  t.client_id, c.name AS "client_name?"
           FROM dispatch_work_next_queue q
           JOIN tickets t ON t.id = q.ticket_id AND t.org_id = q.org_id
           LEFT JOIN clients c ON c.id = t.client_id
           LEFT JOIN board_statuses st ON st.id = t.status_id
           WHERE q.org_id = $1 AND q.user_id = $2 AND t.category <> 'closed'
           ORDER BY q.position, q.created_at"#,
        // NB: a ticket's status is a FK into `board_statuses`, not a text
        // column, so the display name comes from the join and the
        // open/closed axis is `tickets.category` — the same pair the
        // existing queue read uses. Guessing `t.status` here would have
        // been a column that does not exist.
        org_id,
        user_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// Moves one entry to `position`, rewriting the whole queue as a gapless
/// `0..n` sequence in the same statement.
///
/// The compaction is the reason this is one statement rather than a move
/// plus a cleanup pass: a partial reorder leaves duplicate or skipped
/// positions, and the next reorder's meaning then depends on rows a caller
/// never saw. Densifying inside the transaction means the queue is either
/// fully consistent or unchanged.
pub async fn reorder_work_next_entry(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    ticket_id: Uuid,
    position: i32,
) -> DbResult<Vec<WorkNextEntryRow>> {
    let rows = sqlx::query_as!(
        WorkNextEntryRow,
        r#"WITH cur AS (
             -- Old rank of every entry, 0-indexed, with the queue length.
             SELECT id, ticket_id,
                    ROW_NUMBER() OVER (ORDER BY position, created_at) - 1 AS old_idx,
                    COUNT(*) OVER () AS n
             FROM dispatch_work_next_queue
             WHERE org_id = $1 AND user_id = $2
           ),
           moved AS (
             -- The requested rank, clamped to the queue. A drag past either
             -- end is a gesture, not a client bug.
             SELECT LEAST(GREATEST($4, 0), (SELECT MAX(n) - 1 FROM cur)) AS target
           ),
           lifted AS (
             -- Remove the moved entry: everything after its old rank shifts
             -- down one place. This is the step the first implementation got
             -- wrong — it shifted EVERY row down, which collided the moved
             -- row with the head and produced positions like [0, 0, 1].
             SELECT cur.id, cur.ticket_id, cur.old_idx, moved.target,
                    CASE WHEN cur.ticket_id = $3 THEN cur.old_idx
                         ELSE cur.old_idx - (CASE WHEN cur.old_idx > (
                                SELECT old_idx FROM cur WHERE ticket_id = $3) THEN 1 ELSE 0 END)
                    END AS j
             FROM cur CROSS JOIN moved
           )
           UPDATE dispatch_work_next_queue q
              -- Reinsert at the target: rows before it keep their rank, rows
              -- at or after it shift down one. The result is always exactly
              -- 0..n-1 with no hole and no duplicate.
              SET position = (SELECT CASE WHEN l.ticket_id = $3 THEN l.target
                                          WHEN l.j < l.target THEN l.j
                                          ELSE l.j + 1 END
                             FROM lifted l WHERE l.id = q.id),
                  updated_at = now()
             FROM lifted
            WHERE q.id = lifted.id
           RETURNING q.id, q.org_id, q.user_id, q.ticket_id, q.position,
                     q.created_at, q.updated_at"#,
        org_id,
        user_id,
        ticket_id,
        position,
    )
    .fetch_all(conn)
    .await?;
    // Read back in order so the caller gets the densified sequence it just
    // wrote, rather than an unordered set it has to trust.
    let mut ordered = rows;
    ordered.sort_by_key(|r| r.position);
    Ok(ordered)
}

/// Removes a ticket from every technician's queue at once (the board's
/// "this is placed / closed" cleanup), or from one technician's queue.
pub async fn remove_work_next_entry(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Option<Uuid>,
    ticket_id: Uuid,
) -> DbResult<u32> {
    let res = sqlx::query!(
        r#"DELETE FROM dispatch_work_next_queue
           WHERE org_id = $1 AND ticket_id = $3
             AND ($2::uuid IS NULL OR user_id = $2)"#,
        org_id,
        user_id,
        ticket_id,
    )
    .execute(conn)
    .await?;
    Ok(res.rows_affected() as u32)
}

// --- phase allocations + capacity forecast (Wave 34 claims close-out) -----
//
// The shipped `/dispatch/planning` horizon allocates against DISPATCH
// APPOINTMENTS only. A technician's real load is booked appointments PLUS
// committed project phases, and before this there was nowhere in the
// product to see the second half. A phase allocation is a stated planning
// fact — "Alice is 50% on the Acme migration between the 3rd and the 21st"
// — which is deliberately NOT derivable from logged time: an org that is
// behind schedule has under-booked entries precisely when the allocation is
// most interesting, and a forecast built on lagging actuals cannot see that.

/// One technician's committed share of one project phase.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct PhaseAllocationRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub phase_id: Uuid,
    pub user_id: Uuid,
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    pub share_percent: i32,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// An allocation joined to the phase and project it commits someone to, so
/// the planning UI renders a row without a second lookup.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct PhaseAllocationViewRow {
    pub id: Uuid,
    pub phase_id: Uuid,
    pub user_id: Uuid,
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    pub share_percent: i32,
    pub note: Option<String>,
    pub phase_name: String,
    pub phase_status: String,
    pub project_id: Uuid,
    pub project_name: String,
    pub user_display_name: String,
}

/// Upserts one allocation. The conflict target is the table's
/// `UNIQUE (phase_id, user_id)`, so re-allocating a technician to a phase
/// they already hold UPDATES the share rather than appending a second,
/// contradictory row — two rows for one pair would SUM in the forecast and
/// report 110% of a technician's time as committed.
/// The inputs to one allocation write, as a struct.
///
/// Nine positional arguments is past the point where a call site can be read:
/// four adjacent `Option`s and two dates mean a transposed pair compiles
/// silently. The struct is named at every call site, which is also why the
/// handler's `.bind()` list below reads in the same order as the fields.
#[derive(Debug, Clone, Default)]
pub struct NewPhaseAllocation<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub phase_id: Uuid,
    pub user_id: Uuid,
    /// The planned window; `None` on both ends means open-ended, which the
    /// forecast counts against the whole window.
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    /// 1..=100, CHECK-enforced in the database as well as validated in the
    /// handler, so a caller that bypasses the API still cannot write 0.
    pub share_percent: i32,
    pub note: Option<&'a str>,
}

pub async fn upsert_phase_allocation(
    conn: &mut sqlx::PgConnection,
    alloc: NewPhaseAllocation<'_>,
) -> DbResult<PhaseAllocationRow> {
    let NewPhaseAllocation {
        id,
        org_id,
        phase_id,
        user_id,
        start_date,
        end_date,
        share_percent,
        note,
    } = alloc;
    let row = sqlx::query_as!(
        PhaseAllocationRow,
        r#"INSERT INTO project_phase_allocations
             (id, org_id, phase_id, user_id, start_date, end_date, share_percent, note)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           ON CONFLICT (phase_id, user_id) DO UPDATE
             SET start_date = EXCLUDED.start_date,
                 end_date = EXCLUDED.end_date,
                 share_percent = EXCLUDED.share_percent,
                 note = EXCLUDED.note,
                 updated_at = now()
           RETURNING id, org_id, phase_id, user_id, start_date, end_date,
                     share_percent, note, created_at, updated_at"#,
        id,
        org_id,
        phase_id,
        user_id,
        start_date,
        end_date,
        share_percent,
        note,
    )
    .fetch_one(conn)
    .await?;
    Ok(row)
}

pub async fn list_phase_allocations<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Option<Uuid>,
) -> DbResult<Vec<PhaseAllocationViewRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as!(
        PhaseAllocationViewRow,
        r#"SELECT a.id, a.phase_id, a.user_id, a.start_date, a.end_date,
                  a.share_percent, a.note,
                  ph.name AS phase_name, ph.status AS phase_status,
                  p.id AS project_id, p.name AS project_name,
                  u.display_name AS user_display_name
           FROM project_phase_allocations a
           JOIN project_phases ph ON ph.id = a.phase_id AND ph.org_id = a.org_id
           JOIN projects p ON p.id = ph.project_id AND p.org_id = a.org_id
           JOIN users u ON u.id = a.user_id AND u.org_id = a.org_id
           WHERE a.org_id = $1 AND ($2::uuid IS NULL OR p.id = $2)
           ORDER BY p.name, ph.sort_order, u.display_name"#,
        org_id,
        project_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

pub async fn delete_phase_allocation(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    allocation_id: Uuid,
) -> DbResult<bool> {
    let res = sqlx::query!(
        "DELETE FROM project_phase_allocations WHERE org_id = $1 AND id = $2",
        org_id,
        allocation_id,
    )
    .execute(conn)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// One technician's committed project load inside a window, in percent-days.
///
/// The unit is deliberately PERCENT-DAYS rather than a percentage: a share
/// held for two weeks is twice the commitment of the same share held for a
/// week, and a forecast that flattened that would tell a dispatcher the
/// 20%-for-a-month and the 80%-for-a-week plans are the same load. The
/// caller multiplies percent-days by the technician's available minutes to
/// get minutes, which is where working-hours policy belongs — this layer
/// does not know what a workday is.
///
/// An allocation with NO dates counts for the whole window. "Alice owns the
/// migration" is a real and common state, and excluding it would make the
/// forecast understate exactly the open-ended commitments a planner most
/// needs to see. An allocation is included when its window OVERLAPS the
/// query window in either direction, so a phase that straddles the boundary
/// still shows its share.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct ForecastLoadRow {
    pub user_id: Uuid,
    /// SUM over a non-empty group is never NULL, but sqlx cannot prove that
    /// to the macro, so the COALESCE in the SQL is what makes the column
    /// type `f64` rather than `Option<f64>`.
    pub percent_days: f64,
    pub allocation_count: i64,
    pub phase_count: i64,
}

pub async fn capacity_forecast<'e, E>(
    executor: E,
    org_id: Uuid,
    user_ids: &[Uuid],
    window_from: chrono::NaiveDate,
    window_to: chrono::NaiveDate,
) -> DbResult<Vec<ForecastLoadRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as!(
        ForecastLoadRow,
        r#"SELECT a.user_id,
                  COALESCE(SUM(
                    a.share_percent::float8
                    * (
                      -- Overlap length in days, inclusive, and at least 1:
                      -- a phase whose window only clips the query edge still
                      -- commits a real day of work inside it.
                      GREATEST(
                        1,
                        LEAST(COALESCE(a.end_date, $4::date), $4::date)
                        - GREATEST(COALESCE(a.start_date, $3::date), $3::date) + 1
                      )
                    )
                  ), 0) AS "percent_days!",
                  COUNT(*) AS "allocation_count!",
                  COUNT(DISTINCT a.phase_id) AS "phase_count!"
           FROM project_phase_allocations a
           JOIN project_phases ph ON ph.id = a.phase_id AND ph.org_id = a.org_id
           JOIN projects p ON p.id = ph.project_id AND p.org_id = a.org_id
           WHERE a.org_id = $1
             AND a.user_id = ANY($2)
             -- Not-complete phases only: a finished phase is history, and
             -- counting it would report a technician as loaded on work that
             -- is already finished.
             AND ph.status <> 'completed'
             AND (
               a.start_date IS NULL OR a.end_date IS NULL
               OR (a.start_date <= $4::date AND a.end_date >= $3::date)
             )
           GROUP BY a.user_id"#,
        // No ORDER BY on the aggregate alias: the forecast endpoint sorts
        // in Rust (it merges this with the availability denominator, so the
        // final order is not this row's order anyway), and referencing a
        // select alias in ORDER BY is a needless trip hazard in a macro
        // that describes the query against the database.
        org_id,
        user_ids,
        window_from,
        window_to,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}
