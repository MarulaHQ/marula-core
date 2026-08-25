//! SLA policies, business calendars, and per-ticket SLA clock state.
//! Calendar math is pure and heavily tested: business hours, holidays,
//! and category-driven pause states all bind here (Stage 2 engine).

use crate::error::DomainError;
use crate::ids::{OrgId, SlaPolicyId, TicketId};
use crate::status::{Priority, TicketStatusCategory};
use chrono::{DateTime, Datelike, Duration, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// Weekly business hours in a named IANA timezone, plus holiday dates.
/// `None` window = non-business day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BusinessCalendar {
    pub name: String,
    pub timezone: String,
    /// [Mon..Sun] open/close in local wall-clock "HH:MM".
    pub windows: [Option<(String, String)>; 7],
    /// Local dates (YYYY-MM-DD) with no business hours.
    pub holidays: Vec<String>,
}

impl BusinessCalendar {
    pub fn parse_window(w: &(String, String)) -> Result<(NaiveTime, NaiveTime), DomainError> {
        let parse = |s: &str| {
            let (h, m) = s
                .split_once(':')
                .ok_or_else(|| DomainError::Validation(format!("bad time {s}")))?;
            let h: u32 = h.parse().map_err(|_| DomainError::Validation(format!("bad time {s}")))?;
            let m: u32 = m.parse().map_err(|_| DomainError::Validation(format!("bad time {s}")))?;
            NaiveTime::from_hms_opt(h, m, 0).ok_or_else(|| DomainError::Validation(format!("bad time {s}")))
        };
        Ok((parse(&w.0)?, parse(&w.1)?))
    }

    fn tz(&self) -> chrono_tz::Tz {
        self.timezone.parse().unwrap_or(chrono_tz::UTC)
    }

    /// Business-hours window for the local date of `at`, if any.
    #[cfg(test)]
    fn window_on(&self, at: DateTime<Utc>) -> Option<(NaiveTime, NaiveTime)> {
        let tz = self.tz();
        let local = at.with_timezone(&tz);
        let date = local.date_naive();
        if self.holidays.iter().any(|h| h.as_str() == date.to_string()) {
            return None;
        }
        let idx = local.weekday().num_days_from_monday() as usize;
        self.windows.get(idx).and_then(|w| w.as_ref()).and_then(|w| Self::parse_window(w).ok())
    }

    /// Seconds of business time elapsed between two instants.
    pub fn elapsed_business_secs(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> i64 {
        if to <= from {
            return 0;
        }
        let tz = self.tz();
        let mut total = 0_i64;
        // Walk local days; bounded by calendar span, safe for multi-year spans.
        let mut day = from.with_timezone(&tz).date_naive();
        let last_day = to.with_timezone(&tz).date_naive();
        while day <= last_day {
            let local_date = day;
            let is_holiday = self.holidays.iter().any(|h| h.as_str() == local_date.to_string());
            let idx = local_date.weekday().num_days_from_monday() as usize;
            let window = if is_holiday {
                None
            } else {
                self.windows.get(idx).and_then(|w| w.as_ref()).and_then(|w| Self::parse_window(w).ok())
            };
            if let Some((open, close)) = window {
                let day_start = tz
                    .from_local_datetime(&local_date.and_time(open))
                    .earliest()
                    .map(|d| d.with_timezone(&Utc));
                let day_end = tz
                    .from_local_datetime(&local_date.and_time(close))
                    .earliest()
                    .map(|d| d.with_timezone(&Utc));
                if let (Some(ws), Some(we)) = (day_start, day_end) {
                    let s = from.max(ws);
                    let e = to.min(we);
                    if e > s {
                        total += (e - s).num_seconds();
                    }
                }
            }
            day = day.succ_opt().unwrap_or(day);
            if day == local_date {
                break; // defensive: never loop on succ_opt failure
            }
        }
        total
    }

    /// The instant `business_secs` of business time after `from`, skipping
    /// nights/weekends/holidays. Caps iteration at 400 days.
    pub fn advance_business(&self, from: DateTime<Utc>, business_secs: i64) -> DateTime<Utc> {
        if business_secs <= 0 {
            return from;
        }
        let tz = self.tz();
        let mut remaining = business_secs;
        let mut day = from.with_timezone(&tz).date_naive();
        let mut cursor: Option<DateTime<Utc>> = None;
        for _ in 0..400 {
            let is_holiday = self.holidays.iter().any(|h| h.as_str() == day.to_string());
            let idx = day.weekday().num_days_from_monday() as usize;
            let window = if is_holiday {
                None
            } else {
                self.windows.get(idx).and_then(|w| w.as_ref()).and_then(|w| Self::parse_window(w).ok())
            };
            if let Some((open, close)) = window {
                let ws = tz
                    .from_local_datetime(&day.and_time(open))
                    .earliest()
                    .map(|d| d.with_timezone(&Utc));
                let we = tz
                    .from_local_datetime(&day.and_time(close))
                    .earliest()
                    .map(|d| d.with_timezone(&Utc));
                if let (Some(ws), Some(we)) = (ws, we) {
                    let start = cursor.unwrap_or(from).max(ws);
                    if start < we {
                        let avail = (we - start).num_seconds();
                        if avail >= remaining {
                            return start + Duration::seconds(remaining);
                        }
                        remaining -= avail;
                    }
                    cursor = Some(we);
                }
            }
            match day.succ_opt() {
                Some(d) => day = d,
                None => break,
            }
        }
        cursor.unwrap_or(from)
    }
}

/// SLA targets per priority, in business minutes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SlaTargets {
    pub first_response_mins: i64,
    pub resolution_mins: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SlaPolicy {
    pub id: SlaPolicyId,
    pub org_id: OrgId,
    pub name: String,
    pub is_default: bool,
    pub targets: std::collections::HashMap<Priority, SlaTargets>,
    pub calendar: BusinessCalendar,
}

/// Per-ticket SLA clock state. Accumulates paused business seconds so a
/// Waiting stretch does not consume the resolution budget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketSlaState {
    pub org_id: OrgId,
    pub ticket_id: TicketId,
    pub policy_id: SlaPolicyId,
    pub first_response_due: Option<DateTime<Utc>>,
    pub resolution_due: Option<DateTime<Utc>>,
    pub first_responded_at: Option<DateTime<Utc>>,
    /// Business seconds accumulated while the clock was paused.
    pub paused_business_secs: i64,
    /// When the current pause began (None while running).
    pub paused_since: Option<DateTime<Utc>>,
    pub breached: bool,
}

impl TicketSlaState {
    /// Recomputes due dates from ticket creation under a policy + calendar.
    pub fn init(
        org_id: OrgId,
        ticket_id: TicketId,
        policy: &SlaPolicy,
        priority: Priority,
        created_at: DateTime<Utc>,
    ) -> Self {
        let calendar = &policy.calendar;
        let targets = policy.targets.get(&priority);
        let resolution_due = targets.map(|t| {
            calendar.advance_business(created_at, t.resolution_mins * 60)
        });
        let first_response_due = targets
            .map(|t| calendar.advance_business(created_at, t.first_response_mins * 60));
        Self {
            org_id,
            ticket_id,
            policy_id: policy.id,
            first_response_due,
            resolution_due,
            first_responded_at: None,
            paused_business_secs: 0,
            paused_since: None,
            breached: false,
        }
    }

    /// Applies a category change: entering Waiting starts the pause clock,
    /// leaving it banks the elapsed business seconds.
    pub fn apply_category(
        &mut self,
        category: TicketStatusCategory,
        now: DateTime<Utc>,
        calendar: &BusinessCalendar,
    ) {
        if category.pauses_sla() && self.paused_since.is_none() {
            self.paused_since = Some(now);
        } else if !category.pauses_sla() {
            if let Some(since) = self.paused_since.take() {
                self.paused_business_secs += calendar.elapsed_business_secs(since, now);
            }
        }
    }

    /// Minutes of business time remaining before breach (negative = over).
    pub fn mins_to_resolution_breach(
        &self,
        created_at: DateTime<Utc>,
        now: DateTime<Utc>,
        calendar: &BusinessCalendar,
    ) -> Option<i64> {
        let due = self.resolution_due?;
        let consumed = calendar.elapsed_business_secs(created_at, now) - self.paused_business_secs;
        let budget = calendar.elapsed_business_secs(created_at, due);
        Some(((budget - consumed) / 60).max(i64::MIN / 2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn weekday_calendar() -> BusinessCalendar {
        BusinessCalendar {
            name: "9to5 M-F".into(),
            timezone: "UTC".into(),
            windows: [
                Some(("09:00".into(), "17:00".into())),
                Some(("09:00".into(), "17:00".into())),
                Some(("09:00".into(), "17:00".into())),
                Some(("09:00".into(), "17:00".into())),
                Some(("09:00".into(), "17:00".into())),
                None,
                None,
            ],
            holidays: vec![],
        }
    }

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
            .and_utc()
    }

    #[test]
    fn business_elapsed_skips_nights_and_weekends() {
        let cal = weekday_calendar();
        // Friday 16:00 → Monday 10:00 = 1h Fri + 1h Mon = 7200s
        let secs = cal.elapsed_business_secs(at(2026, 5, 15, 16, 0), at(2026, 5, 18, 10, 0));
        assert_eq!(secs, 7200);
    }

    #[test]
    fn holidays_are_excluded() {
        let mut cal = weekday_calendar();
        cal.holidays.push("2026-05-18".into()); // Monday
        let secs = cal.elapsed_business_secs(at(2026, 5, 15, 16, 0), at(2026, 5, 18, 10, 0));
        assert_eq!(secs, 3600); // Friday hour only
    }

    #[test]
    fn advance_business_crosses_weekend() {
        let cal = weekday_calendar();
        // 2h business from Friday 16:00 → Monday 10:00
        let due = cal.advance_business(at(2026, 5, 15, 16, 0), 7200);
        assert_eq!(due, at(2026, 5, 18, 10, 0));
    }

    #[test]
    fn waiting_pauses_then_banks_seconds() {
        let cal = weekday_calendar();
        let policy = SlaPolicy {
            id: uuid::Uuid::now_v7(),
            org_id: uuid::Uuid::now_v7(),
            name: "std".into(),
            is_default: true,
            targets: [
                (Priority::P1, SlaTargets { first_response_mins: 15, resolution_mins: 480 }),
                (Priority::P2, SlaTargets { first_response_mins: 60, resolution_mins: 1440 }),
                (Priority::P3, SlaTargets { first_response_mins: 240, resolution_mins: 4320 }),
                (Priority::P4, SlaTargets { first_response_mins: 480, resolution_mins: 10080 }),
            ]
            .into_iter()
            .collect(),
            calendar: weekday_calendar(),
        };
        let created = at(2026, 5, 18, 9, 0); // Monday 09:00
        let mut state = TicketSlaState::init(uuid::Uuid::now_v7(), uuid::Uuid::now_v7(), &policy, Priority::P1, created);
        // P1 resolution: 8 business hours → Monday 17:00
        assert_eq!(state.resolution_due, Some(at(2026, 5, 18, 17, 0)));
        // Wait from 10:00 Tuesday to 11:00 Wednesday:
        // Tue 10:00–17:00 (7h) + Wed 09:00–11:00 (2h) = 9 business hours.
        state.apply_category(TicketStatusCategory::Waiting, at(2026, 5, 19, 10, 0), &cal);
        state.apply_category(TicketStatusCategory::InProgress, at(2026, 5, 20, 11, 0), &cal);
        assert_eq!(state.paused_business_secs, 9 * 3600);
        assert!(state.paused_since.is_none());
    }
}
