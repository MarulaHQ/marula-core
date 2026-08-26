//! Scheduling rules — the pure decision layer for calendars/availability
//! (TASKLIST 2.12).
//!
//! Purpose: overlap detection for bookings, availability-window
//! containment, and working-minutes computation over absolute spans. All
//! functions are totals over injected inputs: no I/O, no host clocks.
//! Working minutes are computed standalone from [`AvailabilityWindow`]
//! rows rather than `BusinessCalendar`: the shapes differ fundamentally
//! (per-user rows with per-row IANA tz + effective ranges vs one org JSON
//! calendar), and forcing one onto the other would couple SLA config to
//! user preferences. Correctness is pinned by proptests below instead of
//! shared code.
//!
//! Non-purpose: persistence, conflict *enforcement* (the DB EXCLUDE
//! constraint owns that), recurrence expansion, or reminder scheduling
//! (worker lane, deferred).

use crate::entities::{AvailabilityWindow, CalendarExclusion};
use crate::error::DomainError;
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use chrono_tz::Tz;

/// Half-open interval overlap: `[a_s, a_e)` intersects `[b_s, b_e)`.
/// Touching spans (`a_e == b_s`) never overlap — back-to-back booking is
/// legal, which the DB `tstzrange(..., '[)')` constraint mirrors exactly.
pub fn spans_overlap(
    a_start: DateTime<Utc>,
    a_end: DateTime<Utc>,
    b_start: DateTime<Utc>,
    b_end: DateTime<Utc>,
) -> bool {
    a_start < b_end && b_start < a_end
}

/// The rule the API applies on writes: a candidate slot conflicts when it
/// overlaps ANY existing live span. Returns every conflicting index so the
/// dry-run endpoint can name all blockers, not just the first.
pub fn conflict_indices<I>(
    candidate_start: DateTime<Utc>,
    candidate_end: DateTime<Utc>,
    spans: I,
) -> Vec<usize>
where
    I: IntoIterator<Item = (DateTime<Utc>, DateTime<Utc>)>,
{
    spans
        .into_iter()
        .enumerate()
        .filter(|(_, (s, e))| spans_overlap(candidate_start, candidate_end, *s, *e))
        .map(|(i, _)| i)
        .collect()
}

impl AvailabilityWindow {
    /// Structural validation for API writes: real IANA timezone, sane
    /// weekday (0=Mon..6=Sun), open < close in local wall-clock terms.
    pub fn validate(&self) -> Result<(), DomainError> {
        self.timezone
            .parse::<Tz>()
            .map_err(|_| DomainError::Validation(format!("unknown timezone: {}", self.timezone)))?;
        if !(0..=6).contains(&self.weekday) {
            return Err(DomainError::Validation(format!(
                "weekday must be 0-6, got {}",
                self.weekday
            )));
        }
        if self.start_local >= self.end_local {
            return Err(DomainError::Validation(
                "availability window opens at or after it closes".into(),
            ));
        }
        if let (Some(from), Some(until)) = (self.effective_from, self.effective_until) {
            if until < from {
                return Err(DomainError::Validation(
                    "effective_until precedes effective_from".into(),
                ));
            }
        }
        Ok(())
    }

    /// Is this template active for the given LOCAL date?
    pub fn effective_on(&self, local_day: NaiveDate) -> bool {
        self.effective_from.is_none_or(|f| local_day >= f)
            && self.effective_until.is_none_or(|u| local_day <= u)
    }

    /// Does the window contain the absolute instant? The instant's LOCAL
    /// weekday/time (in the window's own timezone) must fall inside
    /// `[start_local, end_local)`. DST-safe: chrono resolves the instant
    /// to the zone-correct wall clock, so a spring-forward gap simply has
    /// no matching local time and fall-back repeats resolve to their real
    /// offsets.
    pub fn contains(&self, instant: DateTime<Utc>) -> bool {
        let Ok(tz) = self.timezone.parse::<Tz>() else {
            return false;
        };
        let local = instant.with_timezone(&tz);
        if !self.effective_on(local.date_naive()) {
            return false;
        }
        if local.weekday().num_days_from_monday() as i16 != self.weekday {
            return false;
        }
        let t = local.time();
        self.start_local <= t && t < self.end_local
    }
}

/// Available at `instant`: inside at least one active availability window
/// AND outside every time-off exclusion. No windows configured means NOT
/// available (defaults-first orgs seed windows; absence is a fact, not an
/// implicit always-on).
pub fn is_available(
    windows: &[AvailabilityWindow],
    exclusions: &[CalendarExclusion],
    instant: DateTime<Utc>,
) -> bool {
    if exclusions
        .iter()
        .any(|x| x.starts_at <= instant && instant < x.ends_at)
    {
        return false;
    }
    windows.iter().any(|w| w.contains(instant))
}

fn merge_intervals(
    mut ivs: Vec<(DateTime<Utc>, DateTime<Utc>)>,
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    ivs.sort_by_key(|(s, e)| (*s, *e));
    let mut out: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::with_capacity(ivs.len());
    for (s, e) in ivs {
        if s >= e {
            continue;
        }
        match out.last_mut() {
            Some((_, last_end)) if s <= *last_end => {
                *last_end = (*last_end).max(e);
            }
            _ => out.push((s, e)),
        }
    }
    out
}

/// Largest representable wall-clock time (23:59:59.999999999); used as the
/// half-open "round-the-clock" day bound since 24:00 is not representable.
fn end_of_day() -> chrono::NaiveTime {
    chrono::NaiveTime::from_hms_nano_opt(23, 59, 59, 999_999_999)
        .expect("end-of-day constant is valid")
}

/// Resolves the window's UTC occurrence(s) on local day `day`, clipping to
/// `[from, to]`. Fall-back ambiguity resolves like the SLA calendar: first
/// occurrence; spring-forward gaps contribute nothing rather than
/// inventing time. An end-of-day bound (`NaiveTime::MAX`) rounds up to the
/// next local midnight so round-the-clock windows are exactly 24h.
/// Overnight windows are not expressible as templates — `start_local <
/// end_local` is enforced upstream.
fn window_occurrence(
    window: &AvailabilityWindow,
    tz: Tz,
    day: NaiveDate,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    use chrono::TimeZone;
    if !window.effective_on(day) {
        return None;
    }
    let start = tz
        .from_local_datetime(&day.and_time(window.start_local))
        .earliest()?
        .with_timezone(&Utc);
    let end_naive_day = if window.end_local == end_of_day() {
        day.succ_opt()?
    } else {
        day
    };
    let end_naive_time = if window.end_local == end_of_day() {
        chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()
    } else {
        window.end_local
    };
    let end = tz
        .from_local_datetime(&end_naive_day.and_time(end_naive_time))
        .earliest()?
        .with_timezone(&Utc);
    if end <= start {
        return None;
    }
    let s = start.max(from);
    let e = end.min(to);
    (e > s).then_some((s, e))
}

/// Working minutes of business coverage for `[from, to]`: union of all
/// availability-window occurrences minus union of exclusions. Windows in
/// different timezones compose correctly because each occurrence is
/// resolved to absolute time before merging.
pub fn covered_minutes(
    windows: &[AvailabilityWindow],
    exclusions: &[CalendarExclusion],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> i64 {
    if to <= from {
        return 0;
    }
    // Walk each window's own-local days around the span (±1 day absorbs
    // any timezone skew between windows and range bounds).
    let mut occurrences: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();
    for window in windows {
        let Ok(tz) = window.timezone.parse::<Tz>() else {
            continue;
        };
        let mut day = from.with_timezone(&tz).date_naive().pred_opt();
        let last = to.with_timezone(&tz).date_naive().succ_opt();
        while let Some(d) = day {
            if d.weekday().num_days_from_monday() as i16 == window.weekday {
                if let Some(occ) = window_occurrence(window, tz, d, from, to) {
                    occurrences.push(occ);
                }
            }
            match d.succ_opt() {
                Some(next) if Some(next) <= last => day = Some(next),
                _ => break,
            }
        }
    }
    let covered = merge_intervals(occurrences);
    let blocked = merge_intervals(
        exclusions
            .iter()
            .filter_map(|x| {
                let s = x.starts_at.max(from);
                let e = x.ends_at.min(to);
                (e > s).then_some((s, e))
            })
            .collect(),
    );
    // Subtract merged exclusions from merged coverage (both sorted and
    // internally disjoint): one forward sweep with a never-backwards
    // pointer into the exclusion list.
    let mut total = 0_i64;
    let mut j = 0_usize;
    for &(cs, ce) in &covered {
        while j < blocked.len() && blocked[j].1 <= cs {
            j += 1;
        }
        let mut cursor = cs;
        let mut k = j;
        while k < blocked.len() && blocked[k].0 < ce {
            if blocked[k].0 > cursor {
                total += (blocked[k].0 - cursor).num_seconds();
            }
            cursor = cursor.max(blocked[k].1);
            k += 1;
        }
        if cursor < ce {
            total += (ce - cursor).num_seconds();
        }
    }
    total / 60
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        chrono::NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
            .and_utc()
    }

    fn win(user: uuid::Uuid, weekday: i16, sh: u32, eh: u32) -> AvailabilityWindow {
        // `eh == 24` means "round-the-clock": the half-open day end.
        let end = if eh == 24 {
            end_of_day()
        } else {
            chrono::NaiveTime::from_hms_opt(eh, 0, 0).unwrap()
        };
        AvailabilityWindow {
            id: uuid::Uuid::now_v7(),
            org_id: uuid::Uuid::now_v7(),
            user_id: user,
            weekday,
            start_local: chrono::NaiveTime::from_hms_opt(sh, 0, 0).unwrap(),
            end_local: end,
            timezone: "UTC".into(),
            effective_from: None,
            effective_until: None,
            created_at: utc(2026, 1, 1, 0, 0),
        }
    }

    fn excl(from: DateTime<Utc>, to: DateTime<Utc>) -> CalendarExclusion {
        CalendarExclusion {
            id: uuid::Uuid::now_v7(),
            org_id: uuid::Uuid::now_v7(),
            user_id: uuid::Uuid::now_v7(),
            starts_at: from,
            ends_at: to,
            reason: None,
            created_at: from,
        }
    }

    #[test]
    fn touching_and_nested_spans() {
        assert!(!spans_overlap(
            at9(9, 0),
            at9(10, 0),
            at9(10, 0),
            at9(11, 0)
        ));
        assert!(spans_overlap(at9(9, 0), at9(10, 0), at9(9, 30), at9(11, 0)));
        assert!(spans_overlap(at9(9, 0), at9(17, 0), at9(10, 0), at9(11, 0)));
        assert!(spans_overlap(at9(9, 0), at9(10, 0), at9(9, 0), at9(10, 0)));
        assert!(!spans_overlap(
            at9(9, 0),
            at9(10, 0),
            at9(11, 0),
            at9(12, 0)
        ));
    }

    fn at9(h: u32, m: u32) -> DateTime<Utc> {
        utc(2026, 5, 18, h, m)
    }

    #[test]
    fn conflict_indices_name_all_blockers() {
        let spans = [
            (at9(9, 0), at9(11, 0)),
            (at9(10, 0), at9(12, 0)),
            (at9(13, 0), at9(14, 0)),
        ];
        assert_eq!(
            conflict_indices(at9(10, 30), at9(11, 30), spans),
            vec![0, 1]
        );
        assert_eq!(
            conflict_indices(at9(10, 30), at9(13, 30), spans),
            vec![0, 1, 2]
        );
        assert!(conflict_indices(at9(14, 0), at9(15, 0), spans).is_empty());
    }

    #[test]
    fn containment_matrix_weekday_time_effective_range() {
        let u = uuid::Uuid::now_v7();
        // Monday 09:00–17:00 UTC, effective from 2026-05-01.
        let mut w = win(u, 0, 9, 17);
        w.effective_from = Some(NaiveDate::from_ymd_opt(2026, 5, 1).unwrap());
        let mon_1000 = utc(2026, 5, 18, 10, 0); // Monday
        assert_eq!(mon_1000.weekday().num_days_from_monday(), 0);
        assert!(w.contains(mon_1000));
        // Outside hours.
        assert!(!w.contains(utc(2026, 5, 18, 8, 59)));
        assert!(!w.contains(utc(2026, 5, 18, 17, 0))); // half-open end
                                                       // Right weekday, wrong date (before effective).
        assert!(!w.contains(utc(2026, 4, 20, 10, 0)));
        // Wrong weekday, right time-of-day.
        assert!(!w.contains(utc(2026, 5, 19, 10, 0)));
        // Effective-until boundary is inclusive.
        w.effective_until = Some(NaiveDate::from_ymd_opt(2026, 5, 18).unwrap());
        assert!(w.contains(mon_1000));
        w.effective_until = Some(NaiveDate::from_ymd_opt(2026, 5, 17).unwrap());
        assert!(!w.contains(mon_1000));
    }

    #[test]
    fn no_windows_means_not_available_but_exclusions_punch_first() {
        let u = uuid::Uuid::now_v7();
        let t = utc(2026, 5, 18, 10, 0);
        assert!(!is_available(&[], &[], t));
        let w = [win(u, 0, 0, 24)];
        assert!(is_available(&w, &[], t));
        assert!(!is_available(
            &w,
            &[excl(t - Duration::hours(1), t + Duration::hours(1))],
            t
        ));
    }

    #[test]
    fn covered_minutes_basic_union() {
        let u = uuid::Uuid::now_v7();
        // Monday 2026-05-18: two windows 09–12 and 13–17 ⇒ 7h; overlapping
        // duplicates union, never double-count.
        let windows = [
            win(u, 0, 9, 12),
            win(u, 0, 13, 17),
            win(u, 0, 11, 14), // overlaps both gaps
        ];
        let mins = covered_minutes(
            &windows,
            &[],
            utc(2026, 5, 18, 0, 0),
            utc(2026, 5, 19, 0, 0),
        );
        assert_eq!(mins, 8 * 60); // 09:00–17:00 continuous via union
    }

    #[test]
    fn exclusions_subtract_once_even_when_stacked() {
        let u = uuid::Uuid::now_v7();
        let windows = [win(u, 0, 9, 17)];
        let day = (utc(2026, 5, 18, 0, 0), utc(2026, 5, 19, 0, 0));
        let stacked = [
            excl(utc(2026, 5, 18, 10, 0), utc(2026, 5, 18, 12, 0)),
            excl(utc(2026, 5, 18, 11, 0), utc(2026, 5, 18, 13, 0)),
        ];
        assert_eq!(covered_minutes(&windows, &stacked, day.0, day.1), 5 * 60);
    }

    #[test]
    fn validate_rejects_bad_shapes() {
        let mut w = win(uuid::Uuid::now_v7(), 0, 9, 17);
        assert!(w.validate().is_ok());
        w.timezone = "Mars/Olympus".into();
        assert!(w.validate().is_err());
        w.timezone = "UTC".into();
        w.weekday = 7;
        assert!(w.validate().is_err());
        w.weekday = 6;
        w.start_local = chrono::NaiveTime::from_hms_opt(17, 0, 0).unwrap();
        assert!(w.validate().is_err());
    }

    #[test]
    fn dst_spring_forward_window_shrinks_in_absolute_minutes() {
        let mut w = win(uuid::Uuid::now_v7(), 6, 1, 4);
        w.timezone = "America/New_York".into();
        // Sunday 2026-03-08: 02:00 EST → 03:00 EDT. Wall 3h = absolute 2h.
        let mins = covered_minutes(
            std::slice::from_ref(&w),
            &[],
            utc(2026, 3, 8, 0, 0),
            utc(2026, 3, 9, 0, 0),
        );
        assert_eq!(mins, 120);
    }

    #[test]
    fn dst_fall_back_window_grows_in_absolute_minutes() {
        let mut w = win(uuid::Uuid::now_v7(), 6, 1, 4);
        w.timezone = "America/New_York".into();
        // Sunday 2026-11-01: 02:00 EDT → 01:00 EST. Wall 3h = absolute 4h
        // (the repeated hour is real elapsed availability).
        let mins = covered_minutes(
            std::slice::from_ref(&w),
            &[],
            utc(2026, 11, 1, 0, 0),
            utc(2026, 11, 2, 0, 0),
        );
        assert_eq!(mins, 240);
    }

    #[test]
    fn contains_resolves_zone_correct_wall_clock() {
        let mut w = win(uuid::Uuid::now_v7(), 0, 9, 17);
        w.timezone = "America/New_York".into();
        // Monday 2026-05-18 09:00 New York == 13:00 UTC (EDT).
        assert!(w.contains(utc(2026, 5, 18, 13, 0)));
        assert!(!w.contains(utc(2026, 5, 18, 12, 59)));
        assert!(w.contains(utc(2026, 5, 18, 16, 59)));
        assert!(!w.contains(utc(2026, 5, 18, 21, 0)));
    }

    // --- property tests -----------------------------------------------------

    use proptest::prelude::*;

    proptest::proptest! {
        /// Greedy acceptance (the write-path rule) can never produce a
        /// double-booked accepted set, whatever insertion order chaos
        /// arrives in.
        #[test]
        fn greedy_acceptance_is_pairwise_disjoint(
            seeds in proptest::collection::vec((0i64..48, 1i64..12), 1..40),
        ) {
            let base = utc(2026, 5, 18, 0, 0);
            let spans: Vec<(DateTime<Utc>, DateTime<Utc>)> = seeds
                .iter()
                .map(|(start_h, dur_min)| {
                    let s = base + Duration::hours(*start_h) + Duration::minutes(start_h % 7);
                    (s, s + Duration::minutes(*dur_min))
                })
                .collect();
            let mut accepted: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();
            for &(s, e) in &spans {
                if conflict_indices(s, e, accepted.iter().copied()).is_empty() {
                    accepted.push((s, e));
                }
            }
            for i in 0..accepted.len() {
                for j in (i + 1)..accepted.len() {
                    prop_assert!(
                        !spans_overlap(accepted[i].0, accepted[i].1, accepted[j].0, accepted[j].1),
                        "accepted spans {:?} and {:?} overlap",
                        accepted[i], accepted[j]
                    );
                }
            }
        }

        /// Boundary law: a candidate placed FLUSH against an existing span
        /// never conflicts; starting even one second earlier does.
        #[test]
        fn touching_never_conflicts_but_any_intrusion_does(len_min in 1i64..=240, gap_min in 0i64..=600) {
            let base = utc(2026, 5, 18, 8, 0);
            let len = Duration::minutes(len_min);
            let existing = (base + Duration::minutes(gap_min), base + Duration::minutes(gap_min) + len);
            let flush = (existing.1, existing.1 + len);
            prop_assert!(conflict_indices(flush.0, flush.1, [existing]).is_empty());
            let intruding = (flush.0 - Duration::seconds(1), flush.1);
            prop_assert_eq!(conflict_indices(intruding.0, intruding.1, [existing]), vec![0]);
        }

        /// Coverage is bounded by elapsed minutes, monotone under added
        /// exclusions, and exact for round-the-clock windows.
        #[test]
        fn coverage_bounded_monotone_exact(
            days in 1i64..=5,
            excl_hours in proptest::collection::vec((0i64..=96, 1i64..=8), 0..6),
        ) {
            let u = uuid::Uuid::now_v7();
            let from = utc(2026, 5, 18, 0, 0);
            let to = from + Duration::days(days);
            let all_day = [win(u, 0, 0, 24), win(u, 1, 0, 24), win(u, 2, 0, 24), win(u, 3, 0, 24), win(u, 4, 0, 24), win(u, 5, 0, 24), win(u, 6, 0, 24)];
            prop_assert_eq!(covered_minutes(&all_day, &[], from, to), days * 24 * 60);

            let exs: Vec<CalendarExclusion> = excl_hours
                .iter()
                .map(|(h, l)| excl(from + Duration::hours(*h), from + Duration::hours(*h + l)))
                .collect();
            let with = covered_minutes(&all_day, &exs, from, to);
            prop_assert!(with <= days * 24 * 60);
            prop_assert!(with >= days * 24 * 60 - exs.iter().map(|x| (x.ends_at - x.starts_at).num_minutes()).sum::<i64>());
            prop_assert_eq!(covered_minutes(&all_day, &exs, from, to), with, "coverage deterministic");
        }

        /// Window containment vs occurrence enumeration agree at random
        /// instants: a minute counts as covered iff some window contains it.
        #[test]
        fn containment_matches_coverage_at_random_instants(
            offsets in proptest::collection::vec(0i64..=20160, 1..60),
        ) {
            let u = uuid::Uuid::now_v7();
            let monday = utc(2026, 5, 18, 0, 0);
            let windows = [win(u, 0, 9, 12), win(u, 2, 13, 17)];
            for off in offsets {
                let t = monday + Duration::minutes(off);
                let contained = windows.iter().any(|w| w.contains(t));
                let counted = covered_minutes(&windows, &[], t, t + Duration::minutes(1)) > 0;
                prop_assert_eq!(contained, counted, "minute {} mismatch", off);
            }
        }
    }
}
