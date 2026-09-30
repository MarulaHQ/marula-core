"use client";
/**
 * The page frame's columns — the two side slots a record-detail surface puts
 * its context and its agent output in.
 *
 * Purpose: the pack fixes both widths and both are load-bearing, because a
 * three-column page is arithmetic. Sheet S2's context column is **290px** in
 * all three of its variants (workbench, client record, none) and sheet S3's
 * assistant column is **340px**, with a **40px** stub as its resting state.
 * At a 1600px canvas that leaves the middle ~880px — the width the pack's own
 * data-surface sheets are drawn at. Change either number and every reference
 * stops being a reference.
 *
 * These are LAYOUT FRAMES WITH SLOTS, not the content:
 *
 * - `ContextColumn` is the frame. The workbench variant's Call/Email buttons,
 *   the client variant's contract block and the none-variant's skeleton are
 *   the surface's own; they go in as children.
 * - `AssistantColumn` is the frame plus the 40px stub, because the stub is
 *   how a surface keeps the affordance visible without spending its one
 *   inverted surface (A3). Passing no children renders the stub, which is the
 *   correct default: alert triage opens there.
 *
 * A column's width is not a suggestion. `ContextColumn` refuses to render
 * below `xl` at full width and stacks instead, because a 290px column beside
 * a fluid one is unreadable at tablet width and a horizontal scroller hides
 * the summary the record exists to show.
 *
 * Non-purpose: no fetching, no assistant state machine, no card styling
 * beyond the ground and the hairline. The one inverted surface on a record is
 * the assistant's proposal card and it is the surface's to render, not this
 * frame's — this never inverts anything.
 */
import type { ReactNode } from "react";
import { cn } from "../lib/cn";

/** The pack's exact context-column width (sheet S2, all three variants). */
export const CONTEXT_COLUMN_WIDTH = 290;
/** The pack's exact assistant-column width (sheet S3). */
export const ASSISTANT_COLUMN_WIDTH = 340;
/** The pack's assistant stub: 40px, a spark and a count. */
export const ASSISTANT_STUB_WIDTH = 40;

export interface ContextColumnProps {
  /**
   * Which of S2's three variants this is. It sets the eyebrow the pack
   * prints above the column's own card and nothing else — the variant's
   * contents are the surface's own, so a surface cannot get the width wrong
   * by picking the wrong variant.
   */
  variant?: "workbench" | "client" | "none";
  /** The dot-prefixed pill S2 puts under the column label. */
  label?: ReactNode;
  children?: ReactNode;
  className?: string;
}

const CONTEXT_VARIANT_LABEL: Record<NonNullable<ContextColumnProps["variant"]>, string> = {
  workbench: "Ticket workbench",
  client: "Client detail",
  none: "List and board surfaces",
};

export function ContextColumn({
  variant = "workbench",
  label,
  children,
  className,
}: ContextColumnProps): React.JSX.Element {
  return (
    <aside
      aria-label={typeof label === "string" ? label : "Record context"}
      data-testid="context-column"
      data-variant={variant}
      className={cn("m-context-column", className)}
    >
      {label ? <div className="m-context-column-label">{label}</div> : null}
      <div className="m-context-column-body">{children}</div>
    </aside>
  );
}

/** The pill text S2 prints for a variant, for a surface that has no better name. */
export function contextVariantLabel(
  variant: NonNullable<ContextColumnProps["variant"]>,
): string {
  return CONTEXT_VARIANT_LABEL[variant];
}

export interface AssistantColumnProps {
  /** The column's contents. Omit to render the 40px stub. */
  children?: ReactNode;
  /**
   * The stub's count — the suggestions waiting. S3's stub shows `2`; S5's
   * "live, nothing to propose" shows no count, because there is nothing to
   * count.
   */
  count?: number | null;
  /** Spark glyph for the stub. A surface passes its own; there is no default glyph. */
  spark?: ReactNode;
  /** Announced to assistive tech. Defaults to the product's own name. */
  label?: string;
  className?: string;
}

export function AssistantColumn({
  children,
  count = null,
  spark,
  label = "Assistant",
  className,
}: AssistantColumnProps): React.JSX.Element {
  if (children == null) {
    return (
      <aside
        aria-label={label}
        data-testid="assistant-stub"
        className={cn("m-assistant-stub", className)}
      >
        <span className="m-assistant-stub-spark" aria-hidden="true">
          {spark}
        </span>
        {count == null ? null : (
          <span className="m-assistant-stub-count" data-testid="assistant-stub-count">
            {count}
          </span>
        )}
      </aside>
    );
  }

  return (
    <aside
      aria-label={label}
      data-testid="assistant-column"
      className={cn("m-assistant-column", className)}
    >
      <div className="m-assistant-column-header">
        <span className="m-assistant-column-name">{label}</span>
      </div>
      <div className="m-assistant-column-body">{children}</div>
    </aside>
  );
}
