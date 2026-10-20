//! Coaching scorecard aggregation (Wave 14 lane 3, R5).
//!
//! Purpose: per-tech / service-desk scorecards computed over the
//! versioned read-model views (`ops.v1_sla_outcomes`,
//! `ops.v1_first_response`, `ops.v1_tickets`, `ops.v1_time_entries`) —
//! the event-trail-derived read model from 0026, approval-aware since
//! 0073. NOTHING here reads physical tables: scorecards ride the same
//! controlled seam as the reporting engine (ADR 0001), so view upgrades
//! stay the single compatibility point.
//!
//! Attribution semantics (documented contract):
//!   * SLA attainment — attributed to the ticket's CURRENT assignee,
//!     windowed on `ticket_created_at` (the SLA clock's anchor).
//!   * Response times — same attribution/window as SLA (first response
//!     is a per-ticket property).
//!   * Resolution times — attributed to the resolver of record
//!     (`assignee_id` at resolution), windowed on `resolved_at`
//!     (resolution THROUGHPUT in the window).
//!   * Workload mix — attributed to the entry author, windowed on
//!     `started_at`, and computed over APPROVED entries ONLY
//!     (`approval_status = 'approved'`): drafts and rejected entries are
//!     never scorecard truth (Wave 14 lane-1 dependency).
//!
//! CSAT: the surveys-on-resolution source does not exist yet (the private build tracker
//! R6 follow-up); the scorecard payload reserves the slot and reports it
//! as `null` until that source lands. This module deliberately has no
//! CSAT query to invent data.
//!
//! Non-purpose: no materialized marts (acknowledged scale limit, ADR
//! 0001), no gamification math (streaks are opt-in presentation, computed
//! client-side over these windows), no write path (the only scorecard
//! state is `scorecard_settings`, see [`settings`]).

use crate::DbResult;
use sqlx::Postgres;
use uuid::Uuid;

/// One staff member's scorecard over a window. Counts are `0` (not NULL)
/// when the member has no activity of that kind in the window; medians /
/// p95s are `None` when the denominator is empty.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScorecardRow {
    pub user_id: Uuid,
    pub display_name: String,
    // SLA attainment (ops.v1_sla_outcomes; three-valued metrics counted
    // only where both target and actual exist).
    pub response_total: i64,
    pub response_met: i64,
    pub resolution_total: i64,
    pub resolution_met: i64,
    pub breached: i64,
    // Response latency (ops.v1_first_response; staff-authored first
    // comments, seconds).
    pub responses: i64,
    pub median_response_secs: Option<f64>,
    pub p95_response_secs: Option<f64>,
    // Resolution latency (ops.v1_tickets resolved in window, seconds).
    pub resolutions: i64,
    pub median_resolution_secs: Option<f64>,
    pub p95_resolution_secs: Option<f64>,
    // Workload mix (ops.v1_time_entries, APPROVED entries only).
    pub approved_entries: i64,
    pub approved_minutes: i64,
    pub approved_billable_minutes: i64,
}

/// APPROVED-minutes mix per work type for the requested users. `work_type`
/// is `None` for unclassified entries — surfaced as `null` in the payload.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkTypeMixRow {
    pub user_id: Uuid,
    pub work_type: Option<String>,
    pub minutes: i64,
}

/// Aggregates one scorecard row per requested user over the half-open
/// window `[from, to)`. Reads ONLY `ops.v1_*` views.
pub async fn scorecard_for_users<'e, E>(
    executor: E,
    org_id: Uuid,
    user_ids: &[Uuid],
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> DbResult<Vec<ScorecardRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"WITH sla AS (
             SELECT o.assignee_id AS user_id,
                    COUNT(*) FILTER (WHERE o.response_met IS NOT NULL)::bigint AS response_total,
                    COUNT(*) FILTER (WHERE o.response_met IS TRUE)::bigint AS response_met,
                    COUNT(*) FILTER (WHERE o.resolution_met IS NOT NULL)::bigint AS resolution_total,
                    COUNT(*) FILTER (WHERE o.resolution_met IS TRUE)::bigint AS resolution_met,
                    COUNT(*) FILTER (WHERE o.breached)::bigint AS breached
             FROM ops.v1_sla_outcomes o
             WHERE o.org_id = $1
               AND o.assignee_id = ANY($2)
               AND o.ticket_created_at >= $3
               AND o.ticket_created_at < $4
             GROUP BY o.assignee_id
           ),
           resp AS (
             SELECT f.assignee_id AS user_id,
                    COUNT(*)::bigint AS responses,
                    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY f.first_response_secs) AS median_secs,
                    PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY f.first_response_secs) AS p95_secs
             FROM ops.v1_first_response f
             WHERE f.org_id = $1
               AND f.assignee_id = ANY($2)
               AND f.created_at >= $3
               AND f.created_at < $4
               AND f.first_response_secs IS NOT NULL
             GROUP BY f.assignee_id
           ),
           res AS (
             SELECT t.assignee_id AS user_id,
                    COUNT(*)::bigint AS resolutions,
                    PERCENTILE_CONT(0.5)
                      WITHIN GROUP (ORDER BY EXTRACT(epoch FROM (t.resolved_at - t.created_at))) AS median_secs,
                    PERCENTILE_CONT(0.95)
                      WITHIN GROUP (ORDER BY EXTRACT(epoch FROM (t.resolved_at - t.created_at))) AS p95_secs
             FROM ops.v1_tickets t
             WHERE t.org_id = $1
               AND t.assignee_id = ANY($2)
               AND t.resolved_at IS NOT NULL
               AND t.resolved_at >= $3
               AND t.resolved_at < $4
             GROUP BY t.assignee_id
           ),
           wk AS (
             SELECT e.user_id,
                    COUNT(*)::bigint AS approved_entries,
                    COALESCE(SUM(e.minutes), 0)::bigint AS approved_minutes,
                    COALESCE(SUM(e.minutes) FILTER (WHERE e.billable), 0)::bigint AS approved_billable_minutes
             FROM ops.v1_time_entries e
             WHERE e.org_id = $1
               AND e.user_id = ANY($2)
               AND e.started_at >= $3
               AND e.started_at < $4
               AND e.approval_status = 'approved'
             GROUP BY e.user_id
           )
         SELECT u.id AS user_id,
                u.display_name,
                COALESCE(s.response_total, 0)::bigint AS response_total,
                COALESCE(s.response_met, 0)::bigint AS response_met,
                COALESCE(s.resolution_total, 0)::bigint AS resolution_total,
                COALESCE(s.resolution_met, 0)::bigint AS resolution_met,
                COALESCE(s.breached, 0)::bigint AS breached,
                COALESCE(r.responses, 0)::bigint AS responses,
                r.median_secs,
                r.p95_secs,
                COALESCE(res_.resolutions, 0)::bigint AS resolutions,
                res_.median_secs AS median_resolution_secs,
                res_.p95_secs AS p95_resolution_secs,
                COALESCE(w.approved_entries, 0)::bigint AS approved_entries,
                COALESCE(w.approved_minutes, 0)::bigint AS approved_minutes,
                COALESCE(w.approved_billable_minutes, 0)::bigint AS approved_billable_minutes
         FROM users u
         LEFT JOIN sla s ON s.user_id = u.id
         LEFT JOIN resp r ON r.user_id = u.id
         LEFT JOIN res res_ ON res_.user_id = u.id
         LEFT JOIN wk w ON w.user_id = u.id
         WHERE u.org_id = $1 AND u.id = ANY($2)
         ORDER BY u.display_name, u.id"#,
        org_id,
        user_ids,
        from,
        to,
    )
    .fetch_all(executor)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ScorecardRow {
            user_id: r.user_id,
            display_name: r.display_name,
            response_total: r.response_total.unwrap_or(0),
            response_met: r.response_met.unwrap_or(0),
            resolution_total: r.resolution_total.unwrap_or(0),
            resolution_met: r.resolution_met.unwrap_or(0),
            breached: r.breached.unwrap_or(0),
            responses: r.responses.unwrap_or(0),
            median_response_secs: r.median_secs,
            p95_response_secs: r.p95_secs,
            resolutions: r.resolutions.unwrap_or(0),
            median_resolution_secs: r.median_resolution_secs,
            p95_resolution_secs: r.p95_resolution_secs,
            approved_entries: r.approved_entries.unwrap_or(0),
            approved_minutes: r.approved_minutes.unwrap_or(0),
            approved_billable_minutes: r.approved_billable_minutes.unwrap_or(0),
        })
        .collect())
}

/// APPROVED-minutes mix per (user, work_type) over the window; unclassified
/// entries group under `NULL`.
pub async fn work_type_mix<'e, E>(
    executor: E,
    org_id: Uuid,
    user_ids: &[Uuid],
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> DbResult<Vec<WorkTypeMixRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT e.user_id AS "user_id!", e.work_type, COALESCE(SUM(e.minutes), 0)::bigint AS "minutes!"
           FROM ops.v1_time_entries e
           WHERE e.org_id = $1
             AND e.user_id = ANY($2)
             AND e.started_at >= $3
             AND e.started_at < $4
             AND e.approval_status = 'approved'
           GROUP BY e.user_id, e.work_type
           ORDER BY e.user_id, SUM(e.minutes) DESC"#,
        org_id,
        user_ids,
        from,
        to,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| WorkTypeMixRow {
            user_id: r.user_id,
            work_type: r.work_type,
            minutes: r.minutes,
        })
        .collect())
}

/// A `scorecard_settings` row. `gamification_opt_in` defaults to false —
/// private-by-default coaching, opt-in gamification only (R5).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScorecardSettings {
    pub user_id: Uuid,
    pub gamification_opt_in: bool,
}

pub mod settings {
    use super::{DbResult, ScorecardSettings};
    use sqlx::Postgres;
    use uuid::Uuid;

    /// Loads the member's settings row; a missing row reads as the default
    /// (`gamification_opt_in = false`) — the table exists only to record
    /// deviations from the default.
    pub async fn get<'e, E>(executor: E, org_id: Uuid, user_id: Uuid) -> DbResult<ScorecardSettings>
    where
        E: sqlx::Executor<'e, Database = Postgres>,
    {
        let row = sqlx::query!(
            r#"SELECT user_id, gamification_opt_in
               FROM scorecard_settings
               WHERE org_id = $1 AND user_id = $2"#,
            org_id,
            user_id,
        )
        .fetch_optional(executor)
        .await?;
        Ok(row
            .map(|r| ScorecardSettings {
                user_id: r.user_id,
                gamification_opt_in: r.gamification_opt_in,
            })
            .unwrap_or(ScorecardSettings {
                user_id,
                gamification_opt_in: false,
            }))
    }

    /// Upserts the gamification opt-in flag and returns the stored row.
    pub async fn set_gamification<'e, E>(
        executor: E,
        org_id: Uuid,
        user_id: Uuid,
        opt_in: bool,
        now: chrono::DateTime<chrono::Utc>,
    ) -> DbResult<ScorecardSettings>
    where
        E: sqlx::Executor<'e, Database = Postgres>,
    {
        let row = sqlx::query!(
            r#"INSERT INTO scorecard_settings (org_id, user_id, gamification_opt_in, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $4)
               ON CONFLICT (org_id, user_id)
               DO UPDATE SET gamification_opt_in = EXCLUDED.gamification_opt_in,
                             updated_at = EXCLUDED.updated_at
               RETURNING user_id, gamification_opt_in"#,
            org_id,
            user_id,
            opt_in,
            now,
        )
        .fetch_one(executor)
        .await?;
        Ok(ScorecardSettings {
            user_id: row.user_id,
            gamification_opt_in: row.gamification_opt_in,
        })
    }
}
