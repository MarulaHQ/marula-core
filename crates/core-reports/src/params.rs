//! Parameter validation — pure, deterministically testable. Date windows
//! are day-granularity UTC ranges capped at 366 days; pagination is a
//! hard-clamped limit; the RBAC seam is an explicit scope enum supplied by
//! the caller (the API layer resolves it from the principal).

use crate::error::ReportsError;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use uuid::Uuid;

/// Hard page-size cap (matches the rest of core-api's keyset endpoints).
pub const PAGE_LIMIT_MAX: i64 = 500;
/// Default page size.
pub const PAGE_LIMIT_DEFAULT: i64 = 100;
/// Maximum inclusive calendar-day span of a report window (366 days).
pub const WINDOW_MAX_DAYS: i64 = 366;
/// Default window when neither bound is provided: trailing 30 days ending
/// "today" (UTC).
pub const WINDOW_DEFAULT_DAYS: i64 = 30;

/// Clamps a raw limit into `[1, PAGE_LIMIT_MAX]`. Out-of-band negatives or
/// absurd values degrade to bounds instead of erroring — matches the
/// `limit.clamp(1, 500)` convention elsewhere in core-api.
#[must_use]
pub fn clamp_limit(raw: Option<i64>) -> i64 {
    raw.unwrap_or(PAGE_LIMIT_DEFAULT).clamp(1, PAGE_LIMIT_MAX)
}

/// Explicitly-invalid limits (used by the API to return 422 rather than
/// silently clamp): zero/negative are treated as caller mistakes, anything
/// above the cap clamps quietly like every other endpoint.
pub fn validate_limit_raw(raw: i64) -> Result<(), ReportsError> {
    if raw <= 0 {
        Err(ReportsError::InvalidLimit(raw))
    } else {
        Ok(())
    }
}

/// A validated half-open UTC instant window `[start, end)` derived from
/// inclusive day inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub start: DateTime<Utc>,
    /// Exclusive upper bound (start of the day AFTER `to`).
    pub end: DateTime<Utc>,
}

impl Window {
    #[must_use]
    pub fn span_days(&self) -> i64 {
        (self.end - self.start).num_days()
    }
}

/// Resolves the report window from optional inclusive date bounds.
///
/// Contract:
/// * Both `None` ⇒ trailing [`WINDOW_DEFAULT_DAYS`] days ending at
///   `now`'s calendar day (inclusive).
/// * One bound missing ⇒ the other collapses it to that single day.
/// * Inclusive span `(to - from) + 1` must be ≤ [`WINDOW_MAX_DAYS`] —
///   anything wider is rejected (`ReportsError::InvalidWindow`), never
///   clamped, so callers never silently receive misread data.
///
/// # Errors
/// [`ReportsError::InvalidWindow`] on inverted ranges and over-wide spans.
pub fn resolve_window(
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    now: DateTime<Utc>,
) -> Result<Window, ReportsError> {
    let today = now.date_naive();
    let (from, to) = match (from, to) {
        (None, None) => (today - Duration::days(WINDOW_DEFAULT_DAYS - 1), today),
        (Some(f), None) => (f, f),
        (None, Some(t)) => (t, t),
        (Some(f), Some(t)) => (f, t),
    };
    if to < from {
        return Err(ReportsError::InvalidWindow(format!(
            "to ({to}) must not precede from ({from})"
        )));
    }
    let span = (to - from).num_days() + 1;
    if span > WINDOW_MAX_DAYS {
        return Err(ReportsError::InvalidWindow(format!(
            "window spans {span} days; maximum is {WINDOW_MAX_DAYS}"
        )));
    }
    let start = from
        .and_hms_opt(0, 0, 0)
        .map(|n| n.and_utc())
        .ok_or_else(|| ReportsError::InvalidWindow("unrepresentable from-date".to_string()))?;
    let end_exclusive = (to + Duration::days(1))
        .and_hms_opt(0, 0, 0)
        .map(|n| n.and_utc())
        .ok_or_else(|| ReportsError::InvalidWindow("unrepresentable to-date".to_string()))?;
    Ok(Window {
        start,
        end: end_exclusive,
    })
}

/// Who the rows may cover. The API layer derives this from the principal:
/// managers get [`Scope::OrgWide`] and MAY pass an explicit user narrowing;
/// everyone else is forced onto themselves regardless of query input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    OrgWide,
    Self_(Uuid),
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use proptest::prelude::*;

    fn anchor() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 15, 12, 0, 0).unwrap()
    }

    #[test]
    fn defaults_cover_trailing_30_days_inclusive() {
        let w = resolve_window(None, None, anchor()).unwrap();
        assert_eq!(w.span_days(), WINDOW_DEFAULT_DAYS);
        assert_eq!(
            w.end.date_naive(),
            anchor().date_naive() + Duration::days(1)
        );
        assert_eq!(
            w.start.date_naive(),
            anchor().date_naive() - Duration::days(WINDOW_DEFAULT_DAYS - 1)
        );
    }

    #[test]
    fn single_sided_bounds_collapse_to_one_day() {
        let d = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let w = resolve_window(Some(d), None, anchor()).unwrap();
        assert_eq!(w.span_days(), 1);
        assert_eq!(w.start.date_naive(), d);
        let w = resolve_window(None, Some(d), anchor()).unwrap();
        assert_eq!(w.span_days(), 1);
        assert_eq!(w.start.date_naive(), d);
    }

    #[test]
    fn inverted_range_is_rejected_not_swapped() {
        let a = NaiveDate::from_ymd_opt(2026, 3, 5).unwrap();
        let b = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let err = resolve_window(Some(a), Some(b), anchor()).unwrap_err();
        assert!(matches!(err, ReportsError::InvalidWindow(_)));
    }

    proptest! {
        /// Roundtrip property: any accepted window re-parses to identical
        /// bounds, spans ≤ 366 days, and end > start.
        #[test]
        fn accepted_windows_are_stable_and_bounded(
            y in 2000i32..2100,
            m in 1u32..=12,
            d_from in 1u32..=28,
            delta in 0i64..=365,
        ) {
            let from = NaiveDate::from_ymd_opt(y, m, d_from).unwrap();
            let to = from + Duration::days(delta);
            let w = resolve_window(Some(from), Some(to), anchor())?;
            prop_assert_eq!(w.start.date_naive(), from);
            prop_assert_eq!(w.span_days(), delta + 1);
            prop_assert_eq!(w.span_days() <= WINDOW_MAX_DAYS, true);
            prop_assert!(w.end > w.start);
        }

        /// Any string-ish garbage or wide spans beyond 366 days must fail —
        /// including leap-day edge arithmetic.
        #[test]
        fn windows_wider_than_cap_fail(delta in 367i64..=4000) {
            let from = NaiveDate::from_ymd_opt(2024, 2, 28).unwrap();
            let to = from + Duration::days(delta);
            let err = resolve_window(Some(from), Some(to), anchor());
            prop_assert!(err.is_err());
        }
    }

    proptest! {
        /// Limit semantics pinned by property: `None` → default; positives
        /// stay within [1, cap]; only non-positives are caller errors.
        #[test]
        fn clamp_limit_respects_bounds(raw in proptest::option::of(any::<i64>())) {
            let v = clamp_limit(raw);
            prop_assert!((1..=PAGE_LIMIT_MAX).contains(&v));
        }

        #[test]
        fn non_positive_limits_error(raw in -(1i64)..=0) {
            prop_assert!(validate_limit_raw(raw).is_err());
        }
    }

    #[test]
    fn positive_limits_pass_validation() {
        for v in [1, 50, PAGE_LIMIT_MAX, 10_000] {
            assert!(validate_limit_raw(v).is_ok());
        }
    }
}
