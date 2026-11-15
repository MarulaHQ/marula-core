"use client";
/**
 * GanttTable — the plan as a table: phase, task, start, end, days, who,
 * depends on, status.
 *
 * This is the non-visual equivalent of the chart (S4.10), not a secondary
 * screen: a real `<table>` with a caption and scoped headers, so a screen
 * reader, a keyboard user and a printout all read the same plan the bars
 * show. It also carries the one thing a chart cannot: the reason a date is
 * what it is, in words, in a cell.
 *
 * "Copy as text" writes the same rows as tab-separated text, because the
 * common next step is pasting the plan into a mail or a ticket.
 */
import { useState } from "react";
import { Button } from "./button";
import {
  STANDARD_CALENDAR,
  addWorkingMinutes,
  formatDayShort,
  formatDayWords,
  parseDay,
  varianceInWords,
  workingDayOffset,
  workingDaysBetween,
  type GanttCalendar,
} from "./gantt-scale";
import type { GanttPhase, GanttTask } from "./gantt-chart";

const STATUS_WORDS: Record<string, string> = {
  pending: "Not started",
  in_progress: "In progress",
  blocked: "Waiting",
  completed: "Done",
  cancelled: "Cancelled",
};

export interface GanttTableProps {
  phases: GanttPhase[];
  tasks: GanttTask[];
  calendar?: GanttCalendar | null;
  criticalPath?: string[] | null;
  showBaseline?: boolean;
  baselineLabel?: string | null;
  /** Opens the set-dates form for a row, the keyboard route from the table. */
  onRequestSetDates?: (taskId: string) => void;
  /** Shown as a quiet line under the caption; the surface's own baseline note. */
  note?: string | null;
}

export function GanttTable({
  phases,
  tasks,
  calendar: calendarProp = null,
  criticalPath = null,
  showBaseline = false,
  baselineLabel = null,
  onRequestSetDates,
  note = null,
}: GanttTableProps): React.JSX.Element {
  const calendar = calendarProp ?? STANDARD_CALENDAR;
  const [copied, setCopied] = useState(false);
  const criticalSet = new Set(criticalPath ?? []);
  const taskById = new Map(tasks.map((task) => [task.id, task]));

  // One row per task, grouped by phase in the plan's order. A task whose phase
  // is gone (or absent) still appears; hiding it would be losing data.
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

  const rows = groups.flatMap((group) =>
    group.tasks.map((task) => ({ phase: group.phase, task })),
  );

  const copy = async (): Promise<void> => {
    const lines = [
      ["Phase", "Task", "Start", "End", "Days", "Who", "Depends on", "Status"].join("\t"),
      ...rows.map(({ phase, task }) => {
        const start = task.start_at === null ? null : parseDay(task.start_at);
        const end =
          start !== null && task.duration_minutes !== null
            ? addWorkingMinutes(start, task.duration_minutes, calendar)
            : null;
        return [
          phase.name,
          task.title,
          start === null ? "Not dated" : formatDayWords(start),
          end === null ? "—" : formatDayWords(end),
          start === null || end === null ? "" : String(Math.max(1, workingDaysBetween(start, end, calendar))),
          task.assignee_name ?? task.external_owner ?? task.role_key ?? "Unassigned",
          task.depends_on
            .map((id) => taskById.get(id)?.title ?? "")
            .filter((title) => title !== "")
            .join("; "),
          STATUS_WORDS[task.status] ?? task.status,
        ].join("\t");
      }),
    ];
    try {
      await navigator.clipboard.writeText(lines.join("\n"));
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      setCopied(false);
    }
  };

  return (
    <div data-slot="gantt-table" data-testid="gantt-table" style={{ display: "grid", gap: 10 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <span style={{ fontSize: 12, color: "var(--m-text-secondary)" }}>
          {rows.length} {rows.length === 1 ? "task" : "tasks"} in the plan, in phase order
        </span>
        <span style={{ marginLeft: "auto", display: "flex", gap: 8 }}>
          <Button size="sm" variant="secondary" onClick={() => void copy()}>
            {copied ? "Copied" : "Copy as text"}
          </Button>
        </span>
      </div>
      <div
        style={{
          border: "1px solid var(--m-border)",
          borderRadius: 16,
          overflow: "auto",
          background: "var(--m-surface)",
        }}
      >
        <table
          style={{
            width: "100%",
            borderCollapse: "collapse",
            fontSize: 13,
            minWidth: 900,
          }}
        >
          <caption
            style={{
              captionSide: "bottom",
              textAlign: "left",
              padding: "10px 14px",
              fontSize: 11,
              color: "var(--m-text-muted)",
              borderTop: "1px solid var(--m-border)",
            }}
          >
            {note ??
              "The same plan the chart draws. Days are working days from the project's calendar."}
          </caption>
          <thead>
            <tr style={{ background: "var(--m-canvas)" }}>
              {[
                { label: "Phase", align: "left" as const },
                { label: "Task", align: "left" as const },
                { label: "Start", align: "left" as const },
                { label: "End", align: "left" as const },
                { label: "Days", align: "right" as const },
                { label: "Who", align: "left" as const },
                { label: "Depends on", align: "left" as const },
                { label: "Status", align: "left" as const },
              ].map((column) => (
                <th
                  key={column.label}
                  scope="col"
                  style={{
                    textAlign: column.align,
                    padding: "12px 14px",
                    fontSize: 11,
                    fontWeight: 600,
                    letterSpacing: "0.06em",
                    textTransform: "uppercase",
                    color: "var(--m-text-muted)",
                    borderBottom: "1px solid var(--m-border)",
                    whiteSpace: "nowrap",
                  }}
                >
                  {column.label}
                </th>
              ))}
            </tr>
          </thead>
          {groups.map((group) => {
            if (group.tasks.length === 0) return null;
            return (
              <tbody key={group.phase.id}>
                <tr>
                  <th
                    scope="rowgroup"
                    colSpan={8}
                    style={{
                      textAlign: "left",
                      padding: "8px 14px",
                      fontSize: 12,
                      fontWeight: 600,
                      background: "var(--m-surface-2)",
                      borderBottom: "1px solid var(--m-border)",
                    }}
                  >
                    {group.phase.name}
                    <span style={{ marginLeft: 8, fontWeight: 400, color: "var(--m-text-muted)" }}>
                      {group.tasks.length} {group.tasks.length === 1 ? "task" : "tasks"}
                    </span>
                  </th>
                </tr>
                {group.tasks.map((task) => {
                  const start = task.start_at === null ? null : parseDay(task.start_at);
                  const end =
                    start !== null && task.duration_minutes !== null
                      ? addWorkingMinutes(start, task.duration_minutes, calendar)
                      : null;
                  const days =
                    start === null || end === null
                      ? null
                      : Math.max(1, workingDaysBetween(start, end, calendar));
                  const baselineStart =
                    task.baseline_start_at === null ? null : parseDay(task.baseline_start_at);
                  const variance =
                    showBaseline && baselineStart !== null && start !== null
                      ? workingDayOffset(baselineStart, start, calendar)
                      : null;
                  const dependsOn = task.depends_on
                    .map((id) => taskById.get(id)?.title ?? "")
                    .filter((title) => title !== "");
                  const status = STATUS_WORDS[task.status] ?? task.status;
                  return (
                    <tr
                      key={task.id}
                      data-testid={`gantt-table-row-${task.id}`}
                      style={{ borderBottom: "1px solid var(--m-surface-2)" }}
                    >
                      <td style={cell}>
                        <span style={{ color: "var(--m-text-muted)", fontSize: 12 }}>{group.phase.name}</span>
                      </td>
                      <th scope="row" style={{ ...cell, textAlign: "left", fontWeight: 500 }}>
                        <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
                          {task.title}
                          {criticalSet.has(task.id) ? (
                            <span
                              style={{
                                fontSize: 10,
                                fontWeight: 700,
                                letterSpacing: "0.04em",
                                textTransform: "uppercase",
                              }}
                            >
                              Critical
                            </span>
                          ) : null}
                        </span>
                        {onRequestSetDates === undefined ? null : (
                          <button
                            type="button"
                            onClick={() => onRequestSetDates(task.id)}
                            style={{
                              marginLeft: 8,
                              border: "none",
                              background: "transparent",
                              padding: 0,
                              font: "inherit",
                              fontSize: 11,
                              color: "var(--m-text-muted)",
                              textDecoration: "underline",
                              textUnderlineOffset: 2,
                              cursor: "pointer",
                            }}
                          >
                            Set dates
                          </button>
                        )}
                      </th>
                      <td style={{ ...cell, ...mono }}>
                        {start === null ? (
                          <span style={{ color: "var(--m-text-muted)" }}>Not dated</span>
                        ) : (
                          formatDayShort(start)
                        )}
                      </td>
                      <td style={{ ...cell, ...mono }}>
                        {end === null ? "—" : formatDayShort(end)}
                        {variance !== null && variance !== 0 ? (
                          <span
                            style={{
                              display: "block",
                              fontSize: 10,
                              color:
                                variance > 0
                                  ? "var(--m-status-warning-fg)"
                                  : "var(--m-status-success-fg)",
                            }}
                          >
                            {varianceInWords(variance)} than {baselineLabel ?? "baseline"}
                          </span>
                        ) : null}
                      </td>
                      <td style={{ ...cell, ...mono, textAlign: "right" }}>{days ?? "—"}</td>
                      <td style={cell}>
                        {task.assignee_name ?? task.external_owner ?? task.role_key ?? "Unassigned"}
                      </td>
                      <td style={{ ...cell, color: "var(--m-text-secondary)" }}>
                        {dependsOn.length === 0 ? "—" : dependsOn.join("; ")}
                      </td>
                      <td style={cell}>{status}</td>
                    </tr>
                  );
                })}
              </tbody>
            );
          })}
        </table>
      </div>
      <span style={{ fontSize: 11, color: "var(--m-text-muted)" }}>
        {rows.length === 0
          ? "No tasks in this project yet."
          : `${rows.length} rows shown. Dates are read in UTC, the same day the chart draws.`}
      </span>
    </div>
  );
}

const cell: React.CSSProperties = {
  padding: "12px 14px",
  verticalAlign: "top",
  borderBottom: "1px solid var(--m-surface-2)",
};

const mono: React.CSSProperties = {
  fontFamily: "var(--m-font-mono)",
  fontSize: 12,
  fontVariantNumeric: "tabular-nums",
  whiteSpace: "nowrap",
};
