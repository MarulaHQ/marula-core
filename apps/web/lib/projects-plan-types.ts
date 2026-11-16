// Local shapes and arithmetic for the project Plan surface.
//
// The Wave 68 project endpoints (schedule preview/apply/revert/scale, the
// working calendar, risk signals and baselines) exist on the API but are not in
// the generated SDK yet. They move to `@marula-core/sdk` at merge, when the
// OpenAPI pins are regenerated; until then this module is the narrow, local
// declaration of exactly the shapes the plan reads and writes.
//
// The date arithmetic is deliberate about one thing: plan dates are day-level
// facts carried as instants. The engine snaps a start to the next working day,
// and every label here is formatted in UTC so a date cannot move by one because
// a browser sits west of the office.

import { api, resolveApiBaseUrl } from "./api";

// --- task and phase extensions -------------------------------------------------------

export type PlanWorkType =
  | "onsite"
  | "bench"
  | "remote"
  | "vendor"
  | "client"
  | "coordination";

export interface PlanTask {
  id: string;
  org_id: string;
  project_id: string;
  phase_id?: string | null;
  title: string;
  description?: string | null;
  status: "pending" | "in_progress" | "blocked" | "completed" | "cancelled";
  sort_order: number;
  assignee_id?: string | null;
  is_milestone: boolean;
  milestone_amount_minor?: number | null;
  milestone_currency?: string | null;
  due_at?: string | null;
  completed_at?: string | null;
  created_at: string;
  updated_at: string;
  start_at?: string | null;
  duration_minutes?: number | null;
  effort_estimate_minutes?: number | null;
  effort_remaining_minutes?: number | null;
  work_type?: PlanWorkType | null;
  priority?: "p1" | "p2" | "p3" | "p4" | null;
  role_key?: string | null;
  required_skills?: string[] | null;
  is_critical?: boolean;
  baseline_start_at?: string | null;
  baseline_duration_minutes?: number | null;
  external_owner?: string | null;
}

export interface PlanPhase {
  id: string;
  org_id: string;
  project_id: string;
  name: string;
  sort_order: number;
  status: "pending" | "active" | "completed" | "cancelled";
  is_milestone: boolean;
  milestone_amount_minor?: number | null;
  milestone_currency?: string | null;
  description?: string | null;
  start_date?: string | null;
  end_date?: string | null;
  completed_at?: string | null;
  created_at: string;
  updated_at: string;
}

// --- schedule ------------------------------------------------------------------------

export interface ScheduleChange {
  task_id: string;
  from_start: string | null;
  to_start: string | null;
  /** `moved` for the task the coordinator placed, `pushed` for a successor. */
  reason: "moved" | "pushed" | string;
}

/** Someone else's work that would sit at the same time as the proposed move. */
export interface ScheduleCollision {
  task_id: string;
  project_id: string;
  project_name: string;
  task_title: string;
  assignee_id: string | null;
  start_at: string;
  end_at: string;
}

export interface SchedulePreview {
  changes: ScheduleChange[];
  critical_path: string[];
  collisions?: ScheduleCollision[];
}

export interface ScheduleRestore {
  task_id: string;
  start_at: string | null;
  duration_minutes: number | null;
}

export interface ScheduleApply extends SchedulePreview {
  revert: ScheduleRestore[];
}

export interface ScheduleMove {
  task_id: string;
  start_at: string | null;
  /** Omit to leave the length alone. `null` clears it with the start. */
  duration_minutes?: number | null;
}

export interface ScheduleScaleRequest {
  factor: number;
  from_start_at?: string | null;
  to_start_at?: string | null;
  reschedule: boolean;
}

// --- calendar, risk, baseline ---------------------------------------------------------

export interface WorkingCalendar {
  project_id: string;
  org_id: string;
  /** Monday at bit 0, Sunday at bit 6. Monday to Friday is 31. */
  weekdays: number;
  minutes_per_day: number;
  note?: string | null;
  created_at: string;
  updated_at: string;
}

export const DEFAULT_CALENDAR: { weekdays: number; minutes_per_day: number } = {
  weekdays: 31,
  minutes_per_day: 480,
};

export interface RiskSignal {
  rule: string;
  severity: "info" | "warn" | "critical" | string;
  subject: "project" | "phase" | "task" | string;
  subject_id: string;
  message: string;
}

export interface RiskResponse {
  signals: RiskSignal[];
  generated_at: string;
}

export interface BaselineView {
  id: string;
  captured_by: string | null;
  captured_at: string;
  note: string | null;
  dated_tasks: number;
  total_tasks: number;
}

export interface PlanPerson {
  id: string;
  display_name: string;
  email: string;
  is_active: boolean;
}

// --- transport ------------------------------------------------------------------------

interface ErrorEnvelope {
  error?: { message?: string; reason?: string };
}

/**
 * One JSON call against the project endpoints the SDK does not carry yet.
 * A non-2xx answer becomes an Error carrying the API's own message, because the
 * plan surface shows the refusal next to the table rather than inventing one.
 */
export async function planFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const token = api.tokens.accessToken;
  const response = await fetch(`${resolveApiBaseUrl()}${path}`, {
    ...init,
    headers: {
      "content-type": "application/json",
      ...(token === null ? {} : { authorization: `Bearer ${token}` }),
      ...init?.headers,
    },
  });
  if (!response.ok) {
    let message = `The server refused this with ${response.status}.`;
    try {
      const body = (await response.json()) as ErrorEnvelope;
      if (body.error?.message !== undefined) message = body.error.message;
    } catch {
      // A non-JSON body is not worth guessing at; the status stands.
    }
    throw new Error(message);
  }
  return (await response.json()) as T;
}

export function previewMoves(projectId: string, moves: ScheduleMove[]): Promise<SchedulePreview> {
  return planFetch<SchedulePreview>(`/api/v1/projects/${projectId}/schedule/preview`, {
    method: "POST",
    body: JSON.stringify({ moves }),
  });
}

export function applyMoves(projectId: string, moves: ScheduleMove[]): Promise<ScheduleApply> {
  return planFetch<ScheduleApply>(`/api/v1/projects/${projectId}/schedule/apply`, {
    method: "POST",
    body: JSON.stringify({ moves }),
  });
}

export function revertMoves(
  projectId: string,
  restore: ScheduleRestore[],
): Promise<SchedulePreview> {
  return planFetch<SchedulePreview>(`/api/v1/projects/${projectId}/schedule/revert`, {
    method: "POST",
    body: JSON.stringify({ restore }),
  });
}

export function scaleSchedule(
  projectId: string,
  request: ScheduleScaleRequest,
): Promise<SchedulePreview> {
  return planFetch<SchedulePreview>(`/api/v1/projects/${projectId}/schedule/scale`, {
    method: "POST",
    body: JSON.stringify(request),
  });
}

/** One group of the plan: the tasks of a phase, or the unphased group. */
export interface PlanOrderGroup {
  phase_id: string | null;
  task_ids: string[];
}

/** The whole plan order, as a drop leaves it. */
export interface PlanOrder {
  phases: string[];
  tasks: PlanOrderGroup[];
}

/**
 * Write the plan's order in one request.
 *
 * A drag can cross a phase boundary, so a task's group is also its phase: the
 * endpoint re-parents it rather than only renumbering it.
 */
export function savePlanOrder(
  projectId: string,
  order: PlanOrder,
): Promise<{ phases: number; tasks: number; rephased: number }> {
  return planFetch<{ phases: number; tasks: number; rephased: number }>(
    `/api/v1/projects/${projectId}/order`,
    { method: "PUT", body: JSON.stringify(order) },
  );
}

export async function loadCalendar(projectId: string): Promise<WorkingCalendar | null> {
  const body = await planFetch<{ calendar: WorkingCalendar | null }>(
    `/api/v1/projects/${projectId}/calendar`,
  );
  return body.calendar;
}

export function loadRisk(projectId: string): Promise<RiskResponse> {
  return planFetch<RiskResponse>(`/api/v1/projects/${projectId}/risk`);
}

export async function loadBaseline(projectId: string): Promise<BaselineView | null> {
  const body = await planFetch<{ baseline: BaselineView | null }>(
    `/api/v1/projects/${projectId}/baseline`,
  );
  return body.baseline;
}

export async function captureBaseline(
  projectId: string,
  note: string,
): Promise<BaselineView | null> {
  const body = await planFetch<{ baseline: BaselineView | null }>(
    `/api/v1/projects/${projectId}/baseline`,
    { method: "POST", body: JSON.stringify({ note: note.trim() === "" ? null : note.trim() }) },
  );
  return body.baseline;
}

export async function loadPeople(): Promise<PlanPerson[]> {
  try {
    const body = await planFetch<{ users: PlanPerson[] }>("/api/v1/org/users?limit=200");
    return body.users;
  } catch {
    // The staff directory needs a manager role. A technician who can see the
    // plan still sees the role the work needs; only the person's name is
    // missing, and the row says so with a dashed placeholder.
    return [];
  }
}

// --- date arithmetic ------------------------------------------------------------------

const MONTHS = [
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
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"] as const;
const DAY_MS = 86_400_000;

function utc(value: string): Date {
  const hasZone = /(?:Z|[+-]\d{2}:?\d{2})$/.test(value);
  const date = new Date(hasZone || value.includes("T") ? value : `${value}T00:00:00Z`);
  return Number.isNaN(date.getTime()) ? new Date(0) : date;
}

/** `9 Nov` — the plan's date face. UTC, so the day cannot shift west of zero. */
export function formatDay(value: string | null | undefined): string {
  if (value === null || value === undefined || value === "") return "—";
  const date = utc(value);
  return `${date.getUTCDate()} ${MONTHS[date.getUTCMonth()]}`;
}

/** `Mon 2 Nov` — for a promise date, where the weekday is part of the claim. */
export function formatWeekday(value: string | null | undefined): string {
  if (value === null || value === undefined || value === "") return "Not set";
  const date = utc(value);
  return `${WEEKDAYS[date.getUTCDay()]} ${formatDay(value)}`;
}

export function formatDayRange(
  start: string | null | undefined,
  end: string | null | undefined,
): string {
  if (start === null || start === undefined || start === "") return "No dates";
  if (end === null || end === undefined || end === "") return formatDay(start);
  return `${formatDay(start)} to ${formatDay(end)}`;
}

/** The value a `<input type="date">` wants. */
export function dateInputValue(value: string | null | undefined): string {
  if (value === null || value === undefined || value === "") return "";
  return utc(value).toISOString().slice(0, 10);
}

/** A date input's day turned into the instant the schedule engine expects. The
 *  engine snaps it to the next working day; midnight UTC keeps the day stable. */
export function startOfDay(value: string): string {
  return `${value}T00:00:00.000Z`;
}

export function isWorkingWeekday(date: Date, weekdays: number): boolean {
  const bit = (date.getUTCDay() + 6) % 7; // Monday = 0
  return (weekdays & (1 << bit)) !== 0;
}

/** Working days in `[start, end]`, inclusive, against the project's pattern. */
export function workingDaysBetween(
  start: string | null | undefined,
  end: string | null | undefined,
  weekdays = DEFAULT_CALENDAR.weekdays,
): number {
  if (start === null || start === undefined || end === null || end === undefined) return 0;
  const from = utc(dateInputValue(start));
  const to = utc(dateInputValue(end));
  if (to.getTime() < from.getTime()) return 0;
  let count = 0;
  for (let t = from.getTime(); t <= to.getTime(); t += DAY_MS) {
    if (isWorkingWeekday(new Date(t), weekdays)) count += 1;
  }
  return count;
}

/** The next working day at or after `start`, `days` worked days later. */
export function addWorkingDays(
  start: string,
  days: number,
  weekdays = DEFAULT_CALENDAR.weekdays,
): string {
  let date = utc(dateInputValue(start));
  let remaining = Math.max(0, Math.trunc(days));
  while (remaining > 0) {
    date = new Date(date.getTime() + DAY_MS);
    if (isWorkingWeekday(date, weekdays)) remaining -= 1;
  }
  return date.toISOString().slice(0, 10);
}

/** The first working instant at or after `value`, keeping its clock time. */
function nextWorkingInstant(value: Date, weekdays: number): Date {
  let cursor = value;
  for (let step = 0; step < 7; step += 1) {
    if (isWorkingWeekday(cursor, weekdays)) return cursor;
    cursor = new Date(cursor.getTime() + DAY_MS);
  }
  return cursor;
}

/**
 * Adds working minutes to an instant the same way the engine does: the first
 * day offers what is left of it, every later day a full one, and a landing on
 * a day boundary belongs to the next working day.
 */
export function addWorkingMinutes(
  start: string,
  minutes: number,
  weekdays = DEFAULT_CALENDAR.weekdays,
  minutesPerDay = DEFAULT_CALENDAR.minutes_per_day,
): string {
  const perDay = Math.max(1, minutesPerDay);
  let cursor = nextWorkingInstant(utc(start), weekdays);
  let remaining = Math.max(0, Math.round(minutes));
  while (remaining > perDay) {
    remaining -= perDay;
    cursor = nextWorkingInstant(new Date(cursor.getTime() + DAY_MS), weekdays);
  }
  return new Date(cursor.getTime() + remaining * 60_000).toISOString();
}

/** The instant a dated task ends, from its start and length, in working time. */
export function taskEnd(
  task: Pick<PlanTask, "start_at" | "duration_minutes">,
  weekdays = DEFAULT_CALENDAR.weekdays,
  minutesPerDay = DEFAULT_CALENDAR.minutes_per_day,
): string | null {
  if (task.start_at === null || task.start_at === undefined) return null;
  if (task.duration_minutes === null || task.duration_minutes === undefined) return null;
  return addWorkingMinutes(task.start_at, task.duration_minutes, weekdays, minutesPerDay);
}

/** Length in working days, as the DAYS column reads it. */
export function taskDays(
  task: Pick<PlanTask, "start_at" | "duration_minutes">,
  minutesPerDay = DEFAULT_CALENDAR.minutes_per_day,
): number | null {
  if (task.duration_minutes === null || task.duration_minutes === undefined) return null;
  return Math.max(1, Math.ceil(task.duration_minutes / Math.max(1, minutesPerDay)));
}

export function hoursFromMinutes(minutes: number | null | undefined): number {
  if (minutes === null || minutes === undefined) return 0;
  return Math.round((minutes / 60) * 10) / 10;
}

/** `2 h`, `0.5 h`, `—`. Numbers stay mono in the calling cell. */
export function hoursLabel(minutes: number | null | undefined): string {
  if (minutes === null || minutes === undefined || minutes <= 0) return "—";
  const hours = hoursFromMinutes(minutes);
  return `${hours} h`;
}

export function effortOf(tasks: readonly PlanTask[]): {
  estimateMinutes: number;
  remainingMinutes: number;
  loggedMinutes: number;
} {
  let estimateMinutes = 0;
  let remainingMinutes = 0;
  for (const task of tasks) {
    estimateMinutes += task.effort_estimate_minutes ?? 0;
    remainingMinutes += task.effort_remaining_minutes ?? 0;
  }
  return {
    estimateMinutes,
    remainingMinutes,
    loggedMinutes: Math.max(0, estimateMinutes - remainingMinutes),
  };
}

/** `+5 d`, `-2 d`, `same day` — the delta between two plan dates, in worked days. */
export function workingDelta(
  from: string | null,
  to: string | null,
  weekdays = DEFAULT_CALENDAR.weekdays,
): string {
  if (from === null || to === null) return "—";
  const forward = utc(dateInputValue(to)).getTime() >= utc(dateInputValue(from)).getTime();
  const span = forward
    ? workingDaysBetween(from, to, weekdays)
    : workingDaysBetween(to, from, weekdays);
  const days = span === 0 ? 0 : (forward ? 1 : -1) * Math.max(0, span - 1);
  if (days === 0) return "same day";
  return `${days > 0 ? "+" : ""}${days} d`;
}

export function todayIso(now = Date.now()): string {
  return new Date(now).toISOString().slice(0, 10);
}

export function isTaskLate(
  task: Pick<PlanTask, "status" | "start_at" | "duration_minutes">,
  now = Date.now(),
  weekdays = DEFAULT_CALENDAR.weekdays,
  minutesPerDay = DEFAULT_CALENDAR.minutes_per_day,
): boolean {
  if (task.status === "completed" || task.status === "cancelled") return false;
  const end = taskEnd(task, weekdays, minutesPerDay);
  if (end === null) return false;
  return utc(end).getTime() < now;
}

export function initialsOf(name: string): string {
  return name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((word) => word[0]!.toUpperCase())
    .join("");
}

/** `field_technician` -> `Field technician`, so a bare role key reads as words. */
export function roleLabel(key: string | null | undefined): string {
  if (key === null || key === undefined || key.trim() === "") return "";
  const words = key.replaceAll("_", " ").replaceAll("-", " ").trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** `p1` is the one priority that takes the red mark (theme.md §6). */
export function priorityLabel(priority: PlanTask["priority"]): string | null {
  if (priority === null || priority === undefined) return null;
  return priority.toUpperCase();
}
