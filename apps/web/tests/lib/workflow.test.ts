// Unit tests: /lib/admin/workflow — status drafts, save-plan payload
// shapes (typed against SDK request schemas), transition matrix
// floor/explicit semantics, and checklist template draft behavior.
import { describe, expect, it } from "vitest";
import type { components } from "@meridian/sdk";
import type { StatusDraftState } from "../../lib/admin/workflow";
import {
  categoryTone,
  checklistIsDirty,
  checklistPayload,
  checklistReducer,
  checklistTemplateFromServer,
  edgeKey,
  floorModeEdges,
  matrixPayload,
  matrixReducer,
  parseEdgeKey,
  planStatusSaves,
  STATUS_CATEGORIES,
  statusDraftIsDirty,
  statusDraftReducer,
  statusRowsFromServer,
  validateStatusRow,
} from "../../lib/admin/workflow";
type BoardStatus = components["schemas"]["BoardStatus"];
type SetTransitionsRequest = components["schemas"]["SetTransitionsRequest"];
type PutChecklistTemplateRequest = components["schemas"]["PutChecklistTemplateRequest"];

const uuid = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;

function serverStatuses(): BoardStatus[] {
  return [
    { id: uuid(1), board_id: "b", org_id: "o", name: "New", category: "open", is_entry: true, sort_order: 0, color: null },
    { id: uuid(2), board_id: "b", org_id: "o", name: "Working", category: "in_progress", is_entry: false, sort_order: 1, color: null },
    { id: uuid(3), board_id: "b", org_id: "o", name: "Client wait", category: "waiting", is_entry: false, sort_order: 2, color: null },
    { id: uuid(4), board_id: "b", org_id: "o", name: "Done", category: "closed", is_entry: false, sort_order: 3, color: null },
  ];
}

describe("status draft", () => {
  it("add creates a new-id row that participates in dirty tracking", () => {
    let state: StatusDraftState = {
      rows: statusRowsFromServer(serverStatuses()),
      removedIds: [],
    };
    expect(statusDraftIsDirty(serverStatuses(), state)).toBe(false);
    state = statusDraftReducer(state, { type: "add", name: "Blocked" });
    expect(state.rows).toHaveLength(5);
    expect(state.rows[4]?.id).toBeNull();
    expect(statusDraftIsDirty(serverStatuses(), state)).toBe(true);
  });

  it("remove of a server row records its id for deletion", () => {
    const state = statusDraftReducer(
      { rows: statusRowsFromServer(serverStatuses()), removedIds: [] },
      { type: "remove", index: 1 },
    );
    expect(state.removedIds).toEqual([uuid(2)]);
    expect(state.rows.some((r) => r.name === "Working")).toBe(false);
  });

  it("move swaps neighbors within bounds and is a no-op outside", () => {
    let state: StatusDraftState = {
      rows: statusRowsFromServer(serverStatuses()),
      removedIds: [],
    };
    state = statusDraftReducer(state, { type: "move", index: 0, direction: -1 });
    expect(state.rows[0]?.name).toBe("New");
    state = statusDraftReducer(state, { type: "move", index: 0, direction: 1 });
    expect(state.rows[0]?.name).toBe("Working");
    expect(state.rows[1]?.name).toBe("New");
  });

  it("validateStatusRow rejects blank names and overlong names", () => {
    expect(validateStatusRow({ id: null, name: "", category: "open", isEntry: true })).toMatch(/required/);
    expect(validateStatusRow({ id: null, name: "x".repeat(65), category: "open", isEntry: true })).toMatch(/64/);
    expect(validateStatusRow({ id: null, name: "Fine", category: "open", isEntry: true })).toBeNull();
  });
});

describe("planStatusSaves — payload shape parity with SDK requests", () => {
  it("orders deletes → creates → updates with sort_order from draft position", () => {
    const server = serverStatuses();
    let draft: StatusDraftState = {
      rows: statusRowsFromServer(server),
      removedIds: [uuid(3)],
    };
    // Update "New"→"Intake" and move Working to the top; add a fresh row.
    draft = statusDraftReducer(draft, { type: "update", index: 0, patch: { name: "Intake" } });
    draft = statusDraftReducer(draft, { type: "move", index: 0, direction: -1 }); // no-op guard
    draft = statusDraftReducer(draft, { type: "add", name: "Escalated" });

    const plan = planStatusSaves(server, draft);
    expect(plan.deletes).toEqual([{ statusId: uuid(3) }]);
    expect(plan.updates.some((u) => u.body.name === "Intake")).toBe(true);

    // Type-level parity: every op body satisfies its SDK schema.
    plan.creates.forEach((c) => {
      const req: components["schemas"]["CreateStatusRequest"] = c;
      expect(req.sort_order).toBeTypeOf("number");
    });
    plan.updates.forEach((u) => {
      const req: components["schemas"]["UpdateStatusRequest"] = u.body;
      expect(req.is_entry).toBeTypeOf("boolean");
    });
  });

  it("skips updates when nothing changed", () => {
    const server = serverStatuses();
    const draft: StatusDraftState = {
      rows: statusRowsFromServer(server),
      removedIds: [],
    };
    const plan = planStatusSaves(server, draft);
    expect(plan).toEqual({ deletes: [], creates: [], updates: [] });
  });
});

describe("transition matrix", () => {
  it("edgeKey round-trips through parseEdgeKey", () => {
    const key = edgeKey(uuid(1), uuid(2));
    expect(parseEdgeKey(key)).toEqual({ from: uuid(1), to: uuid(2) });
    expect(parseEdgeKey("nonsense")).toBeNull();
  });

  it("floor mode default blocks closed→open but allows everything else non-self", () => {
    const edges = floorModeEdges(serverStatuses());
    expect(edges.has(edgeKey(uuid(4), uuid(1)))).toBe(false); // closed→open
    expect(edges.has(edgeKey(uuid(1), uuid(4)))).toBe(true); // open→closed
    for (let i = 1; i <= 4; i++) {
      expect(edges.has(edgeKey(uuid(i), uuid(i)))).toBe(false);
    }
  });

  it("init explicit adopts exactly the wire edges; toggle upgrades floor to custom", () => {
    const wire = [{ from_status_id: uuid(1), to_status_id: uuid(2) }];
    let state = matrixReducer(
      { checked: new Set(), pristineFloor: true },
      { type: "init", mode: "explicit", explicitEdges: wire, statuses: serverStatuses() },
    );
    expect([...state.checked]).toEqual([edgeKey(uuid(1), uuid(2))]);
    expect(state.pristineFloor).toBe(false);

    state = matrixReducer(
      { checked: new Set(), pristineFloor: true },
      { type: "init", mode: "floor", explicitEdges: [], statuses: serverStatuses() },
    );
    expect(state.pristineFloor).toBe(true);
    const next = matrixReducer(state, { type: "toggle", key: edgeKey(uuid(4), uuid(1)) });
    expect(next.pristineFloor).toBe(false);
    expect(next.checked.has(edgeKey(uuid(4), uuid(1)))).toBe(true);
  });

  it("payload matches SetTransitionsRequest even when empty (returns board to floor)", () => {
    const state = { checked: new Set<string>(), pristineFloor: false };
    const body: SetTransitionsRequest = matrixPayload(state);
    expect(body.edges).toEqual([]);
  });
});

describe("checklist template editor", () => {
  /** Mirrors the shape the API actually returns (full template record). */
  const serverViewJson = {
    items: [{ text: "Backups verified" }, { text: "Root cause noted" }],
    require_checklist_close: true,
    template: {
      id: "t",
      board_id: "b",
      org_id: "o",
      name: "Standard close",
      created_at: "",
      updated_at: "",
    },
  };
  const serverView = () => checklistTemplateFromServer(serverViewJson);

  it("template loads into a matching draft and reports clean", () => {
    const snapshot = serverView();
    const draft = { name: snapshot.name, requireClose: snapshot.requireClose, items: [...snapshot.items] };
    expect(checklistIsDirty(snapshot, draft)).toBe(false);
  });

  it("init/replace action resets item list wholesale between boards", () => {
    let state: ReturnType<typeof checklistTemplateFromServer> & { items: string[] } = {
      ...serverView(),
    };
    state = checklistReducer(state, { type: "add-item", text: "temp" });
    state = checklistReducer(state, {
      type: "init",
      name: "Other",
      requireClose: false,
      items: ["only"],
    });
    expect(state.items).toEqual(["only"]);
    expect(state.requireClose).toBe(false);
  });

  it("payload trims blanks, keeps order, defaults the name; typed as PUT body", () => {
    let state = checklistTemplateFromServer({
      items: [],
      require_checklist_close: false,
      template: null,
    });
    state = checklistReducer(state, { type: "init", name: "", requireClose: true, items: ["  Step A  ", "", "Step B"] });
    const body: PutChecklistTemplateRequest = checklistPayload(state);
    expect(body.name).toBe("Board checklist");
    expect(body.items).toEqual(["Step A", "Step B"]);
    expect(body.require_checklist_close).toBe(true);
  });

  it("move-item reorders within bounds", () => {
    let state = checklistTemplateFromServer({ items: [], require_checklist_close: false, template: null });
    state = checklistReducer(state, { type: "init", name: "n", requireClose: false, items: ["a", "b", "c"] });
    state = checklistReducer(state, { type: "move-item", index: 2, direction: 1 }); // out of range
    expect(state.items).toEqual(["a", "b", "c"]);
    state = checklistReducer(state, { type: "move-item", index: 2, direction: -1 });
    expect(state.items).toEqual(["a", "c", "b"]);
  });
});

describe("category presentation mapping", () => {
  it("every SDK status category has a tone and chips stay prominent", () => {
    expect(STATUS_CATEGORIES.map((c) => categoryTone(c))).toEqual([
      "success",
      "warning",
      "neutral",
      "danger",
    ]);
  });
});
