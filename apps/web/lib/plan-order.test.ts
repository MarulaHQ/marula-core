import { describe, expect, it } from "vitest";

import {
  groupOf,
  moveItem,
  movePhase,
  moveTask,
  moveTaskToPhase,
  orderFromPlan,
  positionOf,
  sameOrder,
  stepOrder,
  type PlanOrder,
} from "./plan-order";

const order = (): PlanOrder => ({
  phases: ["p1", "p2", "p3"],
  tasks: [
    { phase_id: "p1", task_ids: ["a", "b"] },
    { phase_id: "p2", task_ids: ["c"] },
    { phase_id: "p3", task_ids: [] },
  ],
});

describe("orderFromPlan", () => {
  it("sorts by sort_order and groups tasks under their phase", () => {
    const result = orderFromPlan(
      [
        { id: "late", sort_order: 2 },
        { id: "early", sort_order: 0 },
        { id: "middle", sort_order: 1 },
      ],
      [
        { id: "t2", phase_id: "early", sort_order: 5 },
        { id: "t1", phase_id: "early", sort_order: 1 },
        { id: "orphan", phase_id: null, sort_order: 0 },
      ],
    );
    expect(result.phases).toEqual(["early", "middle", "late"]);
    expect(result.tasks).toEqual([
      { phase_id: "early", task_ids: ["t1", "t2"] },
      { phase_id: "middle", task_ids: [] },
      { phase_id: "late", task_ids: [] },
      { phase_id: null, task_ids: ["orphan"] },
    ]);
  });

  it("treats a task whose phase is gone as unphased, and omits an empty bucket", () => {
    const result = orderFromPlan([{ id: "p1" }], [{ id: "t1", phase_id: "deleted" }]);
    expect(result).toEqual({
      phases: ["p1"],
      tasks: [
        { phase_id: "p1", task_ids: [] },
        { phase_id: null, task_ids: ["t1"] },
      ],
    });
    expect(orderFromPlan([{ id: "p1" }], []).tasks).toEqual([
      { phase_id: "p1", task_ids: [] },
    ]);
  });
});

describe("movePhase", () => {
  it("moves a phase before and after another", () => {
    expect(movePhase(order(), "p3", "p1", "before").phases).toEqual(["p3", "p1", "p2"]);
    expect(movePhase(order(), "p1", "p2", "after").phases).toEqual(["p2", "p1", "p3"]);
  });

  it("returns the same object when the drop changes nothing", () => {
    const current = order();
    expect(movePhase(current, "p1", "p1", "after")).toBe(current);
    expect(movePhase(current, "p1", "p2", "before")).toBe(current);
  });
});

describe("moveTask", () => {
  it("reorders inside a phase", () => {
    expect(moveTask(order(), "a", "b", "after").tasks[0]?.task_ids).toEqual(["b", "a"]);
    expect(moveTask(order(), "b", "a", "before").tasks[0]?.task_ids).toEqual(["b", "a"]);
  });

  it("moves a task into the phase of the task it was dropped on", () => {
    const next = moveTask(order(), "a", "c", "after");
    expect(next.tasks[0]?.task_ids).toEqual(["b"]);
    expect(next.tasks[1]?.task_ids).toEqual(["c", "a"]);
  });
});

describe("moveTaskToPhase", () => {
  it("places the task at the start or the end of the phase", () => {
    expect(moveTaskToPhase(order(), "c", "p1", "before").tasks[0]?.task_ids).toEqual([
      "c",
      "a",
      "b",
    ]);
    expect(moveTaskToPhase(order(), "a", "p2", "after").tasks[1]?.task_ids).toEqual(["c", "a"]);
  });

  it("creates the group for a phase that held no work", () => {
    const next = moveTaskToPhase(order(), "a", "p3", "after");
    expect(groupOf(next, "a")?.phase_id).toBe("p3");
    expect(next.tasks[2]?.task_ids).toEqual(["a"]);
  });
});

describe("moveItem", () => {
  it("crosses a phase boundary when a task is dropped on a phase row", () => {
    expect(moveItem(order(), { kind: "task", id: "a" }, { kind: "phase", id: "p2", position: "before" }).tasks[1]?.task_ids).toEqual(["a", "c"]);
    expect(moveItem(order(), { kind: "task", id: "a" }, { kind: "phase", id: "p2", position: "after" }).tasks[1]?.task_ids).toEqual(["c", "a"]);
  });

  it("drops a phase beside the phase a task belongs to", () => {
    const next = moveItem(order(), { kind: "phase", id: "p3" }, { kind: "task", id: "c", position: "before" });
    expect(next.phases).toEqual(["p1", "p3", "p2"]);
  });

  it("leaves the order alone when the closest phase cannot be named", () => {
    const current = order();
    const withOrphan: PlanOrder = {
      ...current,
      tasks: [...current.tasks, { phase_id: null, task_ids: ["z"] }],
    };
    expect(moveItem(withOrphan, { kind: "phase", id: "p1" }, { kind: "task", id: "z", position: "after" })).toBe(withOrphan);
  });
});

describe("stepOrder", () => {
  it("moves one place and refuses to move past either end", () => {
    expect(stepOrder(order(), "phase", "p2", -1).phases).toEqual(["p2", "p1", "p3"]);
    expect(stepOrder(order(), "task", "b", -1).tasks[0]?.task_ids).toEqual(["b", "a"]);
    const current = order();
    expect(stepOrder(current, "phase", "p1", -1)).toBe(current);
    expect(stepOrder(current, "phase", "p3", 1)).toBe(current);
    expect(stepOrder(current, "task", "a", -1)).toBe(current);
  });
});

describe("positionOf", () => {
  it("names a row's place in its own list", () => {
    expect(positionOf(order(), "phase", "p2")).toEqual({ index: 1, total: 3 });
    expect(positionOf(order(), "task", "b")).toEqual({ index: 1, total: 2 });
  });
});

describe("sameOrder", () => {
  it("ignores groups that hold nothing", () => {
    const left: PlanOrder = { phases: ["p1"], tasks: [{ phase_id: "p1", task_ids: ["a"] }] };
    const right: PlanOrder = {
      phases: ["p1"],
      tasks: [
        { phase_id: "p1", task_ids: ["a"] },
        { phase_id: "p2", task_ids: [] },
      ],
    };
    expect(sameOrder(left, right)).toBe(true);
  });

  it("notices a different phase for the same task", () => {
    const left: PlanOrder = { phases: ["p1", "p2"], tasks: [{ phase_id: "p1", task_ids: ["a"] }] };
    const right: PlanOrder = {
      phases: ["p1", "p2"],
      tasks: [
        { phase_id: "p1", task_ids: [] },
        { phase_id: "p2", task_ids: ["a"] },
      ],
    };
    expect(sameOrder(left, right)).toBe(false);
  });
});
