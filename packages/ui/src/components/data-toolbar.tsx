"use client";
/**
 * DataToolbar — the strip a data surface puts its controls on.
 *
 * Purpose: one row that ViewSwitch, FilterBar, ColumnChooser and ExportControl
 * all sit in, so a ledger reads the same way whichever of them a surface
 * ships. Controls group left (the view and the filters, which are what the
 * user is LOOKING at) and right (columns and export, which are what the user
 * is DOING to the view), and the strip wraps rather than scrolls, because a
 * horizontal scroller hides the control you needed.
 *
 * Non-purpose: this holds no state and fetches nothing. It is the layout and
 * the landmarks, so the four controls stay independent and a surface that
 * wants only two of them uses only two.
 */
import type { ReactNode } from "react";
import { cn } from "../lib/cn";

export interface DataToolbarProps {
  /** Named views, the active one marked. Goes left, before the filters. */
  views?: ReactNode;
  /** Filter chips plus the compound builder. */
  filters?: ReactNode;
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
  filters,
  columns,
  export: exportControl,
  trailing,
  title,
  className,
  children,
  label = "Table controls",
}: DataToolbarProps): React.JSX.Element {
  const hasRight = Boolean(columns || exportControl || trailing);
  return (
    <div className={cn("m-data-toolbar", className)} role="group" aria-label={label}>
      {title}
      <div className="m-data-toolbar-main">
        {views}
        {filters}
        {/* the spacer is what keeps the right cluster on the right when the
            left cluster is short; without it a two-chip left side and a
            three-button right side meet in the middle. */}
        {hasRight ? <span className="m-data-toolbar-gap" /> : null}
        {columns}
        {exportControl}
        {trailing}
      </div>
      {children}
    </div>
  );
}
