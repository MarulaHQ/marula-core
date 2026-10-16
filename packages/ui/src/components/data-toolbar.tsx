"use client";
/**
 * DataToolbar — the strip a data surface puts its controls on.
 *
 * Purpose: one row that ViewSwitch, FilterBar, ColumnChooser and ExportControl
 * all sit in, so a ledger reads the same way whichever of them a surface
 * ships. Controls group left (the view and the filters, which are what the
 * user is LOOKING at) and right (columns and export, which are what the user
 * is DOING to the view).
 *
 * The strip is **one row that never wraps**, and it is the control height plus
 * 16px (`DESIGN-NOTES-tickets.md` §2 law 1, §3). It used to wrap and to carry up
 * to 24px of padding above itself, so the table moved down whenever the controls
 * did not fit — the reader loses their place on the exact page they are working
 * from. The middle is now the only part allowed to give ground, and it gives it
 * by collapsing (the chips' `+N more`), never by wrapping.
 *
 * The template also puts the **Filter button on the right**, beside Columns and
 * Export, with the chips reading in the middle. That is why `chips` and `filter`
 * are two slots: a surface with both gets the pack's arrangement, and the older
 * `filters` slot still renders one element in the middle for the surfaces that
 * have not split their FilterBar yet.
 *
 * Non-purpose: this holds no state and fetches nothing. It is the layout and
 * the landmarks, so the controls stay independent and a surface that wants only
 * two of them uses only two.
 */
import type { ReactNode } from "react";
import { cn } from "../lib/cn";

export interface DataToolbarProps {
  /** Named views, the active one marked. Goes left, before the filters. */
  views?: ReactNode;
  /** Filter chips. The middle of the strip, and the only part that may shrink. */
  chips?: ReactNode;
  /**
   * A whole control that renders its own chips and trigger. Rendered in the
   * middle when `chips` is absent, so an existing caller keeps its arrangement.
   */
  filters?: ReactNode;
  /** The right-hand Filter trigger (the pack's `Filter` button). */
  filter?: ReactNode;
  /** Column chooser trigger. */
  columns?: ReactNode;
  /** Export trigger. */
  export?: ReactNode;
  /** Anything else that belongs on the strip: a bulk action, a count. */
  trailing?: ReactNode;
  /** The surface's own title row controls, hoisted above the strip. */
  title?: ReactNode;
  className?: string;
  children?: ReactNode;
  /** Announced to assistive tech as the strip's purpose. */
  label?: string;
}

export function DataToolbar({
  views,
  chips,
  filters,
  filter,
  columns,
  export: exportControl,
  trailing,
  title,
  className,
  children,
  label = "Table controls",
}: DataToolbarProps): React.JSX.Element {
  const hasRight = Boolean(filter || columns || exportControl || trailing);
  const middle = chips ?? filters;
  return (
    <div className={cn("m-data-toolbar", className)} role="group" aria-label={label}>
      {title === undefined ? null : <div className="m-data-toolbar-title">{title}</div>}
      <div className="m-data-toolbar-main">
        {views}
        {middle === undefined ? null : (
          <div className="m-data-toolbar-chips">{middle}</div>
        )}
        {/* the spacer is what keeps the right cluster on the right when the
            left cluster is short; without it a two-chip left side and a
            three-button right side meet in the middle. */}
        {hasRight ? <span className="m-data-toolbar-gap" /> : null}
        <div className="m-data-toolbar-right">
          {filter}
          {columns}
          {exportControl}
          {trailing}
        </div>
      </div>
      {children}
    </div>
  );
}
