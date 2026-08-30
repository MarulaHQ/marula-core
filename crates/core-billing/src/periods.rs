//! Billing period arithmetic: deterministic period windows derived from a
//! contract's `billing_anchor` + `billing_frequency`.
//!
//! The anchor is the FIRST period's start date. Every later period start is
//! computed from the anchor (index × frequency step), never by chaining
//! additions, so a month-end anchor cannot drift: `2026-01-31` monthly
//! yields `2026-01-31`, `2026-02-28`, `2026-03-31`, `2026-04-30`, … with
//! every month-end clamped to real month length and no accumulated error.
//! Periods are half-open on the date line: a period starting `s` runs to
//! (but not including) the next period's start.

use chrono::{Datelike, NaiveDate};
use core_domain::entities::BillingFrequency;

/// Upper bound on generated periods per evaluation; a defensive stop so a
/// pathological input can never spin (unreachable with valid dates: 366-day
/// windows generate at most a few hundred weekly periods).
const MAX_PERIODS: u32 = 5_000;

/// Adds `months` calendar months to `date`, clamping the day-of-month to
/// the target month's length.
fn add_months(date: NaiveDate, months: u32) -> NaiveDate {
    let total = date.year() * 12 + (date.month0() as i32) + months as i32;
    let year = total.div_euclid(12);
    let month0 = total.rem_euclid(12);
    let day = date.day();
    let days_in_month = days_in_month(year, month0 as u32 + 1);
    NaiveDate::from_ymd_opt(year, month0 as u32 + 1, day.min(days_in_month))
        .expect("clamped y/m/d is always a valid date")
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(year) {
                29
            } else {
                28
            }
        }
        _ => unreachable!("month0+1 is always 1..=12"),
    }
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// The start date of the `index`-th period (0-based) after `anchor`.
pub fn period_start(
    anchor: chrono::NaiveDate,
    frequency: BillingFrequency,
    index: u32,
) -> chrono::NaiveDate {
    match frequency {
        BillingFrequency::Weekly => anchor + chrono::Duration::weeks(index as i64),
        other => add_months(anchor, other.months() * index),
    }
}

/// Every billing period whose START falls inside the inclusive window
/// `[from, to]`, as `(start, exclusive_end)` pairs in ascending order.
///
/// Start-gated on purpose: an invoice run evaluating January bills the
/// period starting Jan 15, and February's run bills Feb 15's — adjacent
/// windows never double-count a period, and a period that merely overlaps
/// the window boundary belongs to the run whose window contains its start.
pub fn periods_starting_in(
    anchor: chrono::NaiveDate,
    frequency: BillingFrequency,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
) -> Vec<(chrono::NaiveDate, chrono::NaiveDate)> {
    let mut out = Vec::new();
    if to < from {
        return out;
    }
    let mut index: u32 = 0;
    while index < MAX_PERIODS {
        let start = period_start(anchor, frequency, index);
        if start > to {
            break;
        }
        if start >= from {
            let end = period_start(anchor, frequency, index + 1);
            out.push((start, end));
        }
        index += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn month_end_anchor_clamps_without_drift() {
        let anchor = d(2026, 1, 31);
        assert_eq!(
            period_start(anchor, BillingFrequency::Monthly, 1),
            d(2026, 2, 28)
        );
        assert_eq!(
            period_start(anchor, BillingFrequency::Monthly, 2),
            d(2026, 3, 31)
        );
        assert_eq!(
            period_start(anchor, BillingFrequency::Monthly, 3),
            d(2026, 4, 30)
        );
        // Computed from the anchor, not chained: month 13 = next Jan 31.
        assert_eq!(
            period_start(anchor, BillingFrequency::Monthly, 12),
            d(2027, 1, 31)
        );
    }

    #[test]
    fn weekly_steps_by_seven_days() {
        let anchor = d(2026, 3, 2);
        assert_eq!(
            period_start(anchor, BillingFrequency::Weekly, 4),
            d(2026, 3, 30)
        );
    }

    #[test]
    fn quarterly_and_annual_steps() {
        let anchor = d(2026, 2, 15);
        assert_eq!(
            period_start(anchor, BillingFrequency::Quarterly, 1),
            d(2026, 5, 15)
        );
        assert_eq!(
            period_start(anchor, BillingFrequency::Semiannual, 1),
            d(2026, 8, 15)
        );
        assert_eq!(
            period_start(anchor, BillingFrequency::Annual, 1),
            d(2027, 2, 15)
        );
    }

    #[test]
    fn leap_day_clamps() {
        let anchor = d(2024, 2, 29);
        assert_eq!(
            period_start(anchor, BillingFrequency::Annual, 1),
            d(2025, 2, 28)
        );
        assert_eq!(
            period_start(anchor, BillingFrequency::Annual, 4),
            d(2028, 2, 29)
        );
    }

    #[test]
    fn periods_starting_inside_window_are_ascending_and_half_open() {
        let anchor = d(2026, 1, 15);
        let periods = periods_starting_in(
            anchor,
            BillingFrequency::Monthly,
            d(2026, 2, 1),
            d(2026, 3, 20),
        );
        // The Jan-15 period STARTS before the window: it belongs to
        // January's run, not this one.
        assert_eq!(
            periods,
            vec![
                (d(2026, 2, 15), d(2026, 3, 15)),
                (d(2026, 3, 15), d(2026, 4, 15))
            ]
        );
    }

    #[test]
    fn period_starting_exactly_on_window_edges_is_included() {
        let anchor = d(2026, 1, 15);
        let periods = periods_starting_in(
            anchor,
            BillingFrequency::Monthly,
            d(2026, 2, 15),
            d(2026, 2, 15),
        );
        assert_eq!(periods, vec![(d(2026, 2, 15), d(2026, 3, 15))]);
    }

    #[test]
    fn window_entirely_outside_anchor_span_yields_nothing() {
        // Window fully before the anchor: no period has started yet.
        let anchor = d(2026, 6, 1);
        assert!(periods_starting_in(
            anchor,
            BillingFrequency::Monthly,
            d(2026, 1, 1),
            d(2026, 5, 31)
        )
        .is_empty());
        // Window between the anchor and the anchor's month boundary: the
        // first period starts ON the anchor, so this gap yields nothing.
        let mid_month = d(2026, 6, 15);
        assert!(periods_starting_in(
            mid_month,
            BillingFrequency::Monthly,
            d(2026, 6, 1),
            d(2026, 6, 14)
        )
        .is_empty());
    }

    #[test]
    fn inverted_window_yields_nothing() {
        let anchor = d(2026, 1, 1);
        assert!(periods_starting_in(
            anchor,
            BillingFrequency::Monthly,
            d(2026, 5, 1),
            d(2026, 4, 1)
        )
        .is_empty());
    }
}
