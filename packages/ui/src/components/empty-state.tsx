"use client";
/**
 * EmptyState — one sentence, one action, and the fruit/leaf/canopy line
 * illustration (Wave 41 lane 2b).
 *
 * Purpose: the state a surface shows when there is genuinely nothing to
 * show. The illustration is LINE work, not a picture: the fruit, a leaf and
 * a canopy arch in `currentColor` at low weight, so it inherits whatever
 * surface it lands on and needs no per-theme asset.
 *
 * Non-purpose: this is not a mascot and it does not narrate. An empty state
 * that explains why it is empty has two jobs and does neither well; the
 * sentence says what is missing and the action is the way out. `children`
 * replaces the illustration for a surface that has a better one to draw.
 */
import type { ReactNode } from "react";
import { cn } from "../lib/cn";

/**
 * The Marula mark in three strokes: the fruit, the leaf, the canopy arch.
 * `aria-hidden` because it carries no information the sentence does not.
 */
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
  return (
    <div className={cn("m-empty", className)}>
      {illustration === undefined ? <EmptyStateMark /> : illustration}
      <p>{message}</p>
      {action}
    </div>
  );
}
