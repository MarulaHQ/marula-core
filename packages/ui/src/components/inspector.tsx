"use client";
/**
 * Inspector — the right-hand panel a record uses for one selected object.
 *
 * Purpose: the pack gives the plan (S3.11), dispatch (S9.7) and the template
 * editor (S12) the same panel, and each surface that rolled its own drifted on
 * the two things that matter: the body scrolls while the header and footer stay
 * put, and the width is the pack's (372 beside a table, 520 when it sheets over
 * the right edge).
 *
 * Three placements, one component:
 *
 * - `column` — 372px, rounded, sits beside the table.
 * - `sheet` — 520px, absolute against the right edge of a relative surface,
 *   with a left border and the overlay shadow. This is the impact preview and
 *   the wide inspector.
 * - `bottom` — the bottom-sheet shape `BottomSheet` uses at 1024 and below.
 *
 * Non-purpose: no focus trap, no state. A sheet that must behave modally gets
 * `useFocusTrap` from its surface; the panel itself is a landmark.
 */
import type { HTMLAttributes, ReactNode } from "react";
import { cn } from "../lib/cn";

export type InspectorPlacement = "column" | "sheet" | "bottom";

export function Inspector({
  placement = "column",
  className,
  ...props
}: HTMLAttributes<HTMLElement> & { placement?: InspectorPlacement }) {
  return (
    <aside
      data-slot="inspector"
      data-placement={placement}
      className={cn(
        "flex min-h-0 flex-col overflow-hidden bg-card",
        placement === "column" && "w-[372px] shrink-0 rounded-card border border-border",
        placement === "sheet" &&
          "absolute inset-y-0 right-0 z-30 w-[520px] border-l border-border shadow-mc",
        placement === "bottom" &&
          "absolute inset-x-6 bottom-4 z-40 rounded-[16px_16px_12px_12px] border border-border shadow-mc",
        className,
      )}
      {...props}
    />
  );
}

export function InspectorHeader({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      data-slot="inspector-header"
      className={cn("flex shrink-0 items-start gap-[10px] border-b border-border px-4 py-[14px]", className)}
      {...props}
    />
  );
}

export function InspectorTitle({ className, ...props }: HTMLAttributes<HTMLHeadingElement>) {
  return (
    <h2
      data-slot="inspector-title"
      className={cn("m-0 text-[16px] font-semibold leading-[1.3]", className)}
      {...props}
    />
  );
}

export function InspectorBody({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      data-slot="inspector-body"
      className={cn(
        "flex min-h-0 flex-1 flex-col gap-[14px] overflow-y-auto px-4 py-[14px] [&>*]:shrink-0",
        className,
      )}
      {...props}
    />
  );
}

export function InspectorFooter({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      data-slot="inspector-footer"
      className={cn(
        "flex shrink-0 items-center gap-2 border-t border-border bg-background px-4 py-3",
        className,
      )}
      {...props}
    />
  );
}

/** The small uppercase label the inspector uses above a group. */
export function Eyebrow({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn("text-[12px] font-semibold uppercase tracking-[0.08em] text-muted-foreground", className)}
      {...props}
    />
  );
}

export function Rule({ className }: { className?: string }) {
  return <div role="separator" className={cn("h-px shrink-0 bg-border", className)} />;
}

/** Label and value pairs. The label column is 104px, as the reference draws it. */
export function KeyValueList({ className, ...props }: HTMLAttributes<HTMLDListElement>) {
  return (
    <dl
      data-slot="key-value"
      className={cn(
        "m-0 grid grid-cols-[104px_minmax(0,1fr)] items-center gap-x-3 gap-y-[7px] text-[13px]",
        "[&_dt]:text-muted-foreground [&_dd]:m-0 [&_dd]:min-w-0 [&_dd]:font-medium",
        className,
      )}
      {...props}
    />
  );
}

export type ChainState = "done" | "current" | "late" | "todo";

/**
 * The dependency chain: what this task waits on, this task, then what waits on
 * it. The current task is solid ink; done is green; late is the danger pair.
 * The connector is drawn as a 2px rule, never as an arrow the layout can flip.
 */
export function DepChain({
  nodes,
  className,
}: {
  nodes: ReadonlyArray<{ label: ReactNode; state?: ChainState; trailing?: ReactNode }>;
  className?: string;
}) {
  return (
    <div data-slot="dep-chain" className={cn("flex flex-col", className)}>
      {nodes.map((node, index) => (
        <div key={index}>
          {index > 0 ? (
            <div aria-hidden="true" className="ml-2 h-[10px] w-[2px] bg-border" />
          ) : null}
          <div className="grid min-h-[32px] grid-cols-[18px_minmax(0,1fr)_auto] items-center gap-2 text-[13px]">
            <span
              aria-hidden="true"
              className={cn(
                "size-[10px] justify-self-center rounded-full border-2 border-input bg-card",
                node.state === "done" && "border-mc-green bg-mc-green",
                node.state === "current" && "border-foreground bg-foreground",
                node.state === "late" && "border-destructive bg-destructive",
              )}
            />
            <span className="min-w-0 truncate">{node.label}</span>
            {node.trailing === undefined ? null : <span className="text-muted-foreground">{node.trailing}</span>}
          </div>
        </div>
      ))}
    </div>
  );
}
