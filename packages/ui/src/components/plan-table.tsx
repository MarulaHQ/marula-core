"use client";
/**
 * PlanTable — the work plan's grid: phase rows and task rows on one column
 * template, with the pack's column widths and a burn strip per phase.
 *
 * Purpose: the project plan (surface pack S3) and the task list (S5) are the
 * same grid seen twice, so the template, the row states and the burn strip live
 * here rather than in each surface. The column numbers are the designer's
 * measured values, not approximations:
 *
 *   tasks 36 | 1fr | 122 status | 172 who | 76 start | 52 days | 60 effort | 150 depends | 36 actions
 *   phase 36 | 1fr | 240 range | 190 burn | 36 actions
 *
 * The phase row uses its own template on purpose. Its date range and burn strip
 * right-align against the task columns rather than sitting in them, which is
 * what the reference draws and what makes a phase read as a summary instead of
 * a task with empty cells.
 *
 * It is a grid, not a `<table>`, because every row has its own column template
 * (the phase row's differs) and a `<table>` cannot express that without
 * duplicating cell spans. The ARIA grid roles are carried explicitly, so a
 * screen reader gets the same structure a `<table>` would provide.
 *
 * Non-purpose: no sorting, no selection state, no data. A surface owns its rows
 * and passes callbacks; this file owns the geometry, the row states and the
 * marks a row is drawn with.
 */
import type { CSSProperties, HTMLAttributes, ReactNode } from "react";
import { cn } from "../lib/cn";
import { Icon, type IconName } from "../kit";

/** The task-row template, measured at 1440 comfortable density. */
export const PLAN_COLUMNS = "36px minmax(0,1fr) 122px 172px 76px 52px 60px 150px 36px";
/** At 1280 the dependency column drops and the person column tightens. */
export const PLAN_COLUMNS_NARROW = "36px minmax(0,1fr) 122px 150px 76px 48px 62px 36px";
/** The phase-cell template: range and burn are their own measures. */
export const PLAN_PHASE_COLUMNS = "36px minmax(0,1fr) 240px 190px 36px";
export const PLAN_PHASE_COLUMNS_NARROW = "36px minmax(0,1fr) 170px 150px 36px";

/** The plan's work types (pack S3.1; the icon carries the words as a title). */
export type WorkType = "onsite" | "bench" | "remote" | "vendor" | "client" | "coordination";

export const WORK_TYPES: Record<WorkType, { icon: IconName; label: string }> = {
  onsite: { icon: "pin", label: "On site" },
  bench: { icon: "wrench", label: "Bench, in the workshop" },
  remote: { icon: "monitor", label: "Remote" },
  vendor: { icon: "box", label: "Vendor" },
  client: { icon: "user", label: "Client action" },
  coordination: { icon: "chat", label: "Coordination" },
};

export function PlanTable({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      role="grid"
      data-slot="plan-table"
      className={cn(
        "relative min-w-0 overflow-hidden rounded-card border border-border bg-card",
        className,
      )}
      {...props}
    />
  );
}

export function PlanHeaderRow({ columns, children }: { columns: string; children: ReactNode }) {
  return (
    <div
      role="row"
      data-slot="plan-header"
      style={{ gridTemplateColumns: columns }}
      className="relative grid min-h-hdr items-center border-b border-border bg-card text-[12px] font-semibold uppercase tracking-[0.06em] text-muted-foreground [&>*]:min-w-0 [&>*]:overflow-hidden [&>*]:px-[var(--m-density-px)] [&>*]:text-ellipsis [&>*]:whitespace-nowrap"
    >
      {children}
    </div>
  );
}

/** `flash` marks a row a preview would move. `new` is the inline create row. */
export type PlanRowState = "flash" | "new" | "dimmed";

const ROW_STATE: Record<PlanRowState, string> = {
  flash: "bg-mc-act hover:bg-mc-act",
  new: "bg-mc-act",
  dimmed: "text-muted-foreground",
};

export function PlanRow({
  columns,
  state,
  selected,
  current,
  className,
  children,
  ...props
}: HTMLAttributes<HTMLDivElement> & {
  columns: string;
  state?: PlanRowState;
  selected?: boolean;
  current?: boolean;
}) {
  return (
    <div
      role="row"
      data-slot="plan-row"
      data-selected={selected ? "true" : undefined}
      data-current={current ? "true" : undefined}
      style={{ gridTemplateColumns: columns }}
      className={cn(
        "relative grid min-h-row items-center border-b border-border text-[length:var(--m-density-fs)] last:border-b-0",
        "group hover:bg-mc-surface-2",
        // A reorder in flight: the row being carried fades, and the row under
        // the pointer takes a gold edge on the half the drop will land on.
        // `data-drop` and `data-carrying` are set by the consumer; the marks
        // live here so no consumer restyles the row to draw them.
        "data-[carrying=true]:opacity-60",
        "data-[drop=before]:shadow-[inset_0_2px_0_var(--m-gold)]",
        "data-[drop=after]:shadow-[inset_0_-2px_0_var(--m-gold)]",
        state && ROW_STATE[state],
        selected && "bg-mc-act shadow-[inset_3px_0_0_var(--m-gold)] hover:bg-mc-act",
        current &&
          (selected
            ? "shadow-[inset_3px_0_0_var(--m-gold),inset_0_0_0_2px_var(--m-focus-ring)]"
            : "shadow-[inset_0_0_0_2px_var(--m-focus-ring)]"),
        "[&>*]:min-w-0 [&>*]:overflow-hidden [&>*]:px-[var(--m-density-px)] [&>*]:text-ellipsis [&>*]:whitespace-nowrap",
        className,
      )}
      {...props}
    >
      {children}
    </div>
  );
}

export function PlanPhaseRow({
  columns,
  tone = "default",
  className,
  children,
  ...props
}: Omit<HTMLAttributes<HTMLDivElement>, "children"> & {
  columns: string;
  /** `muted` is the undated group's summary: a dashed break from the plan. */
  tone?: "default" | "muted";
  children: ReactNode;
}) {
  return (
    <div
      role="row"
      data-slot="plan-phase-row"
      style={{ gridTemplateColumns: columns }}
      className={cn(
        "group relative grid min-h-row items-center border-b border-border bg-background font-semibold hover:bg-background",
        "text-[length:var(--m-density-fs)] last:border-b-0",
        "data-[carrying=true]:opacity-60",
        "data-[drop=before]:shadow-[inset_0_2px_0_var(--m-gold)]",
        "data-[drop=after]:shadow-[inset_0_-2px_0_var(--m-gold)]",
        tone === "muted" && "border-t border-dashed border-input bg-muted hover:bg-muted",
        "[&>*]:min-w-0 [&>*]:overflow-hidden [&>*]:px-[var(--m-density-px)] [&>*]:text-ellipsis [&>*]:whitespace-nowrap",
        className,
      )}
      {...props}
    >
      {children}
    </div>
  );
}

export function PlanCell({
  right,
  flushLeft,
  className,
  style,
  children,
  ...props
}: HTMLAttributes<HTMLDivElement> & {
  right?: boolean;
  /** Drop the cell's leading inset when the row already indents its content. */
  flushLeft?: boolean;
}) {
  return (
    <div
      role="gridcell"
      className={cn("flex items-center gap-2", right && "justify-end", flushLeft && "pl-0!", className)}
      style={style}
      {...props}
    >
      {children}
    </div>
  );
}

export function PlanHeaderCell({
  right,
  className,
  children,
  ...props
}: HTMLAttributes<HTMLDivElement> & { right?: boolean }) {
  return (
    <div
      role="columnheader"
      className={cn("flex h-full items-center gap-2", right && "justify-end", className)}
      {...props}
    >
      {children}
    </div>
  );
}

export function PlanCheckCell({
  align = "center",
  className,
  ...props
}: HTMLAttributes<HTMLDivElement> & {
  /** `start` is the phase row's disclosure cell: checkbox, then the chevron. */
  align?: "center" | "start";
}) {
  return (
    <div
      role="gridcell"
      className={cn(
        "flex items-center",
        align === "center"
          ? "justify-center px-0"
          : "gap-1 justify-start px-[var(--m-density-px)]",
        className,
      )}
      {...props}
    />
  );
}

/**
 * The phase's burn strip: hours logged against the phase's hours. Three pixels,
 * under the summary, with the number beside it. Tone follows the pack's
 * thresholds (warning past 80%, danger past 100%) and never replaces the words:
 * the fraction is always printed.
 */
export function BurnStrip({
  usedHours,
  totalHours,
  label,
  className,
}: {
  usedHours: number;
  totalHours: number;
  /** The accessible name; defaults to the two numbers. */
  label?: string;
  className?: string;
}) {
  const pct = totalHours > 0 ? Math.round((usedHours / totalHours) * 100) : 0;
  const tone =
    pct >= 100
      ? "var(--m-status-danger-fg)"
      : pct >= 80
        ? "var(--m-status-warning-fg)"
        : "var(--mc-stem)";
  return (
    <span className={cn("flex min-w-0 items-center gap-2", className)}>
      <span
        role="meter"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.min(100, Math.max(0, pct))}
        aria-label={label ?? `${usedHours} of ${totalHours} hours logged`}
        className="block h-[3px] min-w-[40px] flex-1 overflow-hidden rounded-pill"
        style={{ background: "var(--m-border)" }}
      >
        <span
          className="block h-full rounded-pill"
          style={{ width: `${Math.min(100, Math.max(0, pct))}%`, background: tone }}
        />
      </span>
      <span className="whitespace-nowrap font-mono text-[12px] tabular-nums text-muted-foreground">
        {usedHours}/{totalHours} h
      </span>
    </span>
  );
}

/** The small square before a task name: where the work happens. */
export function WorkTypeMark({ type, className }: { type: WorkType; className?: string }) {
  const meta = WORK_TYPES[type];
  return (
    <span
      role="img"
      aria-label={meta.label}
      title={meta.label}
      className={cn(
        "inline-flex size-[22px] shrink-0 items-center justify-center rounded-[6px] border border-border bg-card text-muted-foreground",
        className,
      )}
    >
      <Icon name={meta.icon} size={13} />
    </span>
  );
}

/** A milestone mark for a row: ink, green once reached. */
export function Diamond({ tone = "default", className }: { tone?: "default" | "done"; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "inline-block size-[11px] shrink-0 rounded-[2px] [transform:rotate(45deg)]",
        tone === "done" ? "bg-mc-green" : "bg-foreground",
        className,
      )}
    />
  );
}

/** The row-level service the plan reads: keep the style object explicit so a
 *  caller can add one property without discarding the template. */
export function planRowStyle(columns: string, extra?: CSSProperties): CSSProperties {
  return { gridTemplateColumns: columns, ...extra };
}
