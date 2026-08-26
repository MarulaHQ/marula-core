//! Scheduling persistence (TASKLIST 2.12): appointments, availability
//! windows, on-call blocks, calendar exclusions.
//!
//! Purpose: CRUD plus absolute-time range queries feeding the day/week
//! calendar views. Overlap enforcement lives in the schema (GiST EXCLUDE
//! constraints from migration 0022); violations surface here as
//! `DbError::Sql` database errors carrying the constraint name, which
//! core-api maps to HTTP 409. Enum columns are stored as Postgres enums
//! and mapped via `::text` casts like `board_role`.
//!
//! Non-purpose: conflict *decisions* (core_domain::scheduling owns the
//! rules), outbox emission (routes emit inside their transaction), RLS
//! binding (callers scope every transaction via `set_org`).

use crate::{DbError, DbResult};
use core_domain::entities::{
    Appointment, AppointmentKind, AppointmentStatus, AvailabilityWindow, CalendarExclusion,
    OnCallBlock,
};
use sqlx::{PgConnection, Postgres};
use uuid::Uuid;

const APPOINTMENT_COLS: &str = "id, org_id, user_id, ticket_id, client_id, site_id, title, \
     starts_at, ends_at, kind::text AS kind, status::text AS status, travel_minutes, notes, \
     created_at, updated_at";

fn row_to_appt(row: &sqlx::postgres::PgRow) -> DbResult<Appointment> {
    let kind: String = row.get("kind");
    let status: String = row.get("status");
    Ok(Appointment {
        id: row.get("id"),
        org_id: row.get("org_id"),
        user_id: row.get("user_id"),
        ticket_id: row.get("ticket_id"),
        client_id: row.get("client_id"),
        site_id: row.get("site_id"),
        title: row.get("title"),
        starts_at: row.get("starts_at"),
        ends_at: row.get("ends_at"),
        kind: AppointmentKind::parse(&kind)
            .ok_or_else(|| unknown_enum("appointment_kind", &kind))?,
        status: AppointmentStatus::parse(&status)
            .ok_or_else(|| unknown_enum("appointment_status", &status))?,
        travel_minutes: row.get("travel_minutes"),
        notes: row.get("notes"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn unknown_enum(pg_type: &str, value: &str) -> DbError {
    DbError::Sql(sqlx::Error::Configuration(
        format!("unknown {pg_type} '{value}'").into(),
    ))
}

use sqlx::Row;

/// Inserts a fully-formed appointment. Caller generates ids/timestamps and
/// emits `appointment.created` in the same transaction.
pub async fn create_appointment<'e, E>(executor: E, a: &Appointment) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO appointments (id, org_id, user_id, ticket_id, client_id, site_id, title,
                                     starts_at, ends_at, kind, status, travel_minutes, notes)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::text::appointment_kind,
                   $11::text::appointment_status, $12, $13)"#,
        a.id,
        a.org_id,
        a.user_id,
        a.ticket_id,
        a.client_id,
        a.site_id,
        a.title,
        a.starts_at,
        a.ends_at,
        a.kind.as_str(),
        a.status.as_str(),
        a.travel_minutes,
        a.notes,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_appointment<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Appointment>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {APPOINTMENT_COLS} FROM appointments WHERE org_id = $1 AND id = $2");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    row_to_appt(&row)
}

/// Live (non-cancelled) appointments of `user_ids` intersecting
/// `[from, to)` — the calendar-view workhorse. Cancelled bookings are
/// hidden from views; done ones remain visible history.
pub async fn list_appointments_between<'e, E>(
    executor: E,
    org_id: Uuid,
    user_ids: &[Uuid],
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> DbResult<Vec<Appointment>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"SELECT {APPOINTMENT_COLS} FROM appointments
           WHERE org_id = $1 AND user_id = ANY($2)
             AND status <> 'cancelled'
             AND starts_at < $3 AND ends_at > $4
           ORDER BY starts_at, user_id"#
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(user_ids)
        .bind(to)
        .bind(from)
        .fetch_all(executor)
        .await?;
    rows.iter().map(row_to_appt).collect()
}

/// Persists editable fields (everything except id/org/user identity and
/// created_at). Time changes go through the same statement — the DB
/// EXCLUDE constraint re-validates automatically on UPDATE, so a move onto
/// an occupied slot fails here even without a pre-check.
pub async fn update_appointment<'e, E>(executor: E, a: &Appointment) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        r#"UPDATE appointments SET ticket_id = $3, client_id = $4, site_id = $5, title = $6,
                                   starts_at = $7, ends_at = $8, kind = $9::text::appointment_kind,
                                   status = $10::text::appointment_status,
                                   travel_minutes = $11, notes = $12, updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        a.org_id,
        a.id,
        a.ticket_id,
        a.client_id,
        a.site_id,
        a.title,
        a.starts_at,
        a.ends_at,
        a.kind.as_str(),
        a.status.as_str(),
        a.travel_minutes,
        a.notes,
    )
    .execute(executor)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

// --- availability windows ----------------------------------------------------

pub async fn create_availability_window<'e, E>(executor: E, w: &AvailabilityWindow) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO availability_windows (id, org_id, user_id, weekday, start_local, end_local,
                                             timezone, effective_from, effective_until)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#,
        w.id,
        w.org_id,
        w.user_id,
        w.weekday,
        w.start_local,
        w.end_local,
        w.timezone,
        w.effective_from,
        w.effective_until,
    )
    .execute(executor)
    .await?;
    Ok(())
}

const AVAILABILITY_COLS: &str =
    "id, org_id, user_id, weekday, start_local, end_local, timezone, effective_from, effective_until, created_at";

fn row_to_window(row: &sqlx::postgres::PgRow) -> AvailabilityWindow {
    AvailabilityWindow {
        id: row.get("id"),
        org_id: row.get("org_id"),
        user_id: row.get("user_id"),
        weekday: row.get::<i16, _>("weekday"),
        start_local: row.get("start_local"),
        end_local: row.get("end_local"),
        timezone: row.get("timezone"),
        effective_from: row.get("effective_from"),
        effective_until: row.get("effective_until"),
        created_at: row.get("created_at"),
    }
}

pub async fn list_availability_windows<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
) -> DbResult<Vec<AvailabilityWindow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {AVAILABILITY_COLS} FROM availability_windows \
         WHERE org_id = $1 AND user_id = $2 ORDER BY weekday, start_local"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(user_id)
        .fetch_all(executor)
        .await?;
    Ok(rows.iter().map(row_to_window).collect())
}

pub async fn delete_availability_window(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    user_id: Uuid,
) -> DbResult<()> {
    let result = sqlx::query!(
        "DELETE FROM availability_windows WHERE org_id = $1 AND id = $2 AND user_id = $3",
        org_id,
        id,
        user_id
    )
    .execute(&mut *conn)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

// --- on-call blocks -----------------------------------------------------------

pub async fn create_on_call_block<'e, E>(executor: E, b: &OnCallBlock) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO on_call_blocks (id, org_id, user_id, starts_at, ends_at, category, notes)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        b.id,
        b.org_id,
        b.user_id,
        b.starts_at,
        b.ends_at,
        b.category,
        b.notes,
    )
    .execute(executor)
    .await?;
    Ok(())
}

const ON_CALL_COLS: &str = "id, org_id, user_id, starts_at, ends_at, category, notes, created_at";

fn row_to_on_call(row: &sqlx::postgres::PgRow) -> OnCallBlock {
    OnCallBlock {
        id: row.get("id"),
        org_id: row.get("org_id"),
        user_id: row.get("user_id"),
        starts_at: row.get("starts_at"),
        ends_at: row.get("ends_at"),
        category: row.get("category"),
        notes: row.get("notes"),
        created_at: row.get("created_at"),
    }
}

pub async fn list_on_call_blocks_between<'e, E>(
    executor: E,
    org_id: Uuid,
    user_ids: &[Uuid],
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> DbResult<Vec<OnCallBlock>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"SELECT {ON_CALL_COLS} FROM on_call_blocks
           WHERE org_id = $1 AND user_id = ANY($2)
             AND starts_at < $3 AND ends_at > $4
           ORDER BY starts_at, user_id"#
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(user_ids)
        .bind(to)
        .bind(from)
        .fetch_all(executor)
        .await?;
    Ok(rows.iter().map(row_to_on_call).collect())
}

pub async fn delete_on_call_block(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    user_id: Uuid,
) -> DbResult<()> {
    let result = sqlx::query!(
        "DELETE FROM on_call_blocks WHERE org_id = $1 AND id = $2 AND user_id = $3",
        org_id,
        id,
        user_id
    )
    .execute(&mut *conn)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

// --- calendar exclusions (time-off) --------------------------------------------

pub async fn create_calendar_exclusion<'e, E>(executor: E, x: &CalendarExclusion) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO calendar_exclusions (id, org_id, user_id, starts_at, ends_at, reason)
           VALUES ($1, $2, $3, $4, $5, $6)"#,
        x.id,
        x.org_id,
        x.user_id,
        x.starts_at,
        x.ends_at,
        x.reason,
    )
    .execute(executor)
    .await?;
    Ok(())
}

const EXCLUSION_COLS: &str = "id, org_id, user_id, starts_at, ends_at, reason, created_at";

fn row_to_exclusion(row: &sqlx::postgres::PgRow) -> CalendarExclusion {
    CalendarExclusion {
        id: row.get("id"),
        org_id: row.get("org_id"),
        user_id: row.get("user_id"),
        starts_at: row.get("starts_at"),
        ends_at: row.get("ends_at"),
        reason: row.get("reason"),
        created_at: row.get("created_at"),
    }
}

pub async fn list_calendar_exclusions_between<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> DbResult<Vec<CalendarExclusion>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"SELECT {EXCLUSION_COLS} FROM calendar_exclusions
           WHERE org_id = $1 AND user_id = $2 AND starts_at < $3 AND ends_at > $4
           ORDER BY starts_at"#
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(user_id)
        .bind(to)
        .bind(from)
        .fetch_all(executor)
        .await?;
    Ok(rows.iter().map(row_to_exclusion).collect())
}

pub async fn delete_calendar_exclusion(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    user_id: Uuid,
) -> DbResult<()> {
    let result = sqlx::query!(
        "DELETE FROM calendar_exclusions WHERE org_id = $1 AND id = $2 AND user_id = $3",
        org_id,
        id,
        user_id
    )
    .execute(&mut *conn)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}
