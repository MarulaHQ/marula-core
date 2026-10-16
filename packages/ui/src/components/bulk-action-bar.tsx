"use client";
/**
 * BulkActionBar / BulkHeaderRow — the bar that replaces the column header when
 * rows are selected.
 *
 * The placement is the law (`DESIGN-NOTES-tickets.md` §7): selecting a row puts
 * the action bar into the table's **own header row**, so the table never moves.
 * An extra strip above the table is the same controls in the wrong place — it
 * pushes the first row down at the exact moment the reader is aiming at it, and
 * that is what `/approvals` shipped.
 *
 * It is a gold tint and it is deliberately **not** an inverted surface. There is
 * one inverted surface on the screen and it is the timer bar.
 *
 * Why two components: `<th colSpan>` and the bar's own contents are separate
 * decisions. A surface with a grouped header needs the row without the bar; a
 * surface rendering its own header row needs the bar without the row. Exporting
 * both is what stops the next surface from hand-rolling the pair, which is how
 * three spellings of this bar came to exist (`knowledge/ledger.tsx`,
 * `tickets/ledger-table.tsx`, `kb/page.tsx`).
 *
 * `Merge` is part of the bar because the template puts it there and needs two
 * rows or more. It renders only when the caller passes `onMerge`, so a surface
 * whose product has no merge path does not grow a dead button — and when it does
 * have one, the two-row rule is the kit's, not each surface's to remember.
 *
 * Non-purpose: no writes, no state, no selection modelling. The caller owns the
 * selection, the writes and the dialogs the actions open.
 */
import type { ReactNode } from "react";
import { cn } from "../lib/cn";

export interface BulkActionBarProps {
  /** How many rows are selected. */
  count: number;
  /** Singular noun for the count line, e.g. "ticket". */
  noun?: string;
  /** Always shown, always last. The pack puts it on the right of the bar. */
  onClear: () => void;
  /** The surface's own actions, in the pack's order where they apply. */
  children?: ReactNode;
  /**
   * The merge action, shown when the surface has one. At fewer than two rows the
   * button renders disabled rather than disappearing, because a control that
   * vanishes at one selection reads as "merge is broken".
   */
  onMerge?: () => void;
  mergeDisabledReason?: string;
  label?: string;
  /** `data-testid` on the count span, for the surfaces whose tests read it. */
  countTestId?: string;
  testId?: string;
  className?: string;
}

/** The bar's contents. Goes inside a `<th>` that spans the table's columns. */
export function BulkActionBar({
  count,
  noun = "row",
  onClear,
  children,
  onMerge,
  mergeDisabledReason,
  label = "Bulk actions",
  countTestId,
  testId,
  className,
}: BulkActionBarProps): React.JSX.Element {
  const canMerge = count >= 2;
  return (
    <div
      className={cn("m-bulk-bar", className)}
      role="group"
      aria-label={label}
      data-testid={testId}
    >
      <span className="m-bulk-bar-count" data-testid={countTestId}>
        {count.toLocaleString()} selected
      </span>
      <span className="m-bulk-bar-actions">
        {children}
        {onMerge === undefined ? null : (
          <button
            type="button"
            className="m-btn m-btn-secondary m-btn-sm"
            onClick={onMerge}
            disabled={!canMerge}
            title={
              canMerge
                ? `Merge ${count} ${count === 1 ? noun : `${noun}s`}`
                : (mergeDisabledReason ?? "Select two rows or more to merge")
            }
          >
            Merge
          </button>
        )}
      </span>
      <button
        type="button"
        className="m-btn m-btn-ghost m-btn-sm m-bulk-bar-clear"
        onClick={onClear}
      >
        Clear selection
      </button>
    </div>
  );
}

export interface BulkHeaderRowProps extends BulkActionBarProps {
  /**
   * The table's full column count **including** the leading select column, so the
   * bar covers every column of the row it replaces.
   */
  columnCount: number;
  /**
   * The select column's own cell — the pack keeps its "deselect all" checkbox in
   * place and lets the bar span the rest, so the checkbox never moves out from
   * under the column it belongs to.
   */
  leading?: ReactNode;
}

/**
 * The `<tr><th colSpan>` pair that takes the column header's place. Render it as
 * the first child of `<thead>` and the header row as the second — or render one
 * or the other, which is what the surfaces do.
 */
export function BulkHeaderRow({
  columnCount,
  testId = "bulk-bar",
  leading,
  ...bar
}: BulkHeaderRowProps): React.JSX.Element {
  if (leading === undefined) {
    return (
      <tr>
        <th colSpan={columnCount} className="m-bulk-head" data-testid={testId}>
          <BulkActionBar {...bar} />
        </th>
      </tr>
    );
  }
  return (
    <tr>
      <th className="m-bulk-head m-bulk-head-select">{leading}</th>
      <th colSpan={Math.max(1, columnCount - 1)} className="m-bulk-head" data-testid={testId}>
        <BulkActionBar {...bar} />
      </th>
    </tr>
  );
}
