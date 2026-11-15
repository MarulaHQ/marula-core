/**
 * gantt-scale — the date and working-calendar arithmetic the timeline draws
 * with, kept pure so the chart, the table and the preview all agree.
 *
 * Everything here works in UTC midnight milliseconds. A project date is a day,
 * not an instant: the API stores task starts as timestamptz, but a bar at
 * 09:30 and a bar at 00:00 must paint in the same column, and two machines in
 * different zones must paint the same chart. So a date is read to its UTC day,
 * and every formatter renders in UTC on purpose.
 *
 * The working pattern is the server's own vocabulary (`weekdays` bitmask with
 * Monday at bit 0, `minutes_per_day` with lunch excluded), because a drop that
 * snaps to a day the engine would move is a lie the preview would have to
 * correct.
 */

export const MS_PER_DAY = 86_400_000;

/** A weekly working pattern, mirroring `core_projects::schedule::WorkCalendar`. */
export interface GanttCalendar {
  /** Monday at bit 0, Sunday at bit 6. Monday to Friday is 31. */
  weekdays: number;
  /** Minutes of work in one working day, lunch excluded. */
  minutesPerDay: number;
}

/** Monday to Friday, eight hours a day — the server's `WorkCalendar::STANDARD`. */
export const STANDARD_CALENDAR: GanttCalendar = { weekdays: 0b001_1111, minutesPerDay: 480 };

/** Parses an ISO date (`2026-11-02`) or datetime into UTC midnight. */
export function parseDay(value: string): number {
  const head = value.length >= 10 ? value.slice(0, 10) : value;
  const parsed = Date.parse(`${head}T00:00:00Z`);
  return Number.isNaN(parsed) ? Number.NaN : parsed;
}

/** An ISO date from a UTC-midnight instant. */
export function dayKey(ms: number): string {
  return new Date(ms).toISOString().slice(0, 10);
}

export function addDays(ms: number, days: number): number {
  return ms + days * MS_PER_DAY;
}

/** The Monday of the week a day falls in. */
export function startOfWeek(ms: number): number {
  const dow = (new Date(ms).getUTCDay() + 6) % 7; // Monday = 0
  return addDays(ms, -dow);
}

/** Monday = 0 … Sunday = 6, read in UTC like every other date here. */
export function weekdayIndex(ms: number): number {
  return (new Date(ms).getUTCDay() + 6) % 7;
}

export function isWorkingDay(ms: number, calendar: GanttCalendar): boolean {
  const bit = 1 << weekdayIndex(ms);
  return (calendar.weekdays & bit) !== 0;
}

/** The first working day at or after `ms`. Bounded at a week so a broken
 * calendar (no working day at all) cannot spin. */
export function nextWorkingDay(ms: number, calendar: GanttCalendar): number {
  let cursor = ms;
  for (let i = 0; i < 8; i += 1) {
    if (isWorkingDay(cursor, calendar)) return cursor;
    cursor = addDays(cursor, 1);
  }
  return ms;
}

/** Adds working minutes, skipping days the calendar does not work and never
 * counting more than one day's minutes in a day. Mirrors the Rust engine. */
export function addWorkingMinutes(ms: number, minutes: number, calendar: GanttCalendar): number {
  if (!isWorking(calendar)) return ms;
  let cursor = nextWorkingDay(ms, calendar);
  let remaining = Math.max(0, minutes);
  // 400 blocks of a week is a year and change: far past any project bar.
  for (let guard = 0; guard < 400; guard += 1) {
    if (remaining <= calendar.minutesPerDay) return cursor + remaining * 60_000;
    remaining -= calendar.minutesPerDay;
    cursor = nextWorkingDay(addDays(cursor, 1), calendar);
  }
  return cursor;
}

/** Whole working minutes between two instants, counted a day at a time. */
export function workingMinutesBetween(from: number, to: number, calendar: GanttCalendar): number {
  if (!isWorking(calendar) || to <= from) return 0;
  let cursor = nextWorkingDay(from, calendar);
  let total = 0;
  for (let guard = 0; guard < 4_000 && cursor < to; guard += 1) {
    const dayEnd = Math.min(cursor + calendar.minutesPerDay * 60_000, to);
    total += Math.floor((dayEnd - cursor) / 60_000);
    cursor = nextWorkingDay(addDays(cursor, 1), calendar);
  }
  return Math.max(0, total);
}

/** Whole working days a span covers, rounded up like the engine's own block
 * arithmetic. A zero-length span is one day of work, not zero. */
export function workingDaysBetween(from: number, to: number, calendar: GanttCalendar): number {
  const start = nextWorkingDay(from, calendar);
  const end = Math.max(start, to);
  if (!isWorking(calendar)) return 1;
  let cursor = start;
  let days = 0;
  for (let guard = 0; guard < 4_000; guard += 1) {
    if (cursor >= end) break;
    days += 1;
    cursor = nextWorkingDay(addDays(cursor, 1), calendar);
  }
  return Math.max(1, days);
}

export function isWorking(calendar: GanttCalendar): boolean {
  return calendar.weekdays !== 0 && calendar.minutesPerDay > 0;
}

const WEEKDAY_WORDS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] as const;
const MONTH_WORDS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
] as const;

/** `Mon 9 Nov` — the words the design uses wherever a date is read aloud. */
export function formatDayWords(ms: number): string {
  const date = new Date(ms);
  return `${WEEKDAY_WORDS[weekdayIndex(ms)]} ${date.getUTCDate()} ${MONTH_WORDS[date.getUTCMonth()]}`;
}

/** `9 Nov` — the compact form for cells and labels. */
export function formatDayShort(ms: number): string {
  const date = new Date(ms);
  return `${date.getUTCDate()} ${MONTH_WORDS[date.getUTCMonth()]}`;
}

/** `Nov 2026` — the month band's own label. */
export function formatMonth(ms: number): string {
  const date = new Date(ms);
  return `${MONTH_WORDS[date.getUTCMonth()]} ${date.getUTCFullYear()}`;
}

/** `9 Nov to 18 Nov` — the phrase the table and the list rows print. */
export function formatRangeWords(from: number, toInclusive: number): string {
  return `${formatDayShort(from)} to ${formatDayShort(toInclusive)}`;
}

/** A duration in the two units a planner reads: working days, then hours. */
export function durationInWords(minutes: number, calendar: GanttCalendar): string {
  const safe = Math.max(0, minutes);
  if (safe === 0) return "0 h";
  const day = calendar.minutesPerDay > 0 ? calendar.minutesPerDay : 480;
  const days = Math.floor(safe / day);
  const rest = safe % day;
  if (days === 0) return `${hours(rest)} h`;
  if (rest === 0) return `${days} ${days === 1 ? "day" : "days"}`;
  return `${days} ${days === 1 ? "day" : "days"} ${hours(rest)} h`;
}

function hours(minutes: number): string {
  const value = minutes / 60;
  return Number.isInteger(value) ? String(value) : value.toFixed(1);
}

/** Signed variance in working days, as words — "5 working days later". */
export function varianceInWords(signedDays: number): string {
  if (signedDays === 0) return "on the baseline date";
  const count = Math.abs(signedDays);
  return `${count} working ${count === 1 ? "day" : "days"} ${signedDays > 0 ? "later" : "earlier"}`;
}

/** Inclusive day count between two UTC days. */
export function daysBetween(from: number, to: number): number {
  return Math.round((to - from) / MS_PER_DAY);
}

/** Whole working days from `from` to `to`, signed. A move to the next working
 * day is +1, the same day is 0, and a weekend jump counts the working days it
 * passes, which is the number a planner reads as "five days later". */
export function workingDayOffset(from: number, to: number, calendar: GanttCalendar): number {
  if (from === to) return 0;
  const direction = to > from ? 1 : -1;
  let cursor = from;
  let count = 0;
  for (let guard = 0; guard < 4_000; guard += 1) {
    cursor = nextWorkingDay(addDays(cursor, direction), calendar);
    count += direction;
    if ((direction > 0 && cursor >= to) || (direction < 0 && cursor <= to)) break;
  }
  return count;
}

/** Pads a span out to whole weeks so a week column is never cut in half. */
export function weekAlignedWindow(from: number, to: number): { from: number; to: number } {
  const start = startOfWeek(from);
  const end = addDays(startOfWeek(to), 6);
  return { from: start, to: Math.max(end, addDays(start, 27)) };
}

/** One task as the push simulation needs it. */
export interface GanttPlanNode {
  id: string;
  /** UTC midnight of the start, or null when undated. */
  start: number | null;
  durationMinutes: number | null;
  dependsOn: string[];
}

export interface GanttPlanChange {
  taskId: string;
  fromStart: string | null;
  toStart: string | null;
  reason: "moved" | "pushed";
}

/**
 * What a move does to the plan, before the server is asked.
 *
 * This is the same finish-to-start rule the preview endpoint applies: the moved
 * task lands on the next working day at or after the drop, and a successor is
 * pushed only when its predecessor now ends later than it starts. It exists so
 * the drag tooltip can say how many downstream tasks move while the pointer is
 * still down; the server's answer replaces it on drop, and the server's answer
 * is what commits.
 */
export function simulateMove(
  nodes: GanttPlanNode[],
  calendar: GanttCalendar,
  movedId: string,
  newStart: number,
): GanttPlanChange[] {
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const target = byId.get(movedId);
  if (target === undefined) return [];
  const snapped = nextWorkingDay(newStart, calendar);
  if (target.start === snapped) return [];

  const starts = new Map<string, number | null>(nodes.map((node) => [node.id, node.start]));
  starts.set(movedId, snapped);
  const changes: GanttPlanChange[] = [
    {
      taskId: movedId,
      fromStart: target.start === null ? null : dayKey(target.start),
      toStart: dayKey(snapped),
      reason: "moved",
    },
  ];

  // Longest chain first: a pushed predecessor must settle before its own
  // successors are measured. The engine walks parents recursively; a bounded
  // pass to a fixed point reaches the same answer for a plan this size.
  for (let pass = 0; pass < 8; pass += 1) {
    let moved = false;
    for (const node of nodes) {
      for (const depId of node.dependsOn) {
        const dep = byId.get(depId);
        const depStart = starts.get(depId) ?? null;
        if (dep === undefined || depStart === null || dep.durationMinutes === null) continue;
        const nodeStart = starts.get(node.id) ?? null;
        if (nodeStart === null || node.durationMinutes === null) continue;
        const depEnd = addWorkingMinutes(depStart, dep.durationMinutes, calendar);
        if (depEnd <= nodeStart) continue;
        const pushed = nextWorkingDay(depEnd, calendar);
        starts.set(node.id, pushed);
        const existing = changes.find((change) => change.taskId === node.id);
        if (existing !== undefined) {
          existing.toStart = dayKey(pushed);
        } else {
          changes.push({
            taskId: node.id,
            fromStart: node.start === null ? null : dayKey(node.start),
            toStart: dayKey(pushed),
            reason: "pushed",
          });
        }
        moved = true;
      }
    }
    if (!moved) break;
  }
  return changes;
}
