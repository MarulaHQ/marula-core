//! SLA persistence: policies, org-level calendars, escalation ladders,
//! the fire-once ledger, and evaluation-candidate reads for the worker.
//!
//! Purpose: compile-time-checked queries only. Calendar arithmetic and
//! ladder decisions live in core-domain (`entities::sla`, `sla_engine`);
//! event emission happens in core-api. Nothing here decides WHEN a step
//! fires — it stores what fired so no decision can repeat.
//!
//! Non-purpose: business rules, notification creation, HTTP.

use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use core_domain::entities::{
    BusinessCalendar, EscalationStep, SlaCalendar, SlaPolicy, SlaTargets, TicketSlaState,
};
use sqlx::Postgres;
use std::collections::HashMap;
use uuid::Uuid;

pub async fn get_default_policy<'e, E>(executor: E, org_id: Uuid) -> DbResult<SlaPolicy>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, name, is_default, targets, calendar
           FROM sla_policies WHERE org_id = $1 AND is_default = true LIMIT 1"#,
        org_id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    let targets: HashMap<String, SlaTargets> =
        serde_json::from_value(row.targets).map_err(|e| {
            DbError::Sql(sqlx::Error::ColumnDecode {
                index: "targets".into(),
                source: Box::new(e),
            })
        })?;
    let by_priority = targets
        .into_iter()
        .filter_map(|(k, v)| k.parse::<core_domain::Priority>().ok().map(|p| (p, v)))
        .collect();
    let calendar: BusinessCalendar = serde_json::from_value(row.calendar).map_err(|e| {
        DbError::Sql(sqlx::Error::ColumnDecode {
            index: "calendar".into(),
            source: Box::new(e),
        })
    })?;
    Ok(SlaPolicy {
        id: row.id,
        org_id: row.org_id,
        name: row.name,
        is_default: row.is_default,
        targets: by_priority,
        calendar,
    })
}

pub async fn upsert_ticket_state<'e, E>(executor: E, s: &TicketSlaState) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO ticket_sla_state
             (ticket_id, org_id, policy_id, first_response_due, resolution_due,
              first_responded_at, paused_business_secs, paused_since, breached, updated_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now())
           ON CONFLICT (ticket_id) DO UPDATE SET
             policy_id = $3, first_response_due = $4, resolution_due = $5,
             first_responded_at = $6, paused_business_secs = $7,
             paused_since = $8, breached = $9, updated_at = now()"#,
        s.ticket_id,
        s.org_id,
        s.policy_id,
        s.first_response_due,
        s.resolution_due,
        s.first_responded_at,
        s.paused_business_secs,
        s.paused_since,
        s.breached,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_ticket_state<'e, E>(
    executor: E,
    ticket_id: Uuid,
) -> DbResult<Option<TicketSlaState>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT ticket_id, org_id, policy_id, first_response_due, resolution_due,
                  first_responded_at, paused_business_secs, paused_since, breached
            FROM ticket_sla_state WHERE ticket_id = $1"#,
        ticket_id
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|row| TicketSlaState {
        org_id: row.org_id,
        ticket_id: row.ticket_id,
        policy_id: row.policy_id,
        first_response_due: row.first_response_due,
        resolution_due: row.resolution_due,
        first_responded_at: row.first_responded_at,
        paused_business_secs: row.paused_business_secs,
        paused_since: row.paused_since,
        breached: row.breached,
    }))
}

pub async fn mark_first_response<'e, E>(
    executor: E,
    ticket_id: Uuid,
    at: DateTime<Utc>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"UPDATE ticket_sla_state
           SET first_responded_at = COALESCE(first_responded_at, $2), updated_at = now()
           WHERE ticket_id = $1"#,
        ticket_id,
        at
    )
    .execute(executor)
    .await?;
    Ok(())
}

// --- org-level calendars (0015) ---------------------------------------------

/// Inserts a calendar row; `calendar.name` is mirrored into the row name.
pub async fn insert_calendar<'e, E>(executor: E, cal: &SlaCalendar) -> DbResult<SlaCalendar>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let body = serde_json::to_value(&cal.calendar).map_err(|e| json_err("calendar", e))?;
    let row = sqlx::query!(
        r#"INSERT INTO sla_calendars
             (id, org_id, name, timezone, windows, holidays, maintenance)
           VALUES ($1, $2, $3, $4, $5,
                   $6::jsonb, COALESCE($7::jsonb, '[]'::jsonb))
           RETURNING id, org_id, name, timezone, windows, holidays, maintenance"#,
        cal.id,
        cal.org_id,
        cal.name,
        cal.calendar.timezone,
        serde_json::to_value(cal.calendar.windows.clone()).map_err(|e| json_err("windows", e))?,
        serde_json::to_value(cal.calendar.holidays.clone()).map_err(|e| json_err("holidays", e))?,
        body.get("maintenance").cloned(),
    )
    .fetch_one(executor)
    .await?;
    calendar_from_row(
        row.id,
        row.org_id,
        row.name,
        &row.timezone,
        &row.windows,
        &row.holidays,
        Some(row.maintenance),
    )
}

pub async fn list_calendars<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<SlaCalendar>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, name, timezone, windows, holidays, maintenance
           FROM sla_calendars WHERE org_id = $1 ORDER BY name"#,
        org_id
    )
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(|r| {
            calendar_from_row(
                r.id,
                r.org_id,
                r.name,
                &r.timezone,
                &r.windows,
                &r.holidays,
                Some(r.maintenance),
            )
        })
        .collect()
}

pub async fn get_calendar<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<SlaCalendar>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, name, timezone, windows, holidays, maintenance
           FROM sla_calendars WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    calendar_from_row(
        row.id,
        row.org_id,
        row.name,
        &row.timezone,
        &row.windows,
        &row.holidays,
        Some(row.maintenance),
    )
}

pub struct CalendarPatch<'a> {
    pub name: Option<&'a str>,
    pub timezone: Option<&'a str>,
    pub windows: Option<serde_json::Value>,
    pub holidays: Option<serde_json::Value>,
    pub maintenance: Option<serde_json::Value>,
}

#[allow(clippy::too_many_arguments)]
pub async fn update_calendar<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    patch: &CalendarPatch<'_>,
) -> DbResult<SlaCalendar>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"UPDATE sla_calendars SET
             name = COALESCE($3, name),
             timezone = COALESCE($4, timezone),
             windows = COALESCE($5, windows),
             holidays = COALESCE($6, holidays),
             maintenance = COALESCE($7, maintenance),
             updated_at = now()
           WHERE org_id = $1 AND id = $2
           RETURNING id, org_id, name, timezone, windows, holidays, maintenance"#,
        org_id,
        id,
        patch.name,
        patch.timezone,
        patch.windows,
        patch.holidays,
        patch.maintenance,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    calendar_from_row(
        row.id,
        row.org_id,
        row.name,
        &row.timezone,
        &row.windows,
        &row.holidays,
        Some(row.maintenance),
    )
}

/// Deleting a calendar detaches policies (FK ON DELETE SET NULL); their
/// embedded calendars keep governing — never a data loss.
pub async fn delete_calendar<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        "DELETE FROM sla_calendars WHERE org_id = $1 AND id = $2",
        org_id,
        id
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

// --- policies with attachment + ladders --------------------------------------

/// Policy projection including the calendar attachment.
#[derive(Debug, Clone)]
pub struct SlaPolicyRecord {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub is_default: bool,
    /// Targets document keyed by priority string.
    pub targets: serde_json::Value,
    /// Embedded fallback calendar; governs while `calendar_id` is NULL.
    pub calendar: BusinessCalendar,
    pub calendar_id: Option<Uuid>,
}

pub async fn list_policy_records<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<SlaPolicyRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT p.id, p.org_id, p.name, p.is_default, p.targets,
                  p.calendar AS "calendar: serde_json::Value",
                  p.calendar_id
           FROM sla_policies p WHERE p.org_id = $1 ORDER BY p.name"#,
        org_id
    )
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(|r| {
            policy_record_from(
                r.id,
                r.org_id,
                r.name,
                r.is_default,
                r.targets,
                r.calendar,
                r.calendar_id,
            )
        })
        .collect()
}

pub async fn get_policy_record<'e, E>(
    executor: E,
    org_id: Uuid,
    policy_id: Uuid,
) -> DbResult<SlaPolicyRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT p.id, p.org_id, p.name, p.is_default,
                  p.targets AS "targets: serde_json::Value",
                  p.calendar AS "calendar: serde_json::Value", p.calendar_id
           FROM sla_policies p WHERE p.org_id = $1 AND p.id = $2"#,
        org_id,
        policy_id,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    policy_record_from(
        row.id,
        row.org_id,
        row.name,
        row.is_default,
        row.targets,
        row.calendar,
        row.calendar_id,
    )
}

/// Resolves a policy's governing calendar: attached row wins; the embedded
/// document is the pre-0015 fallback (defaults-first compatibility).
/// Executor-generic so callers may run it inside an org-scoped transaction
/// (RLS then applies) or on a worker's owner connection.
pub async fn resolve_policy_calendar<'e, E>(
    executor: E,
    record: &SlaPolicyRecord,
) -> DbResult<BusinessCalendar>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    match record.calendar_id {
        Some(id) => {
            let row = sqlx::query!(
                r#"SELECT id, org_id, name, timezone, windows, holidays, maintenance
                   FROM sla_calendars WHERE id = $1"#,
                id
            )
            .fetch_optional(executor)
            .await?
            .ok_or(DbError::NotFound)?;
            Ok(calendar_from_row(
                row.id,
                row.org_id,
                row.name,
                &row.timezone,
                &row.windows,
                &row.holidays,
                Some(row.maintenance),
            )?
            .calendar)
        }
        None => Ok(record.calendar.clone()),
    }
}

pub async fn set_policy_calendar<'e, E>(
    executor: E,
    org_id: Uuid,
    policy_id: Uuid,
    calendar_id: Option<Uuid>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"UPDATE sla_policies SET calendar_id = $3 WHERE org_id = $1 AND id = $2"#,
        org_id,
        policy_id,
        calendar_id,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// One ladder step as written through [`replace_ladder`].
#[derive(Debug, Clone)]
pub struct LadderStepInput {
    pub step_index: i32,
    pub trigger_kind: String,
    pub at_pct: i64,
    pub notify_user_ids: Vec<Uuid>,
    pub enabled: bool,
}

/// Replaces a policy's whole ladder in one transaction-scoped call. Step
/// ids are regenerated on every replace — re-firing against NEW steps is
/// possible by design (an edited ladder is a new contract).
pub async fn replace_ladder(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    policy_id: Uuid,
    steps: &[LadderStepInput],
) -> DbResult<Vec<EscalationStep>> {
    sqlx::query!(
        "DELETE FROM sla_escalation_steps WHERE org_id = $1 AND policy_id = $2",
        org_id,
        policy_id
    )
    .execute(&mut *conn)
    .await?;
    let mut out = Vec::with_capacity(steps.len());
    for s in steps {
        let id = Uuid::now_v7();
        let notify =
            serde_json::to_value(&s.notify_user_ids).map_err(|e| json_err("notify_user_ids", e))?;
        let row = sqlx::query!(
            r#"INSERT INTO sla_escalation_steps
                 (id, org_id, policy_id, step_index, trigger_kind, at_pct, notify_user_ids, enabled)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               RETURNING id, org_id, policy_id, step_index, trigger_kind, at_pct, notify_user_ids, enabled"#,
            id,
            org_id,
            policy_id,
            s.step_index,
            s.trigger_kind,
            i32::try_from(s.at_pct).map_err(|_| DbError::NotFound)?,
            notify,
            s.enabled,
        )
        .fetch_one(&mut *conn)
        .await?;
        out.push(EscalationStep {
            id: row.id,
            org_id: row.org_id,
            policy_id: row.policy_id,
            step_index: row.step_index,
            trigger_kind: row.trigger_kind,
            at_pct: i64::from(row.at_pct),
            notify_user_ids: serde_json::from_value(row.notify_user_ids)
                .map_err(|e| json_err("notify_user_ids", e))?,
            enabled: row.enabled,
        });
    }
    Ok(out)
}

pub async fn list_steps_for_policies<'e, E>(
    executor: E,
    policy_ids: &[Uuid],
) -> DbResult<Vec<EscalationStep>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, policy_id, step_index, trigger_kind, at_pct, notify_user_ids, enabled
           FROM sla_escalation_steps
           WHERE enabled AND policy_id = ANY($1)
           ORDER BY policy_id, at_pct, step_index"#,
        policy_ids,
    )
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(EscalationStep {
                id: row.id,
                org_id: row.org_id,
                policy_id: row.policy_id,
                step_index: row.step_index,
                trigger_kind: row.trigger_kind,
                at_pct: i64::from(row.at_pct),
                notify_user_ids: serde_json::from_value(row.notify_user_ids)
                    .map_err(|e| json_err("notify_user_ids", e))?,
                enabled: row.enabled,
            })
        })
        .collect()
}

// --- fire-once ledger ---------------------------------------------------------

/// Records a fired (ticket, step, target). Returns false when the pair has
/// ALREADY fired — the PK is the idempotency floor under racing workers.
pub async fn try_record_fired<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
    step_id: Uuid,
    target: &str,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"INSERT INTO sla_fired_escalations (org_id, ticket_id, step_id, target)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (ticket_id, step_id, target) DO NOTHING"#,
        org_id,
        ticket_id,
        step_id,
        target,
    )
    .execute(executor)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Flips a clock to breached (first write wins via the boolean guard).
pub async fn mark_breached<'e, E>(executor: E, ticket_id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"UPDATE ticket_sla_state SET breached = true, updated_at = now()
           WHERE ticket_id = $1 AND breached = false"#,
        ticket_id,
    )
    .execute(executor)
    .await?;
    Ok(res.rows_affected() > 0)
}

// --- worker evaluation reads ---------------------------------------------------

/// Everything one sweep needs about one open ticket's clocks, with its
/// governing calendar already resolved (attached row or embedded doc).
pub struct SlaEvaluationCandidate {
    pub org_id: Uuid,
    pub ticket_id: Uuid,
    /// Human number ("MER-42") for notification titles.
    pub number: String,
    pub policy_id: Uuid,
    pub priority: String,
    pub category: String,
    pub created_at: DateTime<Utc>,
    pub state: TicketSlaState,
    pub calendar: BusinessCalendar,
    /// Raw targets document keyed by priority ("p1".."p4").
    pub targets: serde_json::Value,
}

pub async fn list_evaluation_candidates<'e, E>(
    executor: E,
    limit: i64,
) -> DbResult<Vec<SlaEvaluationCandidate>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT s.org_id, s.ticket_id, s.policy_id,
                  t.number, t.priority, t.category, t.created_at,
                  s.first_response_due, s.resolution_due, s.first_responded_at,
                  s.paused_business_secs, s.paused_since, s.breached,
                  p.targets AS "targets: serde_json::Value",
                  p.calendar AS "embedded_calendar: serde_json::Value",
                  p.calendar_id AS "calendar_id: Option<Uuid>",
                  c.id AS "cal_id: Option<Uuid>",
                  c.timezone AS "cal_tz: Option<String>",
                  c.windows AS "cal_windows: Option<serde_json::Value>",
                  c.holidays AS "cal_holidays: Option<serde_json::Value>",
                  c.maintenance AS "cal_maintenance: Option<serde_json::Value>",
                  c.name AS "cal_name: Option<String>"
           FROM ticket_sla_state s
           JOIN tickets t ON t.id = s.ticket_id AND t.category <> 'closed'
           JOIN sla_policies p ON p.id = s.policy_id
           LEFT JOIN sla_calendars c ON c.id = p.calendar_id
           ORDER BY s.resolution_due ASC NULLS LAST
           LIMIT $1"#,
        limit,
    )
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(|r| {
            // Attached row wins; a vanished row (deleted between write and
            // read) falls back to the embedded document rather than
            // silently evaluating against an empty calendar.
            let calendar = if let (Some(_), Some(cal_id), Some(cal_name), Some(cal_tz)) =
                (r.calendar_id.flatten(), r.cal_id, r.cal_name, r.cal_tz)
            {
                calendar_from_row(
                    cal_id,
                    r.org_id,
                    cal_name,
                    &cal_tz,
                    &r.cal_windows.unwrap_or_else(|| serde_json::json!([])),
                    &r.cal_holidays.unwrap_or_else(|| serde_json::json!([])),
                    Some(r.cal_maintenance.unwrap_or_else(|| serde_json::json!([]))),
                )?
                .calendar
            } else {
                parse_business_calendar(&r.embedded_calendar)?
            };
            Ok(SlaEvaluationCandidate {
                org_id: r.org_id,
                ticket_id: r.ticket_id,
                number: r.number,
                policy_id: r.policy_id,
                priority: r.priority,
                category: r.category,
                created_at: r.created_at,
                state: TicketSlaState {
                    org_id: r.org_id,
                    ticket_id: r.ticket_id,
                    policy_id: r.policy_id,
                    first_response_due: r.first_response_due,
                    resolution_due: r.resolution_due,
                    first_responded_at: r.first_responded_at,
                    paused_business_secs: r.paused_business_secs,
                    paused_since: r.paused_since,
                    breached: r.breached,
                },
                calendar,
                targets: r.targets,
            })
        })
        .collect()
}

/// Fired (step, target) pairs for a batch of tickets.
pub async fn list_fired_for_tickets<'e, E>(
    executor: E,
    ticket_ids: &[Uuid],
) -> DbResult<Vec<(Uuid, Uuid, String)>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        "SELECT ticket_id, step_id, target FROM sla_fired_escalations WHERE ticket_id = ANY($1)",
        ticket_ids,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.ticket_id, r.step_id, r.target))
        .collect())
}

// --- helpers --------------------------------------------------------------------

fn json_err(field: &str, e: serde_json::Error) -> DbError {
    DbError::Sql(sqlx::Error::ColumnDecode {
        index: field.into(),
        source: Box::new(e),
    })
}

fn parse_business_calendar(v: &serde_json::Value) -> DbResult<BusinessCalendar> {
    serde_json::from_value(v.clone()).map_err(|e| json_err("calendar", e))
}

/// Assembles the domain calendar row from flattened storage columns.
#[allow(clippy::too_many_arguments)]
fn calendar_from_row(
    id: Uuid,
    org_id: Uuid,
    name: String,
    timezone: &str,
    windows: &serde_json::Value,
    holidays: &serde_json::Value,
    maintenance: Option<serde_json::Value>,
) -> DbResult<SlaCalendar> {
    let windows: [Option<(String, String)>; 7] =
        serde_json::from_value(windows.clone()).map_err(|e| json_err("windows", e))?;
    let holidays: Vec<String> =
        serde_json::from_value(holidays.clone()).map_err(|e| json_err("holidays", e))?;
    let maintenance: Vec<core_domain::entities::MaintenanceWindow> = match maintenance {
        Some(v) => serde_json::from_value(v).map_err(|e| json_err("maintenance", e))?,
        None => Vec::new(),
    };
    Ok(SlaCalendar {
        id,
        org_id,
        name: name.clone(),
        calendar: BusinessCalendar {
            name,
            timezone: timezone.to_string(),
            windows,
            holidays,
            maintenance,
        },
    })
}

fn policy_record_from(
    id: Uuid,
    org_id: Uuid,
    name: String,
    is_default: bool,
    targets: serde_json::Value,
    calendar: serde_json::Value,
    calendar_id: Option<Uuid>,
) -> DbResult<SlaPolicyRecord> {
    let calendar: BusinessCalendar =
        serde_json::from_value(calendar).map_err(|e| json_err("calendar", e))?;
    Ok(SlaPolicyRecord {
        id,
        org_id,
        name,
        is_default,
        targets,
        calendar,
        calendar_id,
    })
}
