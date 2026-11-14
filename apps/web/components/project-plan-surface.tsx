"use client";
/**
 * ProjectPlanSurface — the Plan tab (surface pack S3).
 *
 * Purpose: the work plan a PM lives in, with every state the API can actually
 * support:
 *
 * - S3.1 populated plan: phase rows with a milestone mark, date range and burn
 *   strip; task rows with work type, critical mark, status, person or the role
 *   still to fill, start, days, effort and dependencies.
 * - S3.2 undated work in its own group, with a count, its hours and one action
 *   to date it. Undated tasks are never drawn as scheduled.
 * - S3.3 inline creation on `A` or "+ Task", committing on Enter and cancelling
 *   on Escape.
 * - S3.6 impact preview on a date change: headline, grouped consequence, one
 *   Apply and one Cancel, then a named Undo that calls the revert endpoint.
 * - S3.7 bulk scale: select rows or a phase, choose a factor, read the projected
 *   hours and finish, then apply.
 * - S3.11 the task inspector, beside the table and above 1024 only.
 * - S3.13 density control in the footer.
 * - S3.17 at 1024 and below the inspector becomes a bottom sheet over the table.
 *
 * Every read is a real endpoint and every write is a real mutation. Where the
 * API has no endpoint (promote to ticket, dependency writes, notify people,
 * cross-project collision checks) the control is disabled or the sentence says
 * so, rather than being faked.
 *
 * Non-purpose: no tab switching, no identity header. The page owns those.
 */
import {
  Button,
  Checkbox,
  DensitySwitch,
  Dialog,
  DialogActions,
  DialogContent,
  FieldHint,
  Input,
  Label,
  Skeleton,
  SkeletonGroup,
  SurfaceFooter,
  applyDensity,
  useDensity,
} from "@marula-core/ui";
import {
  ImpactGroup,
  ImpactNote,
  ImpactPreview,
  ImpactRow,
} from "@marula-core/ui";
import { Icon, Segmented, SegmentedItem } from "@marula-core/ui/kit";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  DEFAULT_CALENDAR,
  addWorkingDays,
  addWorkingMinutes,
  applyMoves,
  captureBaseline,
  planFetch,
  dateInputValue,
  effortOf,
  formatDay,
  formatWeekday,
  hoursLabel,
  isTaskLate,
  loadPeople,
  previewMoves,
  revertMoves,
  scaleSchedule,
  startOfDay,
  taskDays,
  taskEnd,
  todayIso,
  workingDelta,
  type BaselineView,
  type PlanPerson,
  type PlanPhase,
  type PlanTask,
  type RiskResponse,
  type ScheduleApply,
  type ScheduleChange,
  type ScheduleMove,
  type ScheduleRestore,
  type WorkingCalendar,
} from "../lib/projects-plan-types";
import { ProjectPlanInspector } from "./project-plan-inspector";
import {
  ProjectPlanTable,
  type PlanCreateDraft,
  type PlanFilter,
} from "./project-plan-table";

interface ProjectPlanSurfaceProps {
  projectId: string;
  projectDueDate: string | null;
  tasks: readonly PlanTask[];
  phases: readonly PlanPhase[];
  deps: Readonly<Record<string, PlanTask[]>>;
  calendar: WorkingCalendar | null;
  risk: RiskResponse | null;
  baseline: BaselineView | null;
  /** True until the first task/phase read lands, so the plan never shows a
   *  false "empty" while its rows are still arriving. */
  loading: boolean;
  /** Refreshes the page's reads after a write. */
  onChanged: () => Promise<void> | void;
  onAddPhase: () => void;
}

interface DraftPreview {
  task: PlanTask;
  targetDate: string;
  moves: ScheduleMove[];
  change: ScheduleChange;
  pushed: ScheduleChange[];
  phaseEndBefore: string | null;
  phaseEndAfter: string | null;
  finishBefore: string | null;
  finishAfter: string | null;
}

interface UndoRecord {
  label: string;
  restore: ScheduleRestore[];
}

interface ScaleScope {
  label: string;
  taskIds: ReadonlySet<string>;
}

function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(false);
  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const list = window.matchMedia(query);
    setMatches(list.matches);
    const onChange = (event: MediaQueryListEvent): void => setMatches(event.matches);
    list.addEventListener("change", onChange);
    return () => list.removeEventListener("change", onChange);
  }, [query]);
  return matches;
}

function maxIso(values: readonly (string | null)[]): string | null {
  const sorted = values.filter((value): value is string => value !== null && value !== undefined).sort();
  return sorted.at(-1) ?? null;
}

export function ProjectPlanSurface(props: ProjectPlanSurfaceProps): React.JSX.Element {
  const {
    projectId,
    tasks,
    phases,
    deps,
    calendar,
    risk,
    baseline,
    loading,
    onChanged,
    onAddPhase,
  } = props;
  const weekdays = calendar?.weekdays ?? DEFAULT_CALENDAR.weekdays;
  const minutesPerDay = calendar?.minutes_per_day ?? DEFAULT_CALENDAR.minutes_per_day;
  const today = todayIso();

  const wide = useMediaQuery("(min-width: 1280px)");
  const column = useMediaQuery("(min-width: 1025px)");
  const density = useDensity();

  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [focusId, setFocusId] = useState<string | null>(null);
  const [inspectorId, setInspectorId] = useState<string | null>(null);
  const [filter, setFilter] = useState<PlanFilter>("all");
  const [people, setPeople] = useState<ReadonlyMap<string, PlanPerson>>(new Map());
  const [creating, setCreating] = useState<PlanCreateDraft | null>(null);
  const [createError, setCreateError] = useState<string | null>(null);
  const [preview, setPreview] = useState<DraftPreview | null>(null);
  const [undo, setUndo] = useState<UndoRecord | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [scaleOpen, setScaleOpen] = useState(false);
  const [scaleFactor, setScaleFactor] = useState("1.25");
  const [scaleScope, setScaleScope] = useState<ScaleScope | null>(null);
  const [setDatesOpen, setSetDatesOpen] = useState(false);
  const [setDatesValue, setSetDatesValue] = useState("");
  const [baselineOpen, setBaselineOpen] = useState(false);
  const [baselineNote, setBaselineNote] = useState("");
  const [scheduleCritical, setScheduleCritical] = useState<ReadonlySet<string>>(new Set());

  useEffect(() => {
    let cancelled = false;
    void loadPeople().then((rows) => {
      if (!cancelled) setPeople(new Map(rows.map((person) => [person.id, person])));
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // A completed phase starts collapsed; everything else starts open. Re-running
  // when phases arrive only adds the phases nobody has decided about yet, so a
  // deliberate collapse is not reopened by a refresh.
  useEffect(() => {
    setExpanded((current) => {
      const next = new Set(current);
      for (const phase of phases) {
        // A completed phase starts collapsed; a phase someone opened stays open
        // because the toggle put its id in the set.
        if (!current.has(phase.id) && phase.status !== "completed") next.add(phase.id);
      }
      return next;
    });
  }, [phases]);

  const taskById = useMemo(() => new Map(tasks.map((task) => [task.id, task])), [tasks]);
  const phaseById = useMemo(() => new Map(phases.map((phase) => [phase.id, phase])), [phases]);

  const dependents = useMemo(() => {
    const map: Record<string, PlanTask[]> = {};
    for (const task of tasks) {
      for (const blocker of deps[task.id] ?? []) {
        map[blocker.id] = [...(map[blocker.id] ?? []), task];
      }
    }
    return map;
  }, [tasks, deps]);

  const criticalPath = useMemo(() => {
    const next = new Set(scheduleCritical);
    for (const task of tasks) if (task.is_critical === true) next.add(task.id);
    return next;
  }, [scheduleCritical, tasks]);

  const riskTaskIds = useMemo(() => {
    const next = new Set<string>();
    for (const signal of risk?.signals ?? []) {
      if (signal.subject === "task" && signal.severity !== "info") next.add(signal.subject_id);
    }
    return next;
  }, [risk]);

  const counts = useMemo(() => {
    const undated = tasks.filter((task) => task.start_at === null || task.start_at === undefined);
    const unassigned = tasks.filter(
      (task) =>
        (task.assignee_id === null || task.assignee_id === undefined) &&
        (task.role_key === null || task.role_key === undefined || task.role_key.trim() === ""),
    );
    const blocked = tasks.filter((task) => task.status === "blocked");
    const late = tasks.filter((task) =>
      isTaskLate(task, Date.parse(`${today}T23:59:59Z`), weekdays, minutesPerDay),
    );
    return {
      undated: undated.length,
      undatedHours: Math.round(effortOf(undated).estimateMinutes / 60),
      unassigned: unassigned.length,
      blocked: blocked.length,
      risk: new Set([...late.map((task) => task.id), ...riskTaskIds]).size,
    };
  }, [tasks, today, riskTaskIds, weekdays, minutesPerDay]);

  const inspectorTask = inspectorId === null ? null : (taskById.get(inspectorId) ?? null);

  const refresh = useCallback(async () => {
    await onChanged();
  }, [onChanged]);

  const fail = useCallback((message: string) => {
    setError(message);
  }, []);

  // --- selection ------------------------------------------------------------------

  const toggleSelected = useCallback((taskId: string, next: boolean) => {
    setSelected((current) => {
      const copy = new Set(current);
      if (next) copy.add(taskId);
      else copy.delete(taskId);
      return copy;
    });
  }, []);

  const togglePhaseSelected = useCallback(
    (phaseId: string, next: boolean) => {
      setSelected((current) => {
        const copy = new Set(current);
        for (const task of tasks) {
          if (task.phase_id !== phaseId) continue;
          if (next) copy.add(task.id);
          else copy.delete(task.id);
        }
        return copy;
      });
    },
    [tasks],
  );

  const toggleAll = useCallback(
    (next: boolean) => {
      setSelected(next ? new Set(tasks.map((task) => task.id)) : new Set());
    },
    [tasks],
  );

  const togglePhase = useCallback((phaseId: string) => {
    setExpanded((current) => {
      const copy = new Set(current);
      if (copy.has(phaseId)) copy.delete(phaseId);
      else copy.add(phaseId);
      return copy;
    });
  }, []);

  // --- inline creation (S3.3) ------------------------------------------------------

  const startCreate = useCallback(
    (phaseId: string | null) => {
      const phase = phaseId === null ? null : (phaseById.get(phaseId) ?? null);
      setCreating({
        phaseId,
        title: "",
        role: "",
        days: "1",
        effortHours: "",
        suggestionHours: minutesPerDay / 60,
      });
      setCreateError(null);
      if (phase !== null) setExpanded((current) => new Set(current).add(phase.id));
    },
    [minutesPerDay, phaseById],
  );

  const changeDraft = useCallback(
    (patch: Partial<PlanCreateDraft>) => {
      setCreating((current) => {
        if (current === null) return current;
        const next = { ...current, ...patch };
        const days = Number.parseFloat(next.days);
        const perDay = minutesPerDay / 60;
        next.suggestionHours = Number.isFinite(days) && days > 0 ? Math.round(days * perDay * 10) / 10 : perDay;
        return next;
      });
    },
    [minutesPerDay],
  );

  const commitCreate = useCallback(async () => {
    if (creating === null || creating.title.trim() === "") return;
    setPending(true);
    setCreateError(null);
    const phase = creating.phaseId === null ? null : (phaseById.get(creating.phaseId) ?? null);
    const days = Number.parseFloat(creating.days);
    const hours = Number.parseFloat(creating.effortHours);
    const effortHours = Number.isFinite(hours) && hours > 0 ? hours : creating.suggestionHours;
    const dated = phase !== null && phase.start_date !== null && phase.start_date !== undefined;
    const body: Record<string, unknown> = {
      title: creating.title.trim(),
      phase_id: creating.phaseId,
      role_key: creating.role.trim() === "" ? null : creating.role.trim(),
      effort_estimate_minutes: Math.round(effortHours * 60),
      effort_remaining_minutes: Math.round(effortHours * 60),
      is_milestone: false,
      required_skills: [],
      is_critical: false,
      depends_on: [],
      idempotency_key: crypto.randomUUID(),
    };
    if (dated) {
      const durationDays = Number.isFinite(days) && days > 0 ? days : 1;
      body.start_at = startOfDay(phase.start_date as string);
      body.duration_minutes = Math.round(durationDays * minutesPerDay);
    }
    try {
      await planFetch<{ id: string }>(`/api/v1/projects/${projectId}/tasks`, {
        method: "POST",
        body: JSON.stringify(body),
      });
      setCreating(null);
      setNotice(`Added ${body.title as string}.`);
      await refresh();
    } catch (cause) {
      setCreateError(cause instanceof Error ? cause.message : "The task was not added.");
    } finally {
      setPending(false);
    }
  }, [creating, minutesPerDay, phaseById, projectId, refresh]);

  // --- impact preview, apply and undo (S3.6 / S3.6c) -------------------------------

  const requestMove = useCallback(
    async (task: PlanTask, dateValue: string) => {
      setPending(true);
      setError(null);
      const duration = task.duration_minutes ?? Math.max(minutesPerDay, Math.round((task.effort_estimate_minutes ?? minutesPerDay)));
      const moves: ScheduleMove[] = [
        { task_id: task.id, start_at: startOfDay(dateValue), duration_minutes: duration },
      ];
      try {
        const data = await previewMoves(projectId, moves);
        const change = data.changes.find((row) => row.task_id === task.id && row.reason === "moved");
        if (change === undefined) {
          setNotice("The plan is unchanged: the engine placed the task on the same day.");
          return;
        }
        const pushed = data.changes.filter((row) => row.reason === "pushed");
        const phaseTasks = tasks.filter((item) => item.phase_id === task.phase_id);
        const phaseBefore = maxIso(phaseTasks.map((item) => taskEnd(item, weekdays, minutesPerDay)));
        const phaseAfter = maxIso(
          phaseTasks.map((item) => {
            if (item.id === task.id) {
              return new Date(Date.parse(startOfDay(dateValue)) + duration * 60_000).toISOString();
            }
            const row = data.changes.find((entry) => entry.task_id === item.id);
            if (row !== undefined) {
              if (row.to_start === null) return null;
              return new Date(
                Date.parse(row.to_start) + (item.duration_minutes ?? 0) * 60_000,
              ).toISOString();
            }
            return taskEnd(item, weekdays, minutesPerDay);
          }),
        );
        const finishBefore = maxIso(tasks.map((item) => taskEnd(item, weekdays, minutesPerDay)));
        const finishAfter = maxIso(
          tasks.map((item) => {
            if (item.id === task.id) {
              return new Date(Date.parse(startOfDay(dateValue)) + duration * 60_000).toISOString();
            }
            const row = data.changes.find((entry) => entry.task_id === item.id);
            if (row !== undefined) {
              if (row.to_start === null) return null;
              return new Date(
                Date.parse(row.to_start) + (item.duration_minutes ?? 0) * 60_000,
              ).toISOString();
            }
            return taskEnd(item, weekdays, minutesPerDay);
          }),
        );
        setScheduleCritical(new Set(data.critical_path));
        setPreview({
          task,
          targetDate: dateValue,
          moves,
          change,
          pushed,
          phaseEndBefore: phaseBefore,
          phaseEndAfter: phaseAfter,
          finishBefore,
          finishAfter,
        });
      } catch (cause) {
        fail(cause instanceof Error ? cause.message : "The preview could not be computed.");
      } finally {
        setPending(false);
      }
    },
    [fail, minutesPerDay, projectId, tasks],
  );

  const applyPreview = useCallback(async () => {
    if (preview === null) return;
    setPending(true);
    try {
      const result: ScheduleApply = await applyMoves(projectId, preview.moves);
      setScheduleCritical(new Set(result.critical_path));
      const pushed = result.changes.filter((row) => row.reason === "pushed").length;
      setUndo({
        label:
          pushed === 0
            ? `moved ${preview.task.title} to ${formatDay(preview.targetDate)}`
            : `moved ${preview.task.title} and ${pushed} later ${pushed === 1 ? "task" : "tasks"}`,
        restore: result.revert,
      });
      setPreview(null);
      setNotice(null);
      await refresh();
    } catch (cause) {
      fail(cause instanceof Error ? cause.message : "The change was not applied.");
    } finally {
      setPending(false);
    }
  }, [fail, preview, projectId, refresh]);

  const runUndo = useCallback(async () => {
    if (undo === null) return;
    setPending(true);
    try {
      await revertMoves(projectId, undo.restore);
      setUndo(null);
      setNotice("Reverted the last schedule change.");
      await refresh();
    } catch (cause) {
      fail(cause instanceof Error ? cause.message : "The revert was refused.");
    } finally {
      setPending(false);
    }
  }, [fail, projectId, refresh, undo]);

  // --- scale (S3.7) -----------------------------------------------------------------

  const openScale = useCallback(
    (scope: ScaleScope | null) => {
      setScaleScope(scope);
      setScaleOpen(true);
      setScaleFactor("1.25");
    },
    [],
  );

  const scalePreview = useMemo(() => {
    const factor = Number.parseFloat(scaleFactor);
    const safeFactor = Number.isFinite(factor) && factor > 0 ? factor : 1;
    const scopeTasks =
      scaleScope === null ? tasks : tasks.filter((task) => scaleScope.taskIds.has(task.id));
    const datedScope = scopeTasks.filter(
      (task) => task.start_at !== null && task.start_at !== undefined,
    );
    const startDates = datedScope.map((task) => dateInputValue(task.start_at));
    const from = startDates.sort()[0] ?? null;
    const to = [...startDates].sort().at(-1) ?? null;
    // The scale endpoint applies to tasks whose START sits inside the range,
    // so the preview counts exactly those rather than pretending the
    // selection is a filter the endpoint does not have.
    const affected =
      from === null || to === null
        ? []
        : tasks.filter((task) => {
            if (task.start_at === null || task.start_at === undefined) return false;
            const day = dateInputValue(task.start_at);
            return day >= from && day <= to;
          });
    const before = effortOf(affected);
    const finishBefore = maxIso(tasks.map((task) => taskEnd(task, weekdays, minutesPerDay)));
    const finishAfter = maxIso([
      finishBefore,
      ...affected.map((task) =>
        addWorkingMinutes(
          task.start_at as string,
          Math.round((task.duration_minutes ?? 0) * safeFactor),
          weekdays,
          minutesPerDay,
        ),
      ),
    ]);
    return {
      factor: safeFactor,
      count: affected.length,
      hoursBefore: Math.round(before.estimateMinutes / 60),
      hoursAfter: Math.round((before.estimateMinutes * safeFactor) / 60),
      finishBefore,
      finishAfter,
      from: from === null ? null : startOfDay(from),
      to: to === null ? null : startOfDay(to),
      fromDay: from,
      toDay: to,
    };
  }, [scaleFactor, scaleScope, tasks, weekdays, minutesPerDay]);

  const applyScale = useCallback(async () => {
    setPending(true);
    try {
      const result = await scaleSchedule(projectId, {
        factor: scalePreview.factor,
        from_start_at: scalePreview.from,
        // The endpoint filters on the start instant, so the last day is sent to
        // its end: a task starting at 09:00 is still "in range" on its own day.
        to_start_at:
          scalePreview.toDay === null ? null : `${scalePreview.toDay}T23:59:59.999Z`,
        reschedule: true,
      });
      setScheduleCritical(new Set(result.critical_path));
      setScaleOpen(false);
      setSelected(new Set());
      setNotice(
        `Scaled ${result.changes.length} ${result.changes.length === 1 ? "task" : "tasks"} by ${scalePreview.factor}. This scale cannot be undone from this panel; move the dates back to reverse it.`,
      );
      await refresh();
    } catch (cause) {
      fail(cause instanceof Error ? cause.message : "The scale was refused.");
    } finally {
      setPending(false);
    }
  }, [fail, projectId, refresh, scalePreview]);

  // --- undated work: set dates (S3.2) ------------------------------------------------

  const undatedTasks = useMemo(
    () =>
      tasks
        .filter((task) => task.start_at === null || task.start_at === undefined)
        .sort((a, b) => a.sort_order - b.sort_order),
    [tasks],
  );

  const applySetDates = useCallback(async () => {
    if (setDatesValue === "" || undatedTasks.length === 0) return;
    setPending(true);
    try {
      const moves: ScheduleMove[] = [];
      let cursor = setDatesValue;
      for (const task of undatedTasks) {
        const hours = task.effort_estimate_minutes ?? minutesPerDay;
        const duration = Math.max(30, Math.round(hours));
        moves.push({ task_id: task.id, start_at: startOfDay(cursor), duration_minutes: duration });
        cursor = addWorkingDays(cursor, Math.max(1, Math.ceil(duration / minutesPerDay)), weekdays);
      }
      const result = await applyMoves(projectId, moves);
      setUndo({ label: `dated ${undatedTasks.length} tasks from ${formatDay(setDatesValue)}`, restore: result.revert });
      setSetDatesOpen(false);
      setSetDatesValue("");
      await refresh();
    } catch (cause) {
      fail(cause instanceof Error ? cause.message : "The dates were not set.");
    } finally {
      setPending(false);
    }
  }, [fail, minutesPerDay, projectId, refresh, setDatesValue, undatedTasks, weekdays]);

  // --- baseline ---------------------------------------------------------------------

  const saveBaseline = useCallback(async () => {
    setPending(true);
    try {
      await captureBaseline(projectId, baselineNote);
      setBaselineOpen(false);
      setBaselineNote("");
      setNotice("Baseline captured from the plan as it stands.");
      await refresh();
    } catch (cause) {
      fail(cause instanceof Error ? cause.message : "The baseline was not captured.");
    } finally {
      setPending(false);
    }
  }, [baselineNote, fail, projectId, refresh]);

  // --- keyboard ---------------------------------------------------------------------

  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (event.defaultPrevented) return;
      if (event.key === "Escape") {
        if (preview !== null) {
          setPreview(null);
          return;
        }
        if (creating !== null) {
          setCreating(null);
          setCreateError(null);
          return;
        }
        if (inspectorId !== null) setInspectorId(null);
        return;
      }
      const target = event.target as HTMLElement | null;
      const typing =
        target !== null &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.tagName === "SELECT" ||
          target.isContentEditable);
      if (typing || scaleOpen || setDatesOpen || baselineOpen) return;
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      if (event.key.toLowerCase() === "a" && creating === null) {
        event.preventDefault();
        startCreate(inspectorTask === null ? null : (inspectorTask.phase_id ?? null));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [preview, creating, inspectorId, startCreate, inspectorTask, scaleOpen, setDatesOpen, baselineOpen]);

  // --- render -----------------------------------------------------------------------

  const hasPlan = tasks.length > 0 || phases.length > 0;
  const savingText = pending ? "Saving…" : error === null ? "Saved" : "Not saved";

  return (
    <div className="relative flex min-h-0 flex-col" data-testid="project-schedule">
      {error !== null ? (
        <div
          role="alert"
          className="mb-3 flex items-start gap-2 rounded-[12px] px-3 py-2 text-[13px]"
          style={{ background: "var(--m-status-danger-bg)", color: "var(--m-status-danger-fg)" }}
        >
          <Icon name="alert" size={15} />
          <span className="min-w-0 flex-1">{error}</span>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setError(null)}
          >
            Dismiss
          </Button>
        </div>
      ) : null}

      {undo !== null ? (
        <div className="mb-3 flex flex-wrap items-center gap-2 rounded-[12px] border border-dashed border-input bg-card px-3 py-2 text-[13px]">
          <Icon name="undo" size={15} />
          <span className="min-w-0 flex-1">
            Applied: <b className="font-semibold">{undo.label}</b>. Every change is one audit entry.
          </span>
          <Button size="sm" variant="secondary" onClick={() => void runUndo()} disabled={pending}>
            Undo
          </Button>
          <Button size="sm" variant="ghost" aria-label="Dismiss undo" className="size-8 px-0" onClick={() => setUndo(null)}>
            <Icon name="x" size={16} />
          </Button>
        </div>
      ) : null}

      {notice !== null && undo === null ? (
        <div className="mb-3 flex items-start gap-2 rounded-[12px] border border-border bg-card px-3 py-2 text-[13px] text-muted-foreground">
          <Icon name="check" size={15} />
          <span className="min-w-0 flex-1">{notice}</span>
          <Button size="sm" variant="ghost" aria-label="Dismiss notice" className="size-8 px-0" onClick={() => setNotice(null)}>
            <Icon name="x" size={16} />
          </Button>
        </div>
      ) : null}

      {selected.size > 0 ? (
        <div className="mb-2 flex flex-wrap items-center gap-2 rounded-[12px] bg-mc-act px-3 py-2 text-[13px] text-[var(--m-on-accent)]">
          <b className="font-semibold">
            {selected.size} {selected.size === 1 ? "task" : "tasks"} selected
          </b>
          <span className="flex-1" />
          <Button
            size="sm"
            variant="secondary"
            onClick={() => openScale({ label: `${selected.size} selected tasks`, taskIds: new Set(selected) })}
          >
            <Icon name="rows-4" size={14} />
            Scale selection
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setSelected(new Set())}>
            Clear
          </Button>
        </div>
      ) : null}

      <div className="flex min-h-0 items-start gap-4">
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex min-h-[56px] flex-wrap items-center gap-2 py-3">
            <Segmented
              type="single"
              value="plan"
              onValueChange={() => undefined}
              aria-label="Plan views"
            >
              <SegmentedItem value="plan" size="sm">
                Plan
              </SegmentedItem>
              <SegmentedItem
                value="board"
                disabled
                size="sm"
                title="The board view needs the project workflow map, which is not in this build yet."
              >
                Board
              </SegmentedItem>
            </Segmented>

            <FilterChip
              label="Unassigned"
              count={counts.unassigned}
              active={filter === "unassigned"}
              onClick={() => setFilter(filter === "unassigned" ? "all" : "unassigned")}
            />
            <FilterChip
              label="Late or at risk"
              count={counts.risk}
              active={filter === "risk"}
              onClick={() => setFilter(filter === "risk" ? "all" : "risk")}
            />
            <FilterChip
              label="Blocked"
              count={counts.blocked}
              active={filter === "blocked"}
              onClick={() => setFilter(filter === "blocked" ? "all" : "blocked")}
              className="hidden xl:inline-flex"
            />
            <FilterChip
              label="Critical path"
              icon="link"
              active={filter === "critical"}
              onClick={() => setFilter(filter === "critical" ? "all" : "critical")}
              className="hidden xl:inline-flex"
            />

            <span className="flex-1" />
            <span className="inline-flex items-center gap-1.5 whitespace-nowrap text-[12px] text-muted-foreground" aria-live="polite">
              {savingText === "Saved" ? <Icon name="check" size={13} /> : null}
              {savingText}
            </span>
            <Button size="sm" variant="secondary" onClick={() => openScale(null)}>
              <Icon name="rows-4" size={14} />
              Scale
            </Button>
            <Button size="sm" variant="secondary" onClick={() => setBaselineOpen(true)}>
              <Icon name="layers" size={14} />
              {baseline === null ? "Baseline" : `Baseline ${formatDay(baseline.captured_at)}`}
            </Button>
            <Button size="sm" variant="secondary" onClick={onAddPhase}>
              <Icon name="plus" size={14} />
              Phase
            </Button>
            <Button size="sm" variant="primary" onClick={() => startCreate(null)} data-testid="plan-add-task">
              <Icon name="plus" size={14} />
              Task
              <span className="ml-1 rounded-[4px] border border-current px-1 font-mono text-[11px]">A</span>
            </Button>
          </div>

          {loading && !hasPlan ? (
            <div className="rounded-card border border-border bg-card p-4" data-testid="plan-loading">
              <SkeletonGroup label="Loading the plan">
                <div className="grid gap-2">
                  <Skeleton shape="line" width="42%" />
                  <Skeleton shape="block" />
                  <Skeleton shape="block" />
                  <Skeleton shape="line" width="28%" />
                  <Skeleton shape="block" />
                  <Skeleton shape="block" />
                </div>
              </SkeletonGroup>
            </div>
          ) : !hasPlan ? (
            <div className="rounded-card border border-border bg-card px-6 py-10 text-center">
              <p className="m-0 text-[15px] font-semibold">This plan is empty.</p>
              <p className="mx-auto mt-1 max-w-[52ch] text-[13px] text-muted-foreground">
                A plan is phases, tasks and the dates they hold. Add a phase, or start with a task and give
                it a phase and dates later.
              </p>
              <div className="mt-4 flex justify-center gap-2">
                <Button variant="secondary" onClick={onAddPhase}>
                  <Icon name="plus" size={14} />
                  Phase
                </Button>
                <Button variant="primary" onClick={() => startCreate(null)}>
                  <Icon name="plus" size={14} />
                  Task
                </Button>
              </div>
            </div>
          ) : (
            <ProjectPlanTable
              phases={phases}
              tasks={tasks}
              people={people}
              deps={deps}
              criticalPath={criticalPath}
              riskTaskIds={riskTaskIds}
              filter={filter}
              expanded={expanded}
              selected={selected}
              focusId={focusId}
              creating={creating}
              createError={createError}
              busy={pending}
              narrow={!wide}
              weekdays={weekdays}
              minutesPerDay={minutesPerDay}
              today={today}
              onTogglePhase={togglePhase}
              onToggleSelected={toggleSelected}
              onTogglePhaseSelected={togglePhaseSelected}
              onToggleAll={toggleAll}
              onFocusRow={setFocusId}
              onOpenTask={(task) => setInspectorId(task.id)}
              onAddTask={startCreate}
              onScalePhase={(phase) =>
                openScale({
                  label: phase.name,
                  taskIds: new Set(tasks.filter((task) => task.phase_id === phase.id).map((task) => task.id)),
                })
              }
              onSetDates={() => setSetDatesOpen(true)}
              onCreatingChange={changeDraft}
              onCreateCommit={() => void commitCreate()}
              onCreateCancel={() => {
                setCreating(null);
                setCreateError(null);
              }}
            />
          )}

          <SurfaceFooter
            shown={tasks.length}
            total={tasks.length}
            noun="task"
            density={
              <DensitySwitch
                value={density}
                onChange={(step) => {
                  applyDensity(step);
                }}
                variant="icons"
              />
            }
            hints={[
              { keys: "A", label: "add a task" },
              { keys: "Enter", label: "open a row" },
              { keys: "Esc", label: "cancel" },
            ]}
          >
            <span className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[12px] tabular-nums">
              <span data-testid="rollup-phases">
                {phases.filter((phase) => phase.status === "completed").length}/ {phases.length} phases
              </span>
              <span data-testid="rollup-tasks">
                {tasks.filter((task) => task.status === "completed").length}/ {tasks.length} tasks
              </span>
              <span data-testid="rollup-milestones">
                {tasks.filter((task) => task.status === "completed" && task.is_milestone).length}/{" "}
                {tasks.filter((task) => task.is_milestone).length +
                  phases.filter((phase) => phase.is_milestone).length}{" "}
                milestones
              </span>
            </span>
          </SurfaceFooter>
        </div>

        {inspectorTask !== null && column ? (
          <ProjectPlanInspector
            task={inspectorTask}
            people={people}
            deps={deps}
            dependents={dependents}
            minutesPerDay={minutesPerDay}
            weekdays={weekdays}
            today={today}
            narrow={false}
            onClose={() => setInspectorId(null)}
            onStartDateChange={(task, dateValue) => void requestMove(task, dateValue)}
            onAddDependency={() => undefined}
            onPromote={() => undefined}
          />
        ) : null}
      </div>

      {inspectorTask !== null && !column ? (
        <ProjectPlanInspector
          task={inspectorTask}
          people={people}
          deps={deps}
          dependents={dependents}
          minutesPerDay={minutesPerDay}
          weekdays={weekdays}
          today={today}
          narrow
          onClose={() => setInspectorId(null)}
          onStartDateChange={(task, dateValue) => void requestMove(task, dateValue)}
          onAddDependency={() => undefined}
          onPromote={() => undefined}
        />
      ) : null}

      {preview !== null ? (
        <ImpactPreview
          title={`${preview.task.title} moves from ${formatDay(preview.change.from_start)} to ${formatDay(preview.change.to_start)}`}
          close={
            <Button variant="ghost" size="sm" aria-label="Close impact preview" className="size-8 px-0" onClick={() => setPreview(null)}>
              <Icon name="x" size={18} />
            </Button>
          }
          footer={
            <>
              <Button size="sm" variant="secondary" onClick={() => setPreview(null)} disabled={pending}>
                Cancel
                <span className="ml-1 rounded-[4px] border border-current px-1 font-mono text-[11px]">Esc</span>
              </Button>
              <span className="flex-1" />
              <Button size="sm" variant="primary" onClick={() => void applyPreview()} disabled={pending}>
                {pending ? "Applying…" : "Apply"}
              </Button>
            </>
          }
        >
          <ImpactHeadline preview={preview} weekdays={weekdays} dueDate={props.projectDueDate} />
          <ImpactGroup
            icon={<Icon name="rows-3" size={15} />}
            title="Moves the phase"
            note={
              preview.task.phase_id === null || preview.task.phase_id === undefined
                ? "No phase"
                : `${phaseById.get(preview.task.phase_id)?.name ?? "Phase"} now ends ${formatDay(preview.phaseEndAfter)}`
            }
          >
            <ImpactRow
              label={preview.task.title}
              was={formatDay(preview.change.from_start)}
              now={formatDay(preview.change.to_start)}
              delta={workingDelta(preview.change.from_start, preview.change.to_start, weekdays)}
            />
          </ImpactGroup>
          {preview.pushed.length > 0 ? (
            <ImpactGroup
              icon={<Icon name="flag" size={15} />}
              title="Moves the project finish"
              note={`${preview.pushed.length} ${preview.pushed.length === 1 ? "task" : "tasks"} downstream`}
            >
              {preview.pushed.slice(0, 6).map((change) => (
                <ImpactRow
                  key={change.task_id}
                  label={taskById.get(change.task_id)?.title ?? "Task"}
                  was={formatDay(change.from_start)}
                  now={formatDay(change.to_start)}
                  delta={workingDelta(change.from_start, change.to_start, weekdays)}
                />
              ))}
              {preview.pushed.length > 6 ? (
                <ImpactRow
                  label={`and ${preview.pushed.length - 6} more`}
                  was=""
                  now=""
                  delta=""
                />
              ) : null}
            </ImpactGroup>
          ) : null}
          <ImpactNote>
            <Icon name="check" size={14} className="mt-[3px] shrink-0" />
            <span>
              {Math.max(0, tasks.filter((task) => task.start_at !== null && task.start_at !== undefined).length - preview.pushed.length - 1)}{" "}
              tasks keep their dates. Cross-project collisions are not checked by this preview; the
              dependency engine is this project&rsquo;s own.
            </span>
          </ImpactNote>
          {scheduleCritical.size > 0 ? (
            <ImpactNote>
              <Icon name="link" size={14} className="mt-[3px] shrink-0" />
              <span>
                {scheduleCritical.size} tasks are on the critical path
                {scheduleCritical.has(preview.task.id) ? ", including this one." : "; this task is not on it."}
              </span>
            </ImpactNote>
          ) : null}
        </ImpactPreview>
      ) : null}

      <Dialog open={scaleOpen} onOpenChange={setScaleOpen}>
        <DialogContent
          title="Scale the plan"
          description="Scales the length and effort of every dated task in the scope. Nothing is written until you apply."
        >
          <div className="grid gap-3 p-4">
            <p className="m-0 text-[13px] text-muted-foreground">
              Scope: <b className="font-semibold text-foreground">{scaleScope === null ? "every dated task" : scaleScope.label}</b>
              . The scale applies to dated tasks that start between{" "}
              <span className="font-mono tabular-nums text-foreground">
                {formatDay(scalePreview.fromDay)} and {formatDay(scalePreview.toDay)}
              </span>
              , which is {scalePreview.count} {scalePreview.count === 1 ? "task" : "tasks"}.
            </p>
            <div className="flex flex-wrap items-center gap-2">
              <Label htmlFor="scale-factor">Factor</Label>
              {["0.75", "1.25", "1.5", "2"].map((value) => (
                <button
                  key={value}
                  type="button"
                  onClick={() => setScaleFactor(value)}
                  aria-pressed={scaleFactor === value}
                  className={
                    scaleFactor === value
                      ? "h-[32px] rounded-pill border border-border bg-mc-act px-3 text-[13px] font-semibold text-[var(--m-on-accent)]"
                      : "h-[32px] rounded-pill border border-border bg-card px-3 text-[13px] text-muted-foreground"
                  }
                >
                  ×{value}
                </button>
              ))}
              <Input
                id="scale-factor"
                inputMode="decimal"
                value={scaleFactor}
                onChange={(event) => setScaleFactor(event.target.value)}
                className="h-8 w-[86px] font-mono tabular-nums"
              />
            </div>
            <dl className="m-0 grid grid-cols-[140px_minmax(0,1fr)] gap-x-3 gap-y-1.5 text-[13px]">
              <dt className="text-muted-foreground">Dated tasks</dt>
              <dd className="m-0 font-mono tabular-nums">{scalePreview.count}</dd>
              <dt className="text-muted-foreground">Effort</dt>
              <dd className="m-0 font-mono tabular-nums">
                {scalePreview.hoursBefore} h → {scalePreview.hoursAfter} h
              </dd>
              <dt className="text-muted-foreground">Finish</dt>
              <dd className="m-0 font-mono tabular-nums">
                {formatDay(scalePreview.finishBefore)} → {formatDay(scalePreview.finishAfter)}
              </dd>
            </dl>
            <FieldHint>
              Scaling writes when you apply and has no undo of its own. Move the dates back to reverse it.
            </FieldHint>
            <DialogActions>
              <Button variant="ghost" onClick={() => setScaleOpen(false)} disabled={pending}>
                Cancel
              </Button>
              <Button variant="primary" onClick={() => void applyScale()} disabled={pending || scalePreview.count === 0}>
                {pending ? "Scaling…" : `Scale by ${scaleFactor}`}
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog open={setDatesOpen} onOpenChange={setSetDatesOpen}>
        <DialogContent
          title="Set dates for undated work"
          description="Each task starts on its own working day, in plan order, and is scheduled through the same engine as any other move."
        >
          <div className="grid gap-3 p-4">
            <div>
              <Label htmlFor="undated-start">First working day</Label>
              <Input
                id="undated-start"
                type="date"
                value={setDatesValue}
                onChange={(event) => setSetDatesValue(event.target.value)}
                className="h-9 w-[190px] font-mono tabular-nums"
              />
              <FieldHint>
                {undatedTasks.length} tasks, {hoursLabel(effortOf(undatedTasks).estimateMinutes)} of effort.
                Durations come from each task&rsquo;s effort on an {Math.round(minutesPerDay / 60)}-hour day.
              </FieldHint>
            </div>
            <DialogActions>
              <Button variant="ghost" onClick={() => setSetDatesOpen(false)} disabled={pending}>
                Cancel
              </Button>
              <Button variant="primary" onClick={() => void applySetDates()} disabled={pending || setDatesValue === ""}>
                {pending ? "Scheduling…" : "Set dates"}
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog open={baselineOpen} onOpenChange={setBaselineOpen}>
        <DialogContent
          title="Capture baseline"
          description="Copies every dated task's start and length into its baseline columns, so a later slip has something to be late against."
        >
          <div className="grid gap-3 p-4">
            {baseline === null ? (
              <p className="m-0 text-[13px] text-muted-foreground">
                This project has no baseline yet.
              </p>
            ) : (
              <p className="m-0 text-[13px] text-muted-foreground">
                The last baseline was captured {formatWeekday(baseline.captured_at)} with{" "}
                {baseline.dated_tasks} of {baseline.total_tasks} tasks dated.
              </p>
            )}
            <div>
              <Label htmlFor="baseline-note">Note (optional)</Label>
              <Input
                id="baseline-note"
                value={baselineNote}
                onChange={(event) => setBaselineNote(event.target.value)}
                placeholder="Approved at kickoff"
              />
            </div>
            <DialogActions>
              <Button variant="ghost" onClick={() => setBaselineOpen(false)} disabled={pending}>
                Cancel
              </Button>
              <Button variant="primary" onClick={() => void saveBaseline()} disabled={pending}>
                {pending ? "Capturing…" : "Capture baseline"}
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}

/** The one sentence that says the thing that matters (S3.6). */
function ImpactHeadline({
  preview,
  weekdays,
  dueDate,
}: {
  preview: DraftPreview;
  weekdays: number;
  dueDate: string | null;
}) {
  const pushed = preview.pushed.length;
  const delta = workingDelta(preview.finishBefore, preview.finishAfter, weekdays);
  const deltaWords =
    delta === "same day"
      ? "stays"
      : delta.startsWith("-")
        ? `moves ${delta.slice(1)} earlier`
        : `moves ${delta} later`;
  const late =
    dueDate !== null &&
    preview.finishAfter !== null &&
    dateInputValue(preview.finishAfter) > dateInputValue(dueDate);
  const finish = formatWeekday(preview.finishAfter);
  const detail =
    pushed === 0
      ? delta === "same day"
        ? `The project finish stays ${finish}. No later task waits on this one.`
        : `The project finish ${deltaWords}, to ${finish}. No later task waits on this one.`
      : `The project finish ${deltaWords}, to ${finish}, and ${pushed} later ${pushed === 1 ? "task moves" : "tasks move"} with it.`;
  return (
    <div
      className="rounded-[12px] px-3 py-[10px] text-[13px] leading-[1.5]"
      style={{
        background: late ? "var(--m-status-danger-bg)" : "var(--m-status-warning-bg)",
        color: late ? "var(--m-status-danger-fg)" : "var(--m-status-warning-fg)",
      }}
    >
      <b className="font-bold">
        {pushed === 0 ? "Nothing else moves. " : late ? "This passes the promised date. " : "The plan moves. "}
      </b>
      {detail}
    </div>
  );
}

function FilterChip({
  label,
  count,
  active,
  icon,
  onClick,
  className,
}: {
  label: string;
  count?: number;
  active: boolean;
  icon?: "link";
  onClick: () => void;
  className?: string;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={`${
        active
          ? "inline-flex h-[32px] items-center gap-1.5 rounded-pill border border-border bg-mc-act px-3 text-[13px] font-semibold text-[var(--m-on-accent)]"
          : "inline-flex h-[32px] items-center gap-1.5 rounded-pill border border-border bg-card px-3 text-[13px] text-muted-foreground hover:text-foreground"
      } ${className ?? ""}`}
    >
      {icon === "link" ? <Icon name="link" size={14} /> : null}
      {label}
      {count === undefined ? null : (
        <span className="font-mono text-[12px] tabular-nums">{count}</span>
      )}
    </button>
  );
}
