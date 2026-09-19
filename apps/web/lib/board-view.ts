// Pure board-view logic for the kanban surface (Wave 17 lane 3).
//
// Purpose: testable column grouping, filtering, drag validity, and the
// priority/due chips' semantic tones — everything the kanban renders is
// decided here so the component stays a thin view. Status tint parity and
// category tones mirror the shared Workbench status tokens (the same
// mapping the ticket status bar uses), so light and dark stay in lockstep.
// Non-purpose: no data fetching, no envelope writes, no React.
import type { components } from "@meridian/sdk";

export type BoardStatus = components["schemas"]["BoardStatus"];
export type BoardTicket = components["schemas"]["Ticket"];
export type BoardTransitions = components["schemas"]["BoardTransitionsResponse"];
export type Priority = components["schemas"]["Priority"];

export const PRIORITIES: Priority[] = ["p1", "p2", "p3", "p4"];

/** Category → semantic tone, shared with the ticket status bar. */
export function categoryTone(category: string): "success" | "warning" | "neutral" {
  if (category === "closed") return "success";
  if (category === "waiting") return "warning";
  return "neutral";
}

/** Raw category slugs → the human names the column headers carry
 * (Wave 23 lane 1, item 5): a card with no resolvable status row renders
 * "In progress", never the `in_progress` wire slug. Unknown slugs fall
 * back to themselves (honest) after title-casing is skipped — the known
 * four categories are exhaustive on the wire, so this only guards legacy
 * or future additions. */
const CATEGORY_LABEL: Record<string, string> = {
  open: "Open",
  in_progress: "In progress",
  waiting: "Waiting",
  closed: "Closed",
};

export function humanCategoryLabel(category: string | null | undefined): string {
  if (category === null || category === undefined) return "No status";
  const label = CATEGORY_LABEL[category];
  return label ?? category;
}

/** Client mirror of the core-domain transition verdict (same as status bar). */
export function transitionAllowed(
  statuses: BoardStatus[],
  workflow: BoardTransitions | null,
  fromId: string,
  toId: string,
): boolean {
  if (fromId === toId) return false;
  const explicit =
    workflow !== null && (workflow.mode ?? "floor") === "explicit"
      ? (workflow.edges ?? [])
      : null;
  if (explicit !== null) {
    return explicit.some((e) => e.from_status_id === fromId && e.to_status_id === toId);
  }
  // Floor mode (or unknown workflow): the semantic floor governs.
  const catOf = new Map(statuses.map((s) => [s.id, s.category]));
  const fromCat = catOf.get(fromId);
  const toCat = catOf.get(toId);
  if (fromCat === undefined || toCat === undefined) return false;
  return !(fromCat === "closed" && toCat === "open");
}

export function sortStatuses(statuses: BoardStatus[]): BoardStatus[] {
  return [...statuses].sort((a, b) => a.sort_order - b.sort_order);
}

export interface BoardFilters {
  query: string;
  priorities: Priority[];
  unassignedOnly: boolean;
  /** Per-user assignee filter (Wave 17 lane 12): a directory user id from
   * the org directory picker, or null for no assignee filter. Server-gated:
   * the picker feeds only ids GET /api/v1/org/users returned — no client
   * matrix is invented. */
  assigneeId: string | null;
}

export const EMPTY_FILTERS: BoardFilters = {
  query: "",
  priorities: [],
  unassignedOnly: false,
  assigneeId: null,
};

export function filtersActive(f: BoardFilters): boolean {
  return f.query.trim() !== "" || f.priorities.length > 0 || f.unassignedOnly || (f.assigneeId ?? null) !== null;
}

/** Case-insensitive priority (wire is lowercase; some fixtures uppercase). */
export function filterTickets(tickets: BoardTicket[], f: BoardFilters): BoardTicket[] {
  const q = f.query.trim().toLowerCase();
  return tickets.filter((t) => {
    if (q !== "" && !t.title.toLowerCase().includes(q) && !t.number.toLowerCase().includes(q)) {
      return false;
    }
    if (f.priorities.length > 0 && !f.priorities.includes(t.priority.toLowerCase() as Priority)) {
      return false;
    }
    if (f.unassignedOnly && t.assignee_id !== null && t.assignee_id !== undefined) {
      return false;
    }
    if ((f.assigneeId ?? null) !== null && t.assignee_id !== f.assigneeId) {
      return false;
    }
    return true;
  });
}

/** Group filtered tickets by status id, preserving column order. */
export function groupByStatus(
  tickets: BoardTicket[],
  statuses: BoardStatus[],
): Map<string, BoardTicket[]> {
  const groups = new Map<string, BoardTicket[]>(statuses.map((s) => [s.id, []]));
  for (const t of tickets) {
    if (t.status_id === null) continue;
    const bucket = groups.get(t.status_id);
    if (bucket === undefined) continue;
    bucket.push(t);
  }
  return groups;
}

/** Priority chip tone: P1 danger, P2 warning, P3/P4 neutral. */
export function priorityTone(priority: string): "danger" | "warning" | "neutral" {
  const p = priority.toLowerCase();
  if (p === "p1") return "danger";
  if (p === "p2") return "warning";
  return "neutral";
}

export interface DueChip {
  label: string;
  tone: "danger" | "warning" | "neutral";
}

/**
 * Due-date chip: danger when overdue, warning inside 24h, neutral label
 * otherwise. Absent due date → null (no chip).
 */
export function dueChip(dueAt: string | null | undefined, now: number): DueChip | null {
  if (dueAt === null || dueAt === undefined) return null;
  const due = Date.parse(dueAt);
  if (Number.isNaN(due)) return null;
  const diffMs = due - now;
  const label = new Date(dueAt).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
  if (diffMs < 0) return { label: `Overdue · ${label}`, tone: "danger" };
  if (diffMs < 24 * 60 * 60 * 1000) return { label, tone: "warning" };
  return { label, tone: "neutral" };
}
