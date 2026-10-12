"use client";
/**
 * Badge — reimplemented (wave41 lane 20) as a thin wrapper over the handoff
 * kit's primitives, so the whole app draws these two label families the way
 * the kit defines them instead of restyling a copy.
 *
 * 1. Status (`tone`) -> the kit's `Pill`. The tone names are kept for the
 *    call sites; each maps to the kit tone with the same colour pair where one
 *    exists (see PILL_TONE).
 * 2. Priority (`priority`) -> the kit's `PriorityChip`.
 *
 * The old `.m-badge*` classes in components.css are no longer used by this
 * component; they are left in place until the class sweep, but nothing here
 * depends on them.
 */
import type { HTMLAttributes } from "react";
import { Pill, PriorityChip, type Priority } from "../kit";
import { cn } from "../lib/cn";

/** The status families a badge can carry. `due` is the blush/amber "needs
 * attention soon" pair, which is deliberately NOT the warning pair.
 * `outline` is accepted as an alias for `neutral`: the retired `Tag` prop
 * offered it, and the p41-ui-audit sweep (2026-10-12) found call sites whose
 * local `Tone` unions still name it. It renders the neutral pair, exactly as
 * `Tag` mapped it. */
export type BadgeTone = "success" | "warning" | "due" | "danger" | "neutral" | "offline" | "outline";

/** P1 is the only priority that takes red, per DESIGN-NOTES §5. */
export type BadgePriority = "p1" | "p2" | "p3" | "p4";

/**
 * App tone -> kit Pill tone. The pairs were checked equal while both layers
 * existed; `warning` and `progress` share the same fill, `due` and `attention`
 * share the same fill, and `danger` collapses onto the kit's one red pill.
 * `offline` has no kit tone of its own, so it reads neutral.
 */
const PILL_TONE = {
  success: "success",
  warning: "progress",
  due: "attention",
  danger: "breached",
  neutral: "neutral",
  offline: "neutral",
  outline: "neutral",
} as const satisfies Record<BadgeTone, "success" | "progress" | "attention" | "breached" | "neutral">;

export interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: BadgeTone;
  priority?: BadgePriority;
}

export function Badge({ className, tone, priority, children, ...props }: BadgeProps) {
  if (priority !== undefined) {
    return <PriorityChip level={priority.toUpperCase() as Priority} className={className} {...props} />;
  }
  return (
    <Pill tone={PILL_TONE[tone ?? "neutral"]} className={cn(className)} {...props}>
      {children}
    </Pill>
  );
}
