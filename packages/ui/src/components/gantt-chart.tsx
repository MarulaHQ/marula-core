"use client";
/**
 * GanttChart — the project timeline: month band over week columns, phase
 * summary brackets, task bars that carry the assignee, milestone diamonds,
 * 1px dependency elbows, the today rule, calendar zones, a baseline overlay
 * and the critical path.
 *
 * Geometry is the pack's, measured from `redlines/projects/S4.1.json`:
 * a 52px band (24px month over 28px week), 48px rows, a 408/360/300px tree
 * beside the chart at 1440/1280/1024, 16/13/10px per day, bars at 60% of the
 * row height, an 8px summary bracket, a 4px baseline outline under the bar,
 * a ~20px milestone diamond and 1px elbows. Those numbers are why the chart
 * reads as the reference rather than as a generic bar list.
 *
 * Purpose: one accessible chart. Every bar is a keyboard target; Arrow walks
 * days, Enter asks for the move preview, `S` opens the set-dates form, and the
 * same plan is available as a table (`GanttTable`) so the chart is never the
 * only way to read the schedule.
 *
 * Non-purpose: no fetching, no persistence, no colour as category. Status is
 * the only hue, and every status carries a second cue (icon, hatch or shape).
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "./button";
import { Icon } from "../kit";
import {
  STANDARD_CALENDAR,
  addDays,
  addWorkingMinutes,
  dayKey,
  daysBetween,
  durationInWords,
  formatDayShort,
  formatDayWords,
  formatMonth,
  isWorkingDay,
  nextWorkingDay,
  parseDay,
  simulateMove,
  varianceInWords,
  weekAlignedWindow,
  weekdayIndex,
  workingDayOffset,
  workingDaysBetween,
  type GanttCalendar,
  type GanttPlanChange,
  type GanttPlanNode,
} from "./gantt-scale";

// --- public shapes -----------------------------------------------------------

/** One phase as the chart draws it. Dates are ISO days; null means undated. */
export interface GanttPhase {
  id: string;
  name: string;
  start_at: string | null;
  end_at: string | null;
  status: string;
  is_milestone: boolean;
  /** Logged minutes against the phase, when the surface has them. */
  logged_minutes?: number | null;
  /** Money-budget alert, used only when there is no planned effort to divide by. */
  budget_alert?: "none" | "warn" | "over" | null;
}

/** One task bar. `baseline_*` are the captured plan; `is_critical` is the
 * planner's own mark, and the computed chain arrives separately. */
export interface GanttTask {
  id: string;
  phase_id: string | null;
  title: string;
  start_at: string | null;
  duration_minutes: number | null;
  baseline_start_at: string | null;
  baseline_duration_minutes: number | null;
  is_critical: boolean;
  work_type: string | null;
  status: string;
  priority: string | null;
  /** Resolved name of the person, when the surface could resolve it. */
  assignee_name: string | null;
  /** Name of the vendor or client contact that holds the date. */
  external_owner: string | null;
  role_key: string | null;
  due_at: string | null;
  effort_estimate_minutes: number | null;
  depends_on: string[];
}

/** A calendar constraint drawn under the bars. */
export interface GanttZone {
  id: string;
  kind: "window" | "closed";
  from: string;
  to: string;
  label: string;
}

/** A move the chart is asking the surface to preview. */
export interface GanttMoveRequest {
  taskId: string;
  startAt: string;
  durationMinutes: number | null;
}

export interface GanttChartProps {
  phases: GanttPhase[];
  tasks: GanttTask[];
  /** UTC day used for the today rule; null hides it (server render). */
  today: string | null;
  /** Explicit visible window. Omitted, the chart fits the plan. */
  window?: { from: string; to: string } | null;
  zones?: GanttZone[];
  /** The server's working pattern; the standard week when the project has none. */
  calendar?: GanttCalendar | null;
  showBaseline?: boolean;
  /** `null` while the critical-path view is off; otherwise the chain in order. */
  criticalPath?: string[] | null;
  /** Names the baseline capture, for the variance sentence ("baseline 1"). */
  baselineLabel?: string | null;
  /** The server's preview changes, drawn as dashed ghosts plus outlined bars. */
  preview?: GanttPlanChange[] | null;
  /** Phase ids rendered collapsed to their summary. */
  collapsedPhases?: ReadonlySet<string>;
  onTogglePhase?: (phaseId: string) => void;
  /** Ask the surface for a preview of this move (drop, or Enter on a keyboard move). */
  onMoveRequest?: (move: GanttMoveRequest) => void;
  /** `S` on a bar, and the tray's button: open the set-dates form. */
  onRequestSetDates?: (taskId: string) => void;
  /** The tray is the "Not dated" view; the surface owns the toggle. */
  showTray?: boolean;
  /** The surface is between a drop and the server's answer. */
  busy?: boolean;
  className?: string;
}

// --- measured constants ------------------------------------------------------

const MS_PER_DAY = 86_400_000;
const MONTH_H = 24;
const WEEK_H = 28;
const HEADER_H = MONTH_H + WEEK_H;
const ROW_H = 48;
const BAR_H = 28.8;
const TREE_TITLE = "minmax(0,1fr) 66px 66px";

const TONE_STYLE: Record<
  BarTone,
  {
    bg: string;
    fg: string;
    border: string;
    hatch?: string;
    icon?: "check" | "alert";
  }
> = {
  planned: { bg: "var(--m-surface-2)", fg: "var(--m-text)", border: "var(--m-input)" },
  live: { bg: "var(--m-bar)", fg: "var(--m-cream)", border: "var(--m-bar)" },
  done: {
    bg: "var(--m-status-success-bg)",
    fg: "var(--m-status-success-fg)",
    border: "transparent",
    icon: "check",
  },
  warn: {
    bg: "var(--m-status-warning-bg)",
    fg: "var(--m-status-warning-fg)",
    border: "transparent",
    icon: "alert",
  },
  due: {
    bg: "var(--m-status-due-bg)",
    fg: "var(--m-status-due-fg)",
    border: "transparent",
    icon: "alert",
  },
  over: {
    bg: "var(--m-status-danger-bg)",
    fg: "var(--m-status-danger-fg)",
    border: "transparent",
    icon: "alert",
  },
  wait: {
    bg: "var(--m-status-neutral-bg)",
    fg: "var(--m-status-neutral-fg)",
    border: "transparent",
    hatch:
      "repeating-linear-gradient(135deg, transparent 0 5px, color-mix(in srgb, var(--m-status-neutral-fg) 18%, transparent) 5px 6px)",
  },
  hold: {
    bg: "transparent",
    fg: "var(--m-text-muted)",
    border: "var(--m-input)",
    hatch: "repeating-linear-gradient(90deg, transparent 0 6px, var(--m-border) 6px 7px)",
  },
};

type BarTone = "planned" | "live" | "done" | "warn" | "due" | "over" | "wait" | "hold";

const WORK_TYPE: Record<string, { icon: Parameters<typeof Icon>[0]["name"]; label: string }> = {
  onsite: { icon: "pin", label: "On site" },
  bench: { icon: "box", label: "Bench" },
  remote: { icon: "monitor", label: "Remote" },
  vendor: { icon: "cart", label: "Vendor" },
  client: { icon: "users", label: "Client action" },
  coordination: { icon: "flag", label: "Coordination" },
};

const STATUS_WORDS: Record<string, string> = {
  pending: "Not started",
  in_progress: "In progress",
  blocked: "Waiting",
  completed: "Done",
  cancelled: "Cancelled",
};

const DAYS_DUE_SOON = 3;

// --- the component -----------------------------------------------------------

export function GanttChart({
  phases,
  tasks,
  today,
  window: fixedWindow,
  zones = [],
  calendar: calendarProp = null,
  showBaseline = false,
  criticalPath = null,
  baselineLabel = null,
  preview = null,
  collapsedPhases,
  onTogglePhase,
  onMoveRequest,
  onRequestSetDates,
  showTray = true,
  busy = false,
  className,
}: GanttChartProps): React.JSX.Element {
  const format = useResponsiveFormat();
  const calendar = calendarProp ?? STANDARD_CALENDAR;
  const trackRef = useRef<HTMLDivElement | null>(null);
  const [hovered, setHovered] = useState<string | null>(null);
  const [drag, setDrag] = useState<DragState | null>(null);
  const [trayDrag, setTrayDrag] = useState<string | null>(null);
  const [blocked, setBlocked] = useState<{ taskId: string; message: string } | null>(null);

  const todayMs = today === null ? null : parseDay(today);
  const taskById = useMemo(() => new Map(tasks.map((task) => [task.id, task])), [tasks]);
  const planNodes = useMemo<GanttPlanNode[]>(
    () =>
      tasks.map((task) => ({
        id: task.id,
        start: task.start_at === null ? null : parseDay(task.start_at),
        durationMinutes: task.duration_minutes,
        dependsOn: task.depends_on,
      })),
    [tasks],
  );

  // The plan's own extent, extended by the phases' dates. Undated work never
  // invents a position: it waits in the tray instead.
  const planWindow = useMemo(() => {
    const days: number[] = [];
    for (const task of tasks) {
      if (task.start_at !== null) days.push(parseDay(task.start_at));
      if (task.start_at !== null && task.duration_minutes !== null) {
        days.push(addWorkingMinutes(parseDay(task.start_at), task.duration_minutes, calendar));
      }
      if (task.baseline_start_at !== null) days.push(parseDay(task.baseline_start_at));
    }
    for (const phase of phases) {
      if (phase.start_at !== null) days.push(parseDay(phase.start_at));
      if (phase.end_at !== null) days.push(parseDay(phase.end_at));
    }
    for (const zone of zones) {
      days.push(parseDay(zone.from));
      days.push(parseDay(zone.to));
    }
    if (todayMs !== null) days.push(todayMs);
    const valid = days.filter((day) => !Number.isNaN(day));
    if (valid.length === 0) {
      const anchor = todayMs === null ? Date.parse("2026-01-05T00:00:00Z") : todayMs;
      return weekAlignedWindow(addDays(anchor, -7), addDays(anchor, 28));
    }
    return weekAlignedWindow(Math.min(...valid), Math.max(...valid));
  }, [tasks, phases, zones, calendar, todayMs]);

  const view = useMemo(() => {
    if (fixedWindow === null || fixedWindow === undefined) return planWindow;
    return weekAlignedWindow(parseDay(fixedWindow.from), parseDay(fixedWindow.to));
  }, [fixedWindow, planWindow]);

  const dayWidth = format.dayWidth;
  const days = Math.max(7, daysBetween(view.from, view.to) + 1);
  const chartWidth = days * dayWidth;
  const treeWidth = format.treeWidth;

  const rows = useMemo(
    () => buildRows(phases, tasks, collapsedPhases ?? new Set<string>()),
    [phases, tasks, collapsedPhases],
  );

  const rowIndex = useMemo(() => {
    const map = new Map<string, number>();
    rows.forEach((row, index) => {
      if (row.kind === "task") map.set(row.task.id, index);
    });
    return map;
  }, [rows]);

  const criticalSet = useMemo(() => new Set(criticalPath ?? []), [criticalPath]);
  const criticalActive = criticalPath !== null && criticalPath.length > 0;

  const previewByTask = useMemo(() => {
    const map = new Map<string, GanttPlanChange>();
    for (const change of preview ?? []) map.set(change.taskId, change);
    return map;
  }, [preview]);

  const xOf = useCallback(
    (ms: number) => Math.round(((ms - view.from) / MS_PER_DAY) * dayWidth),
    [view.from, dayWidth],
  );

  const undated = useMemo(
    () => tasks.filter((task) => task.start_at === null || task.duration_minutes === null),
    [tasks],
  );

  // --- pointer drag from a bar ----------------------------------------------
  const beginDrag = useCallback(
    (event: React.PointerEvent, task: GanttTask, startMs: number) => {
      if (onMoveRequest === undefined || busy) return;
      if (task.start_at === null || task.duration_minutes === null) return;
      const target = event.currentTarget as HTMLElement;
      const originX = event.clientX;
      const originStart = startMs;
      let moved = false;
      setBlocked(null);
      setDrag({
        taskId: task.id,
        startMs: originStart,
        nextMs: originStart,
        deltas: 0,
        valid: true,
        message: null,
      });

      const move = (pointerEvent: PointerEvent): void => {
        const deltaX = pointerEvent.clientX - originX;
        if (Math.abs(deltaX) > 2) moved = true;
        const deltaDays = Math.round(deltaX / dayWidth);
        const snapped = nextWorkingDay(addDays(originStart, deltaDays), calendar);
        const guard = predecessorGuard(task, taskById, calendar);
        const valid = guard === null || snapped >= guard.earliest;
        setDrag({
          taskId: task.id,
          startMs: originStart,
          nextMs: snapped,
          deltas: deltaDays,
          valid,
          message: valid || guard === null ? null : guard.message,
        });
      };

      const finish = (pointerEvent: PointerEvent): void => {
        detach();
        const deltaX = pointerEvent.clientX - originX;
        const deltaDays = Math.round(deltaX / dayWidth);
        const snapped = nextWorkingDay(addDays(originStart, deltaDays), calendar);
        const guard = predecessorGuard(task, taskById, calendar);
        setDrag(null);
        if (!moved) return;
        if (guard !== null && snapped < guard.earliest) {
          setBlocked({
            taskId: task.id,
            message: `${guard.message} Release to cancel, or press S to set the dates in a form.`,
          });
          return;
        }
        onMoveRequest({
          taskId: task.id,
          startAt: dayKey(snapped),
          durationMinutes: task.duration_minutes,
        });
      };

      const cancel = (): void => {
        detach();
        setDrag(null);
      };

      const detach = (): void => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", finish);
        window.removeEventListener("pointercancel", cancel);
      };

      try {
        target.setPointerCapture(event.pointerId);
      } catch {
        // Capture is a nicety; the window listeners still track the drag.
      }
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", finish);
      window.addEventListener("pointercancel", cancel);
    },
    [busy, calendar, dayWidth, onMoveRequest, taskById],
  );

  // --- drag from the undated tray -------------------------------------------
  const beginTrayDrag = useCallback(
    (event: React.PointerEvent, task: GanttTask) => {
      if (onMoveRequest === undefined || busy) return;
      event.preventDefault();
      setBlocked(null);
      setTrayDrag(task.id);
      const move = (pointerEvent: PointerEvent): void => {
        const track = trackRef.current;
        if (track === null) return;
        const rect = track.getBoundingClientRect();
        const inside =
          pointerEvent.clientX >= rect.left &&
          pointerEvent.clientX <= rect.right &&
          pointerEvent.clientY >= rect.top &&
          pointerEvent.clientY <= rect.bottom;
        setTrayDrag(inside ? task.id : null);
      };
      const finish = (pointerEvent: PointerEvent): void => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", finish);
        window.removeEventListener("pointercancel", cancel);
        setTrayDrag(null);
        const track = trackRef.current;
        if (track === null) return;
        const rect = track.getBoundingClientRect();
        if (
          pointerEvent.clientX < rect.left ||
          pointerEvent.clientX > rect.right ||
          pointerEvent.clientY < rect.top ||
          pointerEvent.clientY > rect.bottom
        ) {
          return;
        }
        const offset = pointerEvent.clientX - rect.left + track.scrollLeft;
        const dayIndex = Math.min(
          Math.max(0, Math.floor(offset / dayWidth)),
          Math.max(0, days - 1),
        );
        const snapped = nextWorkingDay(addDays(view.from, dayIndex), calendar);
        onMoveRequest({
          taskId: task.id,
          startAt: dayKey(snapped),
          durationMinutes: task.duration_minutes ?? calendar.minutesPerDay,
        });
      };
      const cancel = (): void => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", finish);
        window.removeEventListener("pointercancel", cancel);
        setTrayDrag(null);
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", finish);
      window.addEventListener("pointercancel", cancel);
    },
    [busy, calendar, dayWidth, days, onMoveRequest, view.from],
  );

  // --- keyboard --------------------------------------------------------------
  const keyMove = useCallback(
    (event: React.KeyboardEvent, task: GanttTask) => {
      if (task.start_at === null || task.duration_minutes === null) return;
      const start = parseDay(task.start_at);
      const current = drag !== null && drag.taskId === task.id ? drag.nextMs : start;
      if (event.key === "s" || event.key === "S") {
        if (onRequestSetDates === undefined) return;
        event.preventDefault();
        onRequestSetDates(task.id);
        return;
      }
      if (event.key === "Escape") {
        setDrag(null);
        setBlocked(null);
        return;
      }
      if (event.key === "Enter") {
        if (onMoveRequest === undefined || current === start) return;
        event.preventDefault();
        const guard = predecessorGuard(task, taskById, calendar);
        if (guard !== null && current < guard.earliest) {
          // The pending move is dropped: leaving it live would hide the
          // refusal behind the drag card.
          setDrag(null);
          setBlocked({ taskId: task.id, message: guard.message });
          return;
        }
        setDrag(null);
        onMoveRequest({
          taskId: task.id,
          startAt: dayKey(current),
          durationMinutes: task.duration_minutes,
        });
        return;
      }
      let step = 0;
      if (event.key === "ArrowRight") step = 1;
      else if (event.key === "ArrowLeft") step = -1;
      else if (event.key === "ArrowDown") step = daysInWeek(calendar);
      else if (event.key === "ArrowUp") step = -daysInWeek(calendar);
      if (step === 0) return;
      event.preventDefault();
      const snapped = nextWorkingDay(addDays(current, step), calendar);
      const guard = predecessorGuard(task, taskById, calendar);
      const valid = guard === null || snapped >= guard.earliest;
      setDrag({
        taskId: task.id,
        startMs: start,
        nextMs: snapped,
        deltas: daysBetween(start, snapped),
        valid,
        message: valid || guard === null ? null : guard.message,
      });
      setBlocked(valid || guard === null ? null : { taskId: task.id, message: guard.message });
    },
    [calendar, drag, onMoveRequest, onRequestSetDates, taskById],
  );

  const tooltipTaskId = drag?.taskId ?? (blocked === null ? hovered : blocked.taskId);
  const tooltipTask = tooltipTaskId === null ? null : taskById.get(tooltipTaskId) ?? null;

  if (format.mode === "list") {
    return (
      <GanttList
        phases={phases}
        tasks={tasks}
        window={view}
        todayMs={todayMs}
        calendar={calendar}
        showBaseline={showBaseline}
        criticalPath={criticalPath}
        preview={previewByTask}
        onRequestSetDates={onRequestSetDates}
        className={className}
      />
    );
  }

  const dragNodes = planNodes;
  const trayTask = trayDrag === null ? null : taskById.get(trayDrag) ?? null;

  return (
    <div className={className} data-slot="gantt-chart" data-testid="gantt-chart">
      <GanttMotionStyles />
      <div style={{ display: "flex", flexWrap: "wrap", gap: 16, alignItems: "flex-start" }}>
        <div
          style={{
            flex: "1 1 620px",
            minWidth: 0,
            display: "grid",
            gridTemplateColumns: `${treeWidth}px minmax(0,1fr)`,
            border: "1px solid var(--m-border)",
            borderRadius: 16,
            overflow: "hidden",
            background: "var(--m-surface)",
          }}
        >
          {/* ---- left: the tree ---- */}
          <div
            style={{
              minWidth: 0,
              borderRight: "1px solid var(--m-border)",
              background: "var(--m-surface)",
              position: "relative",
              zIndex: 3,
            }}
          >
            <div
              style={{
                display: "grid",
                gridTemplateColumns: TREE_TITLE,
                alignItems: "end",
                height: HEADER_H,
                paddingBottom: 8,
                boxSizing: "border-box",
                borderBottom: "1px solid var(--m-border)",
                background: "var(--m-canvas)",
                fontSize: 11,
                fontWeight: 600,
                letterSpacing: "0.06em",
                textTransform: "uppercase",
                color: "var(--m-text-muted)",
              }}
            >
              <div style={{ padding: "0 10px" }}>Task</div>
              <div style={{ padding: "0 10px" }}>Start</div>
              <div style={{ padding: "0 10px" }}>End</div>
            </div>
            {rows.map((row) => {
              if (row.kind === "phase") {
                const span = phaseSpan(row.phase, row.tasks, calendar);
                const collapsed = (collapsedPhases ?? new Set<string>()).has(row.phase.id);
                const burn = phaseBurn(row.phase, row.tasks);
                return (
                  <div
                    key={`p-${row.phase.id}`}
                    data-testid={`gantt-phase-${row.phase.id}`}
                    style={{
                      display: "grid",
                      gridTemplateColumns: TREE_TITLE,
                      alignItems: "center",
                      height: ROW_H,
                      borderBottom: "1px solid var(--m-border)",
                      background: "var(--m-canvas)",
                      fontWeight: 600,
                      fontSize: 13,
                    }}
                  >
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: 8,
                        minWidth: 0,
                        padding: "0 10px",
                      }}
                    >
                      <button
                        type="button"
                        aria-expanded={!collapsed}
                        aria-label={`${collapsed ? "Expand" : "Collapse"} ${row.phase.name}`}
                        onClick={() => onTogglePhase?.(row.phase.id)}
                        style={{
                          display: "grid",
                          placeItems: "center",
                          width: 18,
                          height: 18,
                          border: "none",
                          background: "transparent",
                          padding: 0,
                          cursor: "pointer",
                          color: "var(--m-text-secondary)",
                          transform: collapsed ? "rotate(-90deg)" : "rotate(90deg)",
                          transition: "transform 120ms cubic-bezier(0.2,0,0,1)",
                        }}
                      >
                        <Icon name="chev" size={14} />
                      </button>
                      <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                        {row.phase.is_milestone ? (
                          <span
                            aria-label="milestone phase"
                            style={{ marginRight: 5, color: "var(--m-text-secondary)" }}
                          >
                            ◆
                          </span>
                        ) : null}
                        {row.phase.name}
                      </span>
                      {burn.label !== "" ? (
                        <span
                          title={`${burn.label} logged against the phase's plan`}
                          style={{
                            marginLeft: "auto",
                            fontFamily: "var(--m-font-mono)",
                            fontSize: 11,
                            fontVariantNumeric: "tabular-nums",
                            color: burn.tone,
                            whiteSpace: "nowrap",
                          }}
                        >
                          {burn.alert === null ? "" : "▲ "}
                          {burn.label}
                        </span>
                      ) : null}
                    </div>
                    <div style={treeDateCell}>
                      {span.from === null ? "Not dated" : formatDayShort(span.from)}
                    </div>
                    <div style={treeDateCell}>{span.to === null ? "—" : formatDayShort(span.to)}</div>
                  </div>
                );
              }

              const { task } = row;
              const tone = toneOf(task, todayMs);
              const start = task.start_at === null ? null : parseDay(task.start_at);
              const end =
                start !== null && task.duration_minutes !== null
                  ? addWorkingMinutes(start, task.duration_minutes, calendar)
                  : null;
              const muted = criticalActive && !criticalSet.has(task.id);
              return (
                <div
                  key={`t-${task.id}`}
                  data-testid={`gantt-row-${task.id}`}
                  style={{
                    display: "grid",
                    gridTemplateColumns: TREE_TITLE,
                    alignItems: "center",
                    height: ROW_H,
                    borderBottom: "1px solid var(--m-surface-2)",
                    fontSize: 13,
                    background:
                      drag?.taskId === task.id || blocked?.taskId === task.id
                        ? "var(--m-accent-soft)"
                        : "var(--m-surface)",
                    boxShadow:
                      drag?.taskId === task.id || blocked?.taskId === task.id
                        ? "inset 3px 0 0 var(--m-gold)"
                        : undefined,
                    color: muted ? "var(--m-text-muted)" : "var(--m-text)",
                  }}
                >
                  <div
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: 8,
                      minWidth: 0,
                      padding: "0 10px 0 30px",
                    }}
                  >
                    <WorkTypeMark type={task.work_type} />
                    <span
                      style={{
                        minWidth: 0,
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        whiteSpace: "nowrap",
                        color: muted ? "var(--m-text-muted)" : "var(--m-text)",
                      }}
                    >
                      {task.title}
                    </span>
                    {criticalSet.has(task.id) ? (
                      <span
                        style={{
                          flexShrink: 0,
                          fontSize: 10,
                          fontWeight: 700,
                          letterSpacing: "0.04em",
                          textTransform: "uppercase",
                          color: "var(--m-text)",
                        }}
                      >
                        Critical
                      </span>
                    ) : null}
                    {tone === "over" || tone === "due" ? (
                      <span
                        style={{
                          flexShrink: 0,
                          color:
                            tone === "over" ? "var(--m-status-danger-fg)" : "var(--m-status-due-fg)",
                        }}
                      >
                        <Icon
                          name="alert"
                          size={12}
                          aria-label={tone === "over" ? "Overdue" : "Due soon"}
                        />
                      </span>
                    ) : null}
                  </div>
                  <div style={treeDateCell}>{start === null ? "Not dated" : formatDayShort(start)}</div>
                  <div style={treeDateCell}>{end === null ? "—" : formatDayShort(end)}</div>
                </div>
              );
            })}
          </div>

          {/* ---- right: the chart ---- */}
          <div
            ref={trackRef}
            style={{ minWidth: 0, position: "relative", overflowX: "auto" }}
            data-testid="gantt-track"
          >
            <div style={{ position: "relative", width: chartWidth, minWidth: "100%" }}>
              <ChartHeader
                window={view}
                dayWidth={dayWidth}
                days={days}
                todayMs={todayMs}
                calendar={calendar}
              />
              <div style={{ position: "relative", height: rows.length * ROW_H }}>
                <GridBackground
                  window={view}
                  dayWidth={dayWidth}
                  days={days}
                  rows={rows.length}
                  calendar={calendar}
                />
                {zones.map((zone) => (
                  <GanttZoneBand
                    key={zone.id}
                    zone={zone}
                    window={view}
                    dayWidth={dayWidth}
                  />
                ))}
                {todayMs !== null ? (
                  <div
                    aria-hidden="true"
                    style={{
                      position: "absolute",
                      top: 0,
                      bottom: 0,
                      left: xOf(todayMs) + dayWidth / 2,
                      width: 1,
                      background: "var(--m-text)",
                      zIndex: 6,
                    }}
                  />
                ) : null}

                {rows.map((row, index) => {
                  if (row.kind === "phase") {
                    const span = phaseSpan(row.phase, row.tasks, calendar);
                    const collapsed = (collapsedPhases ?? new Set<string>()).has(row.phase.id);
                    const burn = phaseBurn(row.phase, row.tasks);
                    const summaryWidth =
                      span.from === null || span.to === null
                        ? 0
                        : Math.max(8, xOf(addDays(span.to, 1)) - xOf(span.from));
                    return (
                      <div
                        key={`pr-${row.phase.id}`}
                        style={{
                          position: "absolute",
                          insetInline: 0,
                          top: index * ROW_H,
                          height: ROW_H,
                          borderBottom: "1px solid var(--m-border)",
                          background: "color-mix(in srgb, var(--m-canvas) 60%, transparent)",
                        }}
                      >
                        {span.from !== null && span.to !== null ? (
                          <>
                            <GanttSummary
                              left={xOf(span.from)}
                              width={summaryWidth}
                              tone={
                                row.phase.status === "completed"
                                  ? "done"
                                  : row.phase.status === "on_hold"
                                    ? "hold"
                                    : "default"
                              }
                            />
                            {burn.alert !== null ? (
                              <div
                                aria-hidden="true"
                                style={{
                                  position: "absolute",
                                  left: xOf(span.from),
                                  top: ROW_H / 2 + 9,
                                  height: 3,
                                  borderRadius: 2,
                                  width: summaryWidth,
                                  background: burn.tone,
                                }}
                              />
                            ) : null}
                          </>
                        ) : null}
                        {row.phase.is_milestone && span.to !== null ? (
                          <GanttMilestone
                            x={xOf(addDays(span.to, 1))}
                            tone={row.phase.status === "completed" ? "done" : "default"}
                            label={`${row.phase.name} milestone, ${formatDayWords(span.to)}`}
                          />
                        ) : null}
                        {collapsed ? (
                          <span
                            style={{
                              position: "absolute",
                              left: span.from === null ? 8 : xOf(span.from) + 8,
                              top: 4,
                              fontSize: 11,
                              color: "var(--m-text-muted)",
                              fontVariantNumeric: "tabular-nums",
                              whiteSpace: "nowrap",
                            }}
                          >
                            {row.tasks.length} {row.tasks.length === 1 ? "task" : "tasks"} folded
                          </span>
                        ) : null}
                      </div>
                    );
                  }

                  const { task } = row;
                  const tone = toneOf(task, todayMs);
                  const start = task.start_at === null ? null : parseDay(task.start_at);
                  const end =
                    start !== null && task.duration_minutes !== null
                      ? addWorkingMinutes(start, task.duration_minutes, calendar)
                      : null;
                  const change = previewByTask.get(task.id) ?? null;
                  const ghostStart =
                    change === null || change.fromStart === null ? null : parseDay(change.fromStart);
                  const newStart =
                    change !== null && change.toStart !== null
                      ? parseDay(change.toStart)
                      : start;
                  const width =
                    task.duration_minutes === null || newStart === null
                      ? 0
                      : Math.max(
                          12,
                          workingWidth(newStart, task.duration_minutes, calendar, dayWidth),
                        );
                  const baselineStart =
                    task.baseline_start_at === null ? null : parseDay(task.baseline_start_at);
                  const baselineWidth =
                    baselineStart === null || task.baseline_duration_minutes === null
                      ? 0
                      : Math.max(
                          10,
                          workingWidth(
                            baselineStart,
                            task.baseline_duration_minutes,
                            calendar,
                            dayWidth,
                          ),
                        );
                  const muted = criticalActive && !criticalSet.has(task.id);
                  // The variance is in the accessible name as words, not only
                  // in the floating card: a screen reader gets S4.2's sentence
                  // whether or not the pointer ever hovers.
                  const varianceText =
                    showBaseline && baselineStart !== null && start !== null
                      ? `${varianceInWords(
                          workingDayOffset(baselineStart, start, calendar),
                        )} than ${baselineLabel ?? "the baseline"}`
                      : null;
                  const sideline =
                    newStart === null
                      ? null
                      : bareNamePlacement(
                          barLabel(task),
                          xOf(newStart),
                          width,
                          chartWidth,
                          format.dayWidth >= 13 ? 46 : 34,
                        );
                  return (
                    <div
                      key={`tr-${task.id}`}
                      style={{
                        position: "absolute",
                        insetInline: 0,
                        top: index * ROW_H,
                        height: ROW_H,
                        borderBottom: "1px solid var(--m-surface-2)",
                      }}
                    >
                      {change !== null && ghostStart !== null ? (
                        <GhostBar left={xOf(ghostStart)} width={width} />
                      ) : null}
                      {showBaseline && baselineStart !== null ? (
                        <GanttBaseline left={xOf(baselineStart)} width={baselineWidth} />
                      ) : null}
                      {newStart !== null && task.duration_minutes !== null ? (
                        <>
                        <Bar
                          task={task}
                          tone={tone}
                          left={xOf(newStart)}
                          width={width}
                          label={
                            width >= (format.dayWidth >= 13 ? 46 : 34)
                              ? barLabel(task)
                              : null
                          }
                          muted={muted}
                          critical={criticalSet.has(task.id) || task.is_critical}
                          moved={change !== null}
                          pending={drag !== null && drag.taskId === task.id}
                          pendingValid={drag?.valid ?? true}
                          format={format}
                          ariaLabel={
                            start === null
                              ? task.title
                              : `${task.title}, ${formatDayWords(start)} to ${
                                  end === null ? "no end" : formatDayWords(end)
                                }, ${STATUS_WORDS[task.status] ?? task.status}${
                                  varianceText === null ? "" : `, ${varianceText}`
                                }`
                          }
                          busy={busy}
                          canDrag={onMoveRequest !== undefined}
                          testId={`gantt-bar-${task.id}`}
                          onPointerDown={(event) => beginDrag(event, task, start as number)}
                          onFocus={() => setHovered(task.id)}
                          onBlur={() => setHovered((current) => (current === task.id ? null : current))}
                          onMouseEnter={() => setHovered(task.id)}
                          onMouseLeave={() =>
                            setHovered((current) => (current === task.id ? null : current))
                          }
                          onKeyDown={(event) => keyMove(event, task)}
                        />
                        {/*
                          A one-day bar is too narrow to carry a name inside it,
                          so the name sits beside it. The pack does the same:
                          the bar is the fact, the label beside it is the who.
                          When the bar ends near the chart's right edge the name
                          goes to its left instead of being sliced off, and when
                          neither side can hold it the name is simply absent —
                          a half word is worse than no word.
                        */}
                        {sideline !== null ? (
                          <span aria-hidden="true" style={sideline.style}>
                            {sideline.name}
                          </span>
                        ) : null}
                        </>
                      ) : (
                        <span
                          style={{
                            position: "absolute",
                            left: 8,
                            top: ROW_H / 2 - 8,
                            fontSize: 11,
                            color: "var(--m-text-muted)",
                          }}
                        >
                          Not dated — set the dates to place it
                        </span>
                      )}
                      {blocked !== null && blocked.taskId === task.id && start !== null ? (
                        <BlockedBand
                          left={xOf(blockedEarliest(task, taskById, calendar, start))}
                          width={Math.max(
                            0,
                            xOf(start) - xOf(blockedEarliest(task, taskById, calendar, start)),
                          )}
                        />
                      ) : null}
                    </div>
                  );
                })}

                <DependencyArrows
                  rows={rows}
                  rowIndex={rowIndex}
                  taskById={taskById}
                  calendar={calendar}
                  dayWidth={dayWidth}
                  windowFrom={view.from}
                  criticalSet={criticalSet}
                  criticalActive={criticalActive}
                  previewByTask={previewByTask}
                />
              </div>
            </div>
          </div>
        </div>

        {showTray && undated.length > 0 ? (
          <GanttTray
            tasks={undated}
            calendar={calendar}
            busy={busy}
            onRequestSetDates={onRequestSetDates}
            onBeginDrag={beginTrayDrag}
            draggingId={trayDrag}
          />
        ) : null}
      </div>

      {trayTask !== null && trayDrag !== null ? (
        <FloatingCard>
          <strong style={{ fontWeight: 600 }}>
            Drop {trayTask.title} on a day to place it at{" "}
            {durationInWords(
              trayTask.duration_minutes ??
                trayTask.effort_estimate_minutes ??
                calendar.minutesPerDay,
              calendar,
            )}
          </strong>
          <span>Press S on the tray card instead to type the dates.</span>
        </FloatingCard>
      ) : drag !== null && tooltipTask !== null ? (
        <FloatingCard>{dragTooltip(tooltipTask, drag, calendar, dragNodes)}</FloatingCard>
      ) : blocked !== null ? (
        <BlockedNotice
          message={blocked.message}
          onDismiss={() => setBlocked(null)}
          onSetDates={
            onRequestSetDates === undefined ? undefined : () => onRequestSetDates(blocked.taskId)
          }
        />
      ) : tooltipTask !== null ? (
        <FloatingCard>
          {taskTooltip(tooltipTask, toneOf(tooltipTask, todayMs), calendar, baselineLabel, showBaseline)}
        </FloatingCard>
      ) : null}
    </div>
  );
}

// --- bands -------------------------------------------------------------------

function ChartHeader({
  window: view,
  dayWidth,
  days,
  todayMs,
  calendar,
}: {
  window: { from: number; to: number };
  dayWidth: number;
  days: number;
  todayMs: number | null;
  calendar: GanttCalendar;
}): React.JSX.Element {
  const months: { at: number; label: string }[] = [];
  const weeks: { at: number; label: string; current: boolean }[] = [];
  for (let index = 0; index < days; index += 1) {
    const ms = addDays(view.from, index);
    const date = new Date(ms);
    if (index === 0 || date.getUTCDate() === 1) {
      months.push({ at: index * dayWidth, label: formatMonth(ms) });
    }
    if (weekdayIndex(ms) === 0) {
      weeks.push({
        at: index * dayWidth,
        label: String(date.getUTCDate()),
        current:
          todayMs !== null && todayMs >= ms && todayMs < addDays(ms, 7),
      });
    }
  }
  void calendar;
  return (
    <div
      style={{
        position: "relative",
        height: HEADER_H,
        borderBottom: "1px solid var(--m-border)",
        background: "var(--m-canvas)",
      }}
    >
      {months.map((month, index) => {
        const next = months[index + 1];
        const width = (next === undefined ? days * dayWidth : next.at) - month.at;
        return (
          <div
            key={`m-${month.at}`}
            style={{
              position: "absolute",
              top: 0,
              left: month.at,
              width,
              height: MONTH_H,
              padding: "5px 0 0 8px",
              boxSizing: "border-box",
              borderLeft: month.at === 0 ? "none" : "1px solid var(--m-border-strong)",
              fontSize: 12,
              fontWeight: 600,
              whiteSpace: "nowrap",
              color: "var(--m-text)",
            }}
          >
            {month.label}
          </div>
        );
      })}
      {weeks.map((week) => (
        <div
          key={`w-${week.at}`}
          style={{
            position: "absolute",
            top: MONTH_H,
            left: week.at,
            height: WEEK_H,
            padding: "6px 0 0 6px",
            boxSizing: "border-box",
            borderLeft: "1px solid var(--m-border)",
            fontSize: 12,
            fontFamily: "var(--m-font-mono)",
            fontVariantNumeric: "tabular-nums",
            fontWeight: week.current ? 700 : 400,
            color: week.current ? "var(--m-text)" : "var(--m-text-secondary)",
            whiteSpace: "nowrap",
          }}
        >
          {week.label}
        </div>
      ))}
      {todayMs !== null ? (
        <div
          style={{
            position: "absolute",
            zIndex: 7,
            top: MONTH_H + 2,
            left: ((todayMs - view.from) / MS_PER_DAY) * dayWidth + dayWidth / 2,
            transform: "translateX(-50%)",
            display: "flex",
            alignItems: "center",
            height: 20,
            padding: "0 7px",
            borderRadius: 999,
            background: "var(--m-text)",
            color: "var(--m-gold)",
            fontSize: 12,
            fontWeight: 700,
            whiteSpace: "nowrap",
          }}
        >
          Today
        </div>
      ) : null}
    </div>
  );
}

function GridBackground({
  window: view,
  dayWidth,
  days,
  rows,
  calendar,
}: {
  window: { from: number; to: number };
  dayWidth: number;
  days: number;
  rows: number;
  calendar: GanttCalendar;
}): React.JSX.Element {
  const marks: React.ReactNode[] = [];
  for (let index = 0; index < days; index += 1) {
    const ms = addDays(view.from, index);
    const x = index * dayWidth;
    if (!isWorkingDay(ms, calendar) && dayWidth >= 12) {
      // One band per non-working day: adjacent days meet and read as the
      // weekend, so no run-length bookkeeping is needed.
      marks.push(
        <div
          key={`we-${index}`}
          aria-hidden="true"
          style={{
            position: "absolute",
            top: 0,
            bottom: 0,
            left: x,
            width: dayWidth,
            background: "var(--m-canvas)",
            opacity: 0.75,
          }}
        />,
      );
    }
    if (weekdayIndex(ms) === 0) {
      marks.push(
        <div
          key={`v-${index}`}
          aria-hidden="true"
          style={{
            position: "absolute",
            top: 0,
            bottom: 0,
            left: x,
            width: 1,
            background: "var(--m-border)",
          }}
        />,
      );
    }
    if (new Date(ms).getUTCDate() === 1) {
      marks.push(
        <div
          key={`vm-${index}`}
          aria-hidden="true"
          style={{
            position: "absolute",
            top: 0,
            bottom: 0,
            left: x,
            width: 1,
            background: "var(--m-border-strong)",
          }}
        />,
      );
    }
  }
  for (let row = 0; row < rows; row += 1) {
    marks.push(
      <div
        key={`hr-${row}`}
        aria-hidden="true"
        style={{
          position: "absolute",
          left: 0,
          right: 0,
          top: row * ROW_H,
          height: 1,
          background: "var(--m-surface-2)",
        }}
      />,
    );
  }
  return <>{marks}</>;
}

function DependencyArrows({
  rows,
  rowIndex,
  taskById,
  calendar,
  dayWidth,
  windowFrom,
  criticalSet,
  criticalActive,
  previewByTask,
}: {
  rows: Row[];
  rowIndex: Map<string, number>;
  taskById: Map<string, GanttTask>;
  calendar: GanttCalendar;
  dayWidth: number;
  windowFrom: number;
  criticalSet: ReadonlySet<string>;
  criticalActive: boolean;
  previewByTask: Map<string, GanttPlanChange>;
}): React.JSX.Element {
  const paths: React.ReactNode[] = [];
  for (const row of rows) {
    if (row.kind !== "task") continue;
    const to = row.task;
    const toStart = effectiveStart(to, previewByTask);
    const toRow = rowIndex.get(to.id);
    if (toStart === null || toRow === undefined) continue;
    for (const fromId of to.depends_on) {
      const from = taskById.get(fromId);
      const fromRow = rowIndex.get(fromId);
      const fromStart = from === undefined ? null : effectiveStart(from, previewByTask);
      if (
        from === undefined ||
        fromRow === undefined ||
        fromStart === null ||
        from.duration_minutes === null
      ) {
        continue;
      }
      const fromEnd = addWorkingMinutes(fromStart, from.duration_minutes, calendar);
      const x1 = Math.round(((fromEnd - windowFrom) / MS_PER_DAY) * dayWidth);
      const x2 = Math.round(((toStart - windowFrom) / MS_PER_DAY) * dayWidth);
      const y1 = fromRow * ROW_H + ROW_H / 2;
      const y2 = toRow * ROW_H + ROW_H / 2;
      const critical = criticalActive && criticalSet.has(to.id) && criticalSet.has(fromId);
      const colour = critical ? "var(--m-text)" : "var(--m-input)";
      const stroke = critical ? 1.75 : 1;
      const d =
        x2 >= x1 + 12
          ? `M${x1},${y1} H${x1 + 5} V${y2} H${x2 - 1}`
          : `M${x1},${y1} h5 v${(y2 > y1 ? 1 : -1) * (ROW_H / 2)} H${x2 - 9} V${y2} H${x2 - 1}`;
      paths.push(
        <g key={`${fromId}-${to.id}`}>
          <path d={d} fill="none" stroke={colour} strokeWidth={stroke} />
          <polygon
            fill={colour}
            points={`${x2 - 1},${y2} ${x2 - 6},${y2 - 3.5} ${x2 - 6},${y2 + 3.5}`}
          />
        </g>,
      );
    }
  }
  if (paths.length === 0) return <></>;
  return (
    <svg
      aria-hidden="true"
      style={{
        position: "absolute",
        left: 0,
        top: 0,
        pointerEvents: "none",
        zIndex: 3,
        overflow: "visible",
      }}
      width="100%"
      height="100%"
    >
      {paths}
    </svg>
  );
}

// --- marks -------------------------------------------------------------------

function Bar({
  task,
  tone,
  left,
  width,
  label,
  muted,
  critical,
  moved,
  pending,
  pendingValid,
  format,
  ariaLabel,
  busy,
  canDrag,
  testId,
  onPointerDown,
  onFocus,
  onBlur,
  onMouseEnter,
  onMouseLeave,
  onKeyDown,
}: {
  task: GanttTask;
  tone: BarTone;
  left: number;
  width: number;
  label: string | null;
  muted: boolean;
  critical: boolean;
  moved: boolean;
  pending: boolean;
  pendingValid: boolean;
  format: ResponsiveFormat;
  ariaLabel: string;
  busy: boolean;
  canDrag: boolean;
  testId: string;
  onPointerDown: (event: React.PointerEvent) => void;
  onFocus: () => void;
  onBlur: () => void;
  onMouseEnter: () => void;
  onMouseLeave: () => void;
  onKeyDown: (event: React.KeyboardEvent) => void;
}): React.JSX.Element {
  const style = TONE_STYLE[tone];
  const showLabel = width >= (format.dayWidth >= 13 ? 46 : 34) && label !== null;
  const shadows = [
    tone === "live" ? "inset 4px 0 0 var(--m-gold)" : null,
    pending
      ? pendingValid
        ? "0 0 0 2px var(--m-gold)"
        : "0 0 0 2px var(--m-status-danger-fg)"
      : null,
    moved ? "0 0 0 2px var(--m-gold)" : null,
    critical && !moved && !pending ? "0 0 0 1.5px var(--m-text)" : null,
  ]
    .filter((value): value is string => value !== null)
    .join(", ");
  return (
    <span
      role="button"
      tabIndex={0}
      data-testid={testId}
      data-tone={tone}
      aria-label={ariaLabel}
      aria-disabled={busy || !canDrag}
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
      onFocus={onFocus}
      onBlur={onBlur}
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
      style={{
        position: "absolute",
        left,
        width,
        top: (ROW_H - BAR_H) / 2,
        height: BAR_H,
        display: "flex",
        alignItems: "center",
        gap: 5,
        boxSizing: "border-box",
        padding: tone === "live" ? "0 7px 0 11px" : "0 7px",
        borderRadius: 6,
        border: `1px solid ${style.border}`,
        background: style.bg,
        backgroundImage: style.hatch,
        color: style.fg,
        fontSize: 12,
        fontWeight: 600,
        overflow: "hidden",
        whiteSpace: "nowrap",
        cursor: canDrag && task.start_at !== null ? "grab" : "default",
        opacity: muted ? 0.55 : 1,
        boxShadow: shadows === "" ? undefined : shadows,
        outlineOffset: 2,
        zIndex: pending ? 8 : 4,
        touchAction: "none",
        userSelect: "none",
      }}
    >
      {style.icon === "check" ? <Icon name="check" size={12} /> : null}
      {style.icon === "alert" ? <Icon name="alert" size={12} /> : null}
      {showLabel ? (
        <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>{label}</span>
      ) : null}
    </span>
  );
}

function GhostBar({ left, width }: { left: number; width: number }): React.JSX.Element {
  return (
    <span
      aria-hidden="true"
      style={{
        position: "absolute",
        left,
        width,
        top: (ROW_H - BAR_H) / 2,
        height: BAR_H,
        borderRadius: 6,
        border: "1px dashed var(--m-input)",
        background: "transparent",
        opacity: 0.4,
        zIndex: 3,
      }}
    />
  );
}

function BlockedBand({ left, width }: { left: number; width: number }): React.JSX.Element {
  return (
    <span
      aria-hidden="true"
      style={{
        position: "absolute",
        left,
        width,
        top: (ROW_H - BAR_H) / 2,
        height: BAR_H,
        borderRadius: 4,
        backgroundImage:
          "repeating-linear-gradient(135deg, transparent 0 4px, var(--m-border-strong) 4px 5px)",
        opacity: 0.5,
        zIndex: 5,
      }}
    />
  );
}

/** The bracket bar that summarises a phase, clipped into a bracket. */
export function GanttSummary({
  left,
  width,
  tone = "default",
}: {
  left: number;
  width: number;
  tone?: "default" | "done" | "hold" | "warn";
}): React.JSX.Element {
  const background =
    tone === "done"
      ? "var(--m-canopy)"
      : tone === "hold"
        ? "var(--m-input)"
        : tone === "warn"
          ? "var(--m-blush)"
          : "var(--m-text)";
  return (
    <span
      aria-hidden="true"
      style={{
        position: "absolute",
        left,
        width,
        top: ROW_H / 2 - 4,
        height: 8,
        borderRadius: 2,
        background,
        clipPath:
          "polygon(0 0, 100% 0, 100% 100%, calc(100% - 5px) 55%, 5px 55%, 0 100%)",
        zIndex: 4,
      }}
    />
  );
}

/** A milestone diamond, centred on its day. */
export function GanttMilestone({
  x,
  tone = "default",
  label,
  critical = false,
  moved = false,
}: {
  x: number;
  tone?: "default" | "done" | "warn";
  label: string;
  critical?: boolean;
  moved?: boolean;
}): React.JSX.Element {
  const shadow = moved
    ? "0 0 0 2px var(--m-gold)"
    : critical
      ? "0 0 0 2px var(--m-surface), 0 0 0 3.5px var(--m-text)"
      : undefined;
  return (
    <span
      role="img"
      aria-label={label}
      style={{
        position: "absolute",
        left: x,
        top: ROW_H / 2 - 10,
        width: 19.8,
        height: 19.8,
        borderRadius: 3,
        transform: "translateX(-10px) rotate(45deg)",
        background:
          tone === "done"
            ? "var(--m-canopy)"
            : tone === "warn"
              ? "var(--m-blush)"
              : "var(--m-text)",
        boxShadow: shadow,
        zIndex: 5,
      }}
    />
  );
}

/** The baseline outline: what the plan said when it was captured. */
export function GanttBaseline({ left, width }: { left: number; width: number }): React.JSX.Element {
  return (
    <span
      aria-hidden="true"
      style={{
        position: "absolute",
        left,
        width,
        top: ROW_H * 0.8 + 2,
        height: 4,
        borderRadius: 2,
        border: "1px solid var(--m-input)",
        background: "transparent",
        zIndex: 3,
      }}
    />
  );
}

/**
 * Where a zone's kind word sits: inside the band when the band can hold it,
 * else immediately before it (right-aligned), else immediately after.
 */
function zoneLabelPlacement(word: string, bandWidth: number): React.CSSProperties | null {
  const estimate = word.length * 6.4;
  if (bandWidth >= estimate + 12) return { left: 6 };
  return { left: -estimate - 6 };
}

/** A calendar constraint: the agreed change window, or the client's closure. */
export function GanttZoneBand({
  zone,
  window: view,
  dayWidth,
}: {
  zone: GanttZone;
  window: { from: number; to: number };
  dayWidth: number;
}): React.JSX.Element {
  const from = parseDay(zone.from);
  const to = parseDay(zone.to);
  const left = Math.round(((from - view.from) / MS_PER_DAY) * dayWidth);
  const width = Math.max(dayWidth, Math.round(((to - from + 1) / MS_PER_DAY) * dayWidth));
  const kindWord = zone.kind === "window" ? "Change window" : "Closed";
  return (
    <div
      data-slot="gantt-zone"
      data-kind={zone.kind}
      style={{
        position: "absolute",
        top: 0,
        bottom: 0,
        left,
        width,
        zIndex: 1,
        ...(zone.kind === "window"
          ? {
              background: "var(--m-gold-soft)",
              opacity: 0.5,
              borderLeft: "1px dashed var(--m-gold)",
              borderRight: "1px dashed var(--m-gold)",
            }
          : {
              opacity: 0.55,
              backgroundImage:
                "repeating-linear-gradient(135deg, transparent 0 6px, var(--m-border-strong) 6px 7px)",
            }),
      }}
      title={zone.label}
    >
      {/*
        The pack names a band by what it is — "Change window", "Closed" — in ink
        at the band's top. A two-day band is narrower than those words, so the
        name sits just outside on the side with room, the way S4.1 draws it; the
        project's own label stays the tooltip.
      */}
      {zoneLabelPlacement(kindWord, width) !== null ? (
        <span
          style={{
            position: "absolute",
            top: 8,
            ...(zoneLabelPlacement(kindWord, width) ?? {}),
            fontSize: 12,
            fontWeight: 600,
            whiteSpace: "nowrap",
            color: "var(--m-text)",
            zIndex: 2,
          }}
        >
          {kindWord}
        </span>
      ) : null}
    </div>
  );
}

/** The tray of undated tasks: drag one onto a day, or press `S` on it. */
export function GanttTray({
  tasks,
  calendar,
  busy,
  onRequestSetDates,
  onBeginDrag,
  draggingId,
}: {
  tasks: GanttTask[];
  calendar: GanttCalendar;
  busy: boolean;
  onRequestSetDates?: (taskId: string) => void;
  onBeginDrag?: (event: React.PointerEvent, task: GanttTask) => void;
  draggingId?: string | null;
}): React.JSX.Element {
  // Undated work has no duration by definition, so the honest figure is the
  // effort it is expected to need; falling back to zero would tell a planner
  // that two unplaced steps cost no time at all.
  const totalMinutes = tasks.reduce(
    (sum, task) => sum + (task.duration_minutes ?? task.effort_estimate_minutes ?? 0),
    0,
  );
  return (
    <aside
      aria-label="Tasks with no dates"
      data-testid="gantt-undated-tray"
      style={{
        width: 300,
        flexShrink: 0,
        border: "1px solid var(--m-border)",
        borderRadius: 16,
        background: "var(--m-surface)",
        padding: 14,
        boxSizing: "border-box",
        display: "flex",
        flexDirection: "column",
        gap: 10,
      }}
    >
      <div style={{ display: "flex", alignItems: "baseline", gap: 6 }}>
        <Icon name="inbox" size={15} aria-label="Not dated" />
        <h3 style={{ margin: 0, fontSize: 13, fontWeight: 600 }}>Not dated</h3>
        <span
          style={{
            marginLeft: "auto",
            fontFamily: "var(--m-font-mono)",
            fontSize: 11,
            fontVariantNumeric: "tabular-nums",
            color: "var(--m-text-muted)",
          }}
        >
          {tasks.length} · {durationInWords(totalMinutes, calendar)}
        </span>
      </div>
      <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "grid", gap: 8 }}>
        {tasks.map((task) => (
          <li
            key={task.id}
            data-testid={`gantt-undated-${task.id}`}
            onPointerDown={(event) => onBeginDrag?.(event, task)}
            style={{
              border:
                draggingId === task.id
                  ? "1px dashed var(--m-gold)"
                  : "1px dashed var(--m-input)",
              borderRadius: 12,
              padding: "9px 10px",
              display: "grid",
              gap: 6,
              background: draggingId === task.id ? "var(--m-gold-soft)" : "var(--m-surface)",
              cursor: onBeginDrag === undefined ? "default" : "grab",
              touchAction: "none",
              userSelect: "none",
            }}
          >
            <div style={{ display: "flex", alignItems: "flex-start", gap: 8 }}>
              <WorkTypeMark type={task.work_type} />
              <span style={{ fontSize: 13, fontWeight: 600, minWidth: 0 }}>{task.title}</span>
            </div>
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: 6,
                fontSize: 11,
                color: "var(--m-text-muted)",
              }}
            >
              <span style={{ fontVariantNumeric: "tabular-nums" }}>
                {task.duration_minutes === null
                  ? "no length"
                  : durationInWords(task.duration_minutes, calendar)}
              </span>
              <span aria-hidden="true">·</span>
              <span>{task.assignee_name ?? task.external_owner ?? task.role_key ?? "Unassigned"}</span>
            </div>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <Button
                size="sm"
                variant="secondary"
                disabled={busy}
                onClick={() => onRequestSetDates?.(task.id)}
                data-testid={`gantt-set-dates-${task.id}`}
              >
                Set dates <Kbd>S</Kbd>
              </Button>
              <span style={{ fontSize: 11, color: "var(--m-text-muted)" }}>or drag onto a day</span>
            </div>
          </li>
        ))}
      </ul>
      <p style={{ margin: 0, fontSize: 11, lineHeight: "16px", color: "var(--m-text-muted)" }}>
        These hours are not in anyone&apos;s week yet, so they are missing from capacity.
      </p>
    </aside>
  );
}

// --- list (below 1024) -------------------------------------------------------

/** S4.13: under 1024 the chart is not crushed; each task is a row with its
 * dates in words and a compact bar on one shared scale. */
export function GanttList({
  phases,
  tasks,
  window: view,
  todayMs,
  calendar,
  showBaseline,
  criticalPath,
  preview,
  onRequestSetDates,
  className,
}: {
  phases: GanttPhase[];
  tasks: GanttTask[];
  window: { from: number; to: number };
  todayMs: number | null;
  calendar: GanttCalendar;
  showBaseline: boolean;
  criticalPath: string[] | null;
  preview: Map<string, GanttPlanChange>;
  onRequestSetDates?: (taskId: string) => void;
  className?: string;
}): React.JSX.Element {
  const span = Math.max(1, daysBetween(view.from, view.to) + 1);
  const criticalSet = new Set(criticalPath ?? []);
  const groups = phases.map((phase) => ({
    phase,
    tasks: tasks.filter((task) => task.phase_id === phase.id),
  }));
  const orphan = tasks.filter((task) => task.phase_id === null);
  if (orphan.length > 0) {
    groups.push({
      phase: {
        id: "__none__",
        name: "No phase",
        start_at: null,
        end_at: null,
        status: "pending",
        is_milestone: false,
      },
      tasks: orphan,
    });
  }
  return (
    <div
      className={className}
      data-slot="gantt-list"
      data-testid="gantt-list"
      style={{ display: "grid", gap: 12 }}
    >
      {groups.map((group) => {
        if (group.tasks.length === 0) return null;
        return (
          <section
            key={group.phase.id}
            aria-label={group.phase.name}
            style={{
              border: "1px solid var(--m-border)",
              borderRadius: 14,
              overflow: "hidden",
              background: "var(--m-surface)",
            }}
          >
            <h3
              style={{
                margin: 0,
                padding: "10px 14px",
                background: "var(--m-canvas)",
                borderBottom: "1px solid var(--m-border)",
                fontSize: 12,
                fontWeight: 600,
              }}
            >
              {group.phase.name}
            </h3>
            <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
              {group.tasks.map((task) => {
                const start = task.start_at === null ? null : parseDay(task.start_at);
                const end =
                  start !== null && task.duration_minutes !== null
                    ? addWorkingMinutes(start, task.duration_minutes, calendar)
                    : null;
                const change = preview.get(task.id);
                const drawnStart =
                  change !== undefined && change.toStart !== null
                    ? parseDay(change.toStart)
                    : start;
                const tone = toneOf(task, todayMs);
                const style = TONE_STYLE[tone];
                return (
                  <li
                    key={task.id}
                    data-testid={`gantt-list-row-${task.id}`}
                    style={{
                      display: "grid",
                      gap: 8,
                      padding: "10px 14px",
                      borderBottom: "1px solid var(--m-surface-2)",
                    }}
                  >
                    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                      <WorkTypeMark type={task.work_type} />
                      <button
                        type="button"
                        onClick={() => onRequestSetDates?.(task.id)}
                        style={{
                          border: "none",
                          background: "transparent",
                          padding: 0,
                          font: "inherit",
                          fontSize: 13,
                          textAlign: "left",
                          cursor: onRequestSetDates === undefined ? "default" : "pointer",
                          color: "var(--m-text)",
                        }}
                      >
                        {task.title}
                        {criticalSet.has(task.id) ? (
                          <span
                            style={{
                              fontSize: 10,
                              fontWeight: 700,
                              marginLeft: 6,
                              textTransform: "uppercase",
                            }}
                          >
                            Critical
                          </span>
                        ) : null}
                      </button>
                      <span
                        style={{
                          marginLeft: "auto",
                          fontFamily: "var(--m-font-mono)",
                          fontSize: 12,
                          fontVariantNumeric: "tabular-nums",
                          whiteSpace: "nowrap",
                          color: "var(--m-text-secondary)",
                        }}
                      >
                        {start === null
                          ? "Not dated"
                          : `${formatDayShort(start)} to ${end === null ? "—" : formatDayShort(end)}`}
                      </span>
                    </div>
                    <div
                      style={{
                        position: "relative",
                        height: 8,
                        borderRadius: 999,
                        background: "var(--m-surface-2)",
                      }}
                    >
                      {todayMs !== null ? (
                        <span
                          aria-hidden="true"
                          style={{
                            position: "absolute",
                            top: -2,
                            bottom: -2,
                            left: `${((todayMs - view.from) / MS_PER_DAY / span) * 100}%`,
                            width: 1,
                            background: "var(--m-text)",
                          }}
                        />
                      ) : null}
                      {drawnStart !== null && task.duration_minutes !== null ? (
                        <span
                          style={{
                            position: "absolute",
                            top: 0,
                            bottom: 0,
                            left: `${Math.max(
                              0,
                              ((drawnStart - view.from) / MS_PER_DAY / span) * 100,
                            )}%`,
                            width: `${Math.max(
                              1.2,
                              (workingWidth(drawnStart, task.duration_minutes, calendar, 1) / span) * 100,
                            )}%`,
                            borderRadius: 999,
                            background: style.bg,
                            border: `1px solid ${style.border}`,
                            boxShadow:
                              change !== undefined
                                ? "0 0 0 2px var(--m-gold)"
                                : criticalSet.has(task.id)
                                  ? "0 0 0 1.5px var(--m-text)"
                                  : undefined,
                          }}
                        />
                      ) : null}
                      {showBaseline &&
                      task.baseline_start_at !== null &&
                      task.baseline_duration_minutes !== null ? (
                        <span
                          aria-hidden="true"
                          style={{
                            position: "absolute",
                            top: 9,
                            height: 2,
                            left: `${((parseDay(task.baseline_start_at) - view.from) / MS_PER_DAY / span) * 100}%`,
                            width: `${Math.max(
                              0.8,
                              (workingWidth(
                                parseDay(task.baseline_start_at),
                                task.baseline_duration_minutes,
                                calendar,
                                1,
                              ) /
                                span) *
                                100,
                            )}%`,
                            background: "var(--m-input)",
                          }}
                        />
                      ) : null}
                    </div>
                  </li>
                );
              })}
            </ul>
          </section>
        );
      })}
    </div>
  );
}

// --- floating surfaces -------------------------------------------------------

function FloatingCard({ children }: { children: React.ReactNode }): React.JSX.Element {
  return (
    <div
      role="tooltip"
      style={{
        marginTop: 10,
        padding: "8px 10px",
        borderRadius: 10,
        background: "var(--m-bar)",
        color: "var(--m-cream)",
        fontSize: 12,
        lineHeight: "17px",
        maxWidth: 520,
        boxShadow: "var(--m-shadow-strong)",
        display: "grid",
        gap: 2,
      }}
    >
      {children}
    </div>
  );
}

function BlockedNotice({
  message,
  onDismiss,
  onSetDates,
}: {
  message: string;
  onDismiss: () => void;
  onSetDates?: () => void;
}): React.JSX.Element {
  return (
    <div
      role="alert"
      data-testid="gantt-blocked"
      style={{
        marginTop: 10,
        padding: "8px 10px",
        borderRadius: 10,
        border: "1px solid var(--m-status-danger-fg)",
        background: "var(--m-status-danger-bg)",
        color: "var(--m-status-danger-fg)",
        fontSize: 12,
        lineHeight: "17px",
        display: "flex",
        alignItems: "center",
        gap: 10,
      }}
    >
      <Icon name="alert" size={13} aria-label="Drop refused" />
      <span style={{ flex: 1 }}>{message}</span>
      {onSetDates === undefined ? null : (
        <Button size="sm" variant="ghost" onClick={onSetDates}>
          Set dates
        </Button>
      )}
      <Button size="sm" variant="ghost" onClick={onDismiss}>
        Dismiss
      </Button>
    </div>
  );
}

/** The one keyframe pair this component owns: 120ms on transform and opacity,
 * collapsed entirely under reduced motion. */
function GanttMotionStyles(): React.JSX.Element {
  return (
    <style>{`
      [data-slot="gantt-chart"] [data-testid^="gantt-bar-"] { transition: transform 120ms cubic-bezier(0.2,0,0,1), opacity 120ms linear; }
      @media (prefers-reduced-motion: reduce) {
        [data-slot="gantt-chart"] [data-testid^="gantt-bar-"] { transition: none; }
      }
    `}</style>
  );
}

function Kbd({ children }: { children: React.ReactNode }): React.JSX.Element {
  return (
    <kbd
      style={{
        fontFamily: "var(--m-font-mono)",
        fontSize: 10,
        lineHeight: "14px",
        padding: "0 4px",
        borderRadius: 4,
        border: "1px solid var(--m-border)",
        color: "var(--m-text-muted)",
        background: "var(--m-surface-2)",
      }}
    >
      {children}
    </kbd>
  );
}

/** The small square before a task name: where the work happens. */
function WorkTypeMark({ type }: { type: string | null }): React.JSX.Element | null {
  if (type === null) return null;
  const meta = WORK_TYPE[type];
  if (meta === undefined) return null;
  return (
    <span
      role="img"
      aria-label={meta.label}
      title={meta.label}
      style={{
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        width: 20,
        height: 20,
        flexShrink: 0,
        borderRadius: 6,
        border: "1px solid var(--m-border)",
        background: "var(--m-surface-2)",
        color: "var(--m-text-secondary)",
      }}
    >
      <Icon name={meta.icon} size={12} />
    </span>
  );
}

// --- responsive --------------------------------------------------------------

interface ResponsiveFormat {
  mode: "chart" | "list";
  dayWidth: number;
  treeWidth: number;
}

/** Day width and tree width from the pack's three breakpoints, and the list
 * below 1024. Server render uses the 1440 numbers; the first effect corrects
 * it before paint on a narrower window. */
function useResponsiveFormat(): ResponsiveFormat {
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const read = (): void => setWidth(window.innerWidth);
    read();
    window.addEventListener("resize", read);
    return () => window.removeEventListener("resize", read);
  }, []);
  if (width === 0) return { mode: "chart", dayWidth: 16, treeWidth: 408 };
  if (width < 1024) return { mode: "list", dayWidth: 10, treeWidth: 300 };
  if (width >= 1440) return { mode: "chart", dayWidth: 16, treeWidth: 408 };
  if (width >= 1280) return { mode: "chart", dayWidth: 13, treeWidth: 360 };
  return { mode: "chart", dayWidth: 10, treeWidth: 300 };
}

// --- rows -------------------------------------------------------------------

interface TaskRow {
  kind: "task";
  task: GanttTask;
}
interface PhaseRow {
  kind: "phase";
  phase: GanttPhase;
  tasks: GanttTask[];
}
type Row = TaskRow | PhaseRow;

function buildRows(
  phases: GanttPhase[],
  tasks: GanttTask[],
  collapsed: ReadonlySet<string>,
): Row[] {
  const rows: Row[] = [];
  const phaseIds = new Set(phases.map((phase) => phase.id));
  for (const phase of phases) {
    const inPhase = tasks.filter((task) => task.phase_id === phase.id);
    rows.push({ kind: "phase", phase, tasks: inPhase });
    if (!collapsed.has(phase.id)) {
      for (const task of inPhase) rows.push({ kind: "task", task });
    }
  }
  const orphans = tasks.filter(
    (task) => task.phase_id === null || !phaseIds.has(task.phase_id),
  );
  if (orphans.length > 0) {
    rows.push({
      kind: "phase",
      phase: {
        id: "__none__",
        name: "No phase",
        start_at: null,
        end_at: null,
        status: "pending",
        is_milestone: false,
      },
      tasks: orphans,
    });
    if (!collapsed.has("__none__")) {
      for (const task of orphans) rows.push({ kind: "task", task });
    }
  }
  return rows;
}

// --- derived values ----------------------------------------------------------

interface DragState {
  taskId: string;
  startMs: number;
  nextMs: number;
  deltas: number;
  valid: boolean;
  message: string | null;
}

/** The status a bar is drawn in. Due and overdue come from the task's own due
 * date, not from colour: the reason is always available as words. */
function toneOf(task: GanttTask, todayMs: number | null): BarTone {
  if (task.status === "completed") return "done";
  if (task.status === "cancelled") return "hold";
  if (task.status === "blocked") return "wait";
  if (task.status === "in_progress") return "live";
  if (task.due_at !== null && todayMs !== null) {
    const due = parseDay(task.due_at);
    if (due < todayMs) return "over";
    if (due - todayMs <= DAYS_DUE_SOON * MS_PER_DAY) return "due";
  }
  return "planned";
}

/** The span a phase draws: its own dates, else the span of its tasks. */
function phaseSpan(
  phase: GanttPhase,
  tasks: GanttTask[],
  calendar: GanttCalendar,
): { from: number | null; to: number | null; derived: boolean } {
  if (phase.start_at !== null && phase.end_at !== null) {
    return { from: parseDay(phase.start_at), to: parseDay(phase.end_at), derived: false };
  }
  const starts: number[] = [];
  const ends: number[] = [];
  for (const task of tasks) {
    if (task.start_at === null || task.duration_minutes === null) continue;
    const start = parseDay(task.start_at);
    starts.push(start);
    ends.push(addWorkingMinutes(start, task.duration_minutes, calendar));
  }
  if (starts.length === 0) return { from: null, to: null, derived: false };
  return { from: Math.min(...starts), to: Math.max(...ends), derived: true };
}

/** Hours logged against the phase's planned effort, with the money-budget
 * alert as the fallback when no effort estimate exists. */
function phaseBurn(
  phase: GanttPhase,
  tasks: GanttTask[],
): { alert: "warn" | "over" | null; label: string; tone: string } {
  const estimated = tasks.reduce((sum, task) => sum + (task.effort_estimate_minutes ?? 0), 0);
  const planned = estimated > 0 ? estimated : null;
  const logged = phase.logged_minutes ?? null;
  if (planned !== null && logged !== null) {
    const pct = Math.round((logged / planned) * 100);
    const alert = pct > 100 ? "over" : pct >= 80 ? "warn" : null;
    return {
      alert,
      label: `${Math.round(logged / 60)}/${Math.round(planned / 60)} h`,
      tone:
        alert === "over"
          ? "var(--m-status-danger-fg)"
          : alert === "warn"
            ? "var(--m-status-warning-fg)"
            : "var(--m-text-muted)",
    };
  }
  if (phase.budget_alert === "warn" || phase.budget_alert === "over") {
    return {
      alert: phase.budget_alert,
      label: phase.budget_alert === "over" ? "Over budget" : "Near budget",
      tone:
        phase.budget_alert === "over"
          ? "var(--m-status-danger-fg)"
          : "var(--m-status-warning-fg)",
    };
  }
  return { alert: null, label: "", tone: "var(--m-text-muted)" };
}

/** What the assignee slot of a bar says: the person, else the vendor, else the
 * role the work needs. Never invented. */
function barLabel(task: GanttTask): string | null {
  const name = task.assignee_name ?? task.external_owner;
  if (name !== null && name !== "") return firstName(name);
  if (task.role_key !== null && task.role_key !== "") return task.role_key.replaceAll("_", " ");
  return null;
}

/**
 * Where the name of a bar too narrow to hold it goes: to its right, or to its
 * left when the bar ends near the chart's edge, or nowhere. A clipped half-word
 * reads as a bug, so the name is omitted rather than sliced.
 */
function bareNamePlacement(
  name: string | null,
  left: number,
  width: number,
  chartWidth: number,
  inlineMin: number,
): { name: string; style: React.CSSProperties } | null {
  if (name === null || width >= inlineMin) return null;
  // 11px Figtree averages about 5.6px a glyph; 8px of air either side.
  const estimate = name.length * 5.6 + 6;
  const base: React.CSSProperties = {
    position: "absolute",
    top: ROW_H / 2 - 8,
    maxWidth: 110,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    fontSize: 11,
    color: "var(--m-text-secondary)",
    pointerEvents: "none",
  };
  if (left + width + estimate + 4 <= chartWidth) {
    return { name, style: { ...base, left: left + width + 6 } };
  }
  if (left - estimate - 4 >= 0) {
    return {
      name,
      style: { ...base, left: Math.max(0, left - estimate - 2), textAlign: "right", width: estimate },
    };
  }
  return null;
}

function firstName(name: string): string {
  const trimmed = name.trim();
  const space = trimmed.indexOf(" ");
  return space === -1 ? trimmed : trimmed.slice(0, space);
}

/** The earliest day a task may start: the next working day after its latest
 * unfinished predecessor finishes. */
function predecessorGuard(
  task: GanttTask,
  taskById: Map<string, GanttTask>,
  calendar: GanttCalendar,
): { earliest: number; message: string } | null {
  let earliest: number | null = null;
  let holder: GanttTask | null = null;
  let holderEnd: number | null = null;
  for (const depId of task.depends_on) {
    const dep = taskById.get(depId);
    if (dep === undefined || dep.status === "completed" || dep.status === "cancelled") continue;
    if (dep.start_at === null || dep.duration_minutes === null) continue;
    const end = addWorkingMinutes(parseDay(dep.start_at), dep.duration_minutes, calendar);
    const allowed = nextWorkingDay(end, calendar);
    if (earliest === null || allowed > earliest) {
      earliest = allowed;
      holder = dep;
      holderEnd = end;
    }
  }
  if (earliest === null || holder === null || holderEnd === null) return null;
  return {
    earliest,
    message: `Can't start before ${formatDayWords(earliest)}. ${holder.title} finishes ${formatDayWords(holderEnd)} and holds this date.`,
  };
}

function blockedEarliest(
  task: GanttTask,
  taskById: Map<string, GanttTask>,
  calendar: GanttCalendar,
  fallback: number,
): number {
  const guard = predecessorGuard(task, taskById, calendar);
  return guard === null ? fallback : guard.earliest;
}

function daysInWeek(calendar: GanttCalendar): number {
  let count = 0;
  for (let index = 0; index < 7; index += 1) {
    if ((calendar.weekdays & (1 << index)) !== 0) count += 1;
  }
  return Math.max(1, count);
}

/** Width in pixels of a working-minute duration, counting the working days it
 * spans so a bar over a weekend stops at Friday, not Sunday. */
function workingWidth(
  startMs: number,
  minutes: number,
  calendar: GanttCalendar,
  dayWidth: number,
): number {
  const end = addWorkingMinutes(startMs, minutes, calendar);
  const days = Math.max(1, workingDaysBetween(startMs, end, calendar));
  return days * dayWidth;
}

function effectiveStart(
  task: GanttTask,
  preview: Map<string, GanttPlanChange>,
): number | null {
  const change = preview.get(task.id);
  const value = change !== undefined && change.toStart !== null ? change.toStart : task.start_at;
  return value === null ? null : parseDay(value);
}

function dragTooltip(
  task: GanttTask,
  drag: DragState,
  calendar: GanttCalendar,
  nodes: GanttPlanNode[],
): React.ReactNode {
  const newStart = drag.nextMs;
  const end =
    task.duration_minutes === null
      ? null
      : addWorkingMinutes(newStart, task.duration_minutes, calendar);
  const minutes = task.duration_minutes ?? 0;
  const changed = simulateMove(nodes, calendar, task.id, newStart);
  const downstream = changed.filter((change) => change.reason === "pushed").length;
  const previous = task.start_at === null ? null : parseDay(task.start_at);
  const shift = previous === null ? 0 : workingDayOffset(previous, newStart, calendar);
  return (
    <>
      <strong style={{ fontWeight: 600 }}>
        {formatDayWords(newStart)} to {end === null ? "—" : formatDayWords(end)}
        {" · "}
        {durationInWords(minutes, calendar)}
      </strong>
      <span>
        {drag.valid ? "Drop here" : drag.message ?? "This position is refused"}
        {previous !== null && shift !== 0
          ? ` · ${Math.abs(shift)} working ${Math.abs(shift) === 1 ? "day" : "days"} ${
              shift > 0 ? "later" : "earlier"
            }`
          : ""}
      </span>
      <span>
        {downstream === 0
          ? "Nothing else moves"
          : `${downstream} downstream ${downstream === 1 ? "task moves" : "tasks move"}`}
      </span>
    </>
  );
}

function taskTooltip(
  task: GanttTask,
  tone: BarTone,
  calendar: GanttCalendar,
  baselineLabel: string | null,
  showBaseline: boolean,
): React.ReactNode {
  const start = task.start_at === null ? null : parseDay(task.start_at);
  const end =
    start !== null && task.duration_minutes !== null
      ? addWorkingMinutes(start, task.duration_minutes, calendar)
      : null;
  const statusWords = STATUS_WORDS[task.status] ?? task.status.replaceAll("_", " ");
  const who = task.assignee_name ?? task.external_owner ?? task.role_key ?? "Unassigned";
  const baselineStart = task.baseline_start_at === null ? null : parseDay(task.baseline_start_at);
  let variance: string | null = null;
  if (showBaseline && baselineStart !== null && start !== null) {
    variance = `${varianceInWords(
      workingDayOffset(baselineStart, start, calendar),
    )} than ${baselineLabel ?? "the baseline"}`;
  }
  return (
    <>
      <strong style={{ fontWeight: 600 }}>
        {task.title}
        {start === null
          ? " · not dated"
          : ` · ${formatDayWords(start)} to ${end === null ? "—" : formatDayWords(end)}`}
      </strong>
      <span>
        {statusWords} · {who}
        {task.work_type === null ? "" : ` · ${WORK_TYPE[task.work_type]?.label ?? task.work_type}`}
        {tone === "over" ? " · past its due date" : tone === "due" ? " · due within three days" : ""}
      </span>
      {variance !== null && baselineStart !== null && start !== null ? (
        <span>
          Baseline: planned {formatDayWords(baselineStart)}, now {formatDayWords(start)} — {variance}.
        </span>
      ) : null}
    </>
  );
}

const treeDateCell: React.CSSProperties = {
  padding: "0 10px",
  fontFamily: "var(--m-font-mono)",
  fontSize: 12,
  fontVariantNumeric: "tabular-nums",
  color: "var(--m-text-secondary)",
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};
