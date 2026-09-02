//! Scheduled report delivery persistence (TASKLIST R4).
//!
//! Purpose: CRUD for `report_schedules` rows (migration 0038) plus the
//! worker-facing claim sweep and the append-only `report_deliveries` run
//! ledger. This module is deliberately thin: no cadence math (the worker
//! owns the clock and the next-slot computation), no preference decisions
//! (core-domain's chain decides the email leg per recipient), no
//! authorization (core-api owns RBAC; RLS keeps tenancy absolute).
//!
//! Claiming: [`claim_due`] selects enabled schedules whose materialized
//! `next_run_at` has passed `FOR UPDATE SKIP LOCKED` — concurrent workers
//! (or overlapping poll cycles) can never double-claim one slot. The
//! caller advances `next_run_at` per row inside the same transaction,
//! making a claim crash-safe: a worker that dies after claiming simply
//! loses that slot's run (at-most-once per slot; the ledger records what
//! actually ran) rather than re-firing it forever.
//!
//! Non-purpose: no execution (the worker compiles specs on demand via
//! core-reports), no notification creation (the delivery is email via the
//! MailTransport seam, not an inbox row), no schedule events (core-api's
//! CRUD surface emits those through the outbox).

use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use sqlx::Postgres;
use uuid::Uuid;

/// One delivery schedule as persisted. Cadence columns mirror migration
/// 0038 exactly (`day_of_week`/`day_of_month` are NULL unless their
/// cadence is weekly/monthly — the DB CHECK pins the shape, this struct
/// just carries it). `FromRow` backs the runtime keyset-listing queries
/// (the macros cover the static ones).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct ReportScheduleRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub report_definition_id: Uuid,
    pub name: String,
    pub recipients: Vec<Uuid>,
    /// `daily` | `weekly` | `monthly` (the stored slug).
    pub cadence: String,
    pub time_of_day: chrono::NaiveTime,
    pub day_of_week: Option<i16>,
    pub day_of_month: Option<i16>,
    /// IANA zone name (Rust-validated at write time).
    pub timezone: String,
    pub window_days: i32,
    pub enabled: bool,
    pub next_run_at: DateTime<Utc>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// All fields of an insert or a full-row update; updates overwrite every
/// member (the caller re-validates and recomputes `next_run_at` first).
#[derive(Debug, Clone)]
pub struct ScheduleFields {
    pub report_definition_id: Uuid,
    pub name: String,
    pub recipients: Vec<Uuid>,
    pub cadence: String,
    pub time_of_day: chrono::NaiveTime,
    pub day_of_week: Option<i16>,
    pub day_of_month: Option<i16>,
    pub timezone: String,
    pub window_days: i32,
    pub enabled: bool,
    pub next_run_at: DateTime<Utc>,
}

const SCHEDULE_COLUMNS: &str = "id, org_id, report_definition_id, name, recipients, cadence, \
     time_of_day, day_of_week, day_of_month, timezone, window_days, enabled, next_run_at, \
     last_run_at, last_error, created_by, created_at, updated_at";

/// Inserts one schedule. The `(org_id, name)` UNIQUE constraint backs
/// per-org name uniqueness; the mapped conflict surfaces to clients as a
/// typed 409 through the central error layer.
pub async fn insert(
    conn: &mut sqlx::PgConnection,
    id: Uuid,
    org_id: Uuid,
    created_by: Uuid,
    fields: &ScheduleFields,
) -> DbResult<ReportScheduleRow> {
    let row = sqlx::query_as!(
        ReportScheduleRow,
        r#"INSERT INTO report_schedules
             (id, org_id, report_definition_id, name, recipients, cadence, time_of_day,
              day_of_week, day_of_month, timezone, window_days, enabled, next_run_at, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
           RETURNING id, org_id, report_definition_id, name, recipients, cadence,
             time_of_day, day_of_week, day_of_month, timezone, window_days, enabled,
             next_run_at, last_run_at, last_error, created_by, created_at, updated_at"#,
        id,
        org_id,
        fields.report_definition_id,
        fields.name,
        &fields.recipients,
        fields.cadence,
        fields.time_of_day,
        fields.day_of_week,
        fields.day_of_month,
        fields.timezone,
        fields.window_days,
        fields.enabled,
        fields.next_run_at,
        created_by,
    )
    .fetch_one(conn)
    .await?;
    Ok(row)
}

/// Fetches one schedule by id within the org. A miss under RLS (another
/// org's row, deleted, or never existed) is deliberately indistinguishable.
pub async fn get(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<ReportScheduleRow> {
    let sql =
        format!("SELECT {SCHEDULE_COLUMNS} FROM report_schedules WHERE org_id = $1 AND id = $2");
    sqlx::query_as::<_, ReportScheduleRow>(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)
}

/// Cursor-paged ADMIN listing, deterministic `(name, id)` keyset order.
/// `after_name`/`after_id` carry the previous page's last row (both None
/// for the first page). The next-run preview rides on the row itself
/// (`next_run_at` is materialized).
pub async fn list_page(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    after_name: Option<&str>,
    after_id: Option<Uuid>,
    limit: i64,
) -> DbResult<Vec<ReportScheduleRow>> {
    let sql = format!(
        "SELECT {SCHEDULE_COLUMNS} FROM report_schedules
         WHERE org_id = $1 AND (name, id) > ($2, $3)
         ORDER BY name, id
         LIMIT $4"
    );
    let rows = sqlx::query_as::<_, ReportScheduleRow>(&sql)
        .bind(org_id)
        .bind(after_name.unwrap_or(""))
        .bind(after_id.unwrap_or(Uuid::nil()))
        .bind(limit)
        .fetch_all(executor)
        .await?;
    Ok(rows)
}

/// Full-row update; `updated_at` advances server-side. Returns NotFound
/// when the row left the org scope (mutation callers map that to the
/// typed `rls_sparse` envelope code).
pub async fn update(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    fields: &ScheduleFields,
) -> DbResult<ReportScheduleRow> {
    let row = sqlx::query_as!(
        ReportScheduleRow,
        r#"UPDATE report_schedules
           SET report_definition_id = $3, name = $4, recipients = $5, cadence = $6,
               time_of_day = $7, day_of_week = $8, day_of_month = $9, timezone = $10,
               window_days = $11, enabled = $12, next_run_at = $13, updated_at = now()
           WHERE org_id = $1 AND id = $2
           RETURNING id, org_id, report_definition_id, name, recipients, cadence,
             time_of_day, day_of_week, day_of_month, timezone, window_days, enabled,
             next_run_at, last_run_at, last_error, created_by, created_at, updated_at"#,
        org_id,
        id,
        fields.report_definition_id,
        fields.name,
        &fields.recipients,
        fields.cadence,
        fields.time_of_day,
        fields.day_of_week,
        fields.day_of_month,
        fields.timezone,
        fields.window_days,
        fields.enabled,
        fields.next_run_at,
    )
    .fetch_optional(conn)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row)
}

/// Deletes one schedule (its ledger rows cascade). Returns NotFound when
/// nothing in scope matched.
pub async fn delete(conn: &mut sqlx::PgConnection, org_id: Uuid, id: Uuid) -> DbResult<()> {
    let result = sqlx::query!(
        "DELETE FROM report_schedules WHERE org_id = $1 AND id = $2",
        org_id,
        id
    )
    .execute(conn)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

// --- worker-facing claim sweep -------------------------------------------------

/// One claimed slot: the schedule row snapshot plus the instant that was
/// due (the caller renders/records against `due_at`, not the advanced
/// `next_run_at`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimedSchedule {
    pub row: ReportScheduleRow,
    /// The slot that came due — the delivery window ends here.
    pub due_at: DateTime<Utc>,
}

/// Claims every enabled schedule whose materialized `next_run_at` is at
/// or before `now`, in due order, with `FOR UPDATE SKIP LOCKED` so two
/// workers never take the same row. The caller MUST advance each row's
/// `next_run_at` (via [`advance_after_run`]) before committing — a claimed
/// but unadvanced row would re-fire every poll until it does.
///
/// `now` is injected rather than read from the DB so the deterministic
/// delivery harness can drive the sweep over simulated weeks.
pub async fn claim_due(
    conn: &mut sqlx::PgConnection,
    now: DateTime<Utc>,
    batch_limit: i64,
) -> DbResult<Vec<ClaimedSchedule>> {
    let rows = sqlx::query_as!(
        ReportScheduleRow,
        r#"SELECT id, org_id, report_definition_id, name, recipients, cadence,
                  time_of_day, day_of_week, day_of_month, timezone, window_days, enabled,
                  next_run_at, last_run_at, last_error, created_by, created_at, updated_at
           FROM report_schedules
           WHERE enabled AND next_run_at <= $1
           ORDER BY next_run_at, id
           LIMIT $2
           FOR UPDATE SKIP LOCKED"#,
        now,
        batch_limit,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let due_at = row.next_run_at;
            ClaimedSchedule { row, due_at }
        })
        .collect())
}

/// Writes back the next slot instant (computed by the worker's cadence
/// math, strictly after the claimed `due_at`) for one claimed row. The
/// `next_run_at = due_at` compare-and-set makes a double-advance inert:
/// only the claimant that still sees the due slot wins. Returns NotFound
/// when the schedule vanished or was already advanced (deleted
/// concurrently / another claimant won).
pub async fn advance_after_run(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    due_at: DateTime<Utc>,
    next_run_at: DateTime<Utc>,
) -> DbResult<()> {
    let result = sqlx::query!(
        r#"UPDATE report_schedules
           SET next_run_at = $3, updated_at = now()
           WHERE org_id = $1 AND id = $2 AND next_run_at = $4"#,
        org_id,
        id,
        next_run_at,
        due_at,
    )
    .execute(conn)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Stamps the visible run outcome on the schedule after delivery: the
/// slot that ran and, on failure, the reason. Called in a separate
/// transaction AFTER the claim/advance committed — a crash between the
/// two costs only the `last_*` cosmetics, never the schedule state.
pub async fn set_run_outcome(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    ran_at: DateTime<Utc>,
    error: Option<&str>,
) -> DbResult<()> {
    let result = sqlx::query!(
        r#"UPDATE report_schedules
           SET last_run_at = $3, last_error = $4, updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
        ran_at,
        error,
    )
    .execute(conn)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

// --- delivery ledger -----------------------------------------------------------

/// One append-only run record. `detail` carries the per-recipient outcome
/// counters (delivered / disabled / quiet_deferred / failed / skipped).
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryRecord {
    pub id: Uuid,
    pub org_id: Uuid,
    pub schedule_id: Uuid,
    pub slot_at: DateTime<Utc>,
    /// `delivered` | `partial` | `failed` | `skipped`.
    pub status: String,
    pub detail: serde_json::Value,
}

/// Appends one run to the ledger. Never updated after write — retries of
/// the same slot append new rows (at-least-once visibility).
pub async fn record_delivery(
    conn: &mut sqlx::PgConnection,
    record: &DeliveryRecord,
) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO report_deliveries (id, org_id, schedule_id, slot_at, status, detail)
           VALUES ($1, $2, $3, $4, $5, $6)"#,
        record.id,
        record.org_id,
        record.schedule_id,
        record.slot_at,
        record.status,
        record.detail,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Ledger rows for one schedule, newest slot first — the admin listing's
/// recent-run feed.
pub async fn list_deliveries(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    schedule_id: Uuid,
    limit: i64,
) -> DbResult<Vec<(DateTime<Utc>, String, serde_json::Value)>> {
    let rows = sqlx::query!(
        r#"SELECT slot_at, status, detail
           FROM report_deliveries
           WHERE org_id = $1 AND schedule_id = $2
           ORDER BY slot_at DESC, id DESC
           LIMIT $3"#,
        org_id,
        schedule_id,
        limit,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.slot_at, r.status, r.detail))
        .collect())
}
