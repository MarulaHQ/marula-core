//! PM schedules (Wave 11 lane 9, migration 0056): recurring/scheduled
//! ticket templates + the run ledger.
//!
//! Purpose: persistence for the recurring-ticket construct — schedule
//! CRUD, the due-claim sweep the worker tick calls, and the `pm_runs`
//! ledger the core-api generator writes in the same transaction as the
//! ticket insert (unique per schedule+slot ⇒ a retried job can never
//! double-generate).
//!
//! Non-purpose: no cadence math here (core-domain `cadence` owns the
//! math; the schedule row is stored shape), no ticket creation (core-api
//! internal surface does the domain write), no events (routes emit CRUD
//! events; the ledger is the run record).

use crate::{DbError, DbResult};
use core_domain::cadence::{shape_of, CadenceError, ScheduleShape};
use core_domain::entities::{PmRun, PmSchedule};
use sqlx::{PgPool, Postgres};
use uuid::Uuid;

const SCHEDULE_COLS: &str = "id, org_id, name, board_id, title, body, priority, client_id, site_id, assignee_id, cadence, time_of_day, day_of_week, day_of_month, timezone, enabled, next_run_at, last_run_at, last_error, created_by, created_at, updated_at";

fn parse_cadence_row(
    cadence: &str,
    time_of_day: chrono::NaiveTime,
    day_of_week: Option<i16>,
    day_of_month: Option<i16>,
    timezone: &str,
) -> Result<ScheduleShape, CadenceError> {
    shape_of(cadence, time_of_day, day_of_week, day_of_month, timezone)
}

fn schedule_from(row: sqlx::postgres::PgRow) -> DbResult<PmSchedule> {
    use sqlx::Row;
    let cadence: String = row.get("cadence");
    let time_of_day: chrono::NaiveTime = row.get("time_of_day");
    let day_of_week: Option<i16> = row.get("day_of_week");
    let day_of_month: Option<i16> = row.get("day_of_month");
    let timezone: String = row.get("timezone");
    Ok(PmSchedule {
        id: row.get("id"),
        org_id: row.get("org_id"),
        name: row.get("name"),
        board_id: row.get("board_id"),
        title: row.get("title"),
        body: row.get("body"),
        priority: row
            .try_get::<String, _>("priority")
            .ok()
            .and_then(|p| p.parse().ok())
            .ok_or(DbError::NotFound)?,
        client_id: row.get("client_id"),
        site_id: row.get("site_id"),
        assignee_id: row.get("assignee_id"),
        cadence: core_domain::cadence::Cadence::parse(&cadence).ok_or(DbError::NotFound)?,
        time_of_day,
        day_of_week,
        day_of_month,
        timezone,
        enabled: row.get("enabled"),
        next_run_at: row.get("next_run_at"),
        last_run_at: row.get("last_run_at"),
        last_error: row.get("last_error"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub async fn list_schedules<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<PmSchedule>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {SCHEDULE_COLS} FROM pm_schedules WHERE org_id = $1 \
         ORDER BY next_run_at NULLS LAST, name"
    );
    let rows = sqlx::query(&sql).bind(org_id).fetch_all(executor).await?;
    rows.into_iter().map(schedule_from).collect()
}

pub async fn get_schedule<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<PmSchedule>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {SCHEDULE_COLS} FROM pm_schedules WHERE org_id = $1 AND id = $2");
    sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .map(schedule_from)
        .transpose()?
        .ok_or(DbError::NotFound)
}

pub struct NewPmSchedule<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: &'a str,
    pub board_id: Uuid,
    pub title: &'a str,
    pub body: Option<&'a str>,
    pub priority: &'a str,
    pub client_id: Option<Uuid>,
    pub site_id: Option<Uuid>,
    pub assignee_id: Option<Uuid>,
    pub cadence: &'a str,
    pub time_of_day: chrono::NaiveTime,
    pub day_of_week: Option<i16>,
    pub day_of_month: Option<i16>,
    pub timezone: &'a str,
    pub next_run_at: chrono::DateTime<chrono::Utc>,
    pub created_by: Uuid,
}

pub async fn insert_schedule<'e, E>(executor: E, s: &NewPmSchedule<'_>) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO pm_schedules
             (id, org_id, name, board_id, title, body, priority, client_id, site_id,
              assignee_id, cadence, time_of_day, day_of_week, day_of_month, timezone,
              enabled, next_run_at, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
                   true, $16, $17)"#,
        s.id,
        s.org_id,
        s.name,
        s.board_id,
        s.title,
        s.body,
        s.priority,
        s.client_id,
        s.site_id,
        s.assignee_id,
        s.cadence,
        s.time_of_day,
        s.day_of_week,
        s.day_of_month,
        s.timezone,
        s.next_run_at,
        s.created_by
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub struct PmSchedulePatch<'a> {
    pub name: Option<&'a str>,
    pub title: Option<&'a str>,
    pub body: Option<Option<&'a str>>,
    pub priority: Option<&'a str>,
    pub client_id: Option<Option<Uuid>>,
    pub site_id: Option<Option<Uuid>>,
    pub assignee_id: Option<Option<Uuid>>,
    pub cadence: Option<&'a str>,
    pub time_of_day: Option<chrono::NaiveTime>,
    pub day_of_week: Option<Option<i16>>,
    pub day_of_month: Option<Option<i16>>,
    pub timezone: Option<&'a str>,
    pub enabled: Option<bool>,
    pub next_run_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn update_schedule<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    p: &PmSchedulePatch<'_>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"UPDATE pm_schedules SET
             name = COALESCE($3, name),
             title = COALESCE($4, title),
             body = CASE WHEN $5::boolean THEN $6 ELSE body END,
             priority = COALESCE($7, priority),
             client_id = CASE WHEN $8::boolean THEN $9 ELSE client_id END,
             site_id = CASE WHEN $10::boolean THEN $11 ELSE site_id END,
             assignee_id = CASE WHEN $12::boolean THEN $13 ELSE assignee_id END,
             cadence = COALESCE($14, cadence),
             time_of_day = COALESCE($15, time_of_day),
             day_of_week = CASE WHEN $16::boolean THEN $17 ELSE day_of_week END,
             day_of_month = CASE WHEN $18::boolean THEN $19 ELSE day_of_month END,
             timezone = COALESCE($20, timezone),
             enabled = COALESCE($21, enabled),
             next_run_at = COALESCE($22, next_run_at),
             updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
        p.name,
        p.title,
        p.body.is_some(),
        p.body.flatten(),
        p.priority,
        p.client_id.is_some(),
        p.client_id.flatten(),
        p.site_id.is_some(),
        p.site_id.flatten(),
        p.assignee_id.is_some(),
        p.assignee_id.flatten(),
        p.cadence,
        p.time_of_day,
        p.day_of_week.is_some(),
        p.day_of_week.flatten(),
        p.day_of_month.is_some(),
        p.day_of_month.flatten(),
        p.timezone,
        p.enabled,
        p.next_run_at
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

pub async fn delete_schedule<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        "DELETE FROM pm_schedules WHERE org_id = $1 AND id = $2",
        org_id,
        id
    )
    .execute(executor)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// A due claim: enabled schedules whose `next_run_at` has passed. The
/// worker tick claims with SKIP LOCKED and advances `next_run_at` to the
/// cadence's next occurrence IN THE SAME STATEMENT — a slot fires
/// AT-MOST-ONCE per schedule even with overlapping workers (the same
/// crash semantics as report delivery: a crash between advance and
/// apply loses that slot; the enqueue job is idempotent via the ledger).
pub struct DuePmSchedule {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub cadence: String,
    pub cadence_time_of_day: chrono::NaiveTime,
    pub day_of_week: Option<i16>,
    pub day_of_month: Option<i16>,
    pub timezone: String,
    /// The discharged slot (the pre-claim `next_run_at`; the claim's
    /// +1h defensive bump makes it reconstructable from RETURNING).
    pub slot_at: chrono::DateTime<chrono::Utc>,
    /// `next_run_at` after the claim advanced it — `stamp_next_run`'s
    /// CAS anchor so a concurrent re-claim can never double-advance.
    pub next_run_at: chrono::DateTime<chrono::Utc>,
}

pub async fn claim_due_schedules(pool: &PgPool, limit: i64) -> DbResult<Vec<DuePmSchedule>> {
    let rows = sqlx::query!(
        r#"UPDATE pm_schedules SET
             next_run_at = next_run_at + interval '1 hour',
             updated_at = now()
           WHERE id IN (
             SELECT id FROM pm_schedules
             WHERE enabled AND next_run_at <= now()
             ORDER BY next_run_at
             LIMIT $1
             FOR UPDATE SKIP LOCKED
           )
           RETURNING id, org_id, name, cadence, time_of_day, day_of_week,
                     day_of_month, timezone, next_run_at"#,
        limit
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| DuePmSchedule {
            id: r.id,
            org_id: r.org_id,
            name: r.name,
            cadence: r.cadence,
            cadence_time_of_day: r.time_of_day,
            day_of_week: r.day_of_week,
            day_of_month: r.day_of_month,
            timezone: r.timezone,
            slot_at: r.next_run_at - chrono::Duration::hours(1),
            next_run_at: r.next_run_at,
        })
        .collect())
}

/// Stamps the schedule's next occurrence + observability after its claim
/// (the worker holds the slot; the cadence math advances it). A row whose
/// stored cadence violates the shape NEVER wedges: the claim's +1h bump
/// stays as `next_run_at` (the row remains claimable), `last_error`
/// records the reason, and the call returns Ok(false) so the worker
/// skips the slot without losing the schedule.
pub async fn stamp_next_run(
    pool: &PgPool,
    schedule: &DuePmSchedule,
    last_error: Option<&str>,
) -> DbResult<bool> {
    let shape = match parse_cadence_row(
        &schedule.cadence,
        schedule.cadence_time_of_day,
        schedule.day_of_week,
        schedule.day_of_month,
        &schedule.timezone,
    ) {
        Ok(shape) => shape,
        Err(e) => {
            sqlx::query!(
                r#"UPDATE pm_schedules SET last_error = $3, updated_at = now()
                   WHERE id = $1 AND next_run_at = $2"#,
                schedule.id,
                schedule.next_run_at,
                format!("invalid cadence: {e}")
            )
            .execute(pool)
            .await?;
            return Ok(false);
        }
    };
    let next = match core_domain::cadence::next_run_after(&shape, schedule.slot_at) {
        Ok(next) => next,
        Err(e) => {
            sqlx::query!(
                r#"UPDATE pm_schedules SET last_error = $3, updated_at = now()
                   WHERE id = $1 AND next_run_at = $2"#,
                schedule.id,
                schedule.next_run_at,
                format!("cadence math failed: {e}")
            )
            .execute(pool)
            .await?;
            return Ok(false);
        }
    };
    sqlx::query!(
        r#"UPDATE pm_schedules SET
             next_run_at = $3,
             last_run_at = $4,
             last_error = CASE WHEN $5::boolean THEN $6 ELSE NULL END,
             updated_at = now()
           WHERE id = $1 AND next_run_at = $2"#,
        schedule.id,
        schedule.next_run_at,
        next,
        schedule.slot_at,
        last_error.is_some(),
        last_error
    )
    .execute(pool)
    .await?;
    Ok(true)
}

/// The generator's ledger gate + run record: inert when the slot was
/// already discharged (unique per schedule+slot). Returns whether THIS
/// call inserted the ledger row (i.e. the caller should generate).
pub async fn claim_run_slot<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    schedule_id: Uuid,
    slot_at: chrono::DateTime<chrono::Utc>,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"INSERT INTO pm_runs (id, org_id, schedule_id, slot_at, status, detail)
           VALUES ($1, $2, $3, $4, 'generated', '{}'::jsonb)
           ON CONFLICT (schedule_id, slot_at) DO NOTHING"#,
        id,
        org_id,
        schedule_id,
        slot_at
    )
    .execute(executor)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Settles the ledger row's ticket backlink (the ticket was generated).
pub async fn record_run_ticket<'e, E>(
    executor: E,
    org_id: Uuid,
    run_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"UPDATE pm_runs SET ticket_id = $3
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        run_id,
        ticket_id
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Marks the claimed slot skipped (schedule disabled between claim and
/// apply) — the ledger row keeps the outcome visible.
pub async fn record_run_skipped<'e, E>(
    executor: E,
    run_id: Uuid,
    org_id: Uuid,
    reason: &str,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"UPDATE pm_runs SET status = 'skipped', detail = jsonb_build_object('reason', $3::text)
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        run_id,
        reason
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn list_runs<'e, E>(
    executor: E,
    org_id: Uuid,
    schedule_id: Uuid,
    limit: i64,
) -> DbResult<Vec<PmRun>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as!(
        PmRunRow,
        r#"SELECT id, org_id, schedule_id, slot_at, status, ticket_id, detail, created_at
           FROM pm_runs WHERE org_id = $1 AND schedule_id = $2
           ORDER BY slot_at DESC LIMIT $3"#,
        org_id,
        schedule_id,
        limit
    )
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(PmRun::from).collect())
}

#[derive(sqlx::FromRow)]
struct PmRunRow {
    id: Uuid,
    org_id: Uuid,
    schedule_id: Uuid,
    slot_at: chrono::DateTime<chrono::Utc>,
    status: String,
    ticket_id: Option<Uuid>,
    detail: serde_json::Value,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<PmRunRow> for PmRun {
    fn from(r: PmRunRow) -> Self {
        PmRun {
            id: r.id,
            org_id: r.org_id,
            schedule_id: r.schedule_id,
            slot_at: r.slot_at,
            status: r.status,
            ticket_id: r.ticket_id,
            detail: r.detail,
            created_at: r.created_at,
        }
    }
}
