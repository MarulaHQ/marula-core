"use client";
/**
 * ImpactPreview — what a date change does, shown before anything is saved.
 *
 * Purpose: the pack's rule for a reschedule is that changing a date never just
 * saves (S3.6, S4.7). The preview groups the consequence the way a project
 * manager thinks about it — what moves inside the phase, what moves the finish,
 * what collides elsewhere — under one sentence that says the thing that
 * matters, and offers one apply and one cancel.
 *
 * It composes `Inspector placement="sheet"` rather than drawing a second
 * panel: the same header/body/footer shape, the same body that scrolls while
 * the actions stay put, and the same 520px width. The surface mounts it inside a
 * `relative` container, so the sheet covers the table it describes.
 *
 * Non-purpose: no fetching and no write. The caller computes the preview, owns
 * the apply, and supplies the undo afterwards.
 */
import type { HTMLAttributes, ReactNode } from "react";
import { Icon } from "../kit";
import { cn } from "../lib/cn";
import {
  Eyebrow,
  Inspector,
  InspectorBody,
  InspectorFooter,
  InspectorHeader,
  InspectorTitle,
} from "./inspector";

export function ImpactPreview({
  title,
  close,
  footer,
  children,
  className,
}: {
  title: ReactNode;
  close?: ReactNode;
  footer: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <Inspector
      placement="sheet"
      aria-label="Impact preview"
      className={cn("max-w-full motion-safe:transition-transform motion-safe:duration-[120ms]", className)}
    >
      <InspectorHeader>
        <div className="min-w-0 flex-1">
          <Eyebrow>Impact preview, nothing is saved yet</Eyebrow>
          <InspectorTitle className="mt-1">{title}</InspectorTitle>
        </div>
        {close}
      </InspectorHeader>
      <InspectorBody>{children}</InspectorBody>
      <InspectorFooter>{footer}</InspectorFooter>
    </Inspector>
  );
}

/** A group in the preview: one consequence, its count, and the rows it moves. */
export function ImpactGroup({
  icon,
  title,
  note,
  children,
}: {
  icon?: ReactNode;
  title: ReactNode;
  note?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div
      data-slot="impact-group"
      className="overflow-hidden rounded-[12px] border border-border"
    >
      <div className="flex items-center gap-2 border-b border-border bg-background px-3 py-2 text-[12px] font-bold">
        {icon}
        <span>{title}</span>
        {note === undefined ? null : (
          <span className="truncate font-medium text-muted-foreground">{note}</span>
        )}
      </div>
      {children}
    </div>
  );
}

/** One moved task: the old date struck through, the new date, and the delta. */
export function ImpactRow({
  label,
  was,
  now,
  delta,
}: {
  label: ReactNode;
  was: ReactNode;
  now: ReactNode;
  delta: ReactNode;
}) {
  return (
    <div
      data-slot="impact-row"
      className="grid grid-cols-[minmax(0,1fr)_78px_14px_78px_44px] items-center gap-[6px] border-t border-border px-3 py-[7px] text-[13px]"
    >
      <span className="min-w-0 truncate">{label}</span>
      <span className="font-mono text-[0.92em] tabular-nums tracking-[-0.01em] text-muted-foreground line-through">
        {was}
      </span>
      <span aria-hidden="true" className="inline-flex">
        <Icon name="arrow-r" size={13} />
      </span>
      <span className="font-mono text-[0.92em] font-semibold tabular-nums tracking-[-0.01em]">
        {now}
      </span>
      <span className="text-right font-mono text-[12px] font-bold tabular-nums tracking-[-0.01em]">
        {delta}
      </span>
    </div>
  );
}

/**
 * A row that states one fact rather than a move: somebody else's booking, a
 * person, a window. The move rows read `was → now`; a fact has no "was".
 */
export function ImpactFact({ label, value }: { label: ReactNode; value: ReactNode }) {
  return (
    <div
      data-slot="impact-row"
      className="flex items-center justify-between gap-3 border-t border-border px-3 py-[7px] text-[13px]"
    >
      <span className="min-w-0 truncate">{label}</span>
      <span className="shrink-0 font-mono text-[0.92em] font-semibold tabular-nums tracking-[-0.01em]">
        {value}
      </span>
    </div>
  );
}

/** A quiet sentence under the groups: what keeps its dates, or what was not
 *  checked. Never a suggestion the caller cannot substantiate. */
export function ImpactNote({ className, ...props }: HTMLAttributes<HTMLParagraphElement>) {
  return (
    <p
      className={cn(
        "m-0 flex items-start gap-2 text-[13px] leading-[1.5] text-muted-foreground",
        className,
      )}
      {...props}
    />
  );
}
