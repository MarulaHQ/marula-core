// Shared ticket-operations helpers for the ticket surfaces (/tickets, /today,
// /boards, /calendar).
//
// Purpose: one definition of what a ticket's status colour, priority chip and
// SLA countdown mean, so the list, the board and the calendar cannot drift into
// three different answers about the same row. The SLA reading follows the
// dispatch queue's own precedence (`sla_resolution_due ?? due_at`) and, where
// the server has told us, the typed SLA status rather than the raw timestamp.
//
// Non-purpose: no fetching and no rendering. Surfaces own their own requests and
// their own markup; this owns the vocabulary they share.
import { formatInTimeZone } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import type { BadgePriority, BadgeTone } from "@marula-core/ui";
import type { FilterClause, FilterOperator } from "@marula-core/ui";

export type TicketRow = components["schemas"]["TicketWithFields"];
export type TicketCategory = components["schemas"]["TicketStatusCategory"];

/** The semantic categories, in the order the status stepper walks them. */
export const CATEGORIES: TicketCategory[] = ["open", "in_progress", "waiting", "closed"];

export const CATEGORY_LABEL: Record<TicketCategory, string> = {
  open: "Open",
  in_progress: "In progress",
  waiting: "Waiting",
  closed: "Closed",
};

/**
 * The status pill's tone.
 *
 * No lifecycle status wears the WARNING pair. docs/theme.md §3.4 fixes warning
 * as "SLA risk", and the same section warns that two pairs reading at the same
 * urgency "collapse into one row in a review" — an amber pill on every brand
 * new ticket is exactly that, and it was what the first render showed.
 *
 * The palette offers three honest pairs for four categories, so the tie is
 * broken by emphasis rather than by inventing a fourth colour:
 *
 *   closed       success   the green the pack uses for Resolved
 *   in_progress  due       the blush/amber pair, which is the pack's gold
 *                          "you are here" step
 *   open         neutral   sand — nothing has happened yet
 *   waiting      neutral   sand — parked on a customer, which is a fact and
 *                          not a problem
 *
 * Open and waiting share a pair deliberately: both are "nothing is moving and
 * nothing is on fire", and they are told apart by their word and by the SLA and
 * Updated columns beside them. Painting one of them amber to force a
 * distinction would cost more in false alarm than it buys in legibility.
 */
export function statusTone(category: string): BadgeTone {
  if (category === "closed") return "success";
  if (category === "in_progress") return "due";
  // Open and Waiting both take the sand pair. See the note above.
  return "neutral";
}

/** P1 is the only priority that takes red, per DESIGN-NOTES §5. */
export function priorityBadge(priority: string): BadgePriority {
  const p = priority.toLowerCase();
  return p === "p1" || p === "p2" || p === "p3" || p === "p4" ? p : "p4";
}

/** How close to the deadline counts as "at risk", in minutes. */
export const SLA_RISK_MINUTES = 30;

export interface SlaReading {
  label: string;
  tone: BadgeTone;
  /** True while the clock is inside the risk window or already past it. */
  warn: boolean;
}

/**
 * The countdown cell. `sla` is the server's typed SLA status when the surface
 * fetched it, and `due` is the deadline the list payload carries. The two are
 * combined rather than preferred one over the other: an applied SLA is the
 * clock the customer is promised, while a manual due date is the date the
 * technician set, and a row that has both is showing whichever is nearer.
 *
 * A ticket with no SLA policy and no due date reads "—" rather than a zero,
 * because "0m left" is a breach claim and this is not one.
 */
export function slaReading(
  row: { due_at?: string | null; resolved_at?: string | null },
  sla?: components["schemas"]["TicketSlaStatus"] | null,
  now: number = Date.now(),
): SlaReading | null {
  const closed = Boolean(row.resolved_at);
  if (sla?.applied === true && sla.state?.resolution_due) {
    const remaining = Date.parse(sla.state.resolution_due) - now;
    return {
      label: countdownLabel(remaining),
      tone: sla.breached ? "danger" : remaining < SLA_RISK_MINUTES * 60_000 ? "warning" : "neutral",
      warn: sla.breached || remaining < SLA_RISK_MINUTES * 60_000,
    };
  }
  if (!row.due_at) return null;
  const remaining = Date.parse(row.due_at) - now;
  return {
    label: countdownLabel(remaining),
    tone: closed
      ? "neutral"
      : remaining <= 0
        ? "danger"
        : remaining < SLA_RISK_MINUTES * 60_000
          ? "warning"
          : "neutral",
    warn: !closed && remaining < SLA_RISK_MINUTES * 60_000,
  };
}

/** Past two days, hours stop being a unit a person reads. */
const HOURS_VISIBLE = 48;
/** Past a month the exact figure stops mattering, only the distance does. */
const DAYS_VISIBLE = 30;

function countdownLabel(remainingMs: number): string {
  if (remainingMs <= 0) {
    const over = Math.abs(remainingMs);
    return within(over, "over");
  }
  return within(remainingMs, "left");
}

/**
 * The countdown, in the largest unit that still tells a reader something.
 *
 * The first live capture of this ledger printed "303h 28m left", which is
 * twelve and a half days rendered as an hour count: technically true, useless
 * to scan, and it made every far-off deadline the same visual weight as one
 * that is due in an hour. A queue is read by exception, so the number has to
 * shrink in significance as it grows in size, not the other way round. Past a
 * month it stops pretending to be precise.
 */
function within(ms: number, suffix: "left" | "over"): string {
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 60) return `${minutes}m ${suffix}`;
  const hours = Math.floor(minutes / 60);
  if (hours <= HOURS_VISIBLE) {
    const m = minutes % 60;
    return m === 0 ? `${hours}h ${suffix}` : `${hours}h ${m}m ${suffix}`;
  }
  const days = Math.floor(hours / 24);
  if (days <= DAYS_VISIBLE) {
    const h = hours % 24;
    return h === 0 ? `${days}d ${suffix}` : `${days}d ${h}h ${suffix}`;
  }
  return `${days}d ${suffix}`;
}

/**
 * "2h ago", "just now". Absolute dates hide the recency that matters here.
 *
 * Past the month mark the phrase stops helping and a date takes over, and that
 * date is drawn in the reader's zone: an item filed at 23:40 was filed today,
 * not tomorrow, for the person reading it.
 */
export function relativeTime(iso: string, timeZone: string, now: number = Date.now()): string {
  const diffMs = now - new Date(iso).getTime();
  const mins = Math.floor(diffMs / 60_000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  return formatInTimeZone(iso, timeZone, "date");
}

/**
 * The D12 filter fields a ticket ledger offers, with the operators each one
 * actually supports. A presence test on `title` is refused up front rather than
 * shown and then never matching.
 */
export const TICKET_FILTER_FIELDS = [
  { key: "title", label: "Subject", operators: ["contains", "eq"] as FilterOperator[] },
  { key: "number", label: "Number", operators: ["contains", "eq"] as FilterOperator[] },
  { key: "client_id", label: "Client", operators: ["eq", "ne"] as FilterOperator[] },
  { key: "priority", label: "Priority", operators: ["in", "eq"] as FilterOperator[] },
  { key: "category", label: "Status", operators: ["in", "ne"] as FilterOperator[] },
  { key: "assignee_id", label: "Assignee", operators: ["is_set", "is_not_set"] as FilterOperator[] },
  { key: "due_at", label: "Due", operators: ["is_set", "is_not_set"] as FilterOperator[] },
  {
    key: "board_id",
    label: "Board",
    operators: ["eq", "is_set"] as FilterOperator[],
  },
];

/** Apply the saved clauses client-side. The list payload is one page, so the
 * filter has to run here; the server keeps its own category/priority params. */
export function matchesClause(row: TicketRow, clause: FilterClause): boolean {
  {
    const actual = (row as unknown as Record<string, unknown>)[clause.field];
    const value = (clause.value ?? "").trim();
    switch (clause.op) {
      case "eq":
        return String(actual ?? "") === value;
      case "ne":
        return String(actual ?? "") !== value;
      case "contains":
        return String(actual ?? "")
          .toLowerCase()
          .includes(value.toLowerCase());
      case "in":
        return value
          .split(",")
          .map((part) => part.trim())
          .filter(Boolean)
          .includes(String(actual ?? ""));
      case "is_set":
        return actual !== null && actual !== undefined && actual !== "";
      case "is_not_set":
        return actual === null || actual === undefined || actual === "";
      case "gt":
      case "gte":
      case "lt":
      case "lte": {
        if (actual === null || actual === undefined) return false;
        const left = typeof actual === "number" ? actual : Date.parse(String(actual));
        const right = Date.parse(value);
        if (Number.isNaN(left) || Number.isNaN(right)) return false;
        if (clause.op === "gt") return left > right;
        if (clause.op === "gte") return left >= right;
        if (clause.op === "lt") return left < right;
        return left <= right;
      }
      default:
        return true;
    }
  }
}

/** The pack's default: every clause must hold. */
export function matchesClauses(row: TicketRow, clauses: FilterClause[]): boolean {
  return clauses.every((clause) => matchesClause(row, clause));
}

/** The builder's "any": one clause holding is enough. */
export function matchesAnyClause(row: TicketRow, clauses: FilterClause[]): boolean {
  return clauses.some((clause) => matchesClause(row, clause));
}

/** A `Title` sort also moves the number with it, because a sorted list whose
 * numbers scramble reads as a different list. */
export function sortTickets(
  rows: TicketRow[],
  sort: { field: string; dir: "asc" | "desc" } | undefined,
): TicketRow[] {
  if (sort === undefined) return rows;
  const dir = sort.dir === "asc" ? 1 : -1;
  const value = (row: TicketRow): string | number => {
    if (sort.field === "priority") return { p1: 1, p2: 2, p3: 3, p4: 4 }[row.priority] ?? 9;
    const raw = (row as unknown as Record<string, unknown>)[sort.field];
    if (typeof raw === "number") return raw;
    if (raw === null || raw === undefined) return "";
    return String(raw);
  };
  return [...rows].sort((a, b) => {
    const left = value(a);
    const right = value(b);
    if (left === right) return b.number.localeCompare(a.number);
    return left > right ? dir : -dir;
  });
}
