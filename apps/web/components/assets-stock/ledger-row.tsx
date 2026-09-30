"use client";
/**
 * LedgerRow — one body row of a data surface, rendered by column key.
 *
 * Purpose: the column chooser is only honest if a column the user hides
 * disappears from the body too, so the body is built from the same resolved
 * column list the header is. A page hands over a map of cells and this
 * component places them in the order the table is actually showing, plus the
 * select checkbox and the selected-row treatment (a gold tint with a 3px
 * gold bar on the left edge).
 *
 * Non-purpose: no data fetching, no row actions bar, no severity logic. A
 * cell is whatever the page put in the map.
 */
import { Td, Tr } from "@marula-core/ui";
import type { ReactNode } from "react";

import type { ResolvedColumn } from "./ledger-surface";

export function LedgerRow({
  id,
  selected,
  selectable,
  onToggle,
  columns,
  cells,
  testId,
  selectLabel,
}: {
  id: string;
  selected: boolean;
  selectable: boolean;
  onToggle?: (id: string, selected: boolean) => void;
  columns: ResolvedColumn[];
  cells: Record<string, ReactNode>;
  testId?: string;
  selectLabel: string;
}): React.JSX.Element {
  return (
    <Tr selected={selected} className="row-hover" data-testid={testId}>
      {selectable ? (
        <Td className="m-ledger-select-col" data-select="true">
          <input
            type="checkbox"
            className="m-ledger-checkbox"
            aria-label={selectLabel}
            checked={selected}
            onChange={(event) => onToggle?.(id, event.target.checked)}
          />
        </Td>
      ) : null}
      {columns
        .filter((column) => column.visible)
        .map((column) => (
          <Td
            key={column.key}
            data-cell={column.key}
            data-align={column.align === "right" ? "right" : "left"}
          >
            {cells[column.key] ?? null}
          </Td>
        ))}
    </Tr>
  );
}
