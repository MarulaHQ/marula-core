// Unit tests: projects-metrics — schedule risk, phase/task roll-ups,
// triage sets (Wave 17 lane 4). Deterministic via injected `now`.
import { describe, expect, it } from "vitest";
import {
  blockedTasks,
  dueSoonTasks,
  overdueTasks,
  phaseProgress,
  projectRisk,
  projectStatusTone,
  taskProgress,
} from "../../lib/projects-metrics";
import type { ProjectPhase, ProjectTask } from "../../lib/projects-metrics";

const NOW = new Date("2026-09-17T12:00:00Z").getTime();
const DAY = 86_400_000;

function task(over: Partial<ProjectTask>): ProjectTask {
  return {
    id: "t1",
    org_id: "o",
    project_id: "p",
    title: "Task",
    status: "pending",
    sort_order: 0,
    is_milestone: false,
    milestone_amount_minor: null,
    milestone_currency: null,
    phase_id: null,
    assignee_id: null,
    description: null,
    due_at: null,
    completed_at: null,
    created_at: "",
    updated_at: "",
    ...over,
  };
}

function phase(over: Partial<ProjectPhase>): ProjectPhase {
  return {
    id: "ph1",
    org_id: "o",
    project_id: "p",
    name: "Discovery",
    sort_order: 0,
    status: "pending",
    is_milestone: false,
    milestone_amount_minor: null,
    milestone_currency: null,
    description: null,
    completed_at: null,
    created_at: "",
    updated_at: "",
    ...over,
  };
}

describe("projectRisk", () => {
  it("flags overdue open projects as danger with a day count", () => {
    expect(
      projectRisk({ status: "active", due_date: "2026-09-15" }, NOW),
    ).toEqual({ tone: "danger", label: "overdue 2d" });
  });
  it("flags due-within-7d as warning", () => {
    expect(
      projectRisk({ status: "planning", due_date: "2026-09-22" }, NOW),
    ).toEqual({ tone: "warning", label: "due in 5d" });
  });
  it("stays silent beyond the 7-day horizon and for undated/terminal projects", () => {
    expect(projectRisk({ status: "active", due_date: "2027-01-01" }, NOW)).toBeNull();
    expect(projectRisk({ status: "active", due_date: null }, NOW)).toBeNull();
    expect(projectRisk({ status: "completed", due_date: "2026-09-01" }, NOW)).toBeNull();
    expect(projectRisk({ status: "cancelled", due_date: "2026-09-01" }, NOW)).toBeNull();
  });
});

describe("projectStatusTone", () => {
  it("maps the PSA statuses onto semantic tones", () => {
    expect(projectStatusTone("active")).toBe("success");
    expect(projectStatusTone("on_hold")).toBe("warning");
    expect(projectStatusTone("cancelled")).toBe("danger");
    expect(projectStatusTone("planning")).toBe("neutral");
    expect(projectStatusTone("completed")).toBe("neutral");
  });
});

describe("phaseProgress / taskProgress", () => {
  it("rolls up completion and milestone counts", () => {
    const phases = [
      phase({ id: "a", status: "completed" }),
      phase({ id: "b", status: "active" }),
      phase({ id: "c", is_milestone: true, status: "completed" }),
      phase({ id: "d", is_milestone: true, status: "pending" }),
    ];
    expect(phaseProgress(phases)).toEqual({
      done: 2,
      total: 4,
      pct: 50,
      milestonesDone: 1,
      milestonesTotal: 2,
    });
    expect(phaseProgress([])).toEqual({
      done: 0,
      total: 0,
      pct: 0,
      milestonesDone: 0,
      milestonesTotal: 0,
    });
  });
  it("rolls up task completion", () => {
    const tasks = [task({ status: "completed" }), task({}), task({ status: "cancelled" })];
    expect(taskProgress(tasks)).toEqual({ done: 1, total: 3, pct: 33 });
  });
});

describe("triage sets", () => {
  const open = [
    task({ id: "overdue", due_at: new Date(NOW - DAY).toISOString() }),
    task({ id: "soon", due_at: new Date(NOW + 2 * DAY).toISOString() }),
    task({ id: "later", due_at: new Date(NOW + 10 * DAY).toISOString() }),
    task({ id: "done", status: "completed", due_at: new Date(NOW - DAY).toISOString() }),
    task({ id: "nodue" }),
  ];
  it("overdueTasks only counts unfinished past-due tasks", () => {
    expect(overdueTasks(open, NOW).map((t) => t.id)).toEqual(["overdue"]);
  });
  it("dueSoonTasks covers the inclusive horizon window", () => {
    expect(dueSoonTasks(open, 3, NOW).map((t) => t.id)).toEqual(["soon"]);
  });
  it("blockedTasks uses dependency openness", () => {
    const deps = {
      overdue: [task({ id: "dep-done", status: "completed" })],
      soon: [task({ id: "dep-open" })],
    };
    expect(blockedTasks(open, deps).map((t) => t.id)).toEqual(["soon"]);
  });
});
