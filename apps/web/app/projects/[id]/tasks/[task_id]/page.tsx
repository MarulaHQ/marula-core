// The project task workbench: one task, its dates and what it waits on, with
// the pods in the right-hand column (S10.1–S10.11).
//
// Purpose: the destination the "my work" feed, the plan's rows and a pod deep
// link all point at. The record page stays a plan; this page is one task read
// on its own so a technician can pick it up, see the blocker and promote it to
// a ticket without the whole Gantt around it.
//
// Non-purpose: no second task editor. Status and completion still ride the
// project record's mutation seam; this page reads and hosts the pods.

"use client";

import { Badge, Button, Card, Spinner, SurfaceHeader } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useParams, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { TaskPodsPanel } from "../../../../../components/projects/task-pods";
import { api, useRequireAuth } from "../../../../../lib/api";

type Project = components["schemas"]["Project"];
type ProjectTask = components["schemas"]["ProjectTask"];
type ProjectPhase = components["schemas"]["ProjectPhase"];

const STATUS_WORDS: Record<string, string> = {
  todo: "Not started",
  in_progress: "In progress",
  completed: "Done",
  cancelled: "Cancelled",
};

function day(iso: string | null | undefined): string {
  if (iso === null || iso === undefined) return "Not set";
  return new Date(iso).toLocaleDateString(undefined, { dateStyle: "medium" });
}

function hours(minutes: number | null | undefined): string {
  if (minutes === null || minutes === undefined) return "Not estimated";
  return `${Math.round((minutes / 60) * 10) / 10}h`;
}

export default function ProjectTaskPage() {
  const auth = useRequireAuth();
  const params = useParams<{ id: string; task_id: string }>();
  const search = useSearchParams();
  const projectId = params.id;
  const taskId = params.task_id;
  const pod = search.get("pod");

  const [project, setProject] = useState<Project | null>(null);
  const [task, setTask] = useState<ProjectTask | null>(null);
  const [phases, setPhases] = useState<ProjectPhase[]>([]);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setError(null);
    const [p, t, ph] = await Promise.all([
      api.api.GET("/api/v1/projects/{id}", { params: { path: { id: projectId } } }),
      api.api.GET("/api/v1/projects/{id}/tasks", { params: { path: { id: projectId }, query: { limit: 500 } } }),
      api.api.GET("/api/v1/projects/{id}/phases", { params: { path: { id: projectId } } }),
    ]);
    if (p.error !== undefined) {
      setError("This project could not be read. It may not exist, or you may not have access.");
      return;
    }
    setProject(p.data ?? null);
    setPhases(ph.data ?? []);
    const found = (t.data ?? []).find((row) => row.id === taskId) ?? null;
    setTask(found);
    if (found === null) setError("This task is not on the project plan any more.");
  }, [projectId, taskId]);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void load();
  }, [auth.status, load]);

  if (auth.status === "loading" || (project === null && error === null)) {
    return (
      <div className="flex items-center gap-2 p-6 text-[13px]">
        <Spinner /> Reading the task…
      </div>
    );
  }

  if (error !== null || task === null) {
    return (
      <div className="mx-auto flex w-full max-w-4xl flex-col gap-3 p-6">
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error ?? "This task could not be read."}
        </p>
        <Button asChild variant="secondary" size="sm">
          <Link href={`/projects/${projectId}`}>Back to the project</Link>
        </Button>
      </div>
    );
  }

  const phase = phases.find((row) => row.id === task.phase_id) ?? null;
  const waiting = task.status !== "completed" && task.status !== "cancelled";

  return (
    <div className="mx-auto flex w-full flex-col gap-4 p-4 md:px-6 md:py-5">
      <SurfaceHeader
        eyebrow={
          <Link href={`/projects/${projectId}`} className="hover:underline">
            {project?.name ?? "Project"}
            {phase === null ? "" : ` · ${phase.name}`}
          </Link>
        }
        title={task.title}
        meta={
          <>
            <Badge tone={task.status === "completed" ? "success" : waiting ? "neutral" : "neutral"}>
              {STATUS_WORDS[task.status] ?? task.status}
            </Badge>
            {task.is_milestone ? <Badge tone="neutral">Milestone</Badge> : null}
          </>
        }
      />

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1fr)_360px]">
        <Card className="flex flex-col gap-3 p-4">
          <h2 className="text-[15px] font-semibold">The task</h2>
          {task.description === null || task.description === undefined || task.description.trim() === "" ? (
            <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
              No description yet. The work is written on the project plan.
            </p>
          ) : (
            <p className="whitespace-pre-wrap text-[13px]">{task.description}</p>
          )}
          <dl className="grid grid-cols-2 gap-3 text-[13px]">
            <div className="flex flex-col gap-0.5">
              <dt style={{ color: "var(--m-text-secondary)" }}>Starts</dt>
              <dd className="font-mono tabular-nums">{day(task.start_at)}</dd>
            </div>
            <div className="flex flex-col gap-0.5">
              <dt style={{ color: "var(--m-text-secondary)" }}>Due</dt>
              <dd className="font-mono tabular-nums">{day(task.due_at)}</dd>
            </div>
            <div className="flex flex-col gap-0.5">
              <dt style={{ color: "var(--m-text-secondary)" }}>Effort left</dt>
              <dd className="font-mono tabular-nums">{hours(task.effort_remaining_minutes)}</dd>
            </div>
            <div className="flex flex-col gap-0.5">
              <dt style={{ color: "var(--m-text-secondary)" }}>Estimate</dt>
              <dd className="font-mono tabular-nums">{hours(task.effort_estimate_minutes)}</dd>
            </div>
          </dl>
          <div className="flex flex-wrap gap-2 pt-1">
            <Button asChild size="sm" variant="secondary">
              <Link href={`/projects/${projectId}`}>Open the plan</Link>
            </Button>
            <Button asChild size="sm" variant="ghost">
              <Link href="/projects/my-work">My project work</Link>
            </Button>
          </div>
        </Card>

        <aside aria-label="Task pods" className="flex flex-col gap-3">
          <TaskPodsPanel projectId={projectId} taskId={taskId} taskTitle={task.title} initialPod={pod} />
        </aside>
      </div>
    </div>
  );
}
