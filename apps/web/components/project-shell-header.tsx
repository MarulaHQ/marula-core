"use client";
/**
 * ProjectShellHeader — the project record's frame (surface pack S2.1).
 *
 * Purpose: the header holds identity and posture and stays put while the tabs
 * change, so the record never feels like it moved. It carries, in the pack's
 * order: the breadcrumb, the title, the meta row (status, client, billing, the
 * people and the PM), the figures a PM reads first (due and working days, hours
 * used against hours planned, budget), one primary action, the tab strip, and
 * the commitment bar underneath.
 *
 * The figures are `Kpi` slots rather than fixed fields because the record draws
 * different numbers per surface (S2.7 redacts the budget for a technician), and
 * a component that hard-codes three would have to grow a permission flag to do
 * the same job.
 *
 * Non-purpose: no fetching, no state, no tabs of its own. The page owns the
 * project and decides which tab is open; this file owns the geometry the pack
 * measured (56px of header block, 40px tabs, 38px commitment bar).
 */
import type { ReactNode } from "react";
import { cn } from "@marula-core/ui";

export interface ShellTab {
  id: string;
  label: string;
  count?: number;
  /** The test id the existing record tests read. */
  testId?: string;
}

export function ProjectShellHeader({
  breadcrumb,
  title,
  meta,
  stats,
  actions,
  tabs,
  activeTab,
  onTab,
  commitment,
  className,
}: {
  breadcrumb?: ReactNode;
  title: ReactNode;
  meta?: ReactNode;
  stats?: ReactNode;
  actions?: ReactNode;
  tabs: readonly ShellTab[];
  activeTab: string;
  onTab: (id: string) => void;
  commitment?: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("bg-[var(--m-canvas)]", className)}>
      <div className="px-6 pt-[14px]">
        {breadcrumb === undefined ? null : (
          <div className="flex items-center gap-[6px] text-[13px] text-muted-foreground">
            {breadcrumb}
          </div>
        )}
        <div className="mt-[6px] flex flex-wrap items-start gap-5">
          <div className="min-w-0 flex-1">
            <h1 className="m-0 text-[27px] font-semibold leading-[1.2] tracking-[-0.02em]">
              {title}
            </h1>
            {meta === undefined ? null : (
              <div className="mt-[6px] flex flex-wrap items-center gap-x-[10px] gap-y-1 text-[13px] text-muted-foreground">
                {meta}
              </div>
            )}
          </div>
          <div className="flex shrink-0 flex-wrap items-start gap-[22px]">
            {stats}
            {actions}
          </div>
        </div>
        <div role="tablist" className="mt-3 flex gap-[2px]">
          {tabs.map((item) => {
            const current = item.id === activeTab;
            return (
              <button
                key={item.id}
                type="button"
                role="tab"
                aria-selected={current}
                data-testid={item.testId}
                onClick={() => onTab(item.id)}
                className={cn(
                  "inline-flex h-10 cursor-pointer items-center gap-2 whitespace-nowrap rounded-t-[8px] border-0 border-b-2 border-transparent bg-transparent px-[14px] text-[14px] font-medium text-muted-foreground",
                  current && "border-foreground font-semibold text-foreground",
                )}
              >
                {item.label}
                {item.count === undefined ? null : (
                  <span className="inline-flex h-[18px] items-center rounded-pill bg-muted px-[7px] font-mono text-[12px] font-bold tabular-nums">
                    {item.count}
                  </span>
                )}
              </button>
            );
          })}
        </div>
      </div>
      {commitment}
    </div>
  );
}

/** The promise, under the header. Items never wrap; only a flexible item may
 *  truncate, and the at-risk sentence is the one that does. */
export function CommitmentBar({ className, children }: { className?: string; children: ReactNode }) {
  return (
    <div
      data-slot="commitment-bar"
      className={cn(
        "flex h-[38px] shrink-0 items-center gap-[18px] overflow-hidden whitespace-nowrap border-y border-border bg-card px-6 text-[13px] [&>*]:shrink-0",
        className,
      )}
    >
      {children}
    </div>
  );
}

export function CommitKey({ children, className }: { children: ReactNode; className?: string }) {
  return <span className={cn("mr-[6px] text-muted-foreground", className)}>{children}</span>;
}

export function CommitSep() {
  return <span aria-hidden="true" className="h-[16px] w-px bg-border" />;
}

export function CommitFlexible({ className, children }: { className?: string; children: ReactNode }) {
  return (
    <span
      className={cn(
        "flex min-w-0 flex-1 items-center gap-2 overflow-hidden text-ellipsis whitespace-nowrap",
        className,
      )}
    >
      {children}
    </span>
  );
}

/** One header figure: label, value, and a small line or bar under it. */
export function Kpi({
  label,
  value,
  sub,
  mono,
  width,
  children,
}: {
  label: ReactNode;
  value?: ReactNode;
  sub?: ReactNode;
  /** Set the value itself in the mono face (dates, numbers). */
  mono?: boolean;
  width?: string;
  children?: ReactNode;
}) {
  return (
    <div className="flex min-w-[92px] flex-col gap-[3px]" style={width === undefined ? undefined : { width }}>
      <span className="text-[12px] font-semibold text-muted-foreground">{label}</span>
      {value === undefined ? null : (
        <span
          className={cn(
            "whitespace-nowrap text-[15px] font-semibold",
            mono && "font-mono tabular-nums tracking-[-0.01em]",
          )}
        >
          {value}
        </span>
      )}
      {children}
      {sub === undefined ? null : (
        <span className="whitespace-nowrap text-[12px] text-muted-foreground">{sub}</span>
      )}
    </div>
  );
}

/** The record's tabs' chevron and the breadcrumb separators, one shape. */
export function CrumbSep() {
  return (
    <span aria-hidden="true" className="inline-flex text-muted-foreground">
      ›
    </span>
  );
}
