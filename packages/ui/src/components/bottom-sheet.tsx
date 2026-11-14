"use client";
/**
 * BottomSheet — the inspector's shape at the minimum supported width (S3.17).
 *
 * Purpose: below 1024 the 372px inspector column would squeeze the plan into a
 * horizontal scroller, so the panel becomes a sheet over the bottom of the
 * table. It is inset 24px at the sides and 16px at the bottom so the selected
 * row stays visible above it — that visibility is the whole reason it is a
 * bottom sheet and not a full-screen dialog.
 *
 * It is not modal: the row above stays reachable and the table keeps working.
 * A caller that needs containment wraps `useFocusTrap` around its own content;
 * this component supplies the shape and the dialog semantics only.
 *
 * Non-purpose: no open state, no animation, no backdrop. The surface mounts it
 * when a row is selected and unmounts it when the selection clears.
 */
import type { HTMLAttributes, ReactNode } from "react";
import { cn } from "../lib/cn";

export function BottomSheet({
  label,
  fixed = false,
  className,
  children,
  ...props
}: HTMLAttributes<HTMLDivElement> & {
  label: string;
  /** Anchor to the viewport instead of the surface, when the surface can be
   *  shorter than the screen (a short plan at 1024). */
  fixed?: boolean;
  children: ReactNode;
}) {
  return (
    <div
      data-slot="bottom-sheet"
      role="dialog"
      aria-label={label}
      aria-modal={false}
      className={cn(
        "z-40 flex flex-col gap-[10px] rounded-[16px_16px_12px_12px] border border-border bg-card px-4 py-[14px] text-foreground shadow-mc",
        fixed ? "fixed inset-x-6 bottom-20" : "absolute inset-x-6 bottom-4",
        className,
      )}
      {...props}
    >
      {children}
    </div>
  );
}
