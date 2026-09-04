// /projects/[id] — the project record (Workspace-core UI, T3).
//
// Purpose: one engagement's delivery surface: identity header, phases
// (create / complete, milestone amounts), tasks grouped with phase and
// milestone flags, finish-to-start dependency display + editing (PUT full
// set), and the gated completion endpoints whose envelope reconciles the
// optimistic status. Completing a milestone emits the billing seam event.
// Non-purpose: no Gantt/Budget views (List-first; view switchers land with
// those views); no ticket conversion (ticket-compatible fields are shown,
// the link-up is a later lane).
"use client";

import {
  Button,
  Card,
  Checkbox,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  Table,
  Tag,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import Link from "next/link";
import { useParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import { money } from "../../../lib/format";
import { runEnvelopeMutation } from "../../../lib/envelope-write";

type Project = components["schemas"]["Project"];
type ProjectPhase = components["schemas"]["ProjectPhase"];
type ProjectTask = components["schemas"]["ProjectTask"];
type Client = components["schemas"]["Client"];
type ProjectMutationResponse = components["schemas"]["ProjectMutationResponse"];
type PhaseMutationResponse = components["schemas"]["PhaseMutationResponse"];
type TaskMutationResponse = components["schemas"]["TaskMutationResponse"];
type DependencyListResponse = components["schemas"]["DependencyListResponse"];

export default function ProjectDetailPage() {
  const auth = useRequireAuth();
  const params = useParams<{ id: string }>();
  const projectId = params.id;

  const [project, setProject] = useState<Project | null>(null);
  const [client, setClient] = useState<Client | null>(null);
  const [phases, setPhases] = useState<ProjectPhase[]>([]);
  const [tasks, setTasks] = useState<ProjectTask[]>([]);
  const [deps, setDeps] = useState<Record<string, ProjectTask[]>>({});
  const [loadError, setLoadError] = useState<string | null>(null);

  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const [phaseOpen, setPhaseOpen] = useState(false);
  const [phaseName, setPhaseName] = useState("");
  const [phaseMilestone, setPhaseMilestone] = useState(false);
  const [phaseAmount, setPhaseAmount] = useState("");

  const [taskOpen, setTaskOpen] = useState(false);
  const [taskTitle, setTaskTitle] = useState("");
  const [taskPhase, setTaskPhase] = useState<string>("none");
  const [taskMilestone, setTaskMilestone] = useState(false);
  const [taskAmount, setTaskAmount] = useState("");
  const [taskDependsOn, setTaskDependsOn] = useState<Set<string>>(new Set());

  const [depTaskId, setDepTaskId] = useState<ProjectTask | null>(null);
  const [depSelection, setDepSelection] = useState<Set<string>>(new Set());

  const refresh = useCallback(async () => {
    const p = await api.api.GET("/api/v1/projects/{id}", {
      params: { path: { id: projectId } },
    });
    if (p.error !== undefined || p.data === undefined) {
      setLoadError("Failed to load this project. It may not exist or you may not have access.");
      setProject(null);
      return;
    }
    setProject(p.data);
    const clientRef = p.data.client_id;
    if (clientRef !== null && clientRef !== undefined) {
      const c = await api.api.GET("/api/v1/clients/{id}", {
        params: { path: { id: clientRef } },
      });
      setClient(c.data ?? null);
    }
    setLoadError(null);
    const [ph, t] = await Promise.all([
      api.api.GET("/api/v1/projects/{id}/phases", { params: { path: { id: projectId } } }),
      api.api.GET("/api/v1/projects/{id}/tasks", { params: { path: { id: projectId } } }),
    ]);
    setPhases(ph.data ?? []);
    const taskRows = t.data ?? [];
    setTasks(taskRows);
    const entries = await Promise.all(
      taskRows.map(async (task) => {
        const d = await api.api.GET("/api/v1/projects/{id}/tasks/{task_id}/dependencies", {
          params: { path: { id: projectId, task_id: task.id } },
        });
        return [task.id, d.data?.depends_on ?? []] as const;
      }),
    );
    setDeps(Object.fromEntries(entries));
  }, [projectId]);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  const phaseNameOf = useCallback(
    (phaseId: string | null | undefined) =>
      phaseId === null || phaseId === undefined
        ? "No phase"
        : (phases.find((p) => p.id === phaseId)?.name ?? "—"),
    [phases],
  );

  const blockersLabel = useCallback(
    (task: ProjectTask) => {
      const blockers = deps[task.id] ?? [];
      if (blockers.length === 0) return null;
      const open = blockers.filter((b) => b.status !== "completed" && b.status !== "cancelled");
      return open.length === 0
        ? `after ${blockers.length} done`
        : `blocked by ${open.map((b) => b.title).join(", ")}`;
    },
    [deps],
  );

  const taskById = useMemo(() => new Map(tasks.map((t) => [t.id, t])), [tasks]);

  const completeTask = async (task: ProjectTask) => {
    setActionError(null);
    const result = await runEnvelopeMutation<
      components["schemas"]["MutationParams"],
      TaskMutationResponse
    >({
      describe: `Complete task ${task.title}`,
      entityId: task.id,
      optimistic: () =>
        setTasks((prev) =>
          prev.map((t) => (t.id === task.id ? { ...t, status: "completed" } : t)),
        ),
      revert: () =>
        setTasks((prev) =>
          prev.map((t) => (t.id === task.id ? { ...t, status: task.status } : t)),
        ),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key, version: base.version }),
      request: (c, body) =>
        c.POST("/api/v1/projects/{id}/tasks/{task_id}/complete", {
          params: { path: { id: projectId, task_id: task.id } },
          body,
        }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    if (!result.ok) {
      void refresh();
      setActionError(
        result.message ??
          (result.reason === "dependency_not_ready"
            ? "Dependencies must complete first."
            : "The completion was rejected. The task is unchanged."),
      );
      return;
    }
    void refresh();
  };

  const completePhase = async (phase: ProjectPhase) => {
    setActionError(null);
    const result = await runEnvelopeMutation<
      components["schemas"]["MutationParams"],
      PhaseMutationResponse
    >({
      describe: `Complete phase ${phase.name}`,
      entityId: phase.id,
      optimistic: () =>
        setPhases((prev) =>
          prev.map((p) => (p.id === phase.id ? { ...p, status: "completed" } : p)),
        ),
      revert: () =>
        setPhases((prev) =>
          prev.map((p) => (p.id === phase.id ? { ...p, status: phase.status } : p)),
        ),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key, version: base.version }),
      request: (c, body) =>
        c.POST("/api/v1/projects/{id}/phases/{phase_id}/complete", {
          params: { path: { id: projectId, phase_id: phase.id } },
          body,
        }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    if (!result.ok) {
      void refresh();
      setActionError(result.message ?? "The phase completion was rejected.");
      return;
    }
    void refresh();
  };

  const createPhase = async () => {
    if (phaseName.trim() === "") return;
    setBusy(true);
    setActionError(null);
    const tempId = `pending-phase:${phaseName}`;
    const result = await runEnvelopeMutation<
      components["schemas"]["CreatePhaseRequest"],
      PhaseMutationResponse
    >({
      describe: "Create phase",
      entityId: tempId,
      optimistic: () =>
        setPhases((prev) => [
          ...prev,
          {
            id: tempId,
            org_id: "",
            project_id: projectId,
            name: phaseName.trim(),
            sort_order: prev.length,
            status: "pending",
            is_milestone: phaseMilestone,
            milestone_amount_minor:
              phaseMilestone && phaseAmount.trim() !== "" ? Math.round(Number(phaseAmount) * 100) : null,
            milestone_currency:
              phaseMilestone && phaseAmount.trim() !== "" ? "USD" : null,
            description: null,
            completed_at: null,
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
          },
        ]),
      revert: () => setPhases((prev) => prev.filter((p) => p.id !== tempId)),
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        name: phaseName.trim(),
        is_milestone: phaseMilestone,
        milestone_amount_minor:
          phaseMilestone && phaseAmount.trim() !== "" ? Math.round(Number(phaseAmount) * 100) : null,
        milestone_currency: phaseMilestone && phaseAmount.trim() !== "" ? "USD" : null,
      }),
      request: (c, body) =>
        c.POST("/api/v1/projects/{id}/phases", { params: { path: { id: projectId } }, body }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setBusy(false);
    if (!result.ok) {
      setActionError(result.message ?? "The phase was rejected. Nothing was saved.");
      return;
    }
    setPhaseOpen(false);
    setPhaseName("");
    setPhaseMilestone(false);
    setPhaseAmount("");
    void refresh();
  };

  const createTask = async () => {
    if (taskTitle.trim() === "") return;
    setBusy(true);
    setActionError(null);
    const tempId = `pending-task:${taskTitle}`;
    const result = await runEnvelopeMutation<
      components["schemas"]["CreateTaskRequest"],
      TaskMutationResponse
    >({
      describe: "Create task",
      entityId: tempId,
      optimistic: () =>
        setTasks((prev) => [
          ...prev,
          {
            id: tempId,
            org_id: "",
            project_id: projectId,
            title: taskTitle.trim(),
            status: "pending",
            sort_order: prev.length,
            is_milestone: taskMilestone,
            milestone_amount_minor:
              taskMilestone && taskAmount.trim() !== "" ? Math.round(Number(taskAmount) * 100) : null,
            milestone_currency: taskMilestone && taskAmount.trim() !== "" ? "USD" : null,
            phase_id: taskPhase === "none" ? null : taskPhase,
            assignee_id: null,
            description: null,
            due_at: null,
            completed_at: null,
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
          },
        ]),
      revert: () => setTasks((prev) => prev.filter((t) => t.id !== tempId)),
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        title: taskTitle.trim(),
        phase_id: taskPhase === "none" ? null : taskPhase,
        is_milestone: taskMilestone,
        milestone_amount_minor:
          taskMilestone && taskAmount.trim() !== "" ? Math.round(Number(taskAmount) * 100) : null,
        milestone_currency: taskMilestone && taskAmount.trim() !== "" ? "USD" : null,
        depends_on: [...taskDependsOn],
      }),
      request: (c, body) =>
        c.POST("/api/v1/projects/{id}/tasks", { params: { path: { id: projectId } }, body }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setBusy(false);
    if (!result.ok) {
      setActionError(result.message ?? "The task was rejected. Nothing was saved.");
      return;
    }
    setTaskOpen(false);
    setTaskTitle("");
    setTaskMilestone(false);
    setTaskAmount("");
    setTaskPhase("none");
    setTaskDependsOn(new Set());
    void refresh();
  };

  const saveDependencies = async () => {
    if (depTaskId === null) return;
    setBusy(true);
    setActionError(null);
    type DepsBody = components["schemas"]["SetDependenciesRequest"] & components["schemas"]["MutationParams"];
    const result = await runEnvelopeMutation<DepsBody, DependencyListResponse>({
      describe: `Set dependencies for ${depTaskId.title}`,
      entityId: depTaskId.id,
      optimistic: () =>
        setDeps((prev) => ({
          ...prev,
          [depTaskId.id]: [...depSelection].map((id) => taskById.get(id)).filter(
            (t): t is ProjectTask => t !== undefined,
          ),
        })),
      revert: () => void refresh(),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key, depends_on: [...depSelection] }),
      request: (c, body) =>
        c.PUT("/api/v1/projects/{id}/tasks/{task_id}/dependencies", {
          params: { path: { id: projectId, task_id: depTaskId.id } },
          body: body as components["schemas"]["SetDependenciesRequest"],
        }),
    });
    setBusy(false);
    setDepTaskId(null);
    if (!result.ok) {
      setActionError(result.message ?? "The dependency set was rejected (cycle or foreign task?).");
      return;
    }
    void refresh();
  };

  if (auth.status !== "authed") return null;

  if (loadError !== null) {
    return (
      <div className="grid gap-4 p-6">
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {loadError}
        </p>
        <Link href="/projects" className="text-[13px] underline underline-offset-2">
          Back to projects
        </Link>
      </div>
    );
  }

  if (project === null) {
    return (
      <div className="p-6">
        <Spinner />
      </div>
    );
  }

  return (
    <div className="grid gap-6 p-6">
      <div>
        <Link href="/projects" className="text-[13px] underline decoration-[var(--m-border)] underline-offset-2" style={{ color: "var(--m-text-secondary)" }}>
          ← Projects
        </Link>
        <div className="mt-2 flex items-center gap-3">
          <h1 className="text-lg font-semibold">{project.name}</h1>
          <Tag>{project.status.replaceAll("_", " ")}</Tag>
        </div>
        <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {client !== null ? (
            <>
              Client:{" "}
              <Link href={`/clients/${client.id}`} className="underline decoration-[var(--m-border)] underline-offset-2">
                {client.name}
              </Link>
              {" · "}
            </>
          ) : (
            "Internal · "
          )}
          {project.billing_method.replaceAll("_", " ")}
          {project.budget_minor !== null && project.budget_minor !== undefined
            ? ` · budget ${money(project.budget_minor, project.currency ?? "USD")}`
            : ""}
          {project.due_date !== null ? ` · due ${project.due_date}` : ""}
        </p>
        {project.description !== null && project.description !== undefined ? (
          <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            {project.description}
          </p>
        ) : null}
      </div>

      {actionError !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {actionError}
        </p>
      ) : null}

      <Card>
        <div className="flex items-center justify-between p-4 pb-0">
          <h2 className="text-sm font-semibold">Phases</h2>
          <div className="flex items-center gap-2">
            <Link href="/billing" className="text-[13px] underline decoration-[var(--m-border)] underline-offset-2" style={{ color: "var(--m-text-secondary)" }}>
              Billing →
            </Link>
            <Dialog open={phaseOpen} onOpenChange={setPhaseOpen}>
              <DialogTrigger asChild>
                <Button size="sm">Add phase</Button>
              </DialogTrigger>
              <DialogContent title="Add phase">
                <div className="grid gap-3 p-4">
                  <div>
                    <Label htmlFor="phase-name">Name</Label>
                    <Input id="phase-name" value={phaseName} onChange={(e) => setPhaseName(e.target.value)} placeholder="Discovery" />
                  </div>
                  <label className="flex items-center gap-2 text-[13px]">
                    <Checkbox checked={phaseMilestone} onCheckedChange={(v) => setPhaseMilestone(v === true)} />
                    Billing milestone
                  </label>
                  {phaseMilestone ? (
                    <div>
                      <Label htmlFor="phase-amount">Milestone amount (USD)</Label>
                      <Input id="phase-amount" type="number" min={0} step="0.01" value={phaseAmount} onChange={(e) => setPhaseAmount(e.target.value)} />
                      <FieldHint>Released to billing when the phase completes.</FieldHint>
                    </div>
                  ) : null}
                  <DialogActions>
                    <Button variant="ghost" onClick={() => setPhaseOpen(false)}>
                      Cancel
                    </Button>
                    <Button onClick={() => void createPhase()} disabled={busy || phaseName.trim() === ""}>
                      {busy ? <Spinner /> : null} Add phase
                    </Button>
                  </DialogActions>
                </div>
              </DialogContent>
            </Dialog>
          </div>
        </div>
        {phases.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No phases yet. Phases group tasks into delivery stages; a milestone phase releases its amount to billing on completion." />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Name</Th>
                <Th>Status</Th>
                <Th>Milestone</Th>
                <Th className="text-right">Actions</Th>
              </tr>
            </thead>
            <tbody>
              {[...phases]
                .sort((a, b) => a.sort_order - b.sort_order)
                .map((phase) => (
                  <Tr key={phase.id}>
                    <Td>{phase.name}</Td>
                    <Td>
                      <Tag>{phase.status}</Tag>
                    </Td>
                    <Td>
                      {phase.is_milestone ? (
                        phase.milestone_amount_minor !== null && phase.milestone_amount_minor !== undefined ? (
                          <span className="tabular-nums">
                            {money(phase.milestone_amount_minor, phase.milestone_currency ?? "USD")}
                          </span>
                        ) : (
                          <Tag>milestone</Tag>
                        )
                      ) : (
                        <span style={{ color: "var(--m-text-muted)" }}>—</span>
                      )}
                    </Td>
                    <Td className="text-right">
                      {phase.status === "pending" || phase.status === "active" ? (
                        <Button variant="ghost" size="sm" onClick={() => void completePhase(phase)}>
                          Complete
                        </Button>
                      ) : null}
                    </Td>
                  </Tr>
                ))}
            </tbody>
          </Table>
        )}
      </Card>

      <Card>
        <div className="flex items-center justify-between p-4 pb-0">
          <h2 className="text-sm font-semibold">Tasks</h2>
          <Dialog open={taskOpen} onOpenChange={setTaskOpen}>
            <DialogTrigger asChild>
              <Button size="sm">Add task</Button>
            </DialogTrigger>
            <DialogContent title="Add task">
              <div className="grid gap-3 p-4">
                <div>
                  <Label htmlFor="task-title">Title</Label>
                  <Input id="task-title" value={taskTitle} onChange={(e) => setTaskTitle(e.target.value)} />
                </div>
                <div>
                  <Label htmlFor="task-phase">Phase</Label>
                  <Select value={taskPhase} onValueChange={setTaskPhase}>
                    <SelectTrigger id="task-phase" className="w-56">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="none">No phase</SelectItem>
                      {phases.map((p) => (
                        <SelectItem key={p.id} value={p.id}>
                          {p.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <label className="flex items-center gap-2 text-[13px]">
                  <Checkbox checked={taskMilestone} onCheckedChange={(v) => setTaskMilestone(v === true)} />
                  Billing milestone
                </label>
                {taskMilestone ? (
                  <div>
                    <Label htmlFor="task-amount">Milestone amount (USD)</Label>
                    <Input id="task-amount" type="number" min={0} step="0.01" value={taskAmount} onChange={(e) => setTaskAmount(e.target.value)} />
                  </div>
                ) : null}
                <div>
                  <Label>Depends on (finish-to-start)</Label>
                  <div className="grid max-h-40 gap-1 overflow-auto rounded-[2px] border p-2" style={{ borderColor: "var(--m-border)" }}>
                    {tasks.length === 0 ? (
                      <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                        No existing tasks to depend on.
                      </span>
                    ) : (
                      tasks.map((t) => (
                        <label key={t.id} className="flex items-center gap-2 text-[13px]">
                          <Checkbox
                            checked={taskDependsOn.has(t.id)}
                            onCheckedChange={(v) =>
                              setTaskDependsOn((prev) => {
                                const next = new Set(prev);
                                if (v === true) next.add(t.id);
                                else next.delete(t.id);
                                return next;
                              })
                            }
                          />
                          {t.title}
                        </label>
                      ))
                    )}
                  </div>
                  <FieldHint>The new task cannot complete until these do.</FieldHint>
                </div>
                <DialogActions>
                  <Button variant="ghost" onClick={() => setTaskOpen(false)}>
                    Cancel
                  </Button>
                  <Button onClick={() => void createTask()} disabled={busy || taskTitle.trim() === ""}>
                    {busy ? <Spinner /> : null} Add task
                  </Button>
                </DialogActions>
              </div>
            </DialogContent>
          </Dialog>
        </div>
        {tasks.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No tasks yet. Project tasks carry phase, assignee, and finish-to-start dependencies; milestone tasks release their amount to billing on completion." />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Title</Th>
                <Th>Phase</Th>
                <Th>Status</Th>
                <Th>Milestone</Th>
                <Th>Blocked by</Th>
                <Th className="text-right">Actions</Th>
              </tr>
            </thead>
            <tbody>
              {tasks.map((task) => {
                const blockers = deps[task.id] ?? [];
                const openBlockers = blockers.filter(
                  (b) => b.status !== "completed" && b.status !== "cancelled",
                );
                return (
                  <Tr key={task.id}>
                    <Td>
                      {task.title}
                      {task.is_milestone ? (
                        <span className="ml-2">
                          {task.milestone_amount_minor !== null && task.milestone_amount_minor !== undefined ? (
                            <span className="tabular-nums">
                              {money(task.milestone_amount_minor, task.milestone_currency ?? "USD")}
                            </span>
                          ) : (
                            <Tag>milestone</Tag>
                          )}
                        </span>
                      ) : null}
                    </Td>
                    <Td>{phaseNameOf(task.phase_id)}</Td>
                    <Td>
                      <Tag>{task.status.replaceAll("_", " ")}</Tag>
                    </Td>
                    <Td>{blockersLabel(task) ?? <span style={{ color: "var(--m-text-muted)" }}>—</span>}</Td>
                    <Td className="text-right">
                      <div className="flex justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => {
                            setDepTaskId(task);
                            setDepSelection(new Set(blockers.map((b) => b.id)));
                          }}
                        >
                          Dependencies ({openBlockers.length > 0 ? `${openBlockers.length} open` : "clear"})
                        </Button>
                        {task.status !== "completed" && task.status !== "cancelled" ? (
                          <Button variant="ghost" size="sm" onClick={() => void completeTask(task)}>
                            Complete
                          </Button>
                        ) : null}
                      </div>
                    </Td>
                  </Tr>
                );
              })}
            </tbody>
          </Table>
        )}
      </Card>

      <Dialog
        open={depTaskId !== null}
        onOpenChange={(open) => {
          if (!open) setDepTaskId(null);
        }}
      >
        <DialogContent
          title={depTaskId !== null ? `Dependencies — ${depTaskId.title}` : "Dependencies"}
          description="Pick the full set of tasks this one depends on (finish-to-start). Saving replaces the whole set."
        >
          <div className="grid gap-3 p-4">
            <div className="grid max-h-60 gap-1 overflow-auto rounded-[2px] border p-2" style={{ borderColor: "var(--m-border)" }}>
              {tasks
                .filter((t) => depTaskId === null || t.id !== depTaskId.id)
                .map((t) => (
                  <label key={t.id} className="flex items-center gap-2 text-[13px]">
                    <Checkbox
                      checked={depSelection.has(t.id)}
                      onCheckedChange={(v) =>
                        setDepSelection((prev) => {
                          const next = new Set(prev);
                          if (v === true) next.add(t.id);
                          else next.delete(t.id);
                          return next;
                        })
                      }
                    />
                    {t.title}
                    <span style={{ color: "var(--m-text-muted)" }}>{t.status.replaceAll("_", " ")}</span>
                  </label>
                ))}
            </div>
            <DialogActions>
              <Button variant="ghost" onClick={() => setDepTaskId(null)}>
                Cancel
              </Button>
              <Button onClick={() => void saveDependencies()} disabled={busy || depTaskId === null}>
                {busy ? <Spinner /> : null} Save dependencies
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
