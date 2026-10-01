"use client";
/**
 * EmptyState — reimplemented (wave41 lane 20) over the handoff kit's
 * `EmptyState` (the FruitLine mark, one heading, at most one action).
 *
 * The call sites pass a single `message` sentence, so it becomes the kit's
 * heading. A caller that supplies its own `illustration` keeps the old
 * free-form path.
 */
import type { ReactNode } from "react";
import { EmptyState as KitEmptyState } from "../kit";
import { cn } from "../lib/cn";

/** The Marula mark in three strokes, kept for callers that draw it directly. */
export function EmptyStateMark({ className }: { className?: string }): React.JSX.Element {
  return (
    <svg
      className={cn("m-empty-mark", className)}
      viewBox="0 0 96 72"
      width="96"
      height="72"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M48 20c9 0 16 7 16 16s-7 16-16 16-16-7-16-16 7-16 16-16Z" />
      <path d="M48 20V9" />
      <path d="M48 9c7 0 11-3 12-8-6-1-11 2-12 8Z" />
      <path d="M30 62c4-8 11-12 18-12s14 4 18 12" />
      <path d="M18 62h60" />
    </svg>
  );
}

export function EmptyState({
  message,
  action,
  illustration,
  className,
}: {
  message: string;
  /** Single primary action button for the empty state. */
  action?: ReactNode;
  /** Replaces the default mark. */
  illustration?: ReactNode;
  className?: string;
}): React.JSX.Element {
  if (illustration !== undefined) {
    return (
      <div className={cn("flex flex-col items-center gap-2 px-6 py-7 text-center", className)}>
        {illustration}
        <p className="m-0 max-w-[360px] text-muted-foreground">{message}</p>
        {action ? <div className="mt-2.5 flex gap-2.5">{action}</div> : null}
      </div>
    );
  }
  return <KitEmptyState title={message} action={action} className={className} />;
}
