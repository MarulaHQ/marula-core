"use client";
/**
 * Badge — Wave 41 lane 2b: the two label families the Marula workbench draws.
 *
 * Purpose: one primitive for the two shapes a status label takes in the
 * product, both from the brand status pairs and nothing else.
 *
 * 1. Status (`tone`) — a fully-round pill on a soft status fill with the
 *    status' own dark text. Marula §6 sets these at 13px semibold; the tag
 *    row in a ledger is meant to be read at a glance, not scanned.
 * 2. Priority (`priority`) — `p1`–`p4`, squared-off (`rounded-md`), bold, and
 *    a FIXED width, because a priority column is a column: four chips that
 *    change width as the label changes are a column that does not line up.
 *
 * Non-purpose: this does not decide which tone a status maps to. That mapping
 * is domain knowledge and belongs to the surface; a caller that passes the
 * wrong tone renders the wrong colour, exactly as it would with any token.
 * `Tag` is the pre-Marula status label and still has call sites; retiring it
 * is a per-surface sweep, not a change to the design system.
 */
import { cva, type VariantProps } from "class-variance-authority";
import { forwardRef, type HTMLAttributes } from "react";
import { cn } from "../lib/cn";

/** The status families a badge can carry. `due` is the blush/amber "needs
 * attention soon" pair, which is deliberately NOT the warning pair: due
 * today is not a warning, it is a date. */
export type BadgeTone = "success" | "warning" | "due" | "danger" | "neutral" | "offline";

/** P1 is the only priority that takes red, per DESIGN-NOTES §5. */
export type BadgePriority = "p1" | "p2" | "p3" | "p4";

const badgeVariants = cva("m-badge", {
  variants: {
    tone: {
      success: "m-badge-success",
      warning: "m-badge-warning",
      due: "m-badge-due",
      danger: "m-badge-danger",
      neutral: "m-badge-neutral",
      offline: "m-badge-offline",
    },
    priority: {
      p1: "m-badge-p1",
      p2: "m-badge-p2",
      p3: "m-badge-p3",
      p4: "m-badge-p4",
    },
  },
});

export interface BadgeProps
  extends HTMLAttributes<HTMLSpanElement>,
    VariantProps<typeof badgeVariants> {}

export const Badge = forwardRef<HTMLSpanElement, BadgeProps>(
  ({ className, tone, priority, ...props }, ref) => (
    <span
      ref={ref}
      className={cn(badgeVariants({ tone, priority }), className)}
      {...props}
    />
  ),
);

Badge.displayName = "Badge";
