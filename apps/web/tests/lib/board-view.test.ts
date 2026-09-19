// Pure board-view logic: grouping, filtering, drag validity, chip tones.
import { describe, expect, it } from "vitest";
import {
  categoryTone,
  dueChip,
  humanCategoryLabel,
  filterTickets,
  groupByStatus,
  priorityTone,
  sortStatuses,
  transitionAllowed,
  type BoardStatus,
  type BoardTicket,
} from "../../lib/board-view";

const S1: BoardStatus = {
  id: "s1",
  board_id: "b",
  org_id: "o",
  name: "New",
  category: "open",
  sort_order: 1,
  is_entry: true,
};
const S2: BoardStatus = {
  id: "s2",
  board_id: "b",
  org_id: "o",
  name: "Working",
  category: "in_progress",
  sort_order: 0,
  is_entry: false,
};
const S3: BoardStatus = {
  id: "s3",
  board_id: "b",
  org_id: "o",
  name: "Done",
  category: "closed",
  sort_order: 2,
  is_entry: false,
};

function ticket(overrides: Record<string, unknown>): BoardTicket {
  return {
    id: "t",
    board_id: "b",
    number: "MER-1",
    title: "A ticket",
    status_id: "s1",
    category: "open",
    priority: "p3",
    created_at: "2026-09-01T00:00:00Z",
    updated_at: "2026-09-01T00:00:00Z",
    org_id: "o",
    ...overrides,
  } as unknown as BoardTicket;
}

describe("sortStatuses", () => {
  it("orders by sort_order", () => {
    expect(sortStatuses([S1, S2, S3]).map((s) => s.id)).toEqual(["s2", "s1", "s3"]);
  });
});

describe("categoryTone", () => {
  it("maps closed→success, waiting→warning, else neutral", () => {
    expect(categoryTone("closed")).toBe("success");
    expect(categoryTone("waiting")).toBe("warning");
    expect(categoryTone("open")).toBe("neutral");
    expect(categoryTone("in_progress")).toBe("neutral");
  });
});

describe("transitionAllowed", () => {
  it("floor mode blocks closed→open only", () => {
    expect(transitionAllowed([S1, S2, S3], null, "s1", "s2")).toBe(true);
    expect(transitionAllowed([S1, S2, S3], null, "s3", "s1")).toBe(false);
    expect(transitionAllowed([S1, S2, S3], null, "s1", "s1")).toBe(false);
  });
  it("explicit mode consults edges only", () => {
    const wf = {
      mode: "explicit" as const,
      edges: [{ from_status_id: "s1", to_status_id: "s2" }],
    };
    expect(transitionAllowed([S1, S2, S3], wf, "s1", "s2")).toBe(true);
    expect(transitionAllowed([S1, S2, S3], wf, "s1", "s3")).toBe(false);
  });
  it("floor rules hold even for unknown workflow shape", () => {
    expect(transitionAllowed([S1, S2, S3], { mode: "floor", edges: [] }, "s3", "s1")).toBe(false);
  });
});

describe("filterTickets", () => {
  const tickets = [
    ticket({ id: "a", title: "Fix login", number: "MER-1", priority: "p1", assignee_id: "u1" }),
    ticket({ id: "b", title: "Ship docs", number: "MER-2", priority: "P3" }),
  ];
  it("matches query against title and number case-insensitively", () => {
    expect(filterTickets(tickets, { query: "mer-2", priorities: [], unassignedOnly: false, assigneeId: null })).toHaveLength(1);
    expect(filterTickets(tickets, { query: "LOGIN", priorities: [], unassignedOnly: false, assigneeId: null })).toHaveLength(1);
  });
  it("filters by priority case-insensitively", () => {
    expect(filterTickets(tickets, { query: "", priorities: ["p3"], unassignedOnly: false, assigneeId: null })).toHaveLength(1);
  });
  it("unassigned-only drops assigned rows", () => {
    const out = filterTickets(tickets, { query: "", priorities: [], unassignedOnly: true, assigneeId: null });
    expect(out.map((t) => t.id)).toEqual(["b"]);
  });
  it("assignee filter (directory id) narrows to that user; mismatch drops both", () => {
    // note: fixture "b" derives the typed default assignee_id (null) —
    // a directory id never matches unassigned rows.
    expect(filterTickets(tickets, { query: "", priorities: [], unassignedOnly: false, assigneeId: "u1" }).map((t) => t.id)).toEqual(["a"]);
    expect(filterTickets(tickets, { query: "", priorities: [], unassignedOnly: false, assigneeId: "u9" })).toHaveLength(0);
    expect(filterTickets(tickets, { query: "", priorities: [], unassignedOnly: false, assigneeId: null })).toHaveLength(2);
  });
});

describe("groupByStatus", () => {
  it("buckets by status_id and skips unknown statuses", () => {
    const groups = groupByStatus(
      [ticket({ id: "a", status_id: "s2" }), ticket({ id: "b", status_id: "zz" })],
      sortStatuses([S1, S2, S3]),
    );
    expect(groups.get("s2")).toHaveLength(1);
    expect(groups.get("s1")).toHaveLength(0);
  });
  it("skips tickets with null status_id", () => {
    const groups = groupByStatus([ticket({ status_id: null })], [S1]);
    expect(groups.get("s1")).toHaveLength(0);
  });
});

describe("priorityTone", () => {
  it("P1 danger, P2 warning, rest neutral", () => {
    expect(priorityTone("p1")).toBe("danger");
    expect(priorityTone("P2")).toBe("warning");
    expect(priorityTone("p3")).toBe("neutral");
  });
});

describe("dueChip", () => {
  const now = Date.parse("2026-09-17T12:00:00Z");
  it("no due date → no chip", () => {
    expect(dueChip(null, now)).toBeNull();
    expect(dueChip(undefined, now)).toBeNull();
  });
  it("overdue renders danger", () => {
    const chip = dueChip("2026-09-16T00:00:00Z", now);
    expect(chip?.tone).toBe("danger");
    expect(chip?.label).toContain("Overdue");
  });
  it("inside 24h renders warning", () => {
    expect(dueChip("2026-09-17T20:00:00Z", now)?.tone).toBe("warning");
  });
  it("beyond 24h renders neutral with date label", () => {
    const chip = dueChip("2026-09-20T00:00:00Z", now);
    expect(chip?.tone).toBe("neutral");
    expect(chip?.label).not.toContain("Overdue");
  });
});

describe("humanCategoryLabel", () => {
  it("maps the canonical wire categories to the human vocabulary", () => {
    expect(humanCategoryLabel("open")).toBe("Open");
    expect(humanCategoryLabel("in_progress")).toBe("In progress");
    expect(humanCategoryLabel("waiting")).toBe("Waiting");
    expect(humanCategoryLabel("closed")).toBe("Closed");
  });
  it("null/absent status says 'No status'; unknown slang falls back honestly", () => {
    expect(humanCategoryLabel(null)).toBe("No status");
    expect(humanCategoryLabel(undefined)).toBe("No status");
    expect(humanCategoryLabel("weird")).toBe("weird");
  });
});
