// Insight data-surface helpers — the shared half of lane 11's six surfaces.
//
// Purpose: one place that wires the landed D12 controls (ViewSwitch,
// FilterBar, ColumnChooser, ExportControl) and the D7 density footer to the
// per-user preference key, so /reports, /audit, /provenance and /scorecards
// all persist through the SAME `appearance.data_surfaces` document the
// settings seam owns. Column order, visibility and sort are the user's;
// rows are the surface's own endpoint.
//
// Non-purpose: no fetching, no page-level state.

import { useCallback, useMemo } from "react";
import type { ChoosableColumn } from "@marula-core/ui";
import type { DataSurfacePrefs } from "./data-surface-prefs";

/** One column of an insight table, in the caller's canonical order. */
export interface InsightColumn<T> {
  key: string;
  label: string;
  /** A column a user cannot turn off (the row's identity). */
  required?: boolean;
  /** Default visible when the user has never saved this surface. */
  defaultVisible?: boolean;
  /** Sort key when clicked; sort is off when absent. */
  sortValue?: (row: T) => string | number;
  render: (row: T) => React.ReactNode;
}

export type SortDir = "asc" | "desc";

export interface InsightSort {
  field: string;
  dir: SortDir;
}

/**
 * The saved shape this lane keeps under its surface key. The D12 document
 * stores the same fields; this is the subset the insight tables use, so a
 * table that never filters stays honest about the shape it claims.
 */
export interface InsightSaved {
  view?: string;
  columns: { key: string; visible: boolean; width?: number }[];
  filters?: { field: string; op: string; value?: string }[];
  sort?: InsightSort;
}

/**
 * Resolve the effective column order for a table, in the order:
 * saved order → canonical order. Saved widths ride along.
 */
export function resolveColumns<T>(
  columns: readonly InsightColumn<T>[],
  saved: InsightSaved | undefined,
): ChoosableColumn[] {
  const byKey = new Map(columns.map((c) => [c.key, c]));
  const savedKeys = new Set((saved?.columns ?? []).map((c) => c.key));
  const out: ChoosableColumn[] = [];
  for (const { key, visible: _visible, width } of saved?.columns ?? []) {
    const def = byKey.get(key);
    if (def === undefined) continue; // dropped column stops being offered
    out.push({ key, label: def.label, visible: true, width });
  }
  for (const def of columns) {
    if (savedKeys.has(def.key)) continue;
    out.push({
      key: def.key,
      label: def.label,
      visible: def.required === true || def.defaultVisible !== false,
    });
  }
  return out;
}

/** Visible keys only, in chooser order. */
export function visibleKeys(cols: readonly ChoosableColumn[]): string[] {
  return cols.filter((c) => c.visible).map((c) => c.key);
}

/** Apply a chooser edit: merge the new visible/order onto the saved doc. */
export function chooserToSaved(
  saved: InsightSaved | undefined,
  next: readonly ChoosableColumn[],
): InsightSaved {
  return {
    ...saved,
    columns: next.map((c) => ({ key: c.key, visible: c.visible, width: c.width })),
  };
}

/** Toggle or set sort on one column. */
export function toggleSort(
  saved: InsightSorted,
  field: string,
): InsightSaved {
  const cur = saved.sort;
  if (cur === undefined || cur.field !== field) return { ...saved, sort: { field, dir: "asc" } };
  if (cur.dir === "asc") return { ...saved, sort: { field, dir: "desc" } };
  const { sort: _dropped, ...rest } = saved;
  return rest;
}

type InsightSorted = InsightSaved;

/** Unary sort over the caller's rows, honouring each column's sortValue. */
export function sortRows<T>(
  rows: readonly T[],
  kind: string,
  columns: readonly { key: string; sortValue?: (row: T) => string | number }[],
  sort: InsightSort | undefined,
): T[] {
  if (sort === undefined) return [...rows];
  const col = columns.find((c) => c.key === sort.field);
  if (col?.sortValue === undefined) return [...rows];
  const dir = sort.dir === "asc" ? 1 : -1;
  return [...rows].sort((a, b) => {
    const x = col.sortValue!(a);
    const y = col.sortValue!(b);
    const base = x < y ? -1 : x > y ? 1 : 0;
    return base * dir;
  });
}

/** Minimal CSV: values quoted, header from the visible labels. */
export function toCsv<T>(
  keys: readonly string[],
  columns: readonly InsightColumn<T>[],
  rows: readonly T[],
  cellText: (col: InsightColumn<T>, row: T) => string,
): string {
  const q = (v: string) => `"${v.replace(/"/g, '""')}"`;
  const chosen = keys
    .map((k) => columns.find((c) => c.key === k))
    .filter((c): c is InsightColumn<T> => c !== undefined);
  const head = chosen.map((c) => q(c.label)).join(",");
  const body = rows.map((r) => chosen.map((c) => q(cellText(c, r))).join(",")).join("\n");
  return body === "" ? head : `${head}\n${body}`;
}

/** Download helper shared by the surfaces' export paths. */
export function downloadCsv(filename: string, csv: string): void {
  try {
    const blob = new Blob([csv], { type: "text/csv" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  } catch {
    // a blocked download is not a broken page — the ledger stays on screen
  }
}

/**
 * The column state for one table, over the settings document. Exposes the
 * chooser list, the visible keys and the commit functions; a rejected write
 * is the prefs hook's problem (it rolls back and reports).
 */
export function useSurfaceTableColumns<T>(
  surface: string,
  prefs: DataSurfacePrefs,
  columns: readonly InsightColumn<T>[],
) {
  const saved = prefs.view(surface) as unknown as InsightSaved;
  const chooser = useMemo(() => resolveColumns(columns, saved), [columns, saved]);
  const keys = useMemo(() => visibleKeys(chooser), [chooser]);
  const save = useCallback(
    (next: InsightSaved) =>
      // The D12 document always carries a filters array, so the value the
      // settings registry stores is a complete document even when a table
      // holds no filter clauses of its own.
      void prefs.save(surface, { ...next, filters: next.filters ?? [] }),
    [prefs, surface],
  );
  const onChooser = useCallback(
    (next: readonly ChoosableColumn[]) => save(chooserToSaved(saved, next)),
    [save, saved],
  );
  return { saved, chooser, keys, save, onChooser };
}
