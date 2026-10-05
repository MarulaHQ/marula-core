// Unit tests: calendar-view — entry classification, merge order, overlap
// detection, month-grid + week-fetch planning, snap (Wave 17 lane 4).
import { describe, expect, it } from "vitest";
import type { components } from "@marula-core/sdk";
import {
  assignLanes,
  countOverlaps,
  detectOverlaps,
  entryFromExclusion,
  entryFromItem,
  entryFromTicketDeadline,
  entriesOnDay,
  isSyncFedExclusion,
  mergeEntries,
  monthFocusMove,
  monthWeeks,
  sameLocalDay,
  snapMinutes,
  weekMondaysCovering,
  type CalendarEntry,
} from "../../lib/calendar-view";

type CalItem = components["schemas"]["CalendarItem"];
type CalExclusion = components["schemas"]["CalendarExclusion"];

const NOW = new Date("2026-09-17T09:00:00Z");

function item(over: Partial<CalItem>): CalItem {
  return {
    id: "i1",
    item_type: "appointment",
    starts_at: "2026-09-17T09:00:00Z",
    ends_at: "2026-09-17T10:00:00Z",
    title: "Site visit",
    kind: "on_site",
    status: null,
    ticket_id: null,
    user_id: "u1",
    ...over,
  };
}

function exclusion(over: Partial<CalExclusion>): CalExclusion {
  return {
    id: "e1",
    org_id: "o",
    user_id: "u1",
    starts_at: "2026-09-17T09:00:00Z",
    ends_at: "2026-09-17T10:00:00Z",
    reason: "Time off",
    created_at: "",
    ...over,
  };
}

describe("classification", () => {
  it("routes sync-fed exclusions by their reason marker", () => {
    expect(isSyncFedExclusion(exclusion({ reason: "Calendar sync: Flight to HQ" }))).toBe(true);
    expect(isSyncFedExclusion(exclusion({ reason: "Vacation" }))).toBe(false);
    expect(isSyncFedExclusion(exclusion({ reason: null }))).toBe(false);
  });
  it("entryFromExclusion labels synced busy time, falling back to a generic title", () => {
    expect(entryFromExclusion(exclusion({ reason: "Calendar sync: Flight" }))).toMatchObject({
      kind: "synced_busy",
      title: "Flight",
    });
    expect(entryFromExclusion(exclusion({ reason: "Calendar sync:" }))).toMatchObject({
      kind: "synced_busy",
      title: "Synced busy time",
    });
    expect(entryFromExclusion(exclusion({}))).toMatchObject({ kind: "time_off", title: "Time off" });
  });
  it("entryFromItem separates appointments from on-call and keeps booking cross-refs", () => {
    expect(entryFromItem(item({}))).toMatchObject({ kind: "appointment", itemKind: "on_site" });
    expect(entryFromItem(item({ item_type: "on_call", category: "Primary", title: null }))).toMatchObject({
      kind: "on_call",
      title: "Primary",
    });
    expect(entryFromItem(item({ ticket_id: "t9" }))).toMatchObject({ ticketId: "t9" });
  });
  it("entryFromTicketDeadline is null without a due date", () => {
    expect(entryFromTicketDeadline({ id: "t1", number: "MAR-1", title: "X", due_at: null })).toBeNull();
    expect(
      entryFromTicketDeadline({ id: "t1", number: "MAR-1", title: "X", due_at: NOW.toISOString() }),
    ).toMatchObject({ kind: "ticket_deadline", number: "MAR-1" });
  });
});

describe("mergeEntries + overlaps", () => {
  const a: CalendarEntry = {
    kind: "appointment",
    id: "a",
    title: "A",
    startsAt: "2026-09-17T09:00:00Z",
    endsAt: "2026-09-17T10:00:00Z",
    ticketId: null,
    itemKind: null,
    status: null,
    userId: null,
    number: null,
    travelMinutes: 0,
    travelQuality: null,
  };
  const b: CalendarEntry = { ...a, id: "b", startsAt: "2026-09-17T09:30:00Z", endsAt: "2026-09-17T10:30:00Z" };
  const c: CalendarEntry = { ...a, id: "c", startsAt: "2026-09-17T11:00:00Z", endsAt: "2026-09-17T12:00:00Z" };
  it("merge orders by start time and collapses duplicate ids", () => {
    expect(mergeEntries([c, a, b]).map((e) => e.id)).toEqual(["a", "b", "c"]);
    expect(mergeEntries([a, { ...a }, b]).map((e) => e.id)).toEqual(["a", "b"]);
  });
  it("detectOverlaps flags exactly the intersecting pair", () => {
    expect(detectOverlaps([a, b, c])).toEqual(new Set(["a", "b"]));
    expect(detectOverlaps([a, c])).toEqual(new Set());
  });
  it("countOverlaps counts distinct intersecting pairs, not flagged ids", () => {
    // a×b and a×overlapping — three ids in the flagged set, two conflicts.
    const overlapping: CalendarEntry = { ...a, id: "b2", startsAt: "2026-09-17T08:45:00Z", endsAt: "2026-09-17T09:15:00Z" };
    expect(detectOverlaps([a, b, overlapping])).toEqual(new Set(["a", "b", "b2"]));
    expect(countOverlaps([a, b, overlapping])).toBe(2);
    expect(countOverlaps([a, b, c])).toBe(1);
    expect(countOverlaps([a, c])).toBe(0);
  });
  it("entriesOnDay filters by local calendar day", () => {
    expect(entriesOnDay([a], new Date(2026, 8, 17))).toHaveLength(1);
    expect(entriesOnDay([a], new Date(2026, 8, 18))).toHaveLength(0);
    expect(sameLocalDay(a.startsAt, new Date(2026, 8, 17))).toBe(true);
  });
});

describe("grids + snapping", () => {
  it("monthWeeks starts on Mondays and covers the month", () => {
    const weeks = monthWeeks(2026, 8); // September 2026
    expect(weeks.length).toBeGreaterThanOrEqual(5);
    expect(weeks.length).toBeLessThanOrEqual(6);
    for (const week of weeks) {
      expect(week).toHaveLength(7);
      const first = week[0];
      expect(first?.getDay()).toBe(1);
    }
    const days = weeks.flat();
    expect(days.some((d) => d.getDate() === 1 && d.getMonth() === 8)).toBe(true);
    expect(days.some((d) => d.getDate() === 30 && d.getMonth() === 8)).toBe(true);
  });
  it("weekMondaysCovering enumerates the intersecting week windows", () => {
    const mondays = weekMondaysCovering(new Date(2026, 8, 14), new Date(2026, 8, 30));
    expect(mondays.map((d) => d.getDate())).toEqual([14, 21, 28]);
  });
  it("snapMinutes rounds to the nearest slot", () => {
    expect(snapMinutes(new Date(2026, 8, 17, 9, 7)).getMinutes()).toBe(0);
    expect(snapMinutes(new Date(2026, 8, 17, 9, 23)).getMinutes()).toBe(30);
  });
  it("assignLanes splits intersecting runs into shared lanes", () => {
    const mk = (id: string, s: string, e: string): CalendarEntry => ({
      kind: "appointment",
      id,
      title: id,
      startsAt: s,
      endsAt: e,
      ticketId: null,
      itemKind: null,
      status: null,
      userId: null,
      number: null,
      travelMinutes: 0,
      travelQuality: null,
    });
    const a = mk("a", "2026-09-17T09:00:00Z", "2026-09-17T10:00:00Z");
    const b = mk("b", "2026-09-17T09:30:00Z", "2026-09-17T10:30:00Z");
    const c = mk("c", "2026-09-17T11:00:00Z", "2026-09-17T12:00:00Z");
    const slots = assignLanes([a, b, c]);
    const byId = new Map(slots.map((s) => [s.entry.id, s]));
    expect(byId.get("a")).toMatchObject({ lane: 0, laneCount: 2 });
    expect(byId.get("b")).toMatchObject({ lane: 1, laneCount: 2 });
    expect(byId.get("c")).toMatchObject({ lane: 0, laneCount: 1 });
  });
});

describe("monthFocusMove", () => {
  const ROWS = 6;
  it("moves left/right within a row and stops at the edges", () => {
    expect(monthFocusMove(10, "ArrowLeft", ROWS)).toBe(9);
    expect(monthFocusMove(7, "ArrowLeft", ROWS)).toBeNull();
    expect(monthFocusMove(7, "ArrowRight", ROWS)).toBe(8);
    expect(monthFocusMove(13, "ArrowRight", ROWS)).toBeNull();
  });
  it("moves up/down by a full week and clamps at the grid boundary", () => {
    expect(monthFocusMove(10, "ArrowUp", ROWS)).toBe(3);
    expect(monthFocusMove(2, "ArrowUp", ROWS)).toBeNull();
    expect(monthFocusMove(10, "ArrowDown", ROWS)).toBe(17);
    expect(monthFocusMove(38, "ArrowDown", ROWS)).toBeNull();
  });
  it("Home/End jump to the start/end of the week row", () => {
    expect(monthFocusMove(10, "Home", ROWS)).toBe(7);
    expect(monthFocusMove(9, "End", ROWS)).toBe(13);
  });
  it("activation keys keep focus in place; other keys move nothing", () => {
    expect(monthFocusMove(5, "Enter", ROWS)).toBe(5);
    expect(monthFocusMove(5, " ", ROWS)).toBe(5);
    expect(monthFocusMove(5, "Tab", ROWS)).toBeNull();
  });
});
