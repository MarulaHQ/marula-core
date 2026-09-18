//! Scheduling-link persistence (Wave 11 lane 4): free-form groups,
//! meeting-type templates, and shareable booking links.
//!
//! Purpose: org-scoped CRUD over `scheduling_groups(_members)`,
//! `meeting_types`, and `scheduling_links`, plus the public booking
//! seam — token resolution through the SECURITY DEFINER
//! `scheduling_link_resolve` (0049) and the link → booking trail.
//!
//! Non-purpose: availability math or race-safe booking (the API layer
//! re-validates windows − exclusions − bookings at book time inside the
//! transaction; the appointments EXCLUDE constraint is the backstop).

use crate::{DbError, DbResult};
use chrono::Utc;
use core_domain::entities::{
    MeetingAssigneeScope, MeetingType, SchedulingGroup, SchedulingLink, SchedulingLinkBooking,
    SchedulingLinkStatus,
};
use sqlx::{PgConnection, Postgres, Row};
use uuid::Uuid;

// --- groups --------------------------------------------------------------------

const GROUP_COLS: &str =
    "g.id, g.org_id, g.name, g.description, g.created_by, g.created_at, g.updated_at";

fn row_to_group(row: &sqlx::postgres::PgRow) -> DbResult<SchedulingGroup> {
    Ok(SchedulingGroup {
        id: row.get("id"),
        org_id: row.get("org_id"),
        name: row.get("name"),
        description: row.get("description"),
        member_user_ids: Vec::new(),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub async fn create_group<'e, E>(executor: E, g: &SchedulingGroup) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        "INSERT INTO scheduling_groups (id, org_id, name, description, created_by)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(g.id)
    .bind(g.org_id)
    .bind(&g.name)
    .bind(&g.description)
    .bind(g.created_by)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn list_groups<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<SchedulingGroup>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {GROUP_COLS},
                COALESCE(m.member_ids, '{{}}'::uuid[]) AS member_user_ids
         FROM scheduling_groups g
         LEFT JOIN (
           SELECT group_id, array_agg(user_id ORDER BY user_id) AS member_ids
           FROM scheduling_group_members GROUP BY group_id
         ) m ON m.group_id = g.id
         WHERE g.org_id = $1
         ORDER BY g.created_at ASC"
    );
    let rows = sqlx::query(&sql).bind(org_id).fetch_all(executor).await?;
    rows.iter()
        .map(|row| {
            let mut group = row_to_group(row)?;
            group.member_user_ids = row.get("member_user_ids");
            Ok(group)
        })
        .collect()
}

pub async fn get_group<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<SchedulingGroup>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {GROUP_COLS},
                COALESCE(m.member_ids, '{{}}'::uuid[]) AS member_user_ids
         FROM scheduling_groups g
         LEFT JOIN (
           SELECT group_id, array_agg(user_id ORDER BY user_id) AS member_ids
           FROM scheduling_group_members GROUP BY group_id
         ) m ON m.group_id = g.id
         WHERE g.org_id = $1 AND g.id = $2"
    );
    sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)
        .and_then(|row| {
            let mut group = row_to_group(&row)?;
            group.member_user_ids = row.get("member_user_ids");
            Ok(group)
        })
}

pub async fn update_group<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    name: &str,
    description: Option<&str>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        "UPDATE scheduling_groups SET name = $3, description = $4, updated_at = now()
         WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(id)
    .bind(name)
    .bind(description)
    .execute(executor)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

pub async fn delete_group(conn: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<()> {
    let deleted = sqlx::query("DELETE FROM scheduling_groups WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(conn)
        .await?
        .rows_affected();
    if deleted == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

pub async fn add_group_member<'e, E>(
    executor: E,
    org_id: Uuid,
    group_id: Uuid,
    user_id: Uuid,
    added_by: Uuid,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    // The guard join keeps the write org-bound even though the members
    // table carries no org_id column.
    let result = sqlx::query(
        "INSERT INTO scheduling_group_members (group_id, user_id, added_by)
         SELECT $2, $3, $4 FROM scheduling_groups g
         WHERE g.id = $2 AND g.org_id = $1
         ON CONFLICT (group_id, user_id) DO NOTHING",
    )
    .bind(org_id)
    .bind(group_id)
    .bind(user_id)
    .bind(added_by)
    .execute(executor)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

pub async fn remove_group_member<'e, E>(
    executor: E,
    org_id: Uuid,
    group_id: Uuid,
    user_id: Uuid,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        "DELETE FROM scheduling_group_members m
         USING scheduling_groups g
         WHERE m.group_id = g.id AND g.org_id = $1
           AND m.group_id = $2 AND m.user_id = $3",
    )
    .bind(org_id)
    .bind(group_id)
    .bind(user_id)
    .execute(executor)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

// --- meeting types ---------------------------------------------------------------

const MEETING_TYPE_COLS: &str = "id, org_id, name, description, duration_minutes, buffer_minutes, \
     booking_window_days, parameters, assignee_scope::text AS assignee_scope, assignee_user_id, \
     assignee_group_id, auto_email_enabled, auto_email_subject, auto_email_body, is_active, \
     created_by, created_at, updated_at";

fn row_to_meeting_type(row: &sqlx::postgres::PgRow) -> DbResult<MeetingType> {
    let scope: String = row.get("assignee_scope");
    Ok(MeetingType {
        id: row.get("id"),
        org_id: row.get("org_id"),
        name: row.get("name"),
        description: row.get("description"),
        duration_minutes: row.get("duration_minutes"),
        buffer_minutes: row.get("buffer_minutes"),
        booking_window_days: row.get("booking_window_days"),
        parameters: row.get("parameters"),
        assignee_scope: MeetingAssigneeScope::parse(&scope)
            .ok_or_else(|| crate::scheduling::unknown_enum("meeting_assignee_scope", &scope))?,
        assignee_user_id: row.get("assignee_user_id"),
        assignee_group_id: row.get("assignee_group_id"),
        auto_email_enabled: row.get("auto_email_enabled"),
        auto_email_subject: row.get("auto_email_subject"),
        auto_email_body: row.get("auto_email_body"),
        is_active: row.get("is_active"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub async fn create_meeting_type<'e, E>(executor: E, m: &MeetingType) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        "INSERT INTO meeting_types (id, org_id, name, description, duration_minutes,
                                     buffer_minutes, booking_window_days, parameters,
                                     assignee_scope, assignee_user_id, assignee_group_id,
                                     auto_email_enabled, auto_email_subject, auto_email_body,
                                     is_active, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                 $10, $11, $12, $13, $14, $15, $16)",
    )
    .bind(m.id)
    .bind(m.org_id)
    .bind(&m.name)
    .bind(&m.description)
    .bind(m.duration_minutes)
    .bind(m.buffer_minutes)
    .bind(m.booking_window_days)
    .bind(&m.parameters)
    .bind(m.assignee_scope.as_str())
    .bind(m.assignee_user_id)
    .bind(m.assignee_group_id)
    .bind(m.auto_email_enabled)
    .bind(&m.auto_email_subject)
    .bind(&m.auto_email_body)
    .bind(m.is_active)
    .bind(m.created_by)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn list_meeting_types<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<MeetingType>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {MEETING_TYPE_COLS} FROM meeting_types
         WHERE org_id = $1 ORDER BY created_at ASC"
    );
    let rows = sqlx::query(&sql).bind(org_id).fetch_all(executor).await?;
    rows.iter().map(row_to_meeting_type).collect()
}

pub async fn get_meeting_type<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<MeetingType>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {MEETING_TYPE_COLS} FROM meeting_types
         WHERE org_id = $1 AND id = $2"
    );
    sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)
        .and_then(|row| row_to_meeting_type(&row))
}

/// Hard-deletes one meeting type. `Ok(true)` when the row went away;
/// `Ok(false)` = already gone. A scheduling link still referencing the
/// row raises the FK RESTRICT (`23503`) — the route surfaces that as a
/// typed conflict.
pub async fn delete_meeting_type<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query("DELETE FROM meeting_types WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(executor)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Full update; the caller loaded the row first (404 semantics live there).
pub async fn update_meeting_type<'e, E>(executor: E, m: &MeetingType) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        "UPDATE meeting_types SET name = $3, description = $4, duration_minutes = $5,
                                   buffer_minutes = $6, booking_window_days = $7,
                                   parameters = $8, assignee_scope = $9::text,
                                   assignee_user_id = $10, assignee_group_id = $11,
                                   auto_email_enabled = $12, auto_email_subject = $13,
                                   auto_email_body = $14, is_active = $15, updated_at = now()
           WHERE org_id = $1 AND id = $2",
    )
    .bind(m.org_id)
    .bind(m.id)
    .bind(&m.name)
    .bind(&m.description)
    .bind(m.duration_minutes)
    .bind(m.buffer_minutes)
    .bind(m.booking_window_days)
    .bind(&m.parameters)
    .bind(m.assignee_scope.as_str())
    .bind(m.assignee_user_id)
    .bind(m.assignee_group_id)
    .bind(m.auto_email_enabled)
    .bind(&m.auto_email_subject)
    .bind(&m.auto_email_body)
    .bind(m.is_active)
    .execute(executor)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

// --- links ------------------------------------------------------------------------

const LINK_COLS: &str = "id, org_id, meeting_type_id, label, ticket_id, status::text AS status, \
     expires_at, first_viewed_at, view_count::bigint AS view_count, scheduled_at, \
     scheduled_appointment_id, created_by, created_at, updated_at";

fn row_to_link(row: &sqlx::postgres::PgRow) -> DbResult<SchedulingLink> {
    let status: String = row.get("status");
    Ok(SchedulingLink {
        id: row.get("id"),
        org_id: row.get("org_id"),
        meeting_type_id: row.get("meeting_type_id"),
        label: row.get("label"),
        ticket_id: row.get("ticket_id"),
        status: SchedulingLinkStatus::parse(&status)
            .ok_or_else(|| crate::scheduling::unknown_enum("scheduling_link_status", &status))?,
        expires_at: row.get("expires_at"),
        first_viewed_at: row.get("first_viewed_at"),
        view_count: row.get("view_count"),
        scheduled_at: row.get("scheduled_at"),
        scheduled_appointment_id: row.get("scheduled_appointment_id"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub async fn create_link<'e, E>(executor: E, l: &SchedulingLink, token_hash: &str) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        "INSERT INTO scheduling_links (id, org_id, meeting_type_id, token_hash, label, ticket_id,
                                        status, expires_at, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7::text, $8, $9)",
    )
    .bind(l.id)
    .bind(l.org_id)
    .bind(l.meeting_type_id)
    .bind(token_hash)
    .bind(&l.label)
    .bind(l.ticket_id)
    .bind(l.status.as_str())
    .bind(l.expires_at)
    .bind(l.created_by)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn list_links<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<SchedulingLink>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {LINK_COLS} FROM scheduling_links
         WHERE org_id = $1 ORDER BY created_at DESC"
    );
    let rows = sqlx::query(&sql).bind(org_id).fetch_all(executor).await?;
    rows.iter().map(row_to_link).collect()
}

pub async fn get_link<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<SchedulingLink>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {LINK_COLS} FROM scheduling_links WHERE org_id = $1 AND id = $2");
    sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)
        .and_then(|row| row_to_link(&row))
}

/// Revoke (staff): flips status unless already used — a used link keeps
/// its `used` state so the booking trail never changes meaning.
pub async fn revoke_link(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<SchedulingLink> {
    let sql = format!(
        "UPDATE scheduling_links SET status = CASE WHEN status = 'used' THEN 'used' ELSE 'revoked' END,
                                        updated_at = now()
         WHERE org_id = $1 AND id = $2 AND status <> 'revoked'
         RETURNING {LINK_COLS}"
    );
    let updated = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    match updated {
        Some(row) => row_to_link(&row),
        None => get_link(&mut *conn, org_id, id).await, // already revoked → NotFound or the row
    }
}

/// Resolves a token hash through the SECURITY DEFINER seam (public
/// surface, no org scope bound yet). `None` = unknown token.
pub async fn resolve_link_by_token(
    conn: &mut PgConnection,
    token_hash: &str,
) -> DbResult<ResolvedLink> {
    let row = sqlx::query("SELECT * FROM scheduling_link_resolve($1)")
        .bind(token_hash)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(ResolvedLink {
        link_id: row.get("link_id"),
        org_id: row.get("org_id"),
        meeting_type_id: row.get("meeting_type_id"),
        link_status: row.get::<String, _>("link_status"),
        expires_at: row.get("expires_at"),
        label: row.get("label"),
        meeting_type: MeetingType {
            id: row.get("meeting_type_id"),
            org_id: row.get("org_id"),
            name: row.get("meeting_type_name"),
            description: row.get("meeting_type_description"),
            duration_minutes: row.get("duration_minutes"),
            buffer_minutes: row.get("buffer_minutes"),
            booking_window_days: row.get("booking_window_days"),
            parameters: row.get("parameters"),
            assignee_scope: MeetingAssigneeScope::parse(&row.get::<String, _>("assignee_scope"))
                .ok_or_else(|| {
                    DbError::Sql(sqlx::Error::Configuration(
                        "unknown assignee scope in resolved link".into(),
                    ))
                })?,
            assignee_user_id: row.get("assignee_user_id"),
            assignee_group_id: row.get("assignee_group_id"),
            auto_email_enabled: row.get("auto_email_enabled"),
            auto_email_subject: row.get("auto_email_subject"),
            auto_email_body: row.get("auto_email_body"),
            is_active: row.get("meeting_type_is_active"),
            created_by: None,
            created_at: row
                .get::<Option<chrono::DateTime<chrono::Utc>>, _>("first_viewed_at")
                .unwrap_or_else(Utc::now),
            updated_at: row
                .get::<Option<chrono::DateTime<chrono::Utc>>, _>("first_viewed_at")
                .unwrap_or_else(Utc::now),
        },
        first_viewed_at: row.get("first_viewed_at"),
        view_count: row.get::<i32, _>("view_count") as i64,
        scheduled_at: row.get("scheduled_at"),
        ticket_id: row.get("r_ticket_id"),
    })
}

/// The public surface's view of a resolved token (0049 definer seam).
#[derive(Debug)]
pub struct ResolvedLink {
    pub link_id: Uuid,
    pub org_id: Uuid,
    pub meeting_type_id: Uuid,
    pub link_status: String,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub label: Option<String>,
    pub meeting_type: MeetingType,
    pub first_viewed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub view_count: i64,
    pub scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
    /// The originating ticket cross-ref (0064): the ledger-enqueued
    /// auto-email lands back on this ticket's timeline.
    pub ticket_id: Option<Uuid>,
}

/// Lifecycle stamp for a public page load (definer seam; fire-and-forget).
pub async fn mark_link_viewed(conn: &mut PgConnection, token_hash: &str) -> DbResult<()> {
    sqlx::query("SELECT scheduling_link_mark_viewed($1)")
        .bind(token_hash)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Status flip to `used` at booking time (definer seam; called inside
/// the booking transaction BEFORE the appointment insert so a rollback
/// un-flips it atomically). `false` = the link was not in `active`
/// state (revoked, already used, or unknown) — the caller must reject.
pub async fn mark_link_scheduled<'e, E>(
    executor: E,
    token_hash: &str,
    appointment_id: Uuid,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query("SELECT scheduling_link_mark_scheduled($1, $2) AS won")
        .bind(token_hash)
        .bind(appointment_id)
        .fetch_one(executor)
        .await?;
    Ok(row.get::<bool, _>("won"))
}

pub async fn create_link_booking<'e, E>(executor: E, b: &SchedulingLinkBooking) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        "INSERT INTO scheduling_link_bookings (link_id, appointment_id, booker_name,
                                                booker_email, booker_notes)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(b.link_id)
    .bind(b.appointment_id)
    .bind(&b.booker_name)
    .bind(&b.booker_email)
    .bind(&b.booker_notes)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn list_link_bookings<'e, E>(
    executor: E,
    org_id: Uuid,
    link_id: Uuid,
) -> DbResult<Vec<SchedulingLinkBooking>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        "SELECT b.link_id, b.appointment_id, b.booker_name, b.booker_email, b.booker_notes,
                b.created_at
         FROM scheduling_link_bookings b
         JOIN scheduling_links l ON l.id = b.link_id
         WHERE l.org_id = $1 AND b.link_id = $2
         ORDER BY b.created_at DESC",
    )
    .bind(org_id)
    .bind(link_id)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .iter()
        .map(|row| SchedulingLinkBooking {
            link_id: row.get("link_id"),
            appointment_id: row.get("appointment_id"),
            booker_name: row.get("booker_name"),
            booker_email: row.get("booker_email"),
            booker_notes: row.get("booker_notes"),
            created_at: row.get("created_at"),
        })
        .collect())
}
