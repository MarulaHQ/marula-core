/**
 * The project Timeline's wire shapes and calls.
 *
 * The generated SDK in `@marula-core/sdk` still describes the pre-planning
 * `ProjectTask` (no `start_at`, `duration_minutes`, `baseline_*`, `is_critical`
 * or `work_type`), and it does not know the schedule, calendar or baseline
 * routes at all — they landed after the last spec export. The API returns the
 * full entity (serde serialises every field), so the extra members are read
 * through the narrow interfaces below instead of by editing generated files.
 *
 * **At merge these move to the generated client.** When the SDK is re-pinned,
 * delete the wire interfaces here and type the calls from
 * `components["schemas"]`; nothing else in this module should need to change.
 */
import { api, apiBaseUrl, authHeaders } from "./api";
import type { components } from "@marula-core/sdk";

export type TimelineProject = components["schemas"]["Project"];
export type TimelinePhase = components["schemas"]["ProjectPhase"];

/** The plan fields the timeline reads, on top of the generated entity. */
export interface TimelineTaskWire {
  id: string;
  project_id: string;
  phase_id: string | null;
  title: string;
  status: string;
  priority: string | null;
  assignee_id: string | null;
  work_type: string | null;
  role_key: string | null;
  external_owner: string | null;
  is_milestone: boolean;
  due_at: string | null;
  start_at: string | null;
  duration_minutes: number | null;
  effort_estimate_minutes: number | null;
  is_critical: boolean;
  baseline_start_at: string | null;
  baseline_duration_minutes: number | null;
  sort_order: number;
}

export interface CalendarWire {
  weekdays: number;
  minutes_per_day: number;
}

export interface BaselineWire {
  id: string;
  captured_at: string;
  note: string | null;
  dated_tasks: number;
  total_tasks: number;
}

/** `POST /schedule/preview` and `/apply` answer with the same plan shape. */
export interface SchedulePlanWire {
  changes: {
    task_id: string;
    from_start: string | null;
    to_start: string | null;
    reason: "moved" | "pushed" | string;
  }[];
  critical_path: string[];
}

export interface ScheduleApplyWire extends SchedulePlanWire {
  revert: {
    task_id: string;
    start_at: string | null;
    duration_minutes: number | null;
  }[];
}

export interface PhaseBurnWire {
  phase_id: string;
  phase_name: string;
  phase_status: string;
  consumed_minutes: number;
  alert: "none" | "warn" | "over" | string;
}

interface FetchOptions {
  signal?: AbortSignal;
}

async function getJson<T>(path: string, options: FetchOptions = {}): Promise<T> {
  const response = await fetch(`${apiBaseUrl()}${path}`, {
    method: "GET",
    headers: { ...authHeaders() },
    signal: options.signal,
  });
  if (!response.ok) throw new Error(messageFor(response.status, "read"));
  return (await response.json()) as T;
}

async function postJson<T>(path: string, body: unknown): Promise<T> {
  const response = await fetch(`${apiBaseUrl()}${path}`, {
    method: "POST",
    headers: { ...authHeaders(), "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!response.ok) throw new Error(messageFor(response.status, "write"));
  return (await response.json()) as T;
}

function messageFor(status: number, kind: "read" | "write"): string {
  if (status === 403) return "You do not have permission to see this project's plan.";
  if (status === 404) return "This project is not visible to you.";
  if (status === 409) return "Someone else changed the plan. Reload and try again.";
  return kind === "read"
    ? "The plan could not be loaded."
    : "The change was refused. Nothing was saved.";
}

/** The project, its phases and its tasks. The project and phase reads go
 * through the generated client because those shapes are already current. */
export async function fetchProjectPlan(projectId: string, options: FetchOptions = {}) {
  const [project, phases, tasks] = await Promise.all([
    api.api.GET("/api/v1/projects/{id}", { params: { path: { id: projectId } } }),
    api.api.GET("/api/v1/projects/{id}/phases", { params: { path: { id: projectId } } }),
    // The route defaults to 50 per page; a chart must not silently lose the
    // 51st task. 10,000 is the same bound the scheduler's own read uses.
    getJson<TimelineTaskWire[]>(
      `/api/v1/projects/${encodeURIComponent(projectId)}/tasks?limit=10000`,
      options,
    ),
  ]);
  if (project.error !== undefined || project.data === undefined) {
    throw new Error(messageFor(statusOf(project), "read"));
  }
  if (phases.error !== undefined || phases.data === undefined) {
    throw new Error(messageFor(statusOf(phases), "read"));
  }
  return {
    project: project.data,
    phases: phases.data,
    tasks: tasks.slice().sort((a, b) => a.sort_order - b.sort_order),
  };
}

/** The HTTP status of a generated-client answer, or 0 when it never answered. */
function statusOf(result: { response?: Response }): number {
  return result.response?.status ?? 0;
}

/** Dependency edges, as `tasks/{id}/dependencies` reports them per task. The
 * project-wide edge list is not exposed on this branch, so the surface asks
 * only about the tasks that have any, eight at a time. A project-wide edges
 * route is the fix at merge; the count of round trips is the reason to ask. */
export async function fetchDependencyEdges(
  projectId: string,
  taskIds: string[],
): Promise<{ task_id: string; depends_on_task_id: string }[]> {
  const edges: { task_id: string; depends_on_task_id: string }[] = [];
  const queue = [...taskIds];
  const worker = async (): Promise<void> => {
    for (;;) {
      const taskId = queue.shift();
      if (taskId === undefined) return;
      try {
        const response = await fetch(
          `${apiBaseUrl()}/api/v1/projects/${encodeURIComponent(projectId)}/tasks/${encodeURIComponent(taskId)}/dependencies`,
          { headers: { ...authHeaders() } },
        );
        if (!response.ok) continue;
        const body = (await response.json()) as { depends_on?: { id: string }[] };
        for (const dep of body.depends_on ?? []) {
          edges.push({ task_id: taskId, depends_on_task_id: dep.id });
        }
      } catch {
        // A refused edge read leaves that task without arrows, not the page
        // without a timeline.
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(8, queue.length) }, worker));
  return edges;
}

export async function fetchCalendar(projectId: string): Promise<CalendarWire | null> {
  try {
    const body = await getJson<{ calendar: CalendarWire | null }>(
      `/api/v1/projects/${encodeURIComponent(projectId)}/calendar`,
    );
    return body.calendar;
  } catch {
    // A project with no override answers `{calendar: null}`; a refusal here is
    // the same answer for the chart's purposes — the standard week applies.
    return null;
  }
}

export async function fetchBaseline(projectId: string): Promise<BaselineWire | null> {
  const body = await getJson<{ baseline: BaselineWire | null }>(
    `/api/v1/projects/${encodeURIComponent(projectId)}/baseline`,
  );
  return body.baseline;
}

export async function fetchPhaseBurn(projectId: string): Promise<PhaseBurnWire[]> {
  try {
    return await getJson<PhaseBurnWire[]>(`/api/v1/projects/${encodeURIComponent(projectId)}/phase-burn`);
  } catch {
    return [];
  }
}

export interface ScheduleMoveWire {
  task_id: string;
  start_at: string | null;
  duration_minutes?: number | null;
}

export function previewSchedule(
  projectId: string,
  moves: ScheduleMoveWire[],
): Promise<SchedulePlanWire> {
  return postJson<SchedulePlanWire>(`/api/v1/projects/${encodeURIComponent(projectId)}/schedule/preview`, { moves });
}

export function applySchedule(
  projectId: string,
  moves: ScheduleMoveWire[],
): Promise<ScheduleApplyWire> {
  return postJson<ScheduleApplyWire>(`/api/v1/projects/${encodeURIComponent(projectId)}/schedule/apply`, { moves });
}

/** Set (or clear) one task's dates through the ordinary autosave path. */
export async function patchTaskDates(
  projectId: string,
  taskId: string,
  startAt: string | null,
  durationMinutes: number | null,
): Promise<void> {
  const response = await fetch(
    `${apiBaseUrl()}/api/v1/projects/${encodeURIComponent(projectId)}/tasks/${encodeURIComponent(taskId)}`,
    {
      method: "PATCH",
      headers: { ...authHeaders(), "content-type": "application/json" },
      body: JSON.stringify({
        idempotency_key: crypto.randomUUID(),
        start_at: startAt,
        duration_minutes: durationMinutes,
      }),
    },
  );
  if (!response.ok) throw new Error(messageFor(response.status, "write"));
}

/** The org directory, so a bar can carry a name rather than a uuid. */
export async function fetchAssigneeNames(): Promise<Map<string, string>> {
  try {
    const { data } = await api.api.GET("/api/v1/org/users", {
      params: { query: { limit: 200 } },
    });
    const map = new Map<string, string>();
    for (const user of data?.users ?? []) map.set(user.id, user.display_name);
    return map;
  } catch {
    return new Map();
  }
}
