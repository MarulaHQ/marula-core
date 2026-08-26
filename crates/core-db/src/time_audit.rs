//! Missing-time gap audit persistence (TASKLIST 2.10): the (org, user, day)
//! audit ledger, org time-discipline settings, and the activity/logged
//! facts the audit compares.
//!
//! Purpose: compile-time-checked queries for the EOD nudge flow and gap
//! reports — activity detection (comments authored or timer sessions
//! overlapping the day window), logged-minute totals, the idempotent audit
//! upsert whose `nudge_sent_at IS NULL → set` transition is the once-per
//! user/day nudge claim, and the org settings knobs from migration 0017.
//!
//! Non-purpose: no gap *rule* (core-domain::time_audit owns the decision),
//! no notification rows (core-api inserts those in the same transaction as
//! the nudge claim), no scheduling (worker decides when runs are due).

use crate::{DbError, DbResult};
use chrono::{NaiveDate, Utc};
use sqlx::Postgres;
use uuid::Uuid;

/// Org-level time-discipline knobs (migration 0017).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgTimeSettings {
    pub eod_nudges_enabled: bool,
    pub eod_nudge_hour: i16,
    pub time_gap_min_minutes: i32,
}

pub async fn org_time_settings<'e, E>(executor: E, org_id: Uuid) -> DbResult<OrgTimeSettings>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT eod_nudges_enabled, eod_nudge_hour, time_gap_min_minutes
           FROM orgs WHERE id = $1"#,
        org_id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(OrgTimeSettings {
        eod_nudges_enabled: row.eod_nudges_enabled,
        eod_nudge_hour: row.eod_nudge_hour,
        time_gap_min_minutes: row.time_gap_min_minutes,
    })
}

pub async fn update_org_time_settings<'e, E>(
    executor: E,
    org_id: Uuid,
    settings: &OrgTimeSettings,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"UPDATE orgs SET eod_nudges_enabled = $2, eod_nudge_hour = $3,
                           time_gap_min_minutes = $4
           WHERE id = $1"#,
        org_id,
        settings.eod_nudges_enabled,
        settings.eod_nudge_hour,
        settings.time_gap_min_minutes
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// One org the worker's EOD loop may need to audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NudgeOrg {
    pub org_id: Uuid,
    pub eod_nudge_hour: i16,
}

/// All orgs with EOD nudges enabled. Cross-org by design: the worker runs
/// as table owner (RLS backstop not consulted), matching the relay.
pub async fn nudge_enabled_orgs<'e, E>(executor: E) -> DbResult<Vec<NudgeOrg>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id AS org_id, eod_nudge_hour FROM orgs
           WHERE eod_nudges_enabled = true"#,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| NudgeOrg {
            org_id: r.org_id,
            eod_nudge_hour: r.eod_nudge_hour,
        })
        .collect())
}

/// One member with the timezone their local day is computed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberTz {
    pub user_id: Uuid,
    pub timezone: String,
}

pub async fn org_member_timezones<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<MemberTz>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id AS user_id, timezone FROM users WHERE org_id = $1"#,
        org_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| MemberTz {
            user_id: r.user_id,
            timezone: r.timezone,
        })
        .collect())
}

/// Whether the user had tracked ticket activity inside the window:
/// authored a comment OR ran a timer session overlapping `[start, end)`.
/// Timer overlap uses the session span; segment detail lives in JSONB and
/// the session span is its superset (never misses activity).
pub async fn day_activity<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
    start: chrono::DateTime<Utc>,
    end: chrono::DateTime<Utc>,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT EXISTS(
               SELECT 1 FROM ticket_comments
                WHERE org_id = $1 AND author_id = $2
                  AND created_at >= $3 AND created_at < $4
           ) OR EXISTS(
               SELECT 1 FROM timer_sessions
                WHERE org_id = $1 AND user_id = $2
                  AND started_at < $4
                  AND COALESCE(stopped_at, now()) >= $3
           ) AS activity"#,
        org_id,
        user_id,
        start,
        end
    )
    .fetch_one(executor)
    .await?;
    // EXISTS OR EXISTS never yields NULL; sqlx just can't prove it.
    Ok(row.activity.unwrap_or(false))
}

/// The audit ledger row for one local day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GapAudit {
    pub id: Uuid,
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub day: NaiveDate,
    pub activity: bool,
    pub logged_minutes: i32,
    pub expected_minutes: i32,
    pub has_gap: bool,
    pub nudge_sent_at: Option<chrono::DateTime<Utc>>,
}

/// The evaluated facts for one (org, user, day) — the upsert payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GapAuditFacts {
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub day: NaiveDate,
    pub activity: bool,
    pub logged_minutes: i32,
    pub expected_minutes: i32,
    pub has_gap: bool,
}

/// Upserts the day's audit facts and returns the row. The caller decides
/// about nudging from `nudge_sent_at`/`has_gap` and claims the nudge via
/// [`claim_nudge`] in the same transaction.
pub async fn upsert_gap_audit<'e, E>(executor: E, facts: &GapAuditFacts) -> DbResult<GapAudit>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO time_gap_audits
             (id, org_id, user_id, day, activity, logged_minutes, expected_minutes, has_gap)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           ON CONFLICT (org_id, user_id, day) DO UPDATE SET
             activity = EXCLUDED.activity,
             logged_minutes = EXCLUDED.logged_minutes,
             expected_minutes = EXCLUDED.expected_minutes,
             has_gap = EXCLUDED.has_gap,
             updated_at = now()
           RETURNING id, org_id, user_id, day, activity, logged_minutes,
                     expected_minutes, has_gap, nudge_sent_at"#,
        Uuid::now_v7(),
        facts.org_id,
        facts.user_id,
        facts.day,
        facts.activity,
        facts.logged_minutes,
        facts.expected_minutes,
        facts.has_gap
    )
    .fetch_one(executor)
    .await?;
    Ok(GapAudit {
        id: row.id,
        org_id: row.org_id,
        user_id: row.user_id,
        day: row.day,
        activity: row.activity,
        logged_minutes: row.logged_minutes,
        expected_minutes: row.expected_minutes,
        has_gap: row.has_gap,
        nudge_sent_at: row.nudge_sent_at,
    })
}

/// The once-per user/day nudge claim: flips `nudge_sent_at` from NULL and
/// returns true only for the winner. Concurrent runs (worker retries, two
/// workers) see exactly one true.
pub async fn claim_nudge<'e, E>(executor: E, audit_id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let now = Utc::now();
    let res = sqlx::query!(
        r#"UPDATE time_gap_audits SET nudge_sent_at = $2
           WHERE id = $1 AND nudge_sent_at IS NULL"#,
        audit_id,
        now
    )
    .execute(executor)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Ledger rows for a user over an inclusive day range (report enrichment:
/// shows what was audited and when nudges fired).
pub async fn list_gap_audits<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
    from_day: NaiveDate,
    to_day: NaiveDate,
) -> DbResult<Vec<GapAudit>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, user_id, day, activity, logged_minutes,
                  expected_minutes, has_gap, nudge_sent_at
           FROM time_gap_audits
           WHERE org_id = $1 AND user_id = $2 AND day >= $3 AND day <= $4
           ORDER BY day DESC"#,
        org_id,
        user_id,
        from_day,
        to_day
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| GapAudit {
            id: r.id,
            org_id: r.org_id,
            user_id: r.user_id,
            day: r.day,
            activity: r.activity,
            logged_minutes: r.logged_minutes,
            expected_minutes: r.expected_minutes,
            has_gap: r.has_gap,
            nudge_sent_at: r.nudge_sent_at,
        })
        .collect())
}
