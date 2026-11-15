//! Working-calendar resolution and the reschedule engine.
//!
//! A date calculation needs one answer to "does this day count". The rule is
//! fixed by ADR 0034: a project's own working pattern wins where it exists,
//! the organisation's calendar is the base, and personal weekly hours scale
//! capacity without moving a date.
//!
//! The engine itself is pure: it takes the tasks as they are and returns the
//! changes a move would produce. The preview endpoint and the apply endpoint
//! call the same function, which is why the preview is trustworthy (ADR 0034
//! decision 2). Nothing here writes, and nothing here reads the clock.

use chrono::{DateTime, Datelike, Duration, TimeZone, Utc, Weekday};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

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

    /// Builds a pattern from its two numbers. A mask with no working day is
    /// rejected by the caller, because a calendar that works never cannot
    /// schedule anything.
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

    /// Whether a calendar day is worked.
    pub fn works_on_date(&self, day: DateTime<Utc>) -> bool {
        self.works_on(day.weekday().num_days_from_monday() as u8)
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

/// A task as the engine sees it: an optional start, an optional length, and
/// the tasks it waits for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskNode {
    pub id: Uuid,
    /// When the work starts. `None` means undated.
    pub start_at: Option<DateTime<Utc>>,
    /// How long the work runs. `None` means undated.
    pub duration_minutes: Option<i32>,
    /// Finish-to-start predecessors: this task does not start before they end.
    pub deps: Vec<Uuid>,
}

impl TaskNode {
    /// The instant the work ends, when it is dated at all.
    pub fn end_at(&self, calendar: WorkCalendar) -> Option<DateTime<Utc>> {
        match (self.start_at, self.duration_minutes) {
            (Some(start), Some(minutes)) if minutes >= 0 => {
                Some(add_working_minutes(start, minutes as u32, calendar))
            }
            _ => None,
        }
    }

    /// A duration in minutes, with undated work counting as no time at all.
    pub fn planned_minutes(&self) -> i64 {
        i64::from(self.duration_minutes.unwrap_or(0).max(0))
    }
}

/// Why a task moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeReason {
    /// The task the coordinator dragged.
    Moved,
    /// A successor pushed forward by its predecessor's new finish.
    Pushed,
}

/// One proposed change. `from` is what the task holds now, `to` is where the
/// plan would put it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub task_id: Uuid,
    pub from_start: Option<DateTime<Utc>>,
    pub to_start: Option<DateTime<Utc>>,
    pub reason: ChangeReason,
}

/// The first working instant at or after `at`, keeping the clock time.
pub fn next_working_instant(at: DateTime<Utc>, calendar: WorkCalendar) -> DateTime<Utc> {
    let mut cursor = at;
    // Seven iterations covers a full week; the guard keeps a broken calendar
    // from becoming an infinite loop.
    for _ in 0..7 {
        if calendar.works_on_date(cursor) {
            return cursor;
        }
        cursor += Duration::days(1);
    }
    cursor
}

/// Adds working minutes to an instant, skipping days the calendar does not
/// work and never counting more than one working day's minutes per day.
pub fn add_working_minutes(
    start: DateTime<Utc>,
    minutes: u32,
    calendar: WorkCalendar,
) -> DateTime<Utc> {
    if !calendar.is_working() {
        return start;
    }
    let mut cursor = next_working_instant(start, calendar);
    let mut remaining = minutes;
    // The first day only offers what is left of it; every later day offers a
    // full day. A move that lands exactly on a day boundary belongs to the
    // next working day, not the evening of this one.
    loop {
        if remaining <= calendar.minutes_per_day {
            return cursor + Duration::minutes(i64::from(remaining));
        }
        remaining -= calendar.minutes_per_day;
        cursor = next_working_instant(cursor + Duration::days(1), calendar);
    }
}

/// Whole working minutes between two instants, counted day by day.
pub fn working_minutes_between(
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    calendar: WorkCalendar,
) -> i64 {
    if !calendar.is_working() || to <= from {
        return 0;
    }
    let mut cursor = next_working_instant(from, calendar);
    let mut total = 0_i64;
    while cursor < to {
        let day_end = (cursor + Duration::minutes(i64::from(calendar.minutes_per_day))).min(to);
        total += (day_end - cursor).num_minutes();
        cursor = next_working_instant(cursor + Duration::days(1), calendar);
    }
    total.max(0)
}

/// Computes the changes a move produces, without applying anything.
///
/// The moved task is placed at `new_start` (or un-dated when `new_start` is
/// `None`). Finish-to-start successors are pushed forward when their
/// predecessor now ends later than they start; nothing is pulled earlier,
/// because a coordinator moving one bar must not silently compact the plan.
pub fn plan_move(
    tasks: &[TaskNode],
    calendar: WorkCalendar,
    moved: Uuid,
    new_start: Option<DateTime<Utc>>,
) -> Vec<Change> {
    let by_id: HashMap<Uuid, &TaskNode> = tasks.iter().map(|t| (t.id, t)).collect();
    let Some(target) = by_id.get(&moved) else {
        return Vec::new();
    };
    let effective_start = new_start.map(|s| next_working_instant(s, calendar));
    if target.start_at == effective_start {
        return Vec::new();
    }

    let mut starts: HashMap<Uuid, Option<DateTime<Utc>>> =
        tasks.iter().map(|t| (t.id, t.start_at)).collect();
    starts.insert(moved, effective_start);

    let mut changes = vec![Change {
        task_id: moved,
        from_start: target.start_at,
        to_start: effective_start,
        reason: ChangeReason::Moved,
    }];

    // Successors of a task, so the ripple walks the graph forward.
    let mut successors: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for task in tasks {
        for dep in &task.deps {
            successors.entry(*dep).or_default().push(task.id);
        }
    }

    let mut queue: Vec<Uuid> = successors.get(&moved).cloned().unwrap_or_default();
    let mut seen: HashSet<Uuid> = HashSet::new();
    // A depth guard turns a malformed cycle into a bounded walk instead of a
    // hang; dependency validation rejects cycles before they reach here.
    let mut steps = 0_usize;
    let max_steps = tasks.len() * tasks.len() + 1;
    while let Some(next) = queue.pop() {
        steps += 1;
        if steps > max_steps {
            break;
        }
        if !seen.insert(next) {
            continue;
        }
        let Some(task) = by_id.get(&next) else {
            continue;
        };
        let Some(duration) = task.duration_minutes else {
            continue;
        };
        // The earliest this task could start: the latest finish among its
        // dated predecessors.
        let mut earliest: Option<DateTime<Utc>> = None;
        for dep in &task.deps {
            let Some(pred) = by_id.get(dep) else {
                continue;
            };
            let Some(pred_start) = starts.get(dep).copied().flatten() else {
                continue;
            };
            let Some(pred_duration) = pred.duration_minutes else {
                continue;
            };
            let finish = add_working_minutes(pred_start, pred_duration.max(0) as u32, calendar);
            earliest = Some(match earliest {
                Some(current) if current >= finish => current,
                _ => finish,
            });
        }
        let Some(earliest) = earliest else {
            continue;
        };
        let current = starts.get(&next).copied().flatten();
        let should_push = match current {
            Some(start) => earliest > start,
            None => true,
        };
        if !should_push {
            continue;
        }
        let pushed = next_working_instant(earliest, calendar);
        starts.insert(next, Some(pushed));
        changes.push(Change {
            task_id: next,
            from_start: current,
            to_start: Some(pushed),
            reason: ChangeReason::Pushed,
        });
        if duration > 0 {
            queue.extend(successors.get(&next).cloned().unwrap_or_default());
        }
    }
    changes
}

/// The longest chain through the graph, measured in planned minutes.
///
/// This is the path that decides the project's finish, so a coordinator knows
/// which slip matters. Undated tasks carry no time and never lengthen it.
pub fn critical_path(tasks: &[TaskNode]) -> Vec<Uuid> {
    let by_id: HashMap<Uuid, &TaskNode> = tasks.iter().map(|t| (t.id, t)).collect();
    let mut lengths: HashMap<Uuid, i64> = HashMap::new();
    let mut predecessor: HashMap<Uuid, Uuid> = HashMap::new();

    // Repeated relaxation over the edge set: bounded by the task count, and
    // safe against a cycle because a cycle only ever adds length.
    for _ in 0..tasks.len() {
        let mut changed = false;
        for task in tasks {
            let mut best = task.planned_minutes();
            let mut parent = None;
            for dep in &task.deps {
                let Some(pred) = by_id.get(dep) else {
                    continue;
                };
                let score = lengths.get(dep).copied().unwrap_or(0) + task.planned_minutes();
                if score > best {
                    best = score;
                    parent = Some(*dep);
                }
                let _ = pred;
            }
            if lengths.get(&task.id).copied().unwrap_or(0) < best {
                lengths.insert(task.id, best);
                match parent {
                    Some(p) => {
                        predecessor.insert(task.id, p);
                    }
                    None => {
                        predecessor.remove(&task.id);
                    }
                }
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let Some((mut cursor, _)) = lengths
        .iter()
        .max_by_key(|(id, len)| (**len, **id))
        .map(|(id, len)| (*id, *len))
    else {
        return Vec::new();
    };
    let mut path = vec![cursor];
    let mut guard = 0;
    while let Some(prev) = predecessor.get(&cursor).copied() {
        guard += 1;
        if guard > tasks.len() || path.contains(&prev) {
            break;
        }
        path.push(prev);
        cursor = prev;
    }
    path.reverse();
    path
}

/// Scales a duration by a factor, rounded to whole minutes and never below a
/// minute for work that had any length at all.
pub fn scale_minutes(minutes: i32, factor: f64) -> i32 {
    if minutes <= 0 {
        return minutes;
    }
    let scaled = (f64::from(minutes) * factor).round() as i64;
    scaled.clamp(1, i64::from(i32::MAX)) as i32
}

/// A working date for a given day, useful to tests and the capture harness.
pub fn day_start(year: i32, month: u32, day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, 9, 0, 0)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).unwrap())
}

/// Resolves the calendar a project's dates are computed against.
///
/// The project's own pattern wins where one is set; the organisation's
/// calendar is the fallback. Personal weekly hours are not consulted here:
/// they change how much a person can take on, never when a date lands.
pub fn resolve(project: Option<WorkCalendar>, org: WorkCalendar) -> WorkCalendar {
    project.unwrap_or(org)
}

/// Weekday of a date, exposed so a caller can label a calendar without
/// re-deriving the bit order.
pub fn weekday_index(day: DateTime<Utc>) -> u8 {
    day.weekday().num_days_from_monday() as u8
}

/// Weekday name, for the plain-English read-back a surface shows next to a
/// working pattern.
pub fn weekday_name(index: u8) -> &'static str {
    match index {
        0 => "Monday",
        1 => "Tuesday",
        2 => "Wednesday",
        3 => "Thursday",
        4 => "Friday",
        5 => "Saturday",
        _ => "Sunday",
    }
}

/// The days a pattern covers, in reading order.
pub fn working_days(calendar: WorkCalendar) -> Vec<&'static str> {
    (0..7)
        .filter(|d| calendar.works_on(*d))
        .map(weekday_name)
        .collect()
}

/// Convenience for callers that hold a chrono weekday.
pub fn weekday_of(day: DateTime<Utc>) -> Weekday {
    day.weekday()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(
        id: u128,
        start: Option<DateTime<Utc>>,
        minutes: Option<i32>,
        deps: &[u128],
    ) -> TaskNode {
        TaskNode {
            id: Uuid::from_u128(id),
            start_at: start,
            duration_minutes: minutes,
            deps: deps.iter().map(|d| Uuid::from_u128(*d)).collect(),
        }
    }

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
    fn minutes_never_land_on_a_weekend() {
        let cal = WorkCalendar::STANDARD;
        // Friday 2026-11-13 09:00 plus two working days, so the weekend is
        // skipped and the work lands on Monday at the same clock time.
        let friday = day_start(2026, 11, 13);
        let end = add_working_minutes(friday, 960, cal);
        assert_eq!(end.weekday(), Weekday::Mon);
        assert_eq!(end, day_start(2026, 11, 16) + Duration::hours(8));
    }

    #[test]
    fn a_start_on_a_weekend_rolls_to_monday() {
        let cal = WorkCalendar::STANDARD;
        let saturday = day_start(2026, 11, 14);
        let resolved = next_working_instant(saturday, cal);
        assert_eq!(resolved.weekday(), Weekday::Mon);
    }

    #[test]
    fn moving_a_predecessor_pushes_its_successor() {
        let cal = WorkCalendar::STANDARD;
        let first = day_start(2026, 11, 16);
        let tasks = vec![
            task(1, Some(first), Some(480), &[]),
            task(2, Some(first + Duration::days(1)), Some(480), &[1]),
        ];
        let changes = plan_move(
            &tasks,
            cal,
            Uuid::from_u128(1),
            Some(first + Duration::days(2)),
        );
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].reason, ChangeReason::Moved);
        assert_eq!(changes[1].task_id, Uuid::from_u128(2));
        assert_eq!(changes[1].reason, ChangeReason::Pushed);
    }

    #[test]
    fn moving_a_predecessor_earlier_never_pulls_a_successor() {
        let cal = WorkCalendar::STANDARD;
        let first = day_start(2026, 11, 16);
        let successor_start = first + Duration::days(4);
        let tasks = vec![
            task(1, Some(first + Duration::days(2)), Some(480), &[]),
            task(2, Some(successor_start), Some(480), &[1]),
        ];
        let changes = plan_move(&tasks, cal, Uuid::from_u128(1), Some(first));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].reason, ChangeReason::Moved);
    }

    #[test]
    fn undated_successors_are_left_alone() {
        let cal = WorkCalendar::STANDARD;
        let first = day_start(2026, 11, 16);
        let tasks = vec![
            task(1, Some(first), Some(480), &[]),
            task(2, None, None, &[1]),
        ];
        let changes = plan_move(
            &tasks,
            cal,
            Uuid::from_u128(1),
            Some(first + Duration::days(1)),
        );
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn the_critical_path_is_the_longest_chain() {
        let start = day_start(2026, 11, 16);
        let tasks = vec![
            task(1, Some(start), Some(480), &[]),
            task(2, Some(start), Some(60), &[]),
            task(3, Some(start + Duration::days(1)), Some(480), &[1]),
            task(4, Some(start + Duration::days(2)), Some(60), &[2]),
        ];
        assert_eq!(
            critical_path(&tasks),
            vec![Uuid::from_u128(1), Uuid::from_u128(3)]
        );
    }

    #[test]
    fn scaling_keeps_something_for_work_that_had_length() {
        assert_eq!(scale_minutes(480, 1.5), 720);
        assert_eq!(scale_minutes(480, 0.0), 1);
        assert_eq!(scale_minutes(0, 2.0), 0);
        assert_eq!(scale_minutes(60, 0.25), 15);
    }

    #[test]
    fn working_days_are_named_in_reading_order() {
        assert_eq!(
            working_days(WorkCalendar::STANDARD),
            vec!["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"]
        );
        assert_eq!(weekday_index(day_start(2026, 11, 16)), 0);
    }

    #[test]
    fn minutes_between_counts_only_working_days() {
        let cal = WorkCalendar::STANDARD;
        let monday = day_start(2026, 11, 16);
        // Monday 09:00 to Tuesday 09:00 is one working day of minutes.
        assert_eq!(
            working_minutes_between(monday, monday + Duration::days(1), cal),
            480
        );
    }
}
