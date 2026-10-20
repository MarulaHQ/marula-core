//! Timer sessions (segments as JSONB, single-active invariant enforced by a
//! partial unique index) and time entries.

use crate::{DbError, DbResult};
use core_domain::entities::{TimeEntry, TimerSegment, TimerSession};
use sqlx::Postgres;
use uuid::Uuid;

fn parse_segments(v: serde_json::Value) -> DbResult<Vec<TimerSegment>> {
    serde_json::from_value(v).map_err(|e| {
        DbError::Sql(sqlx::Error::ColumnDecode {
            index: "segments".into(),
            source: Box::new(e),
        })
    })
}

pub async fn insert_session<'e, E>(executor: E, s: &TimerSession) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let segments = serde_json::to_value(&s.segments)
        .map_err(|e| DbError::Sql(sqlx::Error::Configuration(e.to_string().into())))?;
    sqlx::query!(
        r#"INSERT INTO timer_sessions (id, org_id, user_id, ticket_id, started_at, stopped_at, segments)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        s.id,
        s.org_id,
        s.user_id,
        s.ticket_id,
        s.started_at,
        s.stopped_at,
        segments
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn update_session<'e, E>(executor: E, s: &TimerSession) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let segments = serde_json::to_value(&s.segments)
        .map_err(|e| DbError::Sql(sqlx::Error::Configuration(e.to_string().into())))?;
    let res = sqlx::query!(
        r#"UPDATE timer_sessions SET stopped_at = $2, segments = $3 WHERE id = $1"#,
        s.id,
        s.stopped_at,
        segments
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// The user's RUNNING session, if any (at most one — DB-enforced via the
/// `is_running` generated column: paused sessions stay in the docked
/// working set but are not active).
pub async fn active_session<'e, E>(executor: E, user_id: Uuid) -> DbResult<Option<TimerSession>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, user_id, ticket_id, started_at, stopped_at, segments
           FROM timer_sessions WHERE user_id = $1 AND is_running"#,
        user_id
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|row| TimerSession {
        id: row.id,
        org_id: row.org_id,
        user_id: row.user_id,
        ticket_id: row.ticket_id,
        started_at: row.started_at,
        stopped_at: row.stopped_at,
        segments: parse_segments(row.segments).unwrap_or_default(),
    }))
}

pub async fn get_session<'e, E>(executor: E, id: Uuid) -> DbResult<TimerSession>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, user_id, ticket_id, started_at, stopped_at, segments
           FROM timer_sessions WHERE id = $1"#,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(TimerSession {
        id: row.id,
        org_id: row.org_id,
        user_id: row.user_id,
        ticket_id: row.ticket_id,
        started_at: row.started_at,
        stopped_at: row.stopped_at,
        segments: parse_segments(row.segments).unwrap_or_default(),
    })
}

/// The user's docked working set: recently touched sessions (running first).
pub async fn recent_sessions<'e, E>(
    executor: E,
    user_id: Uuid,
    limit: i64,
) -> DbResult<Vec<TimerSession>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, user_id, ticket_id, started_at, stopped_at, segments
           FROM timer_sessions WHERE user_id = $1
           ORDER BY is_running DESC, started_at DESC LIMIT $2"#,
        user_id,
        limit
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| TimerSession {
            id: row.id,
            org_id: row.org_id,
            user_id: row.user_id,
            ticket_id: row.ticket_id,
            started_at: row.started_at,
            stopped_at: row.stopped_at,
            segments: parse_segments(row.segments).unwrap_or_default(),
        })
        .collect())
}

// --- time entries ----------------------------------------------------------

pub async fn insert_time_entry<'e, E>(executor: E, e: &TimeEntry) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO time_entries (id, org_id, ticket_id, phase_id, user_id, started_at, ended_at,
                                     minutes, billable, notes, work_type, unbillable_reason,
                                     timer_session_id, created_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)"#,
        e.id,
        e.org_id,
        e.ticket_id,
        e.phase_id,
        e.user_id,
        e.started_at,
        e.ended_at,
        e.minutes as i32,
        e.billable,
        e.notes,
        e.work_type,
        e.unbillable_reason,
        e.timer_session_id,
        e.created_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Content-only time-entry update (import framework, the private build tracker D2
/// remainder). The owner (`user_id`) and ticket attachment are identity —
/// immutable here, exactly like an imported row's parents.
#[allow(clippy::too_many_arguments)]
pub async fn update_time_entry<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    started_at: chrono::DateTime<chrono::Utc>,
    ended_at: chrono::DateTime<chrono::Utc>,
    minutes: i64,
    billable: bool,
    notes: Option<&str>,
    work_type: Option<&str>,
    unbillable_reason: Option<&str>,
) -> DbResult<TimeEntry>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"UPDATE time_entries SET started_at = $3, ended_at = $4, minutes = $5,
                                  billable = $6, notes = $7, work_type = $8,
                                  unbillable_reason = $9
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, ticket_id, phase_id, user_id, started_at, ended_at, minutes,
                      billable, notes, work_type, unbillable_reason, timer_session_id,
                      approval_status, approval_reason_code, created_at"#,
        org_id,
        id,
        started_at,
        ended_at,
        minutes as i32,
        billable,
        notes,
        work_type,
        unbillable_reason,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(TimeEntry {
        id: row.id,
        org_id: row.org_id,
        ticket_id: row.ticket_id,
        phase_id: row.phase_id,
        user_id: row.user_id,
        started_at: row.started_at,
        ended_at: row.ended_at,
        minutes: row.minutes as i64,
        billable: row.billable,
        notes: row.notes,
        work_type: row.work_type,
        timer_session_id: row.timer_session_id,
        approval_status: core_domain::entities::ApprovalStatus::parse(&row.approval_status),
        approval_reason_code: row.approval_reason_code,
        unbillable_reason: row.unbillable_reason,
        created_at: row.created_at,
    })
}

pub async fn get_time_entry<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<TimeEntry>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, ticket_id, phase_id, user_id, started_at, ended_at, minutes, billable,
                  notes, work_type, unbillable_reason, timer_session_id, approval_status,
                  approval_reason_code, created_at
           FROM time_entries WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(TimeEntry {
        id: row.id,
        org_id: row.org_id,
        ticket_id: row.ticket_id,
        phase_id: row.phase_id,
        user_id: row.user_id,
        started_at: row.started_at,
        ended_at: row.ended_at,
        minutes: row.minutes as i64,
        billable: row.billable,
        notes: row.notes,
        work_type: row.work_type,
        timer_session_id: row.timer_session_id,
        approval_status: core_domain::entities::ApprovalStatus::parse(&row.approval_status),
        approval_reason_code: row.approval_reason_code,
        unbillable_reason: row.unbillable_reason,
        created_at: row.created_at,
    })
}

fn row_to_entry(row: sqlx::postgres::PgRow) -> TimeEntry {
    use sqlx::Row;
    let approval_status: Option<String> = row.try_get("approval_status").unwrap_or(None);
    let unbillable_reason: Option<String> = row.try_get("unbillable_reason").unwrap_or(None);
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
        approval_status: approval_status
            .as_deref()
            .and_then(core_domain::entities::ApprovalStatus::parse),
        approval_reason_code: row.get("approval_reason_code"),
        unbillable_reason,
        created_at: row.get("created_at"),
    }
}

pub async fn list_time_entries<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Option<Uuid>,
    user_id: Option<Uuid>,
    since: Option<chrono::DateTime<chrono::Utc>>,
    limit: i64,
) -> DbResult<Vec<TimeEntry>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = r#"SELECT id, org_id, ticket_id, phase_id, user_id, started_at, ended_at, minutes, billable,
                        notes, work_type, unbillable_reason, timer_session_id, approval_status,
                        approval_reason_code, created_at
                  FROM time_entries
                 WHERE org_id = $1
                   AND ($2::uuid IS NULL OR ticket_id = $2 OR phase_id = $2)
                   AND ($3::uuid IS NULL OR user_id = $3)
                   AND ($4::timestamptz IS NULL OR started_at >= $4)
                 ORDER BY started_at DESC LIMIT $5"#;
    let rows = sqlx::query(sql)
        .bind(org_id)
        .bind(ticket_id)
        .bind(user_id)
        .bind(since)
        .bind(limit)
        .fetch_all(executor)
        .await?;
    Ok(rows.into_iter().map(row_to_entry).collect())
}

/// Minutes logged by a user on a given UTC date (missing-time audit input).
pub async fn minutes_logged_on<'e, E>(
    executor: E,
    user_id: Uuid,
    org_id: Uuid,
    day_start: chrono::DateTime<chrono::Utc>,
    day_end: chrono::DateTime<chrono::Utc>,
) -> DbResult<i64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT COALESCE(SUM(minutes), 0) AS total FROM time_entries
           WHERE org_id = $1 AND user_id = $2 AND started_at >= $3 AND started_at < $4"#,
        org_id,
        user_id,
        day_start,
        day_end
    )
    .fetch_one(executor)
    .await?;
    Ok(row.total.unwrap_or(0) as i64)
}
