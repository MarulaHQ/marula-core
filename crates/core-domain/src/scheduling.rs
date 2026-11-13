//! Scheduling rules — the pure decision layer for calendars/availability
//! (the private build tracker 2.12).
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

/// Does this local end bound name the END OF THE LOCAL DAY rather than a
/// wall-clock instant an operator picked?
///
/// `24:00` is not representable, so "through midnight" gets written two
/// ways: the largest representable instant ([`end_of_day`]), and the last
/// whole second, `23:59:59` — which is what a direct insert or a seed
/// naturally writes (the `%H:%M` write path cannot reach past `23:59:00`,
/// and the `time` column's microsecond resolution cannot hold the sentinel's
/// nanos). Both mean the same schedule, so both round up to the next local
/// midnight in [`window_occurrence`]; otherwise every day's block would end
/// in a one-second hole, and a booking that crossed midnight would fit in no
/// single free interval.
///
/// The tolerance is exactly the final second — the resolution at which
/// `23:59:59` and `24:00` are indistinguishable to a human and where the
/// write path effectively bottoms out. Anything coarser (`23:59:58`, `17:00`)
/// keeps its own wall-clock meaning, so an ordinary window is never widened
/// into a round-the-clock one. This governs day arithmetic only: the
/// window's own half-open containment (`AvailabilityWindow::contains`) still
/// stops at the stored bound.
fn ends_at_end_of_day(end_local: chrono::NaiveTime) -> bool {
    end_of_day().signed_duration_since(end_local) < chrono::Duration::seconds(1)
}

/// Resolves the window's UTC occurrence(s) on local day `day`, clipping to
/// `[from, to]`. Fall-back ambiguity resolves like the SLA calendar: first
/// occurrence; spring-forward gaps contribute nothing rather than
/// inventing time. An end-of-day bound ([`ends_at_end_of_day`]) rounds up to
/// the next local midnight so round-the-clock windows are exactly 24h and
/// adjacent days abut exactly — two all-day windows behave as one continuous
/// block, which is what lets a booking cross midnight. Overnight windows are
/// not expressible as templates — `start_local < end_local` is enforced
/// upstream.
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
    let through_midnight = ends_at_end_of_day(window.end_local);
    let end_naive_day = if through_midnight {
        day.succ_opt()?
    } else {
        day
    };
    let end_naive_time = if through_midnight {
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
    available_intervals(windows, exclusions, from, to)
        .iter()
        .map(|(s, e)| (*e - *s).num_seconds())
        .sum::<i64>()
        / 60
}

/// Merged availability coverage of `[from, to]`: union of all
/// availability-window occurrences minus union of exclusions. Returns
/// sorted, internally disjoint intervals (Portal v2 slot computation is
/// the second consumer — the returned shape IS the bookable backdrop).
pub fn available_intervals(
    windows: &[AvailabilityWindow],
    exclusions: &[CalendarExclusion],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    if to <= from {
        return Vec::new();
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
    subtract_intervals(&covered, &blocked)
}

/// The bookable gaps for one staff member: availability coverage minus
/// exclusions ([`available_intervals`]) minus already-occupied spans
/// (live appointments, on-call blocks). All inputs are absolute UTC;
/// output is sorted and internally disjoint, so each interval is a
/// contiguous FREE stretch a client may request a slice of.
pub fn free_intervals(
    windows: &[AvailabilityWindow],
    exclusions: &[CalendarExclusion],
    busy: &[(DateTime<Utc>, DateTime<Utc>)],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let available = available_intervals(windows, exclusions, from, to);
    let occupied = merge_intervals(
        busy.iter()
            .filter_map(|&(s, e)| {
                let s = s.max(from);
                let e = e.min(to);
                (e > s).then_some((s, e))
            })
            .collect(),
    );
    subtract_intervals(&available, &occupied)
}

/// Subtracts merged, internally disjoint `remove` spans from merged,
/// internally disjoint `base` spans in one forward sweep with a
/// never-backwards pointer into `remove`.
fn subtract_intervals(
    base: &[(DateTime<Utc>, DateTime<Utc>)],
    remove: &[(DateTime<Utc>, DateTime<Utc>)],
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let mut out = Vec::new();
    let mut j = 0_usize;
    for &(bs, be) in base {
        let mut cursor = bs;
        while j < remove.len() && remove[j].1 <= cursor {
            j += 1;
        }
        let mut k = j;
        while k < remove.len() && remove[k].0 < be {
            if remove[k].0 > cursor {
                out.push((cursor, remove[k].0));
            }
            cursor = cursor.max(remove[k].1);
            k += 1;
        }
        if cursor < be {
            out.push((cursor, be));
        }
    }
    out
}

/// Booked minutes for a dispatch day: the union of LIVE appointment spans
/// clipped to `[from, to]`, merged so touching/stacked bookings never
/// double-count. Cancelled bookings contribute nothing.
///
/// This is the travel-FREE primitive. Dispatch capacity uses
/// [`occupied_minutes`]: once the RoutingProvider lane lands, travel is
/// occupied capacity the same as the booking itself.
pub fn booked_minutes<I>(appointments: I, from: DateTime<Utc>, to: DateTime<Utc>) -> i64
where
    I: IntoIterator<Item = (DateTime<Utc>, DateTime<Utc>)>,
{
    minutes_from_spans(appointments.into_iter().collect(), from, to)
}

/// Occupied minutes for a dispatch day (Wave 32 lane 7 doctrine): the
/// union of every live booking's **travel block + appointment span**,
/// clipped to `[from, to]` and merged so travel never double-counts
/// against the appointment it precedes (nor against a back-to-back
/// booking that already starts when the previous ends).
///
/// Travel occupies `[starts_at - travel_minutes, starts_at)` — the
/// appointment's own time is never reduced ("the booked travel time ADDS
/// to the appointment time, never replaces it"). Negative travel is
/// clamped to zero; cancelled bookings are filtered by the caller.
pub fn occupied_minutes<I>(appointments: I, from: DateTime<Utc>, to: DateTime<Utc>) -> i64
where
    I: IntoIterator<Item = (DateTime<Utc>, DateTime<Utc>, i64)>,
{
    minutes_from_spans(
        appointments
            .into_iter()
            .map(|(s, e, travel_minutes)| {
                let travel = travel_minutes.max(0);
                (s - chrono::Duration::minutes(travel), e)
            })
            .collect(),
        from,
        to,
    )
}

/// Shared clipping + merging + minute summation for booked/occupied math.
fn minutes_from_spans(
    spans: Vec<(DateTime<Utc>, DateTime<Utc>)>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> i64 {
    let clipped: Vec<(DateTime<Utc>, DateTime<Utc>)> = spans
        .into_iter()
        .filter_map(|(s, e)| {
            let s = s.max(from);
            let e = e.min(to);
            (e > s).then_some((s, e))
        })
        .collect();
    merge_intervals(clipped)
        .iter()
        .map(|(s, e)| (*e - *s).num_seconds())
        .sum::<i64>()
        / 60
}

/// Dispatch capacity utilization as 0–100+ (over-capacity days exceed
/// 100 when bookings outstrip availability). Zero availability is not a
/// division error: an off-day technician is 0% utilized with no bookings,
/// and over-capacity-on-nothing is impossible, so the answer is 0.
pub fn capacity_percent(booked: i64, available: i64) -> i64 {
    if available <= 0 {
        return 0;
    }
    booked * 100 / available
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
    fn booked_minutes_merges_and_clips_and_skips_cancelled() {
        let day = (utc(2026, 6, 1, 0, 0), utc(2026, 6, 2, 0, 0));
        let jun1 = |h: u32, m: u32| utc(2026, 6, 1, h, m);
        // Touching 09–10 + 10–11 unions to 120, not 120+; a span poking
        // outside the day clips; zero-length contributes nothing.
        let spans = [
            (jun1(9, 0), jun1(10, 0)),
            (jun1(10, 0), jun1(11, 0)),
            (utc(2026, 5, 31, 23, 0), jun1(0, 30)),
            (jun1(13, 0), jun1(13, 0)),
        ];
        assert_eq!(booked_minutes(spans, day.0, day.1), 150);
        assert_eq!(booked_minutes([], day.0, day.1), 0);
    }

    // --- adjacent all-day windows behave as one continuous block -----------

    /// A window with explicit wall-clock bounds, seconds included: the shape
    /// a direct insert or a seed writes (`23:59:59`), as opposed to the
    /// `eh == 24` sentinel [`win`] builds.
    fn win_at(
        user: uuid::Uuid,
        weekday: i16,
        start: (u32, u32, u32),
        end: (u32, u32, u32),
    ) -> AvailabilityWindow {
        let at = |(h, m, s): (u32, u32, u32)| chrono::NaiveTime::from_hms_opt(h, m, s).unwrap();
        AvailabilityWindow {
            id: uuid::Uuid::now_v7(),
            org_id: uuid::Uuid::now_v7(),
            user_id: user,
            weekday,
            start_local: at(start),
            end_local: at(end),
            timezone: "UTC".into(),
            effective_from: None,
            effective_until: None,
            created_at: utc(2026, 1, 1, 0, 0),
        }
    }

    /// A booking may cross midnight when the windows on both sides cover it.
    /// Two days written 00:00–23:59:59 — the way a seed says "all day", and
    /// NOT the `23:59:59.999999999` sentinel — are one continuous block, so a
    /// 30-minute slot at 23:56 sits inside a single free interval instead of
    /// straddling a one-second hole.
    #[test]
    fn cross_midnight_booking_inside_two_all_day_windows_is_free() {
        let u = uuid::Uuid::now_v7();
        // 2026-05-18 is a Monday; the pair is Monday then Tuesday.
        let windows = [
            win_at(u, 0, (0, 0, 0), (23, 59, 59)),
            win_at(u, 1, (0, 0, 0), (23, 59, 59)),
        ];
        let starts_at = utc(2026, 5, 18, 23, 56);
        let ends_at = starts_at + Duration::minutes(30);

        let free = free_intervals(&windows, &[], &[], starts_at, ends_at);
        assert!(
            free.iter().any(|(s, e)| *s <= starts_at && ends_at <= *e),
            "23:56–00:26 must fit one free interval, got {free:?}"
        );

        // The two days are one block: 48 hours of coverage, no seam.
        let mins = covered_minutes(
            &windows,
            &[],
            utc(2026, 5, 18, 0, 0),
            utc(2026, 5, 20, 0, 0),
        );
        assert_eq!(mins, 48 * 60);
    }

    /// The tolerance is the final second only: a booking that runs past a
    /// real 09:00–17:00 window is still refused, and one that ends flush with
    /// the close is still fine.
    #[test]
    fn booking_past_a_real_window_end_is_still_refused() {
        let u = uuid::Uuid::now_v7();
        let windows = [win(u, 0, 9, 17)];
        let close = utc(2026, 5, 18, 17, 0);

        let starts_at = utc(2026, 5, 18, 16, 45);
        let ends_at = starts_at + Duration::minutes(30);
        let free = free_intervals(&windows, &[], &[], starts_at, ends_at);
        assert_eq!(free, vec![(starts_at, close)], "coverage stops at 17:00");
        assert!(!free.iter().any(|(s, e)| *s <= starts_at && ends_at <= *e));

        let flush = utc(2026, 5, 18, 16, 30);
        let ok = free_intervals(&windows, &[], &[], flush, close);
        assert!(
            ok.iter().any(|(s, e)| *s <= flush && close <= *e),
            "16:30–17:00 fits exactly, got {ok:?}"
        );
    }

    /// A closed lunch hour is a real gap, not a rounding artefact: two
    /// windows with 12:00–13:00 closed are never merged.
    #[test]
    fn a_real_gap_between_windows_is_not_merged() {
        let u = uuid::Uuid::now_v7();
        let windows = [win(u, 0, 9, 12), win(u, 0, 13, 17)];
        let day = (utc(2026, 5, 18, 0, 0), utc(2026, 5, 19, 0, 0));
        assert_eq!(
            available_intervals(&windows, &[], day.0, day.1),
            vec![
                (utc(2026, 5, 18, 9, 0), utc(2026, 5, 18, 12, 0)),
                (utc(2026, 5, 18, 13, 0), utc(2026, 5, 18, 17, 0)),
            ]
        );

        let starts_at = utc(2026, 5, 18, 11, 45);
        let ends_at = starts_at + Duration::minutes(90); // straight through lunch
        let free = free_intervals(&windows, &[], &[], starts_at, ends_at);
        assert!(!free.iter().any(|(s, e)| *s <= starts_at && ends_at <= *e));
    }

    /// The edge of the tolerance: 23:59:58 is an ordinary bound, so it keeps
    /// its own end and the day keeps its hole instead of becoming 24 hours.
    #[test]
    fn a_bound_one_second_short_of_midnight_is_not_end_of_day() {
        let u = uuid::Uuid::now_v7();
        let windows = [
            win_at(u, 0, (0, 0, 0), (23, 59, 58)),
            win_at(u, 1, (0, 0, 0), (23, 59, 58)),
        ];
        assert_eq!(
            covered_minutes(
                &windows,
                &[],
                utc(2026, 5, 18, 0, 0),
                utc(2026, 5, 20, 0, 0)
            ),
            2_879,
            "48h minus two two-second holes, floored to minutes"
        );

        let starts_at = utc(2026, 5, 18, 23, 57);
        let ends_at = starts_at + Duration::minutes(5);
        let free = free_intervals(&windows, &[], &[], starts_at, ends_at);
        assert!(!free.iter().any(|(s, e)| *s <= starts_at && ends_at <= *e));
    }

    #[test]
    fn capacity_percent_bounds_and_over_capacity() {
        assert_eq!(capacity_percent(0, 480), 0);
        assert_eq!(capacity_percent(240, 480), 50);
        assert_eq!(capacity_percent(480, 480), 100);
        // Overbooked against availability exceeds 100 (the warning state).
        assert_eq!(capacity_percent(540, 480), 112);
        // No availability (off day): 0 regardless of bookings.
        assert_eq!(capacity_percent(120, 0), 0);
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

    #[test]
    fn occupied_minutes_counts_travel_but_never_double_counts() {
        let from = utc(2026, 5, 18, 0, 0);
        let to = from + Duration::days(1);

        // One 60-minute booking with 30 minutes of travel: 90 occupied.
        let appt = utc(2026, 5, 18, 9, 0);
        let end = appt + Duration::minutes(60);
        assert_eq!(occupied_minutes([(appt, end, 30)], from, to), 90);

        // A back-to-back booking at 10:00 (the previous ends 10:00) with
        // its own 30 minutes of travel: the union is 08:30-11:00 and the
        // overlapping minutes between the two travel blocks count ONCE
        // (150, not 90 + 90).
        let second = utc(2026, 5, 18, 10, 0);
        let second_end = second + Duration::minutes(60);
        assert_eq!(
            occupied_minutes([(appt, end, 30), (second, second_end, 30)], from, to),
            150
        );

        // Zero/negative travel degrades to the plain booked span.
        assert_eq!(occupied_minutes([(appt, end, 0)], from, to), 60);
        assert_eq!(occupied_minutes([(appt, end, -15)], from, to), 60);
        assert_eq!(booked_minutes([(appt, end)], from, to), 60);

        // Travel clipped at the window edge: a booking at 00:10 with
        // 30 minutes of travel reaches back to 23:40 the previous day, so
        // only its 10 in-day travel minutes plus the 20-minute booking
        // occupy this day.
        let early = utc(2026, 5, 18, 0, 10);
        let early_end = early + Duration::minutes(20);
        assert_eq!(occupied_minutes([(early, early_end, 30)], from, to), 30);
    }
}
