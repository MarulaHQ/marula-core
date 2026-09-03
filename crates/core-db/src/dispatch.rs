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
              distance_unit, vehicle, billable, notes, status, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'draft', $12)
           RETURNING id, org_id, user_id, ticket_id, started_at, ended_at, distance,
              distance_unit, vehicle, billable, notes, status,
              submitted_time_entry_id, created_by, created_at, updated_at"#,
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
              submitted_time_entry_id, created_by, created_at, updated_at
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
               submitted_time_entry_id, created_by, created_at, updated_at",
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
              submitted_time_entry_id, created_by, created_at, updated_at
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
              submitted_time_entry_id, created_by, created_at, updated_at"#,
        org_id,
        id,
        time_entry_id,
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row)
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
