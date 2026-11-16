// "My work" and the project-shaped Today share this feed: the tasks assigned to
// the signed-in person, across projects, with waiting as its own state
// (S10.1, S10.3, S10.4, S14.1, S14.2, S14.5, S14.13).
//
// Mounting: the Today page (owned by another lane) can render
// <ProjectTodaySection /> as its project block; /projects/my-work is the
// standalone route in this lane.

"use client";

import { Badge, Button, EmptyState, Spinner } from "@marula-core/ui";
import Link from "next/link";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { useRequireAuth } from "../../lib/api";
import { fetchMyWork, type MyTask } from "./delivery-api";

const BLOCKER_WORDS: Record<string, string> = {
  client: "Waiting on the client",
  task: "Waiting on another task",
  vendor: "Waiting on a vendor",
  access: "Waiting for access",
  parts: "Waiting for parts",
  other: "Blocked",
};

function statusWords(task: MyTask): string {
  if (task.blocker_type !== null) return BLOCKER_WORDS[task.blocker_type] ?? "Blocked";
  switch (task.status) {
    case "in_progress":
      return "In progress";
    case "completed":
      return "Done";
    case "cancelled":
      return "Cancelled";
    default:
      return "Not started";
  }
}

function dueWords(task: MyTask): string | null {
  if (task.due_at === null) return task.start_at === null ? "Undated" : null;
  const due = task.due_at.slice(0, 10);
  return `Due ${due}`;
}

export function MyWorkList({
  compact = false,
  hideWhenEmpty = false,
  header = null,
}: {
  compact?: boolean;
  /** The Today block renders nothing at all when the person has no project
   *  work, so a tickets-only day keeps its shape instead of gaining an empty
   *  section. The standalone route keeps its explanatory empty state. */
  hideWhenEmpty?: boolean;
  /** A header drawn above the rows and omitted with them, so the Today block
   *  never leaves a title over nothing. */
  header?: ReactNode;
}) {
  const auth = useRequireAuth();
  const [tasks, setTasks] = useState<MyTask[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      setTasks(await fetchMyWork());
    } catch (e) {
      setError((e as Error).message);
    }
  }, []);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void load();
  }, [auth.status, load]);

  if (auth.status === "loading" || (tasks === null && error === null)) {
    return (
      <div className="flex items-center gap-2 p-6 text-[13px] text-muted-foreground">
        <Spinner /> Reading your work…
      </div>
    );
  }

  if (error !== null) {
    return (
      <div role="alert" className="flex items-center gap-3 p-4 text-[13px]">
        <span className="flex-1">Your work could not be read: {error}</span>
        <Button variant="secondary" onClick={() => void load()}>
          Try again
        </Button>
      </div>
    );
  }

  const rows = tasks ?? [];
  const waiting = rows.filter((task) => task.blocker_type !== null);
  const rest = rows.filter((task) => task.blocker_type === null);

  if (rows.length === 0) {
    if (hideWhenEmpty) return null;
    return (
      <EmptyState message="Nothing assigned to you. When a coordinator offers you project work it appears here with the project, the phase and what it needs." />
    );
  }

  return (
    <div className="flex flex-col gap-4">
      {header}
      {waiting.length > 0 ? (
        <section aria-label="Waiting">
          <h2 className="mb-1 text-[13px] font-semibold">Waiting ({waiting.length})</h2>
          <p className="mb-2 text-[12px] text-muted-foreground">
            These are not yours to start. Each says what it waits on.
          </p>
          <ul className="flex flex-col gap-1">
            {waiting.map((task) => (
              <TaskRow key={task.task_id} task={task} />
            ))}
          </ul>
        </section>
      ) : null}

      <section aria-label="Your work">
        {waiting.length > 0 ? (
          <h2 className="mb-1 text-[13px] font-semibold">Ready for you ({rest.length})</h2>
        ) : null}
        <ul className="flex flex-col gap-1">
          {(compact ? rest.slice(0, 5) : rest).map((task) => (
            <TaskRow key={task.task_id} task={task} />
          ))}
        </ul>
      </section>
    </div>
  );
}

function TaskRow({ task }: { task: MyTask }) {
  const waiting = task.blocker_type !== null;
  return (
    <li className="rounded-ctl border border-border bg-card px-3 py-2">
      <div className="flex flex-wrap items-center gap-2">
        <span
          aria-hidden="true"
          className="inline-block size-[8px] shrink-0 rounded-[2px] border border-foreground"
        />
        <Link
          href={`/projects/${task.project_id}/tasks/${task.task_id}`}
          className="truncate text-[13px] font-medium hover:underline"
        >
          {task.title}
        </Link>
        {task.ticket_id !== null ? <Badge tone="neutral">Ticket</Badge> : null}
        <Badge tone={waiting ? "due" : task.status === "in_progress" ? "success" : "neutral"}>
          {statusWords(task)}
        </Badge>
        <span className="ml-auto flex items-center gap-2 text-[12px] text-muted-foreground">
          {dueWords(task) !== null ? <span className="font-mono tabular-nums">{dueWords(task)}</span> : null}
          {task.effort_remaining_minutes !== null ? (
            <span className="font-mono tabular-nums">
              {Math.round((task.effort_remaining_minutes / 60) * 10) / 10}h left
            </span>
          ) : null}
        </span>
      </div>
      <div className="mt-1 text-[12px] text-muted-foreground">
        {task.project_name}
        {task.phase_name !== null ? ` · ${task.phase_name}` : ""}
        {task.blocker_note !== null ? ` · ${task.blocker_note}` : ""}
      </div>
    </li>
  );
}

/** The Today-page variant: resume, the day, then what needs you. The Today page
 * owns the shell; this is only the project block (S14.1). */
export function ProjectTodaySection() {
  const header = (
    <div className="flex items-center justify-between">
      <h2 className="text-[15px] font-semibold">Project work</h2>
      <Button asChild size="sm" variant="ghost">
        <Link href="/projects/my-work">See all project work</Link>
      </Button>
    </div>
  );
  return (
    <section aria-label="Project work" className="flex flex-col gap-3">
      <MyWorkList compact hideWhenEmpty header={header} />
    </section>
  );
}
