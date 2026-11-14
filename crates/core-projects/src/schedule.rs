//! Working-calendar resolution for planned work.
//!
//! A date calculation needs one answer to "does this day count". The rule is
//! fixed by ADR 0034: a project's own working pattern wins where it exists,
//! the organisation's calendar is the base, and personal weekly hours scale
//! capacity without moving a date.
//!
//! This module owns the vocabulary. The reschedule engine, the ripple preview
//! and the capacity read models all resolve their calendars here so a single
//! change of rule reaches every surface.

/// A weekly working pattern.
///
/// `weekdays` is a bitmask with Monday at bit 0 and Sunday at bit 6, so the
/// standard Monday-to-Friday week is `0b001_1111`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkCalendar {
    /// Days of the week that are worked, Monday at bit 0.
    pub weekdays: u8,
    /// Minutes of work in one working day; lunch is already excluded.
    pub minutes_per_day: u32,
}

impl WorkCalendar {
    /// Monday to Friday, eight hours a day.
    pub const STANDARD: Self = Self {
        weekdays: 0b001_1111,
        minutes_per_day: 480,
    };

    /// Builds a pattern from its two numbers, saturating nothing: a mask with
    /// no working day is rejected by the caller, because a calendar that
    /// works never cannot schedule anything.
    pub const fn new(weekdays: u8, minutes_per_day: u32) -> Self {
        Self {
            weekdays,
            minutes_per_day,
        }
    }

    /// Whether the given weekday is worked. Monday is `0`, Sunday is `6`.
    pub fn works_on(&self, weekday: u8) -> bool {
        weekday < 7 && self.weekdays & (1 << weekday) != 0
    }

    /// Minutes of work in a full week of this pattern.
    pub fn weekly_minutes(&self) -> u32 {
        self.weekdays.count_ones() * self.minutes_per_day
    }

    /// Whether the pattern schedules anything at all.
    pub fn is_working(&self) -> bool {
        self.weekdays != 0 && self.minutes_per_day > 0
    }
}

/// Resolves the calendar a project's dates are computed against.
///
/// The project's own pattern wins where one is set; the organisation's
/// calendar is the fallback. Personal weekly hours are not consulted here:
/// they change how much a person can take on, never when a date lands.
pub fn resolve(project: Option<WorkCalendar>, org: WorkCalendar) -> WorkCalendar {
    project.unwrap_or(org)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_week_is_monday_to_friday() {
        let cal = WorkCalendar::STANDARD;
        assert!(cal.works_on(0));
        assert!(cal.works_on(4));
        assert!(!cal.works_on(5));
        assert!(!cal.works_on(6));
        assert_eq!(cal.weekly_minutes(), 2400);
        assert!(cal.is_working());
    }

    #[test]
    fn a_six_day_site_works_saturday() {
        let cal = WorkCalendar::new(0b011_1111, 480);
        assert!(cal.works_on(5));
        assert_eq!(cal.weekly_minutes(), 2880);
    }

    #[test]
    fn project_override_beats_the_org_calendar() {
        let org = WorkCalendar::STANDARD;
        let project = WorkCalendar::new(0b011_1111, 600);
        assert_eq!(resolve(Some(project), org), project);
        assert_eq!(resolve(None, org), org);
    }

    #[test]
    fn an_empty_mask_is_not_a_calendar() {
        assert!(!WorkCalendar::new(0, 480).is_working());
        assert!(!WorkCalendar::new(0b001_1111, 0).is_working());
    }

    #[test]
    fn weekday_index_out_of_range_is_not_worked() {
        assert!(!WorkCalendar::STANDARD.works_on(7));
    }
}
