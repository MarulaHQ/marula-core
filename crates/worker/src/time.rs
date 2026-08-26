//! worker/time — the EOD missing-time nudge scheduler (TASKLIST 2.10).
//!
//! Purpose: periodically compute which (org, user, local-day) triples are
//! due for a missing-time audit — orgs with nudges enabled, members whose
//! local wall clock has passed the org's nudge hour — and hand the batch
//! to core-api's internal gap-audit endpoint. The worker owns only the
//! CLOCK: all gap math, persistence, and notification creation happen in
//! core-api, whose ledger makes repeated runs inert (once per user/day).
//!
//! Non-purpose: no database writes (core-api is the single writer §4.1),
//! no gap-rule decisions (core-domain), no channel delivery (the
//! notification center + notifier own that downstream).

use crate::config::TimeLoopConfig;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use sqlx::PgPool;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::watch;
use uuid::Uuid;

/// One due audit target: the member's CURRENT local day, nudged once their
/// local clock passes the org hour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DueItem {
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub day: chrono::NaiveDate,
}

/// Org row the scheduler needs, resolved by the caller (testable seam).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedOrg {
    pub org_id: Uuid,
    pub eod_nudge_hour: i16,
}

/// Member timezone as `(user_id, tz name)`; unparseable zones are skipped
/// (a broken tz must not stall the whole org's batch).
pub fn due_items(
    orgs: &[SchedOrg],
    members_by_org: &HashMap<Uuid, Vec<(Uuid, String)>>,
    now: DateTime<Utc>,
) -> Vec<DueItem> {
    let mut out = Vec::new();
    for org in orgs {
        let hour = u8::try_from(org.eod_nudge_hour.clamp(0, 23)).unwrap_or(18);
        let Some(members) = members_by_org.get(&org.org_id) else {
            continue;
        };
        for (user_id, tz_name) in members {
            let Ok(tz) = tz_name.parse::<Tz>() else {
                tracing::warn!(org = %org.org_id, user = %user_id, tz = %tz_name, "skipping member with unparseable timezone");
                continue;
            };
            if !core_domain::time_audit::is_nudge_due(now, tz, hour) {
                continue;
            }
            let day = now.with_timezone(&tz).date_naive();
            let item = DueItem {
                org_id: org.org_id,
                user_id: *user_id,
                day,
            };
            if !out.contains(&item) {
                out.push(item);
            }
        }
    }
    out
}

#[derive(Debug, serde::Deserialize)]
struct RunResponse {
    #[serde(default)]
    results: Vec<core_domain::time_audit::GapAuditRunResult>,
}

/// Runs the scheduler until `shutdown` fires. Unset internal token keeps
/// the loop disabled (the documented none mode, mirroring the email loop).
pub async fn run_time_loop(
    pool: PgPool,
    config: TimeLoopConfig,
    mut shutdown: watch::Receiver<bool>,
) {
    if config.internal_token.is_none() {
        tracing::warn!(
            "EMAIL_INTERNAL_TOKEN unset: missing-time nudge loop disabled (no nudges sent)"
        );
        return;
    }
    let client = reqwest::Client::new();
    let poll = Duration::from_secs(config.poll_secs);
    tracing::info!(api = %config.core_api_url, poll_secs = config.poll_secs, "missing-time nudge loop started");
    loop {
        if *shutdown.borrow_and_update() {
            return;
        }
        match cycle(&pool, &config, &client, Utc::now()).await {
            Ok(0) => {}
            Ok(due) => tracing::info!(due, "gap-audit batch submitted"),
            Err(error) => tracing::error!(%error, "nudge scheduler cycle failed"),
        }
        tokio::time::sleep(poll).await;
    }
}

/// One pass: read orgs + member timezones, select due items, submit the
/// batch. Returns how many items were submitted.
async fn cycle(
    pool: &PgPool,
    config: &TimeLoopConfig,
    client: &reqwest::Client,
    now: DateTime<Utc>,
) -> anyhow::Result<usize> {
    let Some(token) = config.internal_token.as_deref() else {
        return Ok(0);
    };
    let org_rows = core_db::time_audit::nudge_enabled_orgs(pool).await?;
    let orgs: Vec<SchedOrg> = org_rows
        .iter()
        .map(|o| SchedOrg {
            org_id: o.org_id,
            eod_nudge_hour: o.eod_nudge_hour,
        })
        .collect();

    let mut members_by_org: HashMap<Uuid, Vec<(Uuid, String)>> = HashMap::new();
    for org in &org_rows {
        let members = core_db::time_audit::org_member_timezones(pool, org.org_id).await?;
        members_by_org.insert(
            org.org_id,
            members
                .into_iter()
                .map(|m| (m.user_id, m.timezone))
                .collect(),
        );
    }

    let due = due_items(&orgs, &members_by_org, now);
    if due.is_empty() {
        return Ok(0);
    }

    let request = core_domain::time_audit::GapAuditRunRequest {
        items: due
            .iter()
            .map(|d| core_domain::time_audit::GapAuditRunItem {
                org_id: d.org_id,
                user_id: d.user_id,
                day: d.day,
            })
            .collect(),
    };

    let response: RunResponse = client
        .post(format!(
            "{}/api/internal/time/gap-audit/run",
            config.core_api_url.trim_end_matches('/')
        ))
        .header("X-Internal-Token", token)
        .json(&request)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    for result in &response.results {
        match &result.error {
            Some(error) => tracing::warn!(%error, "gap-audit item failed"),
            None if result.nudged => tracing::info!("missing-time nudge delivered"),
            None => {}
        }
    }
    Ok(due.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_uuid(n: u64) -> Uuid {
        Uuid::from_u128(0x6D65_7269_6469_616E_u128 << 64 | u128::from(n))
    }

    fn org(id: u64, hour: i16) -> SchedOrg {
        SchedOrg {
            org_id: fixed_uuid(id),
            eod_nudge_hour: hour,
        }
    }

    fn utc(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn due_only_after_local_nudge_hour() {
        let o = org(1, 18);
        let mut members = HashMap::new();
        members.insert(
            o.org_id,
            vec![(fixed_uuid(2), "America/New_York".into())],
        );
        // 21:00 UTC == 16:00 EST → not due.
        assert!(due_items(&[o], &members, utc(2026, 1, 15, 21)).is_empty());
        // 23:30 UTC == 18:30 EST → due for Jan 15 local.
        let due = due_items(
            &[o],
            &members,
            Utc.with_ymd_and_hms(2026, 1, 15, 23, 30, 0).unwrap(),
        );
        assert_eq!(due.len(), 1);
        assert_eq!(
            due[0].day,
            chrono::NaiveDate::from_ymd_opt(2026, 1, 15).unwrap()
        );
    }

    #[test]
    fn member_timezone_shifts_the_day() {
        let o = org(1, 0);
        let mut members = HashMap::new();
        members.insert(
            o.org_id,
            vec![
                (fixed_uuid(2), "Pacific/Auckland".into()),
                (fixed_uuid(3), "UTC".into()),
            ],
        );
        // 2026-01-15 10:00 UTC: Auckland is Jan 15 23:00 (due, day = Jan 15);
        // UTC is 10:00 → due for hour 0 as well, same local day.
        let due = due_items(&[o], &members, utc(2026, 1, 15, 10));
        assert_eq!(due.len(), 2);
        assert!(due
            .iter()
            .all(|d| d.day == chrono::NaiveDate::from_ymd_opt(2026, 1, 15).unwrap()));
    }

    #[test]
    fn unparseable_timezone_skips_member_not_batch() {
        let o = org(1, 18);
        let mut members = HashMap::new();
        members.insert(
            o.org_id,
            vec![
                (fixed_uuid(2), "Mars/Olympus".into()),
                (fixed_uuid(3), "UTC".into()),
            ],
        );
        let due = due_items(&[o], &members, utc(2026, 1, 15, 18));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].user_id, fixed_uuid(3));
    }

    #[test]
    fn disabled_or_missing_orgs_yield_nothing() {
        // Empty org list → nothing due regardless of members map.
        let members = HashMap::new();
        assert!(due_items(&[], &members, utc(2026, 1, 15, 18)).is_empty());
    }
}
