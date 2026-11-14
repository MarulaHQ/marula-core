//! Capacity: how much planned work a person can carry in a period.
//!
//! Capacity is a read of three inputs and nothing else: the calendar a
//! project resolves to ([`crate::schedule::resolve`]), the person's weekly
//! hours, and the absences and travel already recorded against them. A
//! part-timer is a weekly-hour figure, not a different calendar, which keeps
//! one rule for dates and one rule for load.

use crate::schedule::WorkCalendar;

/// A person's contracted week, in minutes.
///
/// `None` means "whatever the calendar says", which is how a full-timer is
/// recorded: no second number to keep in sync.
pub type WeeklyMinutes = Option<u32>;

/// Minutes a person can work in a week of this calendar.
///
/// A stated weekly figure is capped by the calendar's own week: a person
/// cannot be available on days the calendar does not work.
pub fn weekly_capacity(calendar: WorkCalendar, stated: WeeklyMinutes) -> u32 {
    let full = calendar.weekly_minutes();
    match stated {
        Some(minutes) => minutes.min(full),
        None => full,
    }
}

/// Share of the week a person is contracted for, as a percentage.
///
/// Used where a surface shows load rather than minutes: the team schedule
/// header and the resource-match explanation.
pub fn utilisation_share(calendar: WorkCalendar, stated: WeeklyMinutes) -> u8 {
    let full = calendar.weekly_minutes();
    if full == 0 {
        return 0;
    }
    let taken = weekly_capacity(calendar, stated);
    ((u64::from(taken) * 100 + u64::from(full) / 2) / u64::from(full)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn std_cal() -> WorkCalendar {
        WorkCalendar::STANDARD
    }

    #[test]
    fn a_full_timer_is_the_whole_calendar_week() {
        assert_eq!(weekly_capacity(std_cal(), None), 2400);
        assert_eq!(utilisation_share(std_cal(), None), 100);
    }

    #[test]
    fn a_part_timer_is_a_weekly_figure() {
        // 25 hours a week of a 40-hour week: 62.5%, rounded to 63.
        assert_eq!(weekly_capacity(std_cal(), Some(1500)), 1500);
        assert_eq!(utilisation_share(std_cal(), Some(1500)), 63);
    }

    #[test]
    fn a_stated_week_is_capped_by_the_calendar() {
        assert_eq!(weekly_capacity(std_cal(), Some(6000)), 2400);
    }

    #[test]
    fn a_zero_length_calendar_has_no_share() {
        assert_eq!(utilisation_share(WorkCalendar::new(0, 0), None), 0);
    }
}
