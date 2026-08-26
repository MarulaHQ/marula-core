//! SLA policies, business calendars, and per-ticket SLA clock state.
//! Calendar math is pure and heavily tested: business hours, holidays,
//! and category-driven pause states all bind here (Stage 2 engine).

use crate::error::DomainError;
use crate::ids::{OrgId, SlaPolicyId, TicketId};
use crate::status::{Priority, TicketStatusCategory};
use chrono::{DateTime, Datelike, Duration, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// Planned downtime: an absolute UTC interval during which SLA clocks do
/// not consume business-time budget (vendor maintenance, change freezes).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MaintenanceWindow {
    pub name: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub starts_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub ends_at: DateTime<Utc>,
}

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
    /// Downtime stretches subtracted from every measurement. Absent in
    /// legacy documents (`default`) so stored policies keep deserializing.
    #[serde(default)]
    pub maintenance: Vec<MaintenanceWindow>,
}

impl BusinessCalendar {
    pub fn parse_window(w: &(String, String)) -> Result<(NaiveTime, NaiveTime), DomainError> {
        let parse = |s: &str| {
            let (h, m) = s
                .split_once(':')
                .ok_or_else(|| DomainError::Validation(format!("bad time {s}")))?;
            let h: u32 = h
                .parse()
                .map_err(|_| DomainError::Validation(format!("bad time {s}")))?;
            let m: u32 = m
                .parse()
                .map_err(|_| DomainError::Validation(format!("bad time {s}")))?;
            NaiveTime::from_hms_opt(h, m, 0)
                .ok_or_else(|| DomainError::Validation(format!("bad time {s}")))
        };
        Ok((parse(&w.0)?, parse(&w.1)?))
    }

    fn tz(&self) -> chrono_tz::Tz {
        self.timezone.parse().unwrap_or(chrono_tz::UTC)
    }

    /// Structural validation for API writes: timezone must be a real IANA
    /// name (never the silent UTC fallback), windows well-formed with
    /// open < close, holiday strings valid local dates, maintenance
    /// intervals non-empty and ordered.
    pub fn validate(&self) -> Result<(), DomainError> {
        self.timezone
            .parse::<chrono_tz::Tz>()
            .map_err(|_| DomainError::Validation(format!("unknown timezone: {}", self.timezone)))?;
        for w in self.windows.iter().flatten() {
            let (open, close) = Self::parse_window(w)?;
            if open >= close {
                return Err(DomainError::Validation(format!(
                    "window opens at or after it closes: {w:?}"
                )));
            }
        }
        for h in &self.holidays {
            chrono::NaiveDate::parse_from_str(h, "%Y-%m-%d")
                .map_err(|_| DomainError::Validation(format!("bad holiday date: {h}")))?;
        }
        for m in &self.maintenance {
            if m.ends_at <= m.starts_at {
                return Err(DomainError::Validation(format!(
                    "maintenance window {} ends before it starts",
                    m.name
                )));
            }
        }
        Ok(())
    }

    /// The day's business window as absolute instants (None on holidays,
    /// closed days, or unresolvable DST-local times).
    fn day_window_utc(
        &self,
        tz: chrono_tz::Tz,
        day: chrono::NaiveDate,
    ) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        if self.holidays.iter().any(|h| h.as_str() == day.to_string()) {
            return None;
        }
        let idx = day.weekday().num_days_from_monday() as usize;
        let (open, close) = self
            .windows
            .get(idx)
            .and_then(|w| w.as_ref())
            .and_then(|w| Self::parse_window(w).ok())?;
        // `earliest()` resolves fall-back ambiguity to the first occurrence
        // and returns None inside spring-forward gaps — such a day simply
        // contributes nothing rather than inventing time.
        let start = tz.from_local_datetime(&day.and_time(open)).earliest()?;
        let end = tz.from_local_datetime(&day.and_time(close)).earliest()?;
        Some((start.with_timezone(&Utc), end.with_timezone(&Utc)))
    }

    /// Slices `[ws, we]` into the sub-intervals that survive after merged
    /// maintenance downtime is removed. Output is ascending and disjoint.
    fn free_intervals(
        &self,
        ws: DateTime<Utc>,
        we: DateTime<Utc>,
    ) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
        if ws >= we {
            return Vec::new();
        }
        let mut maint: Vec<(DateTime<Utc>, DateTime<Utc>)> = self
            .maintenance
            .iter()
            .filter(|m| m.ends_at > m.starts_at)
            .map(|m| (m.starts_at.max(ws), m.ends_at.min(we)))
            .filter(|(s, e)| e > s)
            .collect();
        maint.sort();
        let mut out = Vec::new();
        let mut cursor = ws;
        for (ms, me) in maint {
            if ms > cursor {
                out.push((cursor, ms));
            }
            cursor = cursor.max(me);
        }
        if cursor < we {
            out.push((cursor, we));
        }
        out
    }

    /// Business seconds of `[from, to]` lost to maintenance downtime.
    pub fn maintenance_overlap_secs(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> i64 {
        if to <= from {
            return 0;
        }
        let mut maint: Vec<(DateTime<Utc>, DateTime<Utc>)> = self
            .maintenance
            .iter()
            .filter(|m| m.ends_at > m.starts_at)
            .map(|m| (m.starts_at.max(from), m.ends_at.min(to)))
            .filter(|(s, e)| e > s)
            .collect();
        maint.sort();
        // Merge then sum so overlapping declarations are not double-counted.
        let mut total = 0_i64;
        let mut current: Option<(DateTime<Utc>, DateTime<Utc>)> = None;
        for (s, e) in maint {
            match current {
                None => current = Some((s, e)),
                Some((cs, ce)) => {
                    if s <= ce {
                        current = Some((cs, ce.max(e)));
                    } else {
                        total += (ce - cs).num_seconds();
                        current = Some((s, e));
                    }
                }
            }
        }
        if let Some((cs, ce)) = current {
            total += (ce - cs).num_seconds();
        }
        total
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
            if let Some((ws, we)) = self.day_window_utc(tz, day) {
                // Downtime is removed per business window so maintenance
                // falling OUTSIDE working hours never consumes budget.
                for (fs, fe) in self.free_intervals(ws, we) {
                    let s = fs.max(from);
                    let e = fe.min(to);
                    if e > s {
                        total += (e - s).num_seconds();
                    }
                }
            }
            let next = day.succ_opt();
            match next {
                Some(d) if d > day => day = d,
                _ => break, // defensive: never loop on succ_opt failure
            }
        }
        total
    }

    /// The instant `business_secs` of business time after `from`, skipping
    /// nights/weekends/holidays AND maintenance downtime. Caps iteration at
    /// 400 days.
    pub fn advance_business(&self, from: DateTime<Utc>, business_secs: i64) -> DateTime<Utc> {
        if business_secs <= 0 {
            return from;
        }
        let tz = self.tz();
        let mut remaining = business_secs;
        let mut day = from.with_timezone(&tz).date_naive();
        let mut cursor: Option<DateTime<Utc>> = None;
        for _ in 0..400 {
            if let Some((ws, we)) = self.day_window_utc(tz, day) {
                let floor = cursor.unwrap_or(from).max(ws);
                for (free_start, free_end) in self.free_intervals(ws, we) {
                    let s = free_start.max(floor);
                    if free_end > s {
                        let avail = (free_end - s).num_seconds();
                        if avail >= remaining {
                            return s + Duration::seconds(remaining);
                        }
                        remaining -= avail;
                    }
                }
                cursor = Some(cursor.map_or(we, |c: DateTime<Utc>| c.max(we)));
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
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: SlaPolicyId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
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
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
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
        let resolution_due =
            targets.map(|t| calendar.advance_business(created_at, t.resolution_mins * 60));
        let first_response_due =
            targets.map(|t| calendar.advance_business(created_at, t.first_response_mins * 60));
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

/// An org-level business-hours calendar row (`sla_calendars`). The math
/// type is [`BusinessCalendar`]; this carries its identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SlaCalendar {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: crate::ids::SlaCalendarRowId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub name: String,
    /// Flattened calendar body (timezone/windows/holidays/maintenance);
    /// `calendar.name` mirrors `name` on write.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub calendar: BusinessCalendar,
}

impl SlaCalendar {
    /// The math view of this row.
    pub fn business_calendar(&self) -> &BusinessCalendar {
        &self.calendar
    }
}

/// One escalation ladder step as stored (`sla_escalation_steps`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EscalationStep {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub policy_id: crate::ids::SlaPolicyId,
    pub step_index: i32,
    /// 'risk' | 'breach' | 'escalate' — bound to existing notification kinds.
    pub trigger_kind: String,
    pub at_pct: i64,
    /// Extra recipients beyond the ticket assignee.
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<uuid::Uuid>))]
    pub notify_user_ids: Vec<crate::ids::UserId>,
    pub enabled: bool,
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
            maintenance: vec![],
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
                (
                    Priority::P1,
                    SlaTargets {
                        first_response_mins: 15,
                        resolution_mins: 480,
                    },
                ),
                (
                    Priority::P2,
                    SlaTargets {
                        first_response_mins: 60,
                        resolution_mins: 1440,
                    },
                ),
                (
                    Priority::P3,
                    SlaTargets {
                        first_response_mins: 240,
                        resolution_mins: 4320,
                    },
                ),
                (
                    Priority::P4,
                    SlaTargets {
                        first_response_mins: 480,
                        resolution_mins: 10080,
                    },
                ),
            ]
            .into_iter()
            .collect(),
            calendar: weekday_calendar(),
        };
        let created = at(2026, 5, 18, 9, 0); // Monday 09:00
        let mut state = TicketSlaState::init(
            uuid::Uuid::now_v7(),
            uuid::Uuid::now_v7(),
            &policy,
            Priority::P1,
            created,
        );
        // P1 resolution: 8 business hours → Monday 17:00
        assert_eq!(state.resolution_due, Some(at(2026, 5, 18, 17, 0)));
        // Wait from 10:00 Tuesday to 11:00 Wednesday:
        // Tue 10:00–17:00 (7h) + Wed 09:00–11:00 (2h) = 9 business hours.
        state.apply_category(TicketStatusCategory::Waiting, at(2026, 5, 19, 10, 0), &cal);
        state.apply_category(
            TicketStatusCategory::InProgress,
            at(2026, 5, 20, 11, 0),
            &cal,
        );
        assert_eq!(state.paused_business_secs, 9 * 3600);
        assert!(state.paused_since.is_none());
    }

    #[test]
    fn maintenance_inside_a_window_is_subtracted() {
        let mut cal = weekday_calendar();
        cal.maintenance.push(MaintenanceWindow {
            name: "change freeze".into(),
            starts_at: at(2026, 5, 19, 11, 0), // Tuesday 11:00
            ends_at: at(2026, 5, 19, 13, 0),
        });
        // Tuesday full window is 8h; two hours are downtime.
        assert_eq!(
            cal.elapsed_business_secs(at(2026, 5, 19, 9, 0), at(2026, 5, 19, 17, 0)),
            6 * 3600
        );
    }

    #[test]
    fn maintenance_outside_business_hours_costs_nothing() {
        let mut cal = weekday_calendar();
        cal.maintenance.push(MaintenanceWindow {
            name: "overnight".into(),
            starts_at: at(2026, 5, 19, 18, 0),
            ends_at: at(2026, 5, 20, 8, 0),
        });
        assert_eq!(
            cal.elapsed_business_secs(at(2026, 5, 19, 9, 0), at(2026, 5, 20, 17, 0)),
            16 * 3600
        );
    }

    #[test]
    fn advance_routes_around_maintenance() {
        let mut cal = weekday_calendar();
        cal.maintenance.push(MaintenanceWindow {
            name: "lunch freeze".into(),
            starts_at: at(2026, 5, 19, 12, 0),
            ends_at: at(2026, 5, 19, 13, 0),
        });
        // Tuesday free time = 3h (9–12) + 4h (13–17) = 7h; an 8h budget
        // lands one hour into Wednesday's window.
        let due = cal.advance_business(at(2026, 5, 19, 9, 0), 8 * 3600);
        assert_eq!(due, at(2026, 5, 20, 10, 0));
    }

    #[test]
    fn validate_rejects_bad_timezones_windows_and_dates() {
        let mut cal = weekday_calendar();
        assert!(cal.validate().is_ok());
        cal.timezone = "Not/AZone".into();
        assert!(cal.validate().is_err());
        cal.timezone = "UTC".into();
        cal.windows[0] = Some(("17:00".into(), "09:00".into()));
        assert!(cal.validate().is_err());
        cal.windows[0] = Some(("09:00".into(), "17:00".into()));
        cal.holidays.push("2026-02-30".into());
        assert!(cal.validate().is_err());
        cal.holidays.clear();
        cal.maintenance.push(MaintenanceWindow {
            name: "backwards".into(),
            starts_at: at(2026, 1, 2, 0, 0),
            ends_at: at(2026, 1, 1, 0, 0),
        });
        assert!(cal.validate().is_err());
    }

    #[test]
    fn legacy_documents_without_maintenance_still_deserialize() {
        let raw = serde_json::json!({
            "name": "seeded", "timezone": "UTC",
            "windows": [["09:00","17:00"], null, null, null, null, null, null],
            "holidays": []
        });
        let cal: BusinessCalendar =
            serde_json::from_value(raw).expect("pre-maintenance policy documents parse");
        assert!(cal.maintenance.is_empty());
    }

    #[test]
    fn dst_spring_forward_shrinks_the_absolute_window() {
        // US 2026-03-08: 02:00 EST → 03:00 EDT. A Sunday window of
        // [01:00, 04:00] wall-clock spans only TWO absolute hours.
        let cal = BusinessCalendar {
            name: "sunday ops".into(),
            timezone: "America/New_York".into(),
            windows: [
                None,
                None,
                None,
                None,
                None,
                None,
                Some(("01:00".into(), "04:00".into())),
            ],
            holidays: vec![],
            maintenance: vec![],
        };
        // 01:00 EST == 06:00 UTC; 04:00 EDT == 08:00 UTC.
        let secs = cal.elapsed_business_secs(at(2026, 3, 8, 0, 0), at(2026, 3, 8, 23, 0));
        assert_eq!(secs, 2 * 3600);
    }

    #[test]
    fn dst_fall_back_repeated_hour_counts_once_per_wall_pass() {
        // Sydney 2026-04-05: 03:00 AEDT → 02:00 AEST. A Sunday window
        // [01:30, 03:30] spans THREE absolute hours (the repeated hour is
        // real elapsed time and counts).
        let cal = BusinessCalendar {
            name: "sydney sunday".into(),
            timezone: "Australia/Sydney".into(),
            windows: [
                None,
                None,
                None,
                None,
                None,
                None,
                Some(("01:30".into(), "03:30".into())),
            ],
            holidays: vec![],
            maintenance: vec![],
        };
        let secs = cal.elapsed_business_secs(at(2026, 4, 4, 0, 0), at(2026, 4, 5, 12, 0));
        assert_eq!(secs, 3 * 3600);
    }
}
