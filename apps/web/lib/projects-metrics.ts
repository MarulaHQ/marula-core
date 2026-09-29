// Project portfolio + delivery metrics derived client-side from the typed
// SDK shapes (Wave 17 lane 4). Pure functions, injectable `now` for
// deterministic tests. Honest derivations only — every input comes from a
// real endpoint response (projects, phases, tasks, dependencies).
import type { components } from "@marula-core/sdk";

export type Project = components["schemas"]["Project"];
export type ProjectPhase = components["schemas"]["ProjectPhase"];
export type ProjectTask = components["schemas"]["ProjectTask"];

const DAY_MS = 86_400_000;

export type RiskTone = "danger" | "warning" | "neutral";
export type StatusTone = RiskTone | "success";

/** Schedule-risk signal for one project from its own due date + status. */
export function projectRisk(
  project: Pick<Project, "status" | "due_date">,
  now: number = Date.now(),
): { tone: RiskTone; label: string } | null {
  if (project.due_date === null || project.due_date === undefined) return null;
  if (project.status === "completed" || project.status === "cancelled") return null;
  const due = new Date(`${project.due_date}T00:00:00`).getTime();
  if (Number.isNaN(due)) return null;
  const days = Math.ceil((due - now) / DAY_MS);
  if (days < 0) return { tone: "danger", label: `overdue ${Math.abs(days)}d` };
  if (days <= 7) return { tone: "warning", label: days === 0 ? "due today" : `due in ${days}d` };
  return null;
}

export interface PhaseProgress {
  done: number;
  total: number;
  pct: number;
  milestonesDone: number;
  milestonesTotal: number;
}

/** Phase completion roll-up for progress bars (sort-order agnostic). */
export function phaseProgress(phases: ProjectPhase[]): PhaseProgress {
  const total = phases.length;
  const done = phases.filter((p) => p.status === "completed").length;
  const milestones = phases.filter((p) => p.is_milestone);
  return {
    done,
    total,
    pct: total === 0 ? 0 : Math.round((done / total) * 100),
    milestonesDone: milestones.filter((p) => p.status === "completed").length,
    milestonesTotal: milestones.length,
  };
}

export function taskProgress(tasks: ProjectTask[]): { done: number; total: number; pct: number } {
  const total = tasks.length;
  const done = tasks.filter((t) => t.status === "completed").length;
  return { done, total, pct: total === 0 ? 0 : Math.round((done / total) * 100) };
}

export type TaskOpenness = "open" | "done" | "cancelled";

export function taskOpenness(task: ProjectTask): TaskOpenness {
  if (task.status === "completed") return "done";
  if (task.status === "cancelled") return "cancelled";
  return "open";
}

/** Tasks whose due date has passed and are not finished. */
export function overdueTasks(tasks: ProjectTask[], now: number = Date.now()): ProjectTask[] {
  return tasks.filter((t) => {
    if (taskOpenness(t) !== "open" || t.due_at === null || t.due_at === undefined) return false;
    return new Date(t.due_at).getTime() < now;
  });
}

/** Open tasks due within the next `days` days (inclusive). */
export function dueSoonTasks(
  tasks: ProjectTask[],
  days: number,
  now: number = Date.now(),
): ProjectTask[] {
  const horizon = now + days * DAY_MS;
  return tasks.filter((t) => {
    if (taskOpenness(t) !== "open" || t.due_at === null || t.due_at === undefined) return false;
    const due = new Date(t.due_at).getTime();
    return due >= now && due <= horizon;
  });
}

/** Open tasks blocked by at least one unfinished dependency. */
export function blockedTasks(
  tasks: ProjectTask[],
  deps: Record<string, ProjectTask[]>,
): ProjectTask[] {
  return tasks.filter((t) => {
    if (taskOpenness(t) !== "open") return false;
    return (deps[t.id] ?? []).some((b) => taskOpenness(b) === "open");
  });
}

export type ProjectStatusTone = StatusTone;

export function projectStatusTone(status: string): ProjectStatusTone {
  switch (status) {
    case "active":
      return "success";
    case "on_hold":
      return "warning";
    case "cancelled":
      return "danger";
    default:
      return "neutral";
  }
}
