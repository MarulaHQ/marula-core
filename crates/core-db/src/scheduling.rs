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
    OnCallBlock, TravelOrigin,
};
use sqlx::{PgConnection, Postgres};
use uuid::Uuid;

const APPOINTMENT_COLS: &str = "id, org_id, user_id, ticket_id, client_id, site_id, title, \
     starts_at, ends_at, kind::text AS kind, status::text AS status, travel_minutes, notes, \
     scheduling_link_id, travel_from, travel_distance_meters, travel_provider, travel_quality, \
     created_at, updated_at";

fn row_to_appt(row: &sqlx::postgres::PgRow) -> DbResult<Appointment> {
    let kind: String = row.get("kind");
    let status: String = row.get("status");
    let travel_from: Option<serde_json::Value> = row.get("travel_from");
    let travel_from = match travel_from {
        Some(value) => Some(serde_json::from_value::<TravelOrigin>(value).map_err(|e| {
            DbError::Sql(sqlx::Error::Decode(
                format!("appointment travel_from is not a TravelOrigin: {e}").into(),
            ))
        })?),
        None => None,
    };
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
        scheduling_link_id: row.get("scheduling_link_id"),
        travel_from,
        travel_distance_meters: row.get("travel_distance_meters"),
        travel_provider: row.get("travel_provider"),
        travel_quality: row.get("travel_quality"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub(crate) fn unknown_enum(pg_type: &str, value: &str) -> DbError {
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
    let travel_from = a
        .travel_from
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| DbError::Sql(sqlx::Error::Encode(format!("travel_from: {e}").into())))?;
    sqlx::query!(
        r#"INSERT INTO appointments (id, org_id, user_id, ticket_id, client_id, site_id, title,
                                     starts_at, ends_at, kind, status, travel_minutes, notes,
                                     scheduling_link_id, travel_from, travel_distance_meters,
                                     travel_provider, travel_quality)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::text::appointment_kind,
                   $11::text::appointment_status, $12, $13, $14, $15, $16, $17, $18)"#,
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
        a.scheduling_link_id,
        travel_from,
        a.travel_distance_meters,
        a.travel_provider,
        a.travel_quality,
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

/// One board block with the identifiers the dispatcher needs on the block
/// face: ticket number and client name when the booking is ticket-bound.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DispatchAppointmentRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub ticket_id: Option<Uuid>,
    pub client_id: Option<Uuid>,
    pub site_id: Option<Uuid>,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub ends_at: chrono::DateTime<chrono::Utc>,
    pub kind: String,
    pub status: String,
    pub travel_minutes: i32,
    pub travel_provider: Option<String>,
    pub travel_quality: Option<String>,
    pub travel_distance_meters: Option<i32>,
    pub travel_from: Option<serde_json::Value>,
    pub ticket_number: Option<String>,
    pub client_name: Option<String>,
}

/// Live appointments of `user_ids` intersecting `[from, to)` plus the
/// joined ticket number and client name (dispatch day-view fuel). Same
/// predicate as [`list_appointments_between`] — views must agree on what
/// counts as a visible booking.
pub async fn list_dispatch_appointments_between<'e, E>(
    executor: E,
    org_id: Uuid,
    user_ids: &[Uuid],
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> DbResult<Vec<DispatchAppointmentRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = r#"SELECT a.id, a.org_id, a.user_id, a.ticket_id, a.client_id, a.site_id, a.title,
                  a.starts_at, a.ends_at, a.kind::text AS kind, a.status::text AS status,
                  a.travel_minutes, a.travel_provider, a.travel_quality,
                  a.travel_distance_meters, a.travel_from,
                  t.number AS ticket_number,
                  c.name AS client_name
           FROM appointments a
           LEFT JOIN tickets t ON t.id = a.ticket_id
           LEFT JOIN clients c ON c.id = a.client_id
           WHERE a.org_id = $1 AND a.user_id = ANY($2)
             AND a.status <> 'cancelled'
             AND a.starts_at < $3 AND a.ends_at > $4
           ORDER BY a.starts_at, a.user_id"#;
    sqlx::query_as::<_, DispatchAppointmentRow>(sql)
        .bind(org_id)
        .bind(user_ids)
        .bind(to)
        .bind(from)
        .fetch_all(executor)
        .await
        .map_err(DbError::Sql)
}

/// Persists editable fields (everything except id/org/user identity and
/// created_at). Time changes go through the same statement — the DB
/// EXCLUDE constraint re-validates automatically on UPDATE, so a move onto
/// an occupied slot fails here even without a pre-check.
pub async fn update_appointment<'e, E>(executor: E, a: &Appointment) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let travel_from = a
        .travel_from
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| DbError::Sql(sqlx::Error::Encode(format!("travel_from: {e}").into())))?;
    let result = sqlx::query!(
        r#"UPDATE appointments SET ticket_id = $3, client_id = $4, site_id = $5, title = $6,
                                   starts_at = $7, ends_at = $8, kind = $9::text::appointment_kind,
                                   status = $10::text::appointment_status,
                                   travel_minutes = $11, notes = $12,
                                   scheduling_link_id = $13,
                                   travel_from = $14, travel_distance_meters = $15,
                                   travel_provider = $16, travel_quality = $17,
                                   updated_at = now()
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
        a.scheduling_link_id,
        travel_from,
        a.travel_distance_meters,
        a.travel_provider,
        a.travel_quality,
    )
    .execute(executor)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Removes one booking (dispatch bulk-move phase 1): the caller's
/// transaction reinserts moved bookings at their targets via
/// [`create_appointment`], so every statement boundary stays
/// constraint-clean while the batch remains atomic.
pub async fn delete_appointment(conn: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<()> {
    let deleted = sqlx::query!(
        "DELETE FROM appointments WHERE org_id = $1 AND id = $2",
        org_id,
        id
    )
    .execute(conn)
    .await?;
    if deleted.rows_affected() == 0 {
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

/// Every user's windows in one read — the Portal v2 slot computation
/// expands the whole org's published availability at once.
pub async fn list_all_availability_windows<'e, E>(
    executor: E,
    org_id: Uuid,
) -> DbResult<Vec<AvailabilityWindow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {AVAILABILITY_COLS} FROM availability_windows \
         WHERE org_id = $1 ORDER BY user_id, weekday, start_local"
    );
    let rows = sqlx::query(&sql).bind(org_id).fetch_all(executor).await?;
    Ok(rows.iter().map(row_to_window).collect())
}

/// Time-off exclusions for MANY users over a range (Portal v2 slot
/// computation subtracts these from the published availability).
pub async fn list_calendar_exclusions_for_users_between<'e, E>(
    executor: E,
    org_id: Uuid,
    user_ids: &[Uuid],
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> DbResult<Vec<CalendarExclusion>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"SELECT {EXCLUSION_COLS} FROM calendar_exclusions
           WHERE org_id = $1 AND user_id = ANY($2) AND starts_at < $3 AND ends_at > $4
           ORDER BY starts_at, user_id"#
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(user_ids)
        .bind(to)
        .bind(from)
        .fetch_all(executor)
        .await?;
    Ok(rows.iter().map(row_to_exclusion).collect())
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

// --- calendar deadline read model (Wave 17 lane 13, migration 0089) ------------

/// One ticket deadline in the org-wide read model. Org-scoped, windowed,
/// alive-only (not closed, due date present); hydration stays in the row
/// itself so the calendar overlay never issues N+1 follow-ups.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TicketDeadlineRow {
    pub ticket_id: Uuid,
    pub number: String,
    pub title: String,
    pub due_at: chrono::DateTime<chrono::Utc>,
    pub category: String,
    pub assignee_id: Option<Uuid>,
    pub board_id: Uuid,
    pub client_id: Option<Uuid>,
}

/// Org-wide ticket deadlines within [from, to] — the /calendar team-mode
/// overlay replaces the bounded page-1 tickets fetch with THIS read model.
/// Closed tickets drop (a deadline on a closed ticket is not work);
/// `assignee` narrows to one person (the personal mode walks the same
/// model). Capped defensively; a window is small by construction.
pub async fn list_ticket_deadlines_between<'e, E>(
    executor: E,
    org_id: Uuid,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    assignee: Option<Uuid>,
) -> DbResult<Vec<TicketDeadlineRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as::<_, TicketDeadlineRow>(
        r#"SELECT id AS ticket_id, number, title, due_at, category, assignee_id, board_id, client_id
           FROM tickets
           WHERE org_id = $1
             AND due_at IS NOT NULL
             AND due_at >= $2
             AND due_at < $3
             AND ($4::uuid IS NULL OR assignee_id = $4)
             AND category <> 'closed'
           ORDER BY due_at, id
           LIMIT 500"#,
    )
    .bind(org_id)
    .bind(from)
    .bind(to)
    .bind(assignee)
    .fetch_all(executor)
    .await?;
    Ok(rows)
}
