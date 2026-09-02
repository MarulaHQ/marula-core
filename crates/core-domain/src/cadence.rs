//! Schedule cadence math (TASKLIST R4) — the single source of truth for
//! "when does this schedule fire next".
//!
//! Purpose: pure, deterministic next-slot computation for typed report
//! schedules (daily | weekly | monthly at a wall-clock time in an IANA
//! zone). Both surfaces that need the answer share THIS module so they
//! can never disagree:
//!
//! * core-api computes `next_run_at` on every schedule write (the
//!   listing's next-run preview is the materialized column), and
//! * the worker advances a claimed slot to the next occurrence.
//!
//! DST doctrine mirrors `notifications::QuietHours`: the wall clock is
//! read through the schedule's timezone via `chrono-tz`; a spring-forward
//! gap means that day's wall time simply never occurs (the day is
//! skipped); a fall-back ambiguity resolves to the EARLIER offset so a
//! slot holds for its first reading.
//!
//! Non-purpose: no persistence (core-db carries the materialized
//! column), no claiming (the worker owns locks and transactions), no
//! cron strings (the cadence is a typed enum pinned by migration 0038's
//! CHECKs).

use chrono::{DateTime, Datelike, Duration, NaiveTime, TimeZone as _, Utc};
use chrono_tz::Tz;

/// Bounded probe budget for the next-slot search — comfortably past a
/// twice-yearly-unrun daily schedule through any DST irregularity, and
/// decades of monthly slots, while guaranteeing termination.
const MAX_OCCURRENCE_PROBES: usize = 4_000;

/// The typed cadence of a schedule row (the stored slug's Rust shape).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cadence {
    Daily,
    Weekly,
    Monthly,
}

impl Cadence {
    /// Strict slug parser (the stored column's inverse). Anything else
    /// fails closed.
    pub fn parse(slug: &str) -> Option<Cadence> {
        match slug {
            "daily" => Some(Cadence::Daily),
            "weekly" => Some(Cadence::Weekly),
            "monthly" => Some(Cadence::Monthly),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Cadence::Daily => "daily",
            Cadence::Weekly => "weekly",
            Cadence::Monthly => "monthly",
        }
    }
}

/// The schedule-row fields the next-slot math reads. `day_of_week` is
/// 0 = Monday .. 6 = Sunday; `day_of_month` is 1..=28 — the DB CHECK
/// pins which fields each cadence carries, this struct just carries
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleShape {
    pub cadence: Cadence,
    pub time_of_day: NaiveTime,
    pub day_of_week: Option<i16>,
    pub day_of_month: Option<i16>,
    pub timezone: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CadenceError {
    #[error("unknown timezone: {0}")]
    Timezone(String),
    #[error("cadence {0} requires day_of_week 0..=6")]
    MissingWeekday(&'static str),
    #[error("cadence {0} requires day_of_month 1..=28")]
    MissingMonthday(&'static str),
    #[error("no occurrence found within the probe budget")]
    NoOccurrence,
}

/// Stored cadence columns → the shape the math reads. A row whose slug
/// or day fields violate the migration's CHECK shape cannot be
/// scheduled — callers record the run as failed rather than guessing.
///
/// # Errors
/// [`CadenceError::MissingWeekday`] / [`CadenceError::MissingMonthday`]
/// for a shape the DB CHECKs should have prevented.
pub fn shape_of(
    cadence: &str,
    time_of_day: NaiveTime,
    day_of_week: Option<i16>,
    day_of_month: Option<i16>,
    timezone: &str,
) -> Result<ScheduleShape, CadenceError> {
    let cadence = Cadence::parse(cadence).ok_or(CadenceError::NoOccurrence)?;
    Ok(ScheduleShape {
        cadence,
        time_of_day,
        day_of_week,
        day_of_month,
        timezone: timezone.to_string(),
    })
}

/// One wall-clock occurrence of `day`@`time` in `tz`, DST-safe: during a
/// spring-forward gap the local time does not exist (`None` — the day is
/// skipped); fall-back ambiguity resolves to the EARLIER offset.
fn occurrence_at(tz: Tz, day: chrono::NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
    let naive = day.and_time(time);
    match tz.from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) => Some(dt.with_timezone(&Utc)),
        chrono::LocalResult::Ambiguous(earliest, _) => Some(earliest.with_timezone(&Utc)),
        chrono::LocalResult::None => None,
    }
}

/// The first occurrence of the schedule strictly AFTER `after` (UTC),
/// read on the schedule's wall clock. Pure and deterministic: identical
/// inputs produce identical slots, which is what keeps the API's stored
/// `next_run_at`, the listing preview, and the worker's claim sweep in
/// exact agreement.
///
/// # Errors
/// [`CadenceError::Timezone`] for an unparseable IANA name;
/// [`CadenceError::MissingWeekday`] / [`CadenceError::MissingMonthday`]
/// for a shapeless weekly/monthly row; [`CadenceError::NoOccurrence`]
/// when the bounded probe budget runs out (a schedule left un-run for
/// decades).
pub fn next_run_after(
    shape: &ScheduleShape,
    after: DateTime<Utc>,
) -> Result<DateTime<Utc>, CadenceError> {
    let tz: Tz = shape
        .timezone
        .parse()
        .map_err(|_| CadenceError::Timezone(shape.timezone.clone()))?;
    let local_after = after.with_timezone(&tz);
    let start_day = local_after.date_naive();

    match shape.cadence {
        Cadence::Daily => {
            let mut day = start_day;
            for _ in 0..MAX_OCCURRENCE_PROBES {
                if let Some(at) = occurrence_at(tz, day, shape.time_of_day) {
                    if at > after {
                        return Ok(at);
                    }
                }
                day = day.succ_opt().ok_or(CadenceError::NoOccurrence)?;
            }
        }
        Cadence::Weekly => {
            let target = u32::try_from(shape.day_of_week.unwrap_or(-1))
                .ok()
                .filter(|d| *d <= 6)
                .ok_or(CadenceError::MissingWeekday("weekly"))?;
            // Align to the first on-or-after start_day carrying the
            // target weekday, then step in weeks until strictly after.
            let offset = (target + 7 - start_day.weekday().num_days_from_monday()) % 7;
            let mut day = start_day + Duration::days(i64::from(offset));
            for _ in 0..MAX_OCCURRENCE_PROBES {
                if let Some(at) = occurrence_at(tz, day, shape.time_of_day) {
                    if at > after {
                        return Ok(at);
                    }
                }
                day += Duration::weeks(1);
            }
        }
        Cadence::Monthly => {
            let dom = u32::try_from(shape.day_of_month.unwrap_or(-1))
                .ok()
                .filter(|d| (1..=28).contains(d))
                .ok_or(CadenceError::MissingMonthday("monthly"))?;
            // 1..=28 exists in EVERY month, so month stepping never
            // skips: short months are covered by construction.
            let (mut year, mut month) = (start_day.year(), start_day.month());
            for _ in 0..MAX_OCCURRENCE_PROBES {
                let day = chrono::NaiveDate::from_ymd_opt(year, month, dom)
                    .ok_or(CadenceError::NoOccurrence)?;
                if let Some(at) = occurrence_at(tz, day, shape.time_of_day) {
                    if at > after {
                        return Ok(at);
                    }
                }
                let (ny, nm) = if month == 12 {
                    (year + 1, 1)
                } else {
                    (year, month + 1)
                };
                year = ny;
                month = nm;
            }
        }
    }
    Err(CadenceError::NoOccurrence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    fn shape(
        cadence: Cadence,
        time: &str,
        dow: Option<i16>,
        dom: Option<i16>,
        tz: &str,
    ) -> ScheduleShape {
        ScheduleShape {
            cadence,
            time_of_day: NaiveTime::parse_from_str(time, "%H:%M").expect("time"),
            day_of_week: dow,
            day_of_month: dom,
            timezone: tz.to_string(),
        }
    }

    #[test]
    fn daily_slots_step_one_day_strictly_after() {
        let s = shape(Cadence::Daily, "08:00", None, None, "UTC");
        // Exactly at a slot → next is tomorrow's.
        let next = next_run_after(&s, utc(2026, 1, 15, 8, 0)).unwrap();
        assert_eq!(next, utc(2026, 1, 16, 8, 0));
        // Mid-day → today's slot is still ahead.
        let next = next_run_after(&s, utc(2026, 1, 15, 7, 59)).unwrap();
        assert_eq!(next, utc(2026, 1, 15, 8, 0));
    }

    #[test]
    fn weekly_lands_on_the_requested_weekday() {
        // 2026-01-15 is a Thursday. Monday = 0.
        let s = shape(Cadence::Weekly, "09:30", Some(0), None, "UTC");
        let next = next_run_after(&s, utc(2026, 1, 15, 12, 0)).unwrap();
        assert_eq!(next, utc(2026, 1, 19, 9, 30)); // the coming Monday
                                                   // Before 09:30 on the target day itself → same day fires.
        let next = next_run_after(&s, utc(2026, 1, 19, 8, 0)).unwrap();
        assert_eq!(next, utc(2026, 1, 19, 9, 30));
    }

    #[test]
    fn monthly_day_28_survives_short_months() {
        let s = shape(Cadence::Monthly, "06:00", None, Some(28), "UTC");
        // Feb 28 2026 07:00 → next is Mar 28 (Feb has no later 28th slot).
        let next = next_run_after(&s, utc(2026, 2, 28, 7, 0)).unwrap();
        assert_eq!(next, utc(2026, 3, 28, 6, 0));
        // Feb 1 → the coming Feb 28.
        let next = next_run_after(&s, utc(2026, 2, 1, 0, 0)).unwrap();
        assert_eq!(next, utc(2026, 2, 28, 6, 0));
    }

    #[test]
    fn timezone_shifts_the_wall_clock_reading() {
        let s = shape(Cadence::Daily, "08:00", None, None, "America/New_York");
        // 12:00 UTC == 07:00 EST → today's 08:00 EST (13:00 UTC) is ahead.
        let next = next_run_after(&s, utc(2026, 1, 15, 12, 0)).unwrap();
        assert_eq!(next, utc(2026, 1, 15, 13, 0));
    }

    #[test]
    fn spring_forward_gap_skips_the_nonexistent_wall_time() {
        // US DST 2026: 2026-03-08 02:00→0300 in New York; a 02:30 daily
        // slot does not exist that day — the next occurrence is Mar 9,
        // 02:30 EDT == 06:30 UTC.
        let s = shape(Cadence::Daily, "02:30", None, None, "America/New_York");
        let next = next_run_after(&s, utc(2026, 3, 8, 6, 0)).unwrap(); // 01:00 EST Mar 8
        assert_eq!(next, utc(2026, 3, 9, 6, 30));
    }

    #[test]
    fn shapeless_rows_and_bad_zones_fail_closed() {
        let at = utc(2026, 1, 15, 8, 0);
        assert!(next_run_after(&shape(Cadence::Weekly, "08:00", None, None, "UTC"), at).is_err());
        assert!(next_run_after(&shape(Cadence::Monthly, "08:00", None, None, "UTC"), at).is_err());
        assert!(next_run_after(
            &shape(Cadence::Daily, "08:00", None, None, "Mars/Olympus"),
            at
        )
        .is_err());
        assert!(Cadence::parse("annually").is_none());
        assert!(shape_of("hourly", NaiveTime::MIN, None, None, "UTC").is_err());
    }

    #[test]
    fn cadence_slugs_round_trip() {
        for c in [Cadence::Daily, Cadence::Weekly, Cadence::Monthly] {
            assert_eq!(Cadence::parse(c.as_str()), Some(c));
        }
    }
}
