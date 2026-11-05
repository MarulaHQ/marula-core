"use client";
/**
 * Skeleton — the pack's flat loading blocks (surface-pack sheet S4).
 *
 * Purpose: one composer for a surface that is waiting on data. The pack draws
 * the state as still blocks — no shimmer, no pulse — with a sentence that says
 * what is loading, so the shape is composable rather than re-styled per page.
 * `Skeleton` is a single block; `SkeletonGroup` supplies the live region, the
 * 10px stack the sheet draws, and the sentence.
 *
 * Non-purpose: no data semantics and no timing. A caller owns when the read is
 * pending; this component only renders the waiting shape. It never animates:
 * docs/theme.md §5 allows transform/opacity only, and the pack draws the state
 * still, so there is no stagger to own.
 */
import type { CSSProperties, HTMLAttributes, ReactNode } from "react";
import { cn } from "../lib/cn";

/** The three shapes the sheet draws: a text line, a block, an avatar. */
export type SkeletonShape = "line" | "block" | "circle";

export interface SkeletonProps extends HTMLAttributes<HTMLSpanElement> {
  /** `line` is 12px text; `block` is the sheet's 22px block; `circle` is a control-sized dot. */
  shape?: SkeletonShape;
  /** Overrides the shape's width (a length, percentage or number of px). */
  width?: CSSProperties["width"];
  /** Overrides the shape's height (a length, percentage or number of px). */
  height?: CSSProperties["height"];
}

export function Skeleton({
  shape = "line",
  width,
  height,
  className,
  style,
  ...props
}: SkeletonProps): React.JSX.Element {
  return (
    <span
      aria-hidden="true"
      className={cn("m-skel", `m-skel-${shape}`, className)}
      style={{ width, height, ...style }}
      {...props}
    />
  );
}

export interface SkeletonGroupProps extends HTMLAttributes<HTMLDivElement> {
  /** Accessible name; also the sentence a reader sees when `sentence` is unset. */
  label: string;
  /** Visible sentence under the blocks. Defaults to `label` plus an ellipsis. */
  sentence?: ReactNode;
}

/**
 * Several blocks stacked as the sheet draws them, announced once. The sentence
 * is visible on purpose: the pack pairs the blocks with words, so a reader is
 * never left guessing whether anything is happening.
 */
export function SkeletonGroup({
  label,
  sentence,
  className,
  children,
  ...props
}: SkeletonGroupProps): React.JSX.Element {
  return (
    <div
      role="status"
      aria-busy="true"
      aria-label={label}
      className={cn("m-skel-group", className)}
      {...props}
    >
      {children}
      <p className="m-skel-sentence">{sentence ?? `${label}…`}</p>
    </div>
  );
}
