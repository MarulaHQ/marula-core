// Calendar presentation helpers for Wave 17 lane 4: event classification,
// overlap detection, month-grid construction, and week fetch planning.
// Everything derives from the CalendarViewResponse shape (items +
// exclusions) plus ticket deadlines fetched separately — no invented data.
import type { components } from "@marula-core/sdk";

export type CalendarItem = components["schemas"]["CalendarItem"];
export type CalendarExclusion = components["schemas"]["CalendarExclusion"];

const SYNC_REASON_PREFIX = "Calendar sync:";

export type EntryKind =
  | "appointment"
  | "on_call"
  | "synced_busy"
  | "time_off"
  | "ticket_deadline";

export interface CalendarEntry {
  kind: EntryKind;
  id: string;
  title: string;
  startsAt: string;
  endsAt: string;
  ticketId: string | null;
  itemKind: CalendarItem["kind"];
  status: CalendarItem["status"];
  userId: string | null;
  number: string | null;
  /** Wave 32 lane 7: booked travel minutes immediately before the
   *  appointment (0 = none). Rendered as a distinct band, never merged
   *  into the appointment block. */
  travelMinutes: number;
  /** `heuristic` | `measured` when the estimate carries provenance. */
  travelQuality: string | null;
}

/** The routing fields ride `CalendarItem` additively; the generated SDK
 *  type gains them on the next openapi re-pin (Wave 32 lane 7). */
type CalendarItemWithTravel = CalendarItem & {
  travel_minutes?: number | null;
  travel_quality?: string | null;
};

/** One calendar entry per external busy mirror, from the exclusion reason marker. */
export function isSyncFedExclusion(exclusion: CalendarExclusion): boolean {
  return (exclusion.reason ?? "").startsWith(SYNC_REASON_PREFIX);
}

export function entryFromItem(item: CalendarItem): CalendarEntry {
  const routed = item as CalendarItemWithTravel;
  return {
    kind: item.item_type === "on_call" ? "on_call" : "appointment",
    id: item.id,
    title: item.title ?? item.category ?? "Appointment",
    startsAt: item.starts_at,
    endsAt: item.ends_at,
    ticketId: item.ticket_id ?? null,
    itemKind: item.kind ?? null,
    status: item.status ?? null,
    userId: item.user_id,
    number: null,
    travelMinutes: routed.travel_minutes ?? 0,
    travelQuality: routed.travel_quality ?? null,
  };
}

export function entryFromExclusion(exclusion: CalendarExclusion): CalendarEntry {
  const synced = isSyncFedExclusion(exclusion);
  const rawTitle = (exclusion.reason ?? "").slice(SYNC_REASON_PREFIX.length).trim();
  return {
    kind: synced ? "synced_busy" : "time_off",
    id: exclusion.id,
    title: synced
      ? rawTitle === ""
        ? "Synced busy time"
        : rawTitle
      : (exclusion.reason ?? "Time off"),
    startsAt: exclusion.starts_at,
    endsAt: exclusion.ends_at,
    ticketId: null,
    itemKind: null,
    status: null,
    userId: exclusion.user_id,
    number: null,
    travelMinutes: 0,
    travelQuality: null,
  };
}

export function entryFromTicketDeadline(ticket: {
  id: string;
  number: string;
  title: string;
  due_at: string | null;
}): CalendarEntry | null {
  if (ticket.due_at === null) return null;
  return {
    kind: "ticket_deadline",
    id: `deadline:${ticket.id}`,
    title: `${ticket.number} ${ticket.title}`,
    startsAt: ticket.due_at,
    endsAt: ticket.due_at,
    ticketId: ticket.id,
    itemKind: null,
    status: null,
    userId: null,
    number: ticket.number,
    travelMinutes: 0,
    travelQuality: null,
  };
}

/** Sorted merged list for agenda rendering; duplicate ids collapse (a month view reads overlapping week windows). */
export function mergeEntries(entries: CalendarEntry[]): CalendarEntry[] {
  const seen = new Set<string>();
  return [...entries]
    .sort(
      (a, b) => new Date(a.startsAt).getTime() - new Date(b.startsAt).getTime() || a.id.localeCompare(b.id),
    )
    .filter((entry) => {
      const key = `${entry.kind}:${entry.id}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
}

const intersects = (a: CalendarEntry, b: CalendarEntry): boolean =>
  new Date(a.startsAt).getTime() < new Date(b.endsAt).getTime() &&
  new Date(b.startsAt).getTime() < new Date(a.endsAt).getTime();

/**
 * Ids of entries that intersect another entry (same owner scope). The
 * appointment × synced-busy intersection is the sync conflict signal the
 * sync panel's overlap counters report.
 */
export function detectOverlaps(entries: CalendarEntry[]): Set<string> {
  const flagged = new Set<string>();
  for (let i = 0; i < entries.length; i += 1) {
    const a = entries[i];
    if (a === undefined) continue;
    for (let j = i + 1; j < entries.length; j += 1) {
      const b = entries[j];
      if (b === undefined) continue;
      if (intersects(a, b)) {
        flagged.add(a.id);
        flagged.add(b.id);
      }
    }
  }
  return flagged;
}

export function entriesOnDay(entries: CalendarEntry[], day: Date): CalendarEntry[] {
  return entries.filter((entry) => sameLocalDay(entry.startsAt, day));
}

export function sameLocalDay(iso: string, day: Date): boolean {
  const d = new Date(iso);
  return (
    d.getFullYear() === day.getFullYear() &&
    d.getMonth() === day.getMonth() &&
    d.getDate() === day.getDate()
  );
}

export function mondayOf(d: Date): Date {
  const out = new Date(d);
  const shift = (out.getDay() + 6) % 7;
  out.setDate(out.getDate() - shift);
  out.setHours(0, 0, 0, 0);
  return out;
}

export function isoDay(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** Week-start rows (Mon) covering a month view, padding days included (5–6 rows). */
export function monthWeeks(year: number, monthIndex: number): Date[][] {
  const gridStart = mondayOf(new Date(year, monthIndex, 1));
  const monthEnd = new Date(year, monthIndex + 1, 0); // last day of month
  const weeks: Date[][] = [];
  let cursor = gridStart;
  while (cursor <= monthEnd) {
    const week: Date[] = [];
    for (let i = 0; i < 7; i += 1) {
      week.push(new Date(cursor));
      cursor = new Date(cursor.getFullYear(), cursor.getMonth(), cursor.getDate() + 1);
    }
    weeks.push(week);
  }
  return weeks;
}

/** Week Mondays whose [Mon, Sun) windows intersect [from, to). */
export function weekMondaysCovering(from: Date, to: Date): Date[] {
  const start = mondayOf(from);
  const out: Date[] = [];
  for (let cursor = start; cursor.getTime() < to.getTime(); ) {
    out.push(new Date(cursor));
    cursor = new Date(cursor.getFullYear(), cursor.getMonth(), cursor.getDate() + 7);
  }
  return out;
}

/** Snap minutes to a slot grid (default 15m) for drag-reschedule. */
export function snapMinutes(d: Date, slotMinutes: number = 15): Date {
  const out = new Date(d);
  const ms = slotMinutes * 60_000;
  out.setTime(Math.round(out.getTime() / ms) * ms);
  return out;
}

export function overlapsInterval(aStart: Date, aEnd: Date, bStart: Date, bEnd: Date): boolean {
  return aStart.getTime() < bEnd.getTime() && bStart.getTime() < aEnd.getTime();
}

export interface LaneSlot {
  entry: CalendarEntry;
  lane: number;
  laneCount: number;
}

/**
 * Side-by-side lane assignment for a day's entries: greedy lanes plus a
 * per-overlap-cluster lane count, so intersecting entries share the row
 * width instead of stacking invisibly.
 */
export function assignLanes(dayEntries: CalendarEntry[]): LaneSlot[] {
  const sorted = [...dayEntries].sort(
    (a, b) => new Date(a.startsAt).getTime() - new Date(b.startsAt).getTime() || a.id.localeCompare(b.id),
  );
  const laneEnds: number[] = [];
  const laneOf = new Map<string, number>();
  for (const entry of sorted) {
    const start = new Date(entry.startsAt).getTime();
    let lane = laneEnds.findIndex((end) => end <= start);
    if (lane === -1) {
      lane = laneEnds.length;
      laneEnds.push(new Date(entry.endsAt).getTime());
    } else {
      laneEnds[lane] = new Date(entry.endsAt).getTime();
    }
    laneOf.set(entry.id, lane);
  }
  const slots: LaneSlot[] = [];
  let cluster: LaneSlot[] = [];
  let clusterEnd = 0;
  let clusterLanes = 0;
  const flush = () => {
    for (const slot of cluster) slots.push({ ...slot, laneCount: clusterLanes });
    cluster = [];
  };
  for (const entry of sorted) {
    const start = new Date(entry.startsAt).getTime();
    const end = new Date(entry.endsAt).getTime();
    if (cluster.length > 0 && start >= clusterEnd) {
      flush();
      clusterEnd = 0;
      clusterLanes = 0;
    }
    cluster.push({ entry, lane: laneOf.get(entry.id) ?? 0, laneCount: 1 });
    clusterEnd = Math.max(clusterEnd, end);
    clusterLanes = Math.max(clusterLanes, (laneOf.get(entry.id) ?? 0) + 1);
  }
  flush();
  return slots;
}

// --- month-grid keyboard navigation (Wave 17 lane 12) -------------------------

/**
 * Arrow-key day focus over the fixed 7-wide month grid. `index` is the
 * linear 0-based cell index (row*7 + weekday); returns the next index or
 * null for non-movement keys. Movement clamps inside the rendered grid —
 * the 6 week rows always cover the month, so day focus can traverse the
 * whole month without repainting a different anchor.
 */
export function monthFocusMove(index: number, key: string, rowCount: number): number | null {
  const count = rowCount * 7;
  const row = Math.floor(index / 7);
  const col = index % 7;
  switch (key) {
    case "ArrowLeft":
      return col > 0 ? index - 1 : null;
    case "ArrowRight":
      return col < 6 ? index + 1 : null;
    case "ArrowUp":
      return row > 0 ? index - 7 : null;
    case "ArrowDown":
      return row < rowCount - 1 ? index + 7 : null;
    case "Home":
      return index - col;
    case "End":
      return index + (6 - col);
    case "Enter":
    case " ":
      return index; // activation stays on the cell (the click handler owns it)
    default:
      return null;
  }
}
