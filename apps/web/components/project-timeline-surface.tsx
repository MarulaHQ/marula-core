"use client";
/**
 * ProjectTimelineSurface — the data and the controls behind the timeline.
 *
 * Purpose: own everything the chart must not: the reads (project, phases,
 * tasks, calendar, baseline, dependency edges, phase burn, the computed
 * critical path), the two view modes, the preview lifecycle and the set-dates
 * form. The chart stays a pure drawing that asks for a move and reports one.
 *
 * The preview is the server's, not the client's: a drop asks
 * `/schedule/preview`, the answer is drawn as dashed ghosts plus outlined bars,
 * and only Apply writes through `/schedule/apply`. Nothing here invents a date.
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import Link from "next/link";
import {
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  EmptyState,
  FieldHint,
  Input,
  Label,
  Spinner,
} from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import {
  GanttChart,
  GanttTable,
  type GanttMoveRequest,
  type GanttPhase,
  type GanttTask,
} from "@marula-core/ui";
import {
  addWorkingMinutes,
  addDays,
  dayKey,
  daysBetween,
  durationInWords,
  formatDayWords,
  parseDay,
  startOfWeek,
  weekAlignedWindow,
  workingDayOffset,
  workingDaysBetween,
  type GanttCalendar,
  type GanttPlanChange,
} from "@marula-core/ui";
import {
  applySchedule,
  fetchAssigneeNames,
  fetchBaseline,
  fetchCalendar,
  fetchCalendarZones,
  fetchDependencyEdges,
  fetchPhaseBurn,
  fetchProjectPlan,
  patchTaskDates,
  previewSchedule,
  type BaselineWire,
  type CalendarWire,
  type CalendarZoneWire,
  type ScheduleMoveWire,
  type SchedulePlanWire,
  type TimelinePhase,
  type TimelineProject,
  type TimelineTaskWire,
} from "../lib/projects-timeline-types";
import { ProjectTimelineShell } from "./project-timeline-shell";

type Scale = "day" | "week" | "month";
type ViewMode = "chart" | "table";

const SCALE_LABEL: Record<Scale, string> = {
  day: "Day",
  week: "Week",
  month: "Month",
};

export function ProjectTimelineSurface({ projectId }: { projectId: string }): React.JSX.Element {
  const [project, setProject] = useState<TimelineProject | null>(null);
  const [phases, setPhases] = useState<TimelinePhase[]>([]);
  const [tasks, setTasks] = useState<TimelineTaskWire[]>([]);
  const [edges, setEdges] = useState<{ task_id: string; depends_on_task_id: string }[]>([]);
  const [calendar, setCalendar] = useState<CalendarWire | null>(null);
  const [baseline, setBaseline] = useState<BaselineWire | null>(null);
  const [zones, setZones] = useState<CalendarZoneWire[]>([]);
  const [assignees, setAssignees] = useState<Map<string, string>>(new Map());
  const [burn, setBurn] = useState<Map<string, { consumed: number; alert: string }>>(new Map());
  const [computedCritical, setComputedCritical] = useState<string[]>([]);

  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const [view, setView] = useState<ViewMode>("chart");
  const [scale, setScale] = useState<Scale>("week");
  const [fitWindow, setFitWindow] = useState<{ from: string; to: string } | null>(null);
  const [showBaseline, setShowBaseline] = useState(false);
  const [showCritical, setShowCritical] = useState(false);
  const [showTray, setShowTray] = useState(true);
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
  const [today, setToday] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const [preview, setPreview] = useState<SchedulePlanWire | null>(null);
  const [pendingMoves, setPendingMoves] = useState<ScheduleMoveWire[]>([]);
  const [setDatesTask, setSetDatesTask] = useState<TimelineTaskWire | null>(null);

  useEffect(() => {
    if (preview === null) return;
    const onKey = (event: KeyboardEvent): void => {
      if (event.key !== "Escape") return;
      setPreview(null);
      setPendingMoves([]);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [preview]);

  useEffect(() => {
    // The today rule is a clock read: it must happen after hydration, or the
    // server and the client disagree about which day is today.
    setToday(dayKey(Date.now()));
  }, []);

  const load = useCallback(
    async (signal?: AbortSignal) => {
      setLoading(true);
      setError(null);
      try {
        const plan = await fetchProjectPlan(projectId, { signal });
        const [calendarValue, baselineValue, names, burnRows, zoneRows] = await Promise.all([
          fetchCalendar(projectId),
          fetchBaseline(projectId),
          fetchAssigneeNames(),
          fetchPhaseBurn(projectId),
          fetchCalendarZones(projectId).catch(() => []),
        ]);
        const edgesValue = await fetchDependencyEdges(
          projectId,
          plan.tasks.map((task) => task.id),
        );
        // The computed chain is a read: an empty move list asks the engine for
        // the critical path and nothing else.
        const chain = await previewSchedule(projectId, []).catch(() => null);
        if (signal?.aborted === true) return;
        setProject(plan.project);
        setPhases(plan.phases);
        setTasks(plan.tasks);
        setEdges(edgesValue);
        setCalendar(calendarValue);
        setBaseline(baselineValue);
        setZones(zoneRows);
        setAssignees(names);
        setBurn(
          new Map(
            burnRows.map((row) => [row.phase_id, { consumed: row.consumed_minutes, alert: row.alert }]),
          ),
        );
        setComputedCritical(chain?.critical_path ?? []);
        setFitWindow(fitFromPlan(plan.tasks, plan.phases, dayKey(Date.now()), {
          weekdays: 0b001_1111,
          minutesPerDay: 480,
        }));
      } catch (cause) {
        if (signal?.aborted === true) return;
        setError(cause instanceof Error ? cause.message : "The plan could not be loaded.");
      } finally {
        if (signal?.aborted !== true) setLoading(false);
      }
    },
    [projectId],
  );

  useEffect(() => {
    const controller = new AbortController();
    void load(controller.signal);
    return () => controller.abort();
  }, [load]);

  const calendarModel: GanttCalendar = useMemo(
    () => ({
      weekdays: calendar?.weekdays ?? 0b001_1111,
      minutesPerDay: calendar?.minutes_per_day ?? 480,
    }),
    [calendar],
  );

  const chartZones = useMemo(
    () =>
      zones.map((zone) => ({
        id: zone.id,
        kind: zone.kind === "closed" ? ("closed" as const) : ("window" as const),
        from: zone.starts_on,
        to: zone.ends_on,
        label: zone.label,
      })),
    [zones],
  );

  const edgesByTask = useMemo(() => {
    const map = new Map<string, string[]>();
    for (const edge of edges) {
      const list = map.get(edge.task_id) ?? [];
      list.push(edge.depends_on_task_id);
      map.set(edge.task_id, list);
    }
    return map;
  }, [edges]);

  const chartTasks: GanttTask[] = useMemo(
    () =>
      tasks.map((task) => ({
        id: task.id,
        phase_id: task.phase_id,
        title: task.title,
        start_at: task.start_at,
        duration_minutes: task.duration_minutes,
        baseline_start_at: task.baseline_start_at,
        baseline_duration_minutes: task.baseline_duration_minutes,
        is_critical: task.is_critical,
        work_type: task.work_type,
        status: task.status,
        priority: task.priority,
        assignee_name:
          task.assignee_id === null ? null : assignees.get(task.assignee_id) ?? null,
        external_owner: task.external_owner,
        role_key: task.role_key,
        due_at: task.due_at,
        effort_estimate_minutes: task.effort_estimate_minutes,
        depends_on: edgesByTask.get(task.id) ?? [],
      })),
    [tasks, assignees, edgesByTask],
  );

  const chartPhases: GanttPhase[] = useMemo(
    () =>
      phases.map((phase) => {
        const rows = tasks.filter((task) => task.phase_id === phase.id);
        const planned = rows.reduce((sum, task) => sum + (task.effort_estimate_minutes ?? 0), 0);
        const row = burn.get(phase.id);
        return {
          id: phase.id,
          name: phase.name,
          start_at: phase.start_date ?? null,
          end_at: phase.end_date ?? null,
          status: phase.status,
          is_milestone: phase.is_milestone,
          logged_minutes: row?.consumed ?? null,
          budget_alert:
            row === undefined
              ? null
              : row.alert === "over"
                ? "over"
                : row.alert === "warn"
                  ? "warn"
                  : "none",
          planned_minutes: planned > 0 ? planned : null,
        };
      }),
    [phases, tasks, burn],
  );

  const visibleWindow = useMemo(() => {
    if (fitWindow === null) return null;
    if (scale === "week") return fitWindow;
    const anchor = parseDay(fitWindow.from);
    const span = scale === "day" ? 21 : 126;
    const aligned = weekAlignedWindow(anchor, addDays(anchor, span));
    return { from: dayKey(aligned.from), to: dayKey(aligned.to) };
  }, [fitWindow, scale]);

  const undatedCount = chartTasks.filter(
    (task) => task.start_at === null || task.duration_minutes === null,
  ).length;

  const criticalChain = showCritical ? computedCritical : null;
  const criticalNames = computedCritical
    .map((id) => chartTasks.find((task) => task.id === id)?.title ?? "")
    .filter((title) => title !== "");

  const baselineLabel =
    baseline === null ? null : formatDayWords(parseDay(baseline.captured_at.slice(0, 10)));

  const requestPreview = useCallback(
    async (move: GanttMoveRequest) => {
      setActionError(null);
      setBusy(true);
      const moves: ScheduleMoveWire[] = [
        {
          task_id: move.taskId,
          start_at: `${move.startAt}T00:00:00Z`,
          duration_minutes: move.durationMinutes,
        },
      ];
      try {
        const plan = await previewSchedule(projectId, moves);
        setPreview(plan);
        setPendingMoves(moves);
      } catch (cause) {
        setActionError(cause instanceof Error ? cause.message : "The move could not be previewed.");
      } finally {
        setBusy(false);
      }
    },
    [projectId],
  );

  const applyPreview = useCallback(async () => {
    if (preview === null) return;
    setBusy(true);
    setActionError(null);
    try {
      await applySchedule(projectId, pendingMoves);
      setPreview(null);
      setPendingMoves([]);
      await load();
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : "The change was refused.");
    } finally {
      setBusy(false);
    }
  }, [preview, pendingMoves, projectId, load]);

  const saveDates = useCallback(
    async (startAt: string, durationMinutes: number) => {
      if (setDatesTask === null) return;
      setBusy(true);
      setActionError(null);
      try {
        await patchTaskDates(
          projectId,
          setDatesTask.id,
          `${startAt}T00:00:00Z`,
          durationMinutes,
        );
        setSetDatesTask(null);
        await load();
      } catch (cause) {
        setActionError(cause instanceof Error ? cause.message : "The dates were refused.");
      } finally {
        setBusy(false);
      }
    },
    [setDatesTask, projectId, load],
  );

  if (loading) {
    return (
      <div style={{ padding: 24 }}>
        <Spinner label="Loading the timeline" />
      </div>
    );
  }

  if (error !== null) {
    return (
      <div style={{ display: "grid", gap: 12, padding: 24 }}>
        <p role="alert" style={{ fontSize: 13, color: "var(--m-status-danger-fg)" }}>
          <Icon name="alert" size={13} aria-label="Error" /> {error}
        </p>
        <div style={{ display: "flex", gap: 8 }}>
          <Button size="sm" onClick={() => void load()}>
            Try again
          </Button>
          <Link href={`/projects/${projectId}`} style={{ alignSelf: "center", fontSize: 13 }}>
            Back to the project
          </Link>
        </div>
      </div>
    );
  }

  if (project === null) {
    return (
      <div style={{ padding: 24 }}>
        <EmptyState message="This project could not be read. It may have been removed, or your access changed." />
      </div>
    );
  }

  const finishShift = previewFinishShift(tasks, preview, calendarModel);

  const changes: GanttPlanChange[] = (preview?.changes ?? []).map((change) => ({
    taskId: change.task_id,
    fromStart: change.from_start === null ? null : change.from_start.slice(0, 10),
    toStart: change.to_start === null ? null : change.to_start.slice(0, 10),
    reason: change.reason === "pushed" ? "pushed" : "moved",
  }));

  return (
    <ProjectTimelineShell
      project={project}
      phases={phases}
      taskCount={tasks.length}
      baselineLabel={baselineLabel}
    >
      <div style={{ display: "grid", gap: 12 }}>
        {actionError !== null ? (
          <p role="alert" style={{ margin: 0, fontSize: 13, color: "var(--m-status-danger-fg)" }}>
            <Icon name="alert" size={13} aria-label="Error" /> {actionError}
          </p>
        ) : null}

        {showCritical && criticalChain !== null ? (
          <p style={{ margin: 0, fontSize: 12, color: "var(--m-text-secondary)" }}>
            <strong style={{ color: "var(--m-text)" }}>Critical path:</strong>{" "}
            {criticalNames.length === 0
              ? "no chain to name yet — no task has dates."
              : criticalNames.join(" → ")}
            {" · "}
            the chain is drawn in ink with heavier elbows; slack is not exposed by the planner yet.
          </p>
        ) : null}

        {preview !== null ? (
          <PreviewBar
            changes={changes}
            finishShift={finishShift}
            busy={busy}
            onCancel={() => {
              setPreview(null);
              setPendingMoves([]);
            }}
            onApply={() => void applyPreview()}
          />
        ) : null}

        <Toolbar
          view={view}
          onView={setView}
          scale={scale}
          onScale={setScale}
          undatedCount={undatedCount}
          showTray={showTray}
          onToggleTray={() => setShowTray((value) => !value)}
          showBaseline={showBaseline}
          onToggleBaseline={() => setShowBaseline((value) => !value)}
          showCritical={showCritical}
          onToggleCritical={() => setShowCritical((value) => !value)}
          onFit={() => {
            setScale("week");
            setFitWindow(fitFromPlan(tasks, phases, dayKey(Date.now()), calendarModel));
          }}
          onToday={() => {
            const anchor = today === null ? Date.now() : parseDay(today);
            const aligned = weekAlignedWindow(addDays(startOfWeek(anchor), -7), addDays(anchor, 28));
            setFitWindow({ from: dayKey(aligned.from), to: dayKey(aligned.to) });
            setScale("week");
          }}
          onCollapseAll={() =>
            setCollapsed((current) =>
              current.size === phases.length ? new Set() : new Set(phases.map((phase) => phase.id)),
            )
          }
          collapsedAll={collapsed.size > 0 && collapsed.size === phases.length}
        />

        <SecondCueNote />

        {tasks.length === 0 ? (
          <div
            style={{
              border: "1px solid var(--m-border)",
              borderRadius: 16,
              background: "var(--m-surface)",
              padding: 24,
            }}
          >
            <EmptyState
              message="No tasks yet. Add tasks on the Plan tab and their dates will draw here."
              action={
                <Link href={`/projects/${projectId}`} style={{ fontSize: 13 }}>
                  Open the Plan tab
                </Link>
              }
            />
          </div>
        ) : view === "table" ? (
          <GanttTable
            phases={chartPhases}
            tasks={chartTasks}
            calendar={calendarModel}
            criticalPath={criticalChain}
            showBaseline={showBaseline}
            baselineLabel={baselineLabel === null ? null : `the baseline captured ${baselineLabel}`}
            onRequestSetDates={(taskId) => {
              const task = tasks.find((row) => row.id === taskId) ?? null;
              setSetDatesTask(task);
            }}
            note={
              showTray && undatedCount > 0
                ? `${undatedCount} ${undatedCount === 1 ? "task has" : "tasks have"} no dates and ${undatedCount === 1 ? "is" : "are"} not on the chart.`
                : null
            }
          />
        ) : (
          <GanttChart
            phases={chartPhases}
            tasks={chartTasks}
            today={today}
            window={visibleWindow}
            calendar={calendarModel}
            zones={chartZones}
            showBaseline={showBaseline}
            criticalPath={criticalChain}
            baselineLabel={baselineLabel === null ? null : `the baseline captured ${baselineLabel}`}
            preview={changes}
            collapsedPhases={collapsed}
            onTogglePhase={(phaseId) =>
              setCollapsed((current) => {
                const next = new Set(current);
                if (next.has(phaseId)) next.delete(phaseId);
                else next.add(phaseId);
                return next;
              })
            }
            onMoveRequest={(move) => void requestPreview(move)}
            onRequestSetDates={(taskId) => {
              const task = tasks.find((row) => row.id === taskId) ?? null;
              setSetDatesTask(task);
            }}
            showTray={showTray}
            busy={busy}
          />
        )}
      </div>

      <SetDatesDialog
        task={setDatesTask}
        calendar={calendarModel}
        busy={busy}
        onClose={() => setSetDatesTask(null)}
        onSave={(startAt, minutes) => void saveDates(startAt, minutes)}
      />
    </ProjectTimelineShell>
  );
}

// --- the toolbar -------------------------------------------------------------

function Toolbar({
  view,
  onView,
  scale,
  onScale,
  undatedCount,
  showTray,
  onToggleTray,
  showBaseline,
  onToggleBaseline,
  showCritical,
  onToggleCritical,
  onFit,
  onToday,
  onCollapseAll,
  collapsedAll,
}: {
  view: ViewMode;
  onView: (view: ViewMode) => void;
  scale: Scale;
  onScale: (scale: Scale) => void;
  undatedCount: number;
  showTray: boolean;
  onToggleTray: () => void;
  showBaseline: boolean;
  onToggleBaseline: () => void;
  showCritical: boolean;
  onToggleCritical: () => void;
  onFit: () => void;
  onToday: () => void;
  onCollapseAll: () => void;
  collapsedAll: boolean;
}): React.JSX.Element {
  const scales: Scale[] = ["day", "week", "month"];
  return (
    <div
      style={{
        display: "flex",
        flexWrap: "wrap",
        alignItems: "center",
        gap: 8,
        minHeight: 44,
      }}
    >
      <div
        role="group"
        aria-label="Timeline range"
        style={{
          display: "flex",
          gap: 4,
          padding: 3,
          borderRadius: 12,
          background: "var(--m-surface-2)",
        }}
      >
        {scales.map((value) => (
          <button
            key={value}
            type="button"
            aria-pressed={scale === value}
            onClick={() => onScale(value)}
            style={{
              border: "none",
              borderRadius: 9,
              padding: "5px 12px",
              fontSize: 13,
              fontWeight: scale === value ? 600 : 500,
              cursor: "pointer",
              background: scale === value ? "var(--m-surface)" : "transparent",
              color: scale === value ? "var(--m-text)" : "var(--m-text-secondary)",
              boxShadow: scale === value ? "0 1px 2px color-mix(in srgb, var(--m-ink) 8%, transparent)" : "none",
            }}
          >
            {SCALE_LABEL[value]}
          </button>
        ))}
      </div>

      <Button size="sm" variant="secondary" onClick={onFit}>
        <Icon name="expand" size={14} /> Fit
      </Button>
      <Button size="sm" variant="secondary" onClick={onToday}>
        <Icon name="today" size={14} /> Today
      </Button>
      <Toggle active={showBaseline} onClick={onToggleBaseline} icon="layers" label="Baseline" />
      <Toggle active={showCritical} onClick={onToggleCritical} icon="grid" label="Critical path" />
      <Toggle active={collapsedAll} onClick={onCollapseAll} icon="rows-3" label="Summary only" />
      {undatedCount > 0 ? (
        <Toggle
          active={showTray}
          onClick={onToggleTray}
          icon="inbox"
          label={`Not dated ${undatedCount}`}
        />
      ) : null}

      <span
        aria-hidden="true"
        style={{ width: 1, height: 20, background: "var(--m-border)", margin: "0 4px" }}
      />

      <Legend />

      <span style={{ marginLeft: "auto", display: "flex", gap: 4 }}>
        <Button
          size="sm"
          variant={view === "chart" ? "primary" : "secondary"}
          aria-pressed={view === "chart"}
          onClick={() => onView("chart")}
        >
          <Icon name="chart" size={14} /> Chart
        </Button>
        <Button
          size="sm"
          variant={view === "table" ? "primary" : "secondary"}
          aria-pressed={view === "table"}
          onClick={() => onView("table")}
        >
          <Icon name="rows-4" size={14} /> View as table
        </Button>
      </span>
    </div>
  );
}

function Toggle({
  active,
  onClick,
  icon,
  label,
}: {
  active: boolean;
  onClick: () => void;
  icon: Parameters<typeof Icon>[0]["name"];
  label: string;
}): React.JSX.Element {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 6,
        height: 30,
        padding: "0 11px",
        borderRadius: 999,
        cursor: "pointer",
        border: `1px solid ${active ? "var(--m-gold)" : "var(--m-border)"}`,
        background: active ? "var(--m-gold-soft)" : "var(--m-surface)",
        color: active ? "var(--m-text)" : "var(--m-text-secondary)",
        fontSize: 13,
        fontWeight: 600,
        whiteSpace: "nowrap",
      }}
    >
      <Icon name={icon} size={13} />
      {label}
      {active ? <Icon name="check" size={12} /> : null}
    </button>
  );
}

function Legend(): React.JSX.Element {
  return (
    <span
      aria-label="Legend"
      style={{ display: "flex", flexWrap: "wrap", alignItems: "center", gap: 12 }}
    >
      <LegendItem label="In progress" background="var(--m-bar)" edge="var(--m-gold)" />
      <LegendItem label="Done" background="var(--m-status-success-bg)" />
      <LegendItem label="At risk" background="var(--m-status-warning-bg)" />
      <LegendItem label="Planned" background="var(--m-surface-2)" border="var(--m-input)" />
      <span style={{ display: "inline-flex", alignItems: "center", gap: 6, fontSize: 12, color: "var(--m-text-secondary)" }}>
        <span
          aria-hidden="true"
          style={{
            width: 12,
            height: 12,
            borderRadius: 2,
            background: "var(--m-text)",
            transform: "rotate(45deg)",
          }}
        />
        Milestone
      </span>
    </span>
  );
}

function LegendItem({
  label,
  background,
  border,
  edge,
}: {
  label: string;
  background: string;
  border?: string;
  edge?: string;
}): React.JSX.Element {
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 6, fontSize: 12, color: "var(--m-text-secondary)" }}>
      <span
        aria-hidden="true"
        style={{
          width: 22,
          height: 10,
          borderRadius: 3,
          background,
          border: border === undefined ? "none" : `1px solid ${border}`,
          boxShadow: edge === undefined ? undefined : `inset 4px 0 0 ${edge}`,
        }}
      />
      {label}
    </span>
  );
}

/** S4.14's own note, in the product rather than only in the design pack: each
 * status names the cue that survives a greyscale print. */
function SecondCueNote(): React.JSX.Element {
  return (
    <details
      style={{
        border: "1px solid var(--m-border)",
        borderRadius: 12,
        padding: "8px 12px",
        background: "var(--m-surface)",
      }}
    >
      <summary
        style={{
          cursor: "pointer",
          fontSize: 12,
          fontWeight: 600,
          color: "var(--m-text-secondary)",
        }}
      >
        Not by colour alone
      </summary>
      <ul
        style={{
          listStyle: "none",
          margin: "8px 0 0",
          padding: 0,
          display: "grid",
          gridTemplateColumns: "repeat(auto-fit, minmax(230px, 1fr))",
          gap: "6px 20px",
          fontSize: 12,
        }}
      >
        {[
          ["In progress", "ink fill with a gold leading edge"],
          ["Done", "check icon in the bar"],
          ["At risk", "alert icon in the bar and on the row"],
          ["Waiting", "diagonal hatch"],
          ["On hold or cancelled", "outline only, vertical hatch"],
          ["Critical", "heavier outline, heavier elbows, the word in the row"],
          ["Milestone", "a diamond, never a bar"],
          ["Baseline", "a thin outline under the bar, variance in words"],
        ].map(([label, cue]) => (
          <li key={label} style={{ display: "flex", gap: 8 }}>
            <span style={{ fontWeight: 600, color: "var(--m-text)" }}>{label}</span>
            <span style={{ color: "var(--m-text-muted)" }}>{cue}</span>
          </li>
        ))}
      </ul>
    </details>
  );
}

// --- the preview bar (S4.7) --------------------------------------------------

function PreviewBar({
  changes,
  finishShift,
  busy,
  onCancel,
  onApply,
}: {
  changes: GanttPlanChange[];
  finishShift: number;
  busy: boolean;
  onCancel: () => void;
  onApply: () => void;
}): React.JSX.Element {
  const pushed = changes.filter((change) => change.reason === "pushed").length;
  return (
    <div
      role="region"
      aria-label="Impact preview"
      data-testid="timeline-preview"
      style={{
        display: "flex",
        flexWrap: "wrap",
        alignItems: "center",
        gap: 14,
        padding: "10px 14px",
        borderRadius: 12,
        background: "var(--m-bar)",
        color: "var(--m-cream)",
      }}
    >
      <div style={{ display: "grid", gap: 2, minWidth: 0 }}>
        <span style={{ fontSize: 12, color: "color-mix(in srgb, var(--m-cream) 78%, transparent)" }}>
          Preview, nothing saved
        </span>
        <span style={{ fontSize: 13, fontWeight: 600 }}>
          {changes.length} {changes.length === 1 ? "task moves" : "tasks move"}
          {pushed > 0
            ? ` · ${pushed} pushed by a predecessor`
            : " · nothing downstream changes"}
          {finishShift === 0
            ? " · the finish holds"
            : ` · the finish moves ${Math.abs(finishShift)} working ${
                Math.abs(finishShift) === 1 ? "day" : "days"
              } ${finishShift > 0 ? "later" : "earlier"}`}
        </span>
      </div>
      <div style={{ marginLeft: "auto", display: "flex", gap: 8 }}>
        <button
          type="button"
          onClick={onCancel}
          style={{
            height: 32,
            padding: "0 12px",
            borderRadius: 10,
            border: "1px solid color-mix(in srgb, var(--m-cream) 45%, transparent)",
            background: "transparent",
            color: "var(--m-cream)",
            fontSize: 13,
            fontWeight: 600,
            cursor: "pointer",
          }}
        >
          Cancel <kbd style={previewKbd}>Esc</kbd>
        </button>
        <button
          type="button"
          onClick={onApply}
          disabled={busy}
          style={{
            height: 32,
            padding: "0 14px",
            borderRadius: 10,
            border: "none",
            background: "var(--m-gold)",
            color: "var(--m-ink)",
            fontSize: 13,
            fontWeight: 600,
            cursor: busy ? "default" : "pointer",
          }}
        >
          Apply
        </button>
      </div>
    </div>
  );
}

/** How far the plan's finish moves under the preview, in working days. */
function previewFinishShift(
  tasks: TimelineTaskWire[],
  preview: SchedulePlanWire | null,
  calendar: GanttCalendar,
): number {
  if (preview === null) return 0;
  const to = new Map(
    preview.changes
      .filter((change) => change.to_start !== null)
      .map((change) => [change.task_id, parseDay(change.to_start as string)]),
  );
  let before = 0;
  let after = 0;
  for (const task of tasks) {
    if (task.start_at === null || task.duration_minutes === null) continue;
    const start = parseDay(task.start_at);
    const end = addWorkingMinutes(start, task.duration_minutes, calendar);
    const nextStart = to.get(task.id);
    const nextEnd = nextStart === undefined ? end : addWorkingMinutes(nextStart, task.duration_minutes, calendar);
    before = Math.max(before, end);
    after = Math.max(after, nextEnd);
  }
  if (before === 0 || after === 0 || before === after) return 0;
  return workingDayOffset(Math.min(before, after), Math.max(before, after), calendar) * (after > before ? 1 : -1);
}

const previewKbd: React.CSSProperties = {
  marginLeft: 4,
  fontFamily: "var(--m-font-mono)",
  fontSize: 10,
  padding: "0 4px",
  borderRadius: 4,
  border: "1px solid color-mix(in srgb, var(--m-cream) 45%, transparent)",
  color: "inherit",
};

// --- the set-dates form ------------------------------------------------------

function SetDatesDialog({
  task,
  calendar,
  busy,
  onClose,
  onSave,
}: {
  task: TimelineTaskWire | null;
  calendar: GanttCalendar;
  busy: boolean;
  onClose: () => void;
  onSave: (startAt: string, durationMinutes: number) => void;
}): React.JSX.Element {
  const [start, setStart] = useState("");
  const [days, setDays] = useState("1");
  const [hours, setHours] = useState("0");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (task === null) return;
    setStart(task.start_at === null ? "" : task.start_at.slice(0, 10));
    const minutes = task.duration_minutes ?? calendar.minutesPerDay;
    const perDay = calendar.minutesPerDay > 0 ? calendar.minutesPerDay : 480;
    setDays(String(Math.floor(minutes / perDay)));
    setHours(String(Math.round(((minutes % perDay) / 60) * 10) / 10));
    setError(null);
  }, [task, calendar]);

  const durationMinutes = Math.round(
    Number(days) * calendar.minutesPerDay + Number(hours) * 60,
  );
  const startMs = start === "" ? null : parseDay(start);
  const endMs =
    startMs === null || Number.isNaN(startMs) || durationMinutes <= 0
      ? null
      : addWorkingMinutes(startMs, durationMinutes, calendar);

  const dayCount =
    startMs === null || endMs === null ? 0 : Math.max(1, workingDaysBetween(startMs, endMs, calendar));

  const submit = (): void => {
    if (start === "") {
      setError("Pick a start date.");
      return;
    }
    if (durationMinutes <= 0) {
      setError("A task needs a length. Enter days or hours.");
      return;
    }
    onSave(start, durationMinutes);
  };

  return (
    <Dialog
      open={task !== null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent
        title={task === null ? "Set dates" : `Set dates — ${task.title}`}
        description="Both a start and a length are required: the plan never places a bar without them."
      >
        <div style={{ display: "grid", gap: 12, padding: 16 }}>
          <div>
            <Label htmlFor="timeline-start">Start</Label>
            <Input
              id="timeline-start"
              type="date"
              value={start}
              onChange={(event) => setStart(event.target.value)}
            />
            <FieldHint>Drops snap forward to the next working day automatically.</FieldHint>
          </div>
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 10 }}>
            <div>
              <Label htmlFor="timeline-days">Working days</Label>
              <Input
                id="timeline-days"
                type="number"
                min={0}
                step={1}
                value={days}
                onChange={(event) => setDays(event.target.value)}
              />
            </div>
            <div>
              <Label htmlFor="timeline-hours">Hours</Label>
              <Input
                id="timeline-hours"
                type="number"
                min={0}
                step={0.5}
                value={hours}
                onChange={(event) => setHours(event.target.value)}
              />
            </div>
          </div>
          <p style={{ margin: 0, fontSize: 12, color: "var(--m-text-secondary)" }}>
            {endMs === null || startMs === null
              ? "Pick a start and a length to see the finish."
              : `Finishes ${formatDayWords(endMs)} · ${durationInWords(durationMinutes, calendar)} · ${dayCount} working ${dayCount === 1 ? "day" : "days"}`}
          </p>
          {error !== null ? (
            <p role="alert" style={{ margin: 0, fontSize: 12, color: "var(--m-status-danger-fg)" }}>
              {error}
            </p>
          ) : null}
          <DialogActions>
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button onClick={submit} disabled={busy}>
              {busy ? <Spinner /> : null} Save dates
            </Button>
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
  );
}

// --- window fitting ----------------------------------------------------------

/** The plan's own span, padded to whole weeks, with a month minimum so a
 * two-day project does not render as a two-column chart. */
function fitFromPlan(
  tasks: TimelineTaskWire[],
  phases: TimelinePhase[],
  todayIso: string,
  calendar: GanttCalendar,
): { from: string; to: string } {
  const days: number[] = [];
  for (const task of tasks) {
    if (task.start_at !== null) days.push(parseDay(task.start_at));
    if (task.start_at !== null && task.duration_minutes !== null) {
      days.push(addWorkingMinutes(parseDay(task.start_at), task.duration_minutes, calendar));
    }
    if (task.baseline_start_at !== null) days.push(parseDay(task.baseline_start_at));
  }
  for (const phase of phases) {
    if (phase.start_date !== null && phase.start_date !== undefined) days.push(parseDay(phase.start_date));
    if (phase.end_date !== null && phase.end_date !== undefined) days.push(parseDay(phase.end_date));
  }
  const today = parseDay(todayIso);
  if (days.length === 0) {
    return { from: dayKey(startOfWeek(today)), to: dayKey(addDays(startOfWeek(today), 41)) };
  }
  // Today is not forced into the window: a project that finished last year is
  // read as it was, with no today rule stranded at the right edge.
  const from = Math.min(...days);
  const to = Math.max(...days);
  const aligned = weekAlignedWindow(from, to);
  const span = daysBetween(aligned.from, aligned.to);
  if (span < 28) {
    return { from: dayKey(aligned.from), to: dayKey(addDays(aligned.from, 34)) };
  }
  return { from: dayKey(aligned.from), to: dayKey(aligned.to) };
}
