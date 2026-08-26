//! Missing-time gap audit — the pure decision layer (TASKLIST 2.10).
//!
//! Purpose: date/window math over IANA timezones (DST-safe) and the
//! activity-vs-entries gap rule, shared verbatim by the EOD nudge run
//! (core-api internal endpoint), the self/manager gap reports, and the
//! worker scheduler. A "gap day" is a local day where the user had tracked
//! ticket activity (comments authored or timer segments running) but logged
//! fewer minutes than the org's expected minimum.
//!
//! Non-purpose: no I/O and no persistence — callers supply activity and
//! logged-minutes facts from core-db; nothing here decides *delivery*
//! (notification preferences and quiet hours own that downstream).

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

/// One evaluated local day for one user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GapDay {
    /// Local calendar day (the audit grain — never a UTC day).
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub day: NaiveDate,
    /// True when the user touched tickets (comment/timer activity) that day.
    pub activity: bool,
    /// Minutes logged via time entries starting inside the day window.
    pub logged_minutes: i64,
    /// Org-configured minimum expected for an active day.
    pub expected_minutes: i64,
}

impl GapDay {
    /// The gap rule: activity without enough logged time. Days without
    /// activity are never gaps (vacation, weekend, meetings-only).
    pub fn has_gap(&self) -> bool {
        self.activity && self.logged_minutes < self.expected_minutes
    }

    /// Minutes below the expectation on an active day; 0 otherwise.
    pub fn unaccounted_minutes(&self) -> i64 {
        if self.activity {
            (self.expected_minutes - self.logged_minutes).max(0)
        } else {
            0
        }
    }
}

/// Evaluates one day. Kept as a total function so the rule under test is
/// exactly the rule in production.
pub fn evaluate_day(
    day: NaiveDate,
    activity: bool,
    logged_minutes: i64,
    expected_minutes: i64,
) -> GapDay {
    GapDay {
        day,
        activity,
        logged_minutes: logged_minutes.max(0),
        expected_minutes: expected_minutes.max(0),
    }
}

/// The UTC instant range `[start, end)` covering local `day` in `tz`.
///
/// DST-safe by construction: the bounds are local midnights converted
/// through the zone. Fall-back repeats resolve to the first (earliest)
/// midnight; a skipped local midnight (some zones spring forward at 00:00)
/// falls forward to 01:00 so the window always exists and start < end.
pub fn local_day_window(day: NaiveDate, tz: Tz) -> (DateTime<Utc>, DateTime<Utc>) {
    let midnight = NaiveTime::MIN;
    let naive_start = day.and_time(midnight);
    let naive_end = day.succ_opt().unwrap_or(day).and_time(midnight);
    let start = resolve_local(tz, naive_start);
    let end = resolve_local(tz, naive_end);
    debug_assert!(start < end, "day window must be forward-moving");
    (start, end.max(start + chrono::Duration::seconds(1)))
}

fn resolve_local(tz: Tz, naive: chrono::NaiveDateTime) -> DateTime<Utc> {
    use chrono::TimeZone;
    match tz.from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) => dt.with_timezone(&Utc),
        // Fall-back: two midnights exist — audit the earlier (first) day.
        chrono::LocalResult::Ambiguous(dt, _) => dt.with_timezone(&Utc),
        // Spring-forward skipped midnight (e.g. Santiago): use 01:00.
        chrono::LocalResult::None => {
            let shifted = naive + chrono::Duration::hours(1);
            tz.from_local_datetime(&shifted)
                .earliest()
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|| naive.and_utc())
        }
    }
}

/// Inclusive local-day series `from..=to` (empty when `to < from`).
/// Bounded by construction: callers clamp ranges to sane windows.
pub fn day_series(from: NaiveDate, to: NaiveDate) -> Vec<NaiveDate> {
    let mut out = Vec::new();
    let mut cur = from;
    while cur <= to && out.len() < 366 {
        out.push(cur);
        cur = match cur.succ_opt() {
            Some(next) => next,
            None => break,
        };
    }
    out
}

/// The local calendar date containing `now` shifted back one day — the
/// default audit target for EOD flows and "yesterday" report defaults.
pub fn previous_local_day(now: DateTime<Utc>, tz: Tz) -> NaiveDate {
    let local = now.with_timezone(&tz);
    let today = local.date_naive();
    today.pred_opt().unwrap_or(today)
}

/// True when `now`, in `tz`, has reached local hour `nudge_hour` — the EOD
/// trigger. Later hours on the same day stay due (the audit is once per
/// user/day; the nudge ledger makes repeats inert).
pub fn is_nudge_due(now: DateTime<Utc>, tz: Tz, nudge_hour: u8) -> bool {
    use chrono::Timelike;
    let local = now.with_timezone(&tz);
    let hour = local.hour() as u8;
    // At/after the hour. A 24+ hour outage self-heals because every later
    // hour of the day still satisfies this; the (user, day) ledger keeps
    // the nudge itself single-shot.
    hour >= nudge_hour.min(23)
}

// --- internal run wire contract (worker ↔ core-api) -------------------------
//
// The worker schedules; core-api computes, persists, and nudges. These DTOs
// are the shared request/response shapes for the token-gated internal
// endpoint — deliberately NOT part of the public OpenAPI surface.

/// One due audit target chosen by the worker's EOD scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GapAuditRunItem {
    pub org_id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    /// The user's LOCAL calendar day being audited.
    pub day: NaiveDate,
}

/// Batch of due targets.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GapAuditRunRequest {
    pub items: Vec<GapAuditRunItem>,
}

/// Per-item outcome; `error` isolates one bad item (bad tz, missing user)
/// so a nightly batch never dies wholesale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapAuditRunResult {
    pub org_id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub day: NaiveDate,
    pub activity: bool,
    pub logged_minutes: i32,
    pub expected_minutes: i32,
    pub has_gap: bool,
    /// True only for the run that won the once-per user/day nudge claim.
    pub nudged: bool,
    pub error: Option<String>,
}

/// Batch outcome.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GapAuditRunResponse {
    pub results: Vec<GapAuditRunResult>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use proptest::prelude::*;

    fn utc(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).expect("valid date")
    }

    #[test]
    fn window_is_local_midnight_to_local_midnight() {
        let tz: Tz = "America/New_York".parse().unwrap();
        let (start, end) = local_day_window(d(2026, 1, 15), tz);
        // EST is UTC-5 in January: local midnight = 05:00 UTC.
        assert_eq!(start, utc(2026, 1, 15, 5));
        assert_eq!(end, utc(2026, 1, 16, 5));
    }

    #[test]
    fn dst_spring_forward_day_is_shorter_but_valid() {
        let tz: Tz = "America/New_York".parse().unwrap();
        // 2026-03-08: clocks spring forward; the day is 23 hours long.
        let (start, end) = local_day_window(d(2026, 3, 8), tz);
        assert_eq!(start, utc(2026, 3, 8, 5));
        assert_eq!(end, utc(2026, 3, 9, 4));
        assert_eq!((end - start).num_hours(), 23);
    }

    #[test]
    fn dst_fall_back_day_is_longer_but_valid() {
        let tz: Tz = "America/New_York".parse().unwrap();
        // 2026-11-01: clocks fall back; the day is 25 hours long.
        let (start, end) = local_day_window(d(2026, 11, 1), tz);
        assert_eq!((end - start).num_hours(), 25);
    }

    #[test]
    fn skipped_midnight_zone_still_produces_a_window() {
        // Santiago springs forward at midnight: 2026-09-06 00:00 does not
        // exist. The window must still start on the right local day.
        let tz: Tz = "America/Santiago".parse().unwrap();
        let (start, end) = local_day_window(d(2026, 9, 6), tz);
        assert!(start < end);
        // The shifted start is 01:00 local = 04:00 UTC (UTC-3 after jump).
        assert_eq!(start, utc(2026, 9, 6, 4));
    }

    #[test]
    fn gap_rule_requires_activity() {
        // No activity ⇒ never a gap, even with zero logged minutes.
        assert!(!evaluate_day(d(2026, 1, 15), false, 0, 60).has_gap());
        // Activity with enough logged time ⇒ no gap.
        assert!(!evaluate_day(d(2026, 1, 15), true, 60, 60).has_gap());
        assert!(!evaluate_day(d(2026, 1, 15), true, 90, 60).has_gap());
        // Activity under the bar ⇒ gap.
        assert!(evaluate_day(d(2026, 1, 15), true, 59, 60).has_gap());
    }

    #[test]
    fn unaccounted_never_negative() {
        assert_eq!(
            evaluate_day(d(2026, 1, 15), true, 90, 60).unaccounted_minutes(),
            0
        );
        assert_eq!(
            evaluate_day(d(2026, 1, 15), true, 10, 60).unaccounted_minutes(),
            50
        );
        assert_eq!(
            evaluate_day(d(2026, 1, 15), false, 0, 60).unaccounted_minutes(),
            0
        );
    }

    #[test]
    fn day_series_is_inclusive_and_empty_when_inverted() {
        assert_eq!(day_series(d(2026, 1, 1), d(2026, 1, 3)).len(), 3);
        assert!(day_series(d(2026, 1, 3), d(2026, 1, 1)).is_empty());
        // Leap year handled by date arithmetic.
        assert_eq!(day_series(d(2024, 2, 28), d(2024, 3, 1)).len(), 3);
    }

    #[test]
    fn previous_local_day_respects_the_zone() {
        let tz: Tz = "Pacific/Auckland".parse().unwrap();
        // 2026-01-15 10:00 UTC is already Jan 15 23:00 in Auckland →
        // yesterday is Jan 14, not Jan 14/15 ambiguity for UTC readers.
        assert_eq!(previous_local_day(utc(2026, 1, 15, 10), tz), d(2026, 1, 14));
        let utc_tz: Tz = "UTC".parse().unwrap();
        assert_eq!(
            previous_local_day(utc(2026, 1, 15, 10), utc_tz),
            d(2026, 1, 14)
        );
    }

    #[test]
    fn nudge_due_at_and_after_the_local_hour_only() {
        let tz: Tz = "America/Denver".parse().unwrap();
        // 18:00 MDT = 2026-06-15 00:00 UTC next day boundary… concretely:
        // 2026-06-16 00:30 UTC == 18:30 MDT Jun 15 → due for hour 18.
        assert!(is_nudge_due(utc(2026, 6, 16, 0), tz, 18));
        // 17:59 MDT = 2026-06-15 23:59 UTC → not yet.
        assert!(!is_nudge_due(
            Utc.with_ymd_and_hms(2026, 6, 15, 23, 59, 0).unwrap(),
            tz,
            18
        ));
        // Midnight local (hour 0) is due for every configured hour ≥ 0.
        assert!(is_nudge_due(utc(2026, 6, 16, 6), tz, 0));
    }

    proptest! {
        /// Windows across many zones/dates are always non-empty and within
        /// 23–25 hours (DST days), and the gap rule stays monotonic.
        #[test]
        fn windows_are_sane_and_rule_monotonic(
            day_offset in 0..3000_i64,
            logged in 0..600_i64,
            expected in 0..600_i64,
            tz_idx in 0..5_usize,
        ) {
            let zones = ["UTC", "America/New_York", "America/Santiago",
                         "Australia/Sydney", "Europe/Berlin"];
            let tz: Tz = zones[tz_idx].parse().unwrap();
            let day = d(2020, 1, 1) + chrono::Duration::days(day_offset);
            let (start, end) = local_day_window(day, tz);
            prop_assert!(start < end);
            let hours = (end - start).num_hours();
            prop_assert!((23..=25).contains(&hours), "window {hours}h for {} on {day}", zones[tz_idx]);

            let below = evaluate_day(day, true, logged, expected);
            let more = evaluate_day(day, true, logged + 1, expected);
            // Logging more can only shrink (never create) a gap.
            prop_assert!(more.has_gap() <= below.has_gap());
            prop_assert_eq!(
                below.has_gap(),
                below.activity && below.logged_minutes < below.expected_minutes
            );
        }
    }
}
