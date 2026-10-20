// Headless tests for the dispatch drag-state machine (the private build tracker 2.19):
// ghost placement conflict classes, optimistic move/assign flows with
// rollback on typed rejection, realtime reconciliation, and presence.
// All instants use the Europe/London-free "UTC" zone so expectations are
// offset-stable; timezone expansion itself is covered separately.

import { describe, expect, it } from "vitest";
import {
  coveredSpans,
  coversSpans,
  dispatchReducer,
  findConflict,
  initialDispatchState,
  withBlock,
  withoutBlock,
  type Block,
  type BoardState,
  type QueueItem,
  type Tech,
} from "./dispatch-reducer";

const DAY = "2026-06-01"; // a Monday

function iso(at: string): string {
  return `${DAY}T${at}:00Z`;
}

function block(partial: Partial<Block> = {}): Block {
  return {
    id: "b1",
    user_id: "tech-a",
    ticket_id: null,
    ticket_number: null,
    client_name: null,
    title: "booking",
    starts_at: iso("09:00"),
    ends_at: iso("10:00"),
    status: "scheduled",
    ...partial,
  };
}

/** Indexed access under noUncheckedIndexedAccess with a clear failure. */
function first<T>(xs: T[]): T {
  const v = xs[0];
  if (v === undefined) throw new Error("expected non-empty array");
  return v;
}

function tech(partial: Partial<Tech> = {}): Tech {
  return {
    user_id: "tech-a",
    display_name: "A. Okafor",
    available_minutes: 480,
    booked_minutes: 0,
    utilization_percent: 0,
    appointments: [],
    availability_windows: [
      { weekday: 0, start_local: "09:00", end_local: "17:00", timezone: "UTC" },
    ],
    exclusions: [],
    ...partial,
  };
}

function queueItem(partial: Partial<QueueItem> = {}): QueueItem {
  return {
    id: "t1",
    number: "MAR-1001",
    title: "server down",
    priority: "p1",
    due_at: null,
    sla_resolution_due: null,
    ...partial,
  };
}

function stateWith(
  technicians: Tech[],
  unassigned: QueueItem[] = [],
): BoardState {
  return dispatchReducer(initialDispatchState(), {
    type: "loaded",
    technicians,
    unassigned,
  });
}

describe("findConflict", () => {
  const base = tech();

  it("flags overlap and names the blocker", () => {
    const t = tech({
      appointments: [block({ id: "x", title: "Acme visit" })],
    });
    expect(findConflict(t, iso("09:30"), iso("10:30"))).toEqual({
      kind: "overlap",
      blockerTitle: "Acme visit",
    });
  });

  it("ignores the dragged block when told to", () => {
    const t = tech({
      appointments: [block({ id: "self", starts_at: iso("09:00"), ends_at: iso("10:00") })],
    });
    expect(findConflict(t, iso("09:00"), iso("10:00"), ["self"])).toBeNull();
  });

  it("treats touching spans as non-conflicting", () => {
    const t = tech({ appointments: [block()] });
    expect(findConflict(t, iso("10:00"), iso("11:00"))).toBeNull();
  });

  it("flags placements outside availability windows", () => {
    expect(findConflict(base, iso("08:00"), iso("08:30"))).toEqual({
      kind: "outside_availability",
    });
    expect(findConflict(base, iso("18:00"), iso("18:30"))).toEqual({
      kind: "outside_availability",
    });
  });

  it("flags placements inside calendar exclusions even within windows", () => {
    const t = tech({
      exclusions: [{ id: "e", starts_at: iso("12:00"), ends_at: iso("13:00") }],
    });
    expect(findConflict(t, iso("12:15"), iso("12:45"))).toEqual({
      kind: "outside_availability",
    });
  });

  it("overlap takes precedence over outside-availability", () => {
    const t = tech({
      appointments: [block({ starts_at: iso("07:30"), ends_at: iso("07:45") })],
    });
    const result = findConflict(t, iso("07:00"), iso("08:00"));
    expect(result?.kind).toBe("overlap");
  });
});

describe("coverage expansion (zoned)", () => {
  it("expands templates only on the matching weekday", () => {
    const spans = coveredSpans(tech(), iso("10:00")); // Monday
    expect(coversSpans(spans, [[Date.parse(iso("10:00")), Date.parse(iso("11:00"))]])).toBe(
      true,
    );
    // Wednesday template never covers Monday.
    const wrongDay = tech({
      availability_windows: [
        { weekday: 2, start_local: "00:00", end_local: "23:59", timezone: "UTC" },
      ],
    });
    expect(coveredSpans(wrongDay, iso("10:00"))).toEqual([]);
  });

  it("honors the window's own timezone, not just UTC", () => {
    // 09:00–17:00 in New York on Jun 1 2026 is 13:00–21:00 UTC.
    const ny = tech({
      availability_windows: [
        { weekday: 0, start_local: "09:00", end_local: "17:00", timezone: "America/New_York" },
      ],
    });
    const spans = coveredSpans(ny, iso("14:00"));
    expect(
      coversSpans(spans, [[Date.parse(iso("13:00")), Date.parse(iso("14:00"))]]),
    ).toBe(true);
    expect(
      coversSpans(spans, [[Date.parse(iso("21:00")), Date.parse(iso("21:30"))]]),
    ).toBe(false);
  });
});

describe("optimistic move flow", () => {
  const techA = tech({ appointments: [block()] });
  const techB = tech({ user_id: "tech-b", display_name: "L. Chen" });

  function dragPicked(s: BoardState): BoardState {
    return dispatchReducer(dispatchReducer(s, {
      type: "drag_start",
      source: "board",
      id: "b1",
      title: "booking",
    }), { type: "ghost_move", userId: "tech-b", startIso: iso("11:00"), endIso: iso("12:00") });
  }

  it("ghost reports no conflict on a free slot of another technician", () => {
    const s = dragPicked(stateWith([techA, techB]));
    expect(s.ghost?.userId).toBe("tech-b");
    expect(s.ghost?.conflict).toBeNull();
  });

  it("drop applies optimistic move into one column only", () => {
    let s = stateWith([techA, techB]);
    s = dragPicked(s);
    s = dispatchReducer(s, {
      type: "drop_move",
      apptId: "b1",
      userId: "tech-b",
      startsAt: iso("11:00"),
      endsAt: iso("12:00"),
    });
    expect(s.technicians.find((t) => t.user_id === "tech-a")?.appointments).toHaveLength(0);
    const moved = s.technicians.find((t) => t.user_id === "tech-b")?.appointments ?? [];
    expect(moved).toHaveLength(1);
    expect(first(moved!).starts_at).toBe(iso("11:00"));
    expect(s.pending["b1"]?.kind).toBe("move");
  });

  it("rejection rolls the block back to its original slot", () => {
    let s = stateWith([techA, techB]);
    s = dragPicked(s);
    s = dispatchReducer(s, {
      type: "drop_move",
      apptId: "b1",
      userId: "tech-b",
      startsAt: iso("11:00"),
      endsAt: iso("12:00"),
    });
    s = dispatchReducer(s, {
      type: "reject",
      key: "b1",
      message: "dispatch move conflicts with existing booking",
    });
    expect(s.pending["b1"]).toBeUndefined();
    const restored = first(
      s.technicians.find((t) => t.user_id === "tech-a")?.appointments ?? [],
    );
    expect(restored).toMatchObject({ id: "b1", starts_at: iso("09:00") });
    expect(s.technicians.find((t) => t.user_id === "tech-b")?.appointments).toHaveLength(0);
    expect(s.error?.message).toContain("conflicts");
  });

  it("confirmation clears pending state", () => {
    let s = stateWith([techA, techB]);
    s = dragPicked(s);
    s = dispatchReducer(s, {
      type: "drop_move",
      apptId: "b1",
      userId: "tech-b",
      startsAt: iso("11:00"),
      endsAt: iso("12:00"),
    });
    s = dispatchReducer(s, { type: "confirm", key: "b1" });
    expect(s.pending).toEqual({});
    expect(s.error).toBeNull();
  });

  it("booked minutes recompute optimistically after the move", () => {
    let s = stateWith([
      tech({ user_id: "a", appointments: [block()], available_minutes: 480 }),
      tech({ user_id: "b", available_minutes: 480 }),
    ]);
    s = dispatchReducer(s, {
      type: "drag_start",
      source: "board",
      id: "b1",
      title: "booking",
    });
    s = dispatchReducer(s, {
      type: "drop_move",
      apptId: "b1",
      userId: "b",
      startsAt: iso("11:00"),
      endsAt: iso("13:00"), // stretched while moving
    });
    expect(s.technicians.find((t) => t.user_id === "a")?.booked_minutes).toBe(0);
    expect(s.technicians.find((t) => t.user_id === "b")?.booked_minutes).toBe(120);
  });
});

describe("optimistic assign flow", () => {
  it("drop removes the item from the queue and inserts a temp block", () => {
    const s0 = stateWith([tech()], [queueItem({})]);
    let s = dispatchReducer(s0, {
      type: "drag_start",
      source: "queue",
      id: "t1",
      title: "MAR-1001 server down",
    });
    s = dispatchReducer(s, {
      type: "drop_assign",
      tempId: "tmp-1",
      ticket: queueItem({}),
      userId: "tech-a",
      startsAt: iso("09:00"),
      endsAt: iso("10:00"),
    });
    expect(s.unassigned).toHaveLength(0);
    const inserted = first(s.technicians).appointments;
    expect(inserted).toHaveLength(1);
    expect(first(inserted).id).toBe("tmp-1");
    expect(first(inserted).title).toBe("MAR-1001 server down");
    expect(s.pending["tmp-1"]?.kind).toBe("assign");
  });

  it("rejection restores the queue item and drops the temp block", () => {
    const item = queueItem({});
    let s = stateWith([tech()], [item]);
    s = dispatchReducer(s, {
      type: "drag_start",
      source: "queue",
      id: "t1",
      title: "x",
    });
    s = dispatchReducer(s, {
      type: "drop_assign",
      tempId: "tmp-1",
      ticket: item,
      userId: "tech-a",
      startsAt: iso("09:00"),
      endsAt: iso("10:00"),
    });
    s = dispatchReducer(s, { type: "reject", key: "tmp-1", message: "slot already booked: x" });
    expect(s.unassigned.some((t) => t.id === "t1")).toBe(true);
    expect(first(s.technicians).appointments).toHaveLength(0);
  });

  it("confirmation swaps the temp id out via reconcile of the real block", () => {
    const item = queueItem({});
    let s = stateWith([tech()], [item]);
    s = dispatchReducer(s, {
      type: "drag_start",
      source: "queue",
      id: "t1",
      title: "x",
    });
    s = dispatchReducer(s, {
      type: "drop_assign",
      tempId: "tmp-1",
      ticket: item,
      userId: "tech-a",
      startsAt: iso("09:00"),
      endsAt: iso("10:00"),
    });
    s = dispatchReducer(s, { type: "confirm", key: "tmp-1" });
    expect(s.pending).toEqual({});
    // The hook then replaces the temp block via reconcile_upsert.
    s = dispatchReducer(s, {
      type: "reconcile_upsert",
      block: block({ id: "real-1", title: "MAR-1001 server down" }),
    });
    expect(first(s.technicians).appointments.map((a) => a.id)).toEqual(["real-1"]);
  });
});

describe("realtime reconciliation", () => {
  it("upserts appointment.created/updated payloads not locally pending", () => {
    let s = stateWith([tech()]);
    s = dispatchReducer(s, {
      type: "reconcile_upsert",
      block: block({ id: "remote-1", title: "from another dispatcher" }),
    });
    expect(first(s.technicians).appointments).toHaveLength(1);
    s = dispatchReducer(s, {
      type: "reconcile_upsert",
      block: block({ id: "remote-1", starts_at: iso("14:00"), ends_at: iso("15:00") }),
    });
    expect(first(first(s.technicians).appointments).starts_at).toBe(iso("14:00"));
  });

  it("never lets an echo fight an in-flight optimistic move", () => {
    let s = stateWith([tech({ appointments: [block()] })]);
    s = dispatchReducer(s, {
      type: "drag_start",
      source: "board",
      id: "b1",
      title: "booking",
    });
    s = dispatchReducer(s, {
      type: "drop_move",
      apptId: "b1",
      userId: "tech-a",
      startsAt: iso("11:00"),
      endsAt: iso("12:00"),
    });
    // Stale echo of the OLD position arrives while the move is pending.
    s = dispatchReducer(s, {
      type: "reconcile_upsert",
      block: block({ starts_at: iso("09:00"), ends_at: iso("10:00") }),
    });
    expect(first(first(s.technicians).appointments).starts_at).toBe(iso("11:00"));
  });

  it("removes cancelled blocks", () => {
    let s = stateWith([tech({ appointments: [block({ id: "gone" })] })]);
    s = dispatchReducer(s, { type: "reconcile_remove", apptId: "gone" });
    expect(first(s.technicians).appointments).toHaveLength(0);
  });
});

describe("presence + drag lifecycle", () => {
  it("stores presence ids and clears drag state on cancel", () => {
    let s = stateWith([tech()]);
    s = dispatchReducer(s, { type: "presence", userIds: ["tech-a", "u2"] });
    expect(s.presence).toEqual(["tech-a", "u2"]);
    s = dispatchReducer(s, {
      type: "drag_start",
      source: "board",
      id: "b1",
      title: "booking",
    });
    expect(s.drag).not.toBeNull();
    s = dispatchReducer(s, { type: "drag_end" });
    expect(s.drag).toBeNull();
    expect(s.ghost).toBeNull();
  });

  it("announces keyboard instructions on pickup", () => {
    const s = dispatchReducer(stateWith([tech()]), {
      type: "drag_start",
      source: "board",
      id: "b1",
      title: "booking",
    });
    expect(s.announcement).toContain("Enter");
    expect(s.announcement).toContain("Escape");
  });
});

describe("block placement helpers", () => {
  it("withoutBlock strips everywhere, withBlock re-homes onto owner", () => {
    const a = tech({ appointments: [block()] });
    const b = tech({ user_id: "tech-b" });
    const stripped = withoutBlock([a, b], "b1");
    expect(first(stripped).appointments).toHaveLength(0);
    const movedHome = withBlock(stripped, block({ user_id: "tech-b" }));
    expect(movedHome.find((t) => t.user_id === "tech-b")?.appointments).toHaveLength(1);
    expect(movedHome.find((t) => t.user_id === "tech-a")?.appointments).toHaveLength(0);
  });

  it("unknown-owner blocks are dropped silently by withBlock", () => {
    const a = tech();
    expect(withBlock([a], block({ user_id: "ghost-tech" }))).toHaveLength(1);
    expect(first(withBlock([a], block({ user_id: "ghost-tech" }))).appointments).toHaveLength(0);
  });
});
