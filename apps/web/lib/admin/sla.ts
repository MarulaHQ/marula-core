// Pure state logic for the /settings/sla editors.
//
// Purpose: ladder-step helpers (offset → notification-kind suggestion,
// validation, payload shaping matching the SDK's ReplaceLadderRequest) and
// business-calendar draft helpers (weekday windows, holidays, timezone).
// Unit-testable pure functions only.
//
// Non-purpose: no network I/O; SLA evaluation itself stays server-side.
import type { components } from "@meridian/sdk";

export type EscalationStep = components["schemas"]["EscalationStep"];
export type LadderStepSpec = components["schemas"]["LadderStepSpec"];
export type ReplaceLadderRequest = components["schemas"]["ReplaceLadderRequest"];
export type PolicyView = components["schemas"]["PolicyView"];

export const LADDER_KINDS = ["risk", "escalate", "breach"] as const;
export type LadderKind = (typeof LADDER_KINDS)[number];

export function isLadderKind(value: string): value is LadderKind {
  return (LADDER_KINDS as readonly string[]).includes(value);
}

/**
 * Offset → notification-kind mapping used to prefill a new step:
 *   <50% of budget consumed → "risk"     (early heads-up, SlaRisk kind)
 *   50–99%                  → "escalate" (fan-out beyond assignee)
 *   ≥100%                   → "breach"   (target missed, SlaBreach kind)
 */
export function suggestKindForOffset(atPct: number): LadderKind {
  if (atPct >= 100) return "breach";
  if (atPct >= 50) return "escalate";
  return "risk";
}

/** Human explanation for each trigger kind (notification-center kinds). */
export function kindExplanation(kind: LadderKind): string {
  switch (kind) {
    case "risk":
      return "Warning as the clock runs down — notifies via the SLA-risk kind.";
    case "escalate":
      return "Re-alert beyond the assignee — fans out to the escalation kind.";
    case "breach":
      return "Target missed — raises the SLA-breach kind.";
  }
}

/**
 * Fire-once contract text rendered with the ladder editor: every enabled
 * step fires at most once per ticket per SLA target — the sweep records
 * fired (ticket, step, target) rows and never repeats them.
 */
export const FIRE_ONCE_NOTE =
  "Each step fires at most once per ticket per target — re-arming requires a new breach cycle.";

// --- ladder draft ------------------------------------------------------------

/** One editable ladder row (rows may pre-date enable toggles). */
export interface LadderRow {
  /** Stable client key; null until adopted from the server step id. */
  key: string;
  /** Copy of server id when this row came from the API, else null. */
  id: string | null;
  atPct: number;
  kind: LadderKind | null;
  notifyUserIds: string[];
  enabled: boolean;
}

export interface LadderDraftState {
  rows: LadderRow[];
}

export type LadderDraftAction =
  | { type: "init"; steps: EscalationStep[] }
  | { type: "add-row"; defaultAtPct?: number }
  | { type: "update"; key: string; patch: Partial<Omit<LadderRow, "key" | "id">> }
  | { type: "remove"; key: string };

let ladderKeySeq = 0;
function nextKey(): string {
  ladderKeySeq += 1;
  return `row-${ladderKeySeq}`;
}

export function ladderReducer(
  state: LadderDraftState,
  action: LadderDraftAction,
): LadderDraftState {
  switch (action.type) {
    case "init": {
      const rows = [...action.steps]
        .sort((a, b) => a.step_index - b.step_index)
        .map((s) => ({
          key: `srv-${s.id}`,
          id: s.id,
          atPct: s.at_pct,
          kind: isLadderKind(s.trigger_kind) ? s.trigger_kind : null,
          notifyUserIds: [...s.notify_user_ids],
          enabled: s.enabled,
        }));
      return { rows };
    }
    case "add-row": {
      const atPct = action.defaultAtPct ?? 75;
      return {
        rows: [
          ...state.rows,
          {
            key: nextKey(),
            id: null,
            atPct,
            kind: suggestKindForOffset(atPct),
            notifyUserIds: [],
            enabled: true,
          },
        ],
      };
    }
    case "update": {
      return {
        rows: state.rows.map((row) => {
          if (row.key !== action.key) return row;
          const next = { ...row, ...action.patch };
          // Keep the suggested kind in sync ONLY while it is still pristine
          // (untouched rows follow their offset); explicit choices stick.
          if (
            action.patch.atPct !== undefined &&
            action.patch.kind === undefined &&
            (row.kind === null || row.kind === suggestKindForOffset(row.atPct))
          ) {
            next.kind = suggestKindForOffset(next.atPct);
          }
          return next;
        }),
      };
    }
    case "remove":
      return { rows: state.rows.filter((row) => row.key !== action.key) };
  }
}

export interface LadderValidation {
  ok: boolean;
  errorsByIndex: Record<number, string>;
}

/**
 * Mirrors server constraints: percent integer within 1..=500, known
 * trigger kind, contiguous unique step indices after ordering.
 */
export function validateLadder(state: LadderDraftState): LadderValidation {
  const errorsByIndex: Record<number, string> = {};
  state.rows.forEach((row, index) => {
    if (!Number.isInteger(row.atPct) || row.atPct < 1 || row.atPct > 500) {
      errorsByIndex[index] = "Percent must be an integer between 1 and 500.";
      return;
    }
    if (row.kind === null || !isLadderKind(row.kind)) {
      errorsByIndex[index] = "Pick a notification kind.";
    }
  });
  return { ok: Object.keys(errorsByIndex).length === 0, errorsByIndex };
}

/** Ordered PUT body; disabled rows persist so toggles survive reloads. */
export function ladderPayload(state: LadderDraftState): ReplaceLadderRequest {
  const steps: LadderStepSpec[] = state.rows.map((row, index) => ({
    at_pct: row.atPct,
    enabled: row.enabled,
    notify_user_ids: [...row.notifyUserIds],
    step_index: index,
    trigger_kind: row.kind ?? "risk",
  }));
  return { steps };
}

/** Plain-language target summary for one policy (priority → minutes). */
export function policyTargetSummary(policy: PolicyView): string {
  const targets = (policy.targets ?? {}) as Record<string, unknown>;
  const entries = Object.entries(targets);
  if (entries.length === 0) return "No targets set";
  return entries
    .map(([prio, mins]) => `${prio.toUpperCase()} ${String(mins)}m`)
    .join(" · ");
}

// --- calendar draft ----------------------------------------------------------

export const WEEKDAY_LABELS = [
  "Monday",
  "Tuesday",
  "Wednesday",
  "Thursday",
  "Friday",
  "Saturday",
  "Sunday",
] as const;

const HH_MM = /^([01]\d|2[0-3]):[0-5]\d$/;

export interface CalendarDraftState {
  name: string;
  timezone: string;
  /** [Mon..Sun]; null = closed. Each open window is an [open, close] pair. */
  windows: ([string, string] | null)[];
  /** Local dates YYYY-MM-DD. */
  holidays: string[];
}

export type CalendarDraftAction =
  | { type: "init"; draft: CalendarDraftState }
  | { type: "set-name"; value: string }
  | { type: "set-timezone"; value: string }
  | { type: "toggle-day"; index: number }
  | { type: "set-window"; index: number; half: 0 | 1; value: string }
  | { type: "add-holiday"; date: string }
  | { type: "remove-holiday"; date: string };

/** Empty (always-closed) draft used by the create dialog. */
export function emptyCalendarDraft(): CalendarDraftState {
  return {
    name: "",
    timezone: "UTC",
    windows: WEEKDAY_LABELS.map(() => null),
    holidays: [],
  };
}

export function calendarDraftFromBusinessCalendar(
  name: string,
  calendar: {
    timezone: string;
    windows: ([string, string] | null)[];
    holidays: string[];
  },
): CalendarDraftState {
  // Normalize short wires (legacy calendars may store fewer days).
  const windows = WEEKDAY_LABELS.map((_, i) => calendar.windows[i] ?? null);
  return {
    name,
    timezone: calendar.timezone,
    windows,
    holidays: [...calendar.holidays],
  };
}

export function calendarReducer(
  state: CalendarDraftState,
  action: CalendarDraftAction,
): CalendarDraftState {
  switch (action.type) {
    case "init":
      return action.draft;
    case "set-name":
      return { ...state, name: action.value };
    case "set-timezone":
      return { ...state, timezone: action.value };
    case "toggle-day": {
      const windows = [...state.windows];
      const current = windows[action.index];
      // Closed → standard 09:00–17:00; open → closed.
      windows[action.index] = current === null || current === undefined ? ["09:00", "17:00"] : null;
      return { ...state, windows };
    }
    case "set-window": {
      const windows = [...state.windows];
      const current = windows[action.index];
      if (current === null || current === undefined) return state;
      const next: [string, string] = [current[0], current[1]];
      next[action.half] = action.value;
      windows[action.index] = next;
      return { ...state, windows };
    }
    case "add-holiday": {
      if (state.holidays.includes(action.date)) return state;
      return { ...state, holidays: [...state.holidays, action.date].sort() };
    }
    case "remove-holiday":
      return { ...state, holidays: state.holidays.filter((d) => d !== action.date) };
  }
}

/** Row-level check shown inline before save; mirrors domain rules loosely. */
export function validateCalendar(state: CalendarDraftState): Record<string, string> {
  const errors: Record<string, string> = {};
  if (state.name.trim().length === 0) errors.name = "Name is required.";
  state.windows.forEach((w, i) => {
    if (w === null) return;
    if (!HH_MM.test(w[0]) || !HH_MM.test(w[1])) {
      errors[`window-${i}`] = "Use HH:MM (24h).";
      return;
    }
    if (w[0] >= w[1]) errors[`window-${i}`] = "Close must be after open.";
  });
  return errors;
}
