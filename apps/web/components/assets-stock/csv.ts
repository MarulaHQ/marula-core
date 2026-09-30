"use client";
/**
 * Client-side CSV export for a data surface.
 *
 * Purpose: the pack's export contract is "exactly what you see, nothing
 * else" (DESIGN-NOTES-tickets §8) — the CURRENT view, with visible columns
 * in the user's order — so the browser builds the file from the rows it is
 * already showing rather than asking the server for a different set.
 *
 * Non-purpose: this is not the async path. `ExportControl` owns the threshold
 * decision above 5,000 rows and says so; this writes the small case, which is
 * the case a ledger on a user's screen actually reaches.
 */

function escapeCell(value: string): string {
  if (/[",\n]/.test(value)) return `"${value.replace(/"/g, '""')}"`;
  return value;
}

/** `projects.csv` from "Q3 projects" — the file name a human would pick. */
export function csvFileName(label: string, stamp: string): string {
  return `${label.replace(/[^\w-]+/g, "-").toLowerCase()}-${stamp}.csv`;
}

export function downloadCsv(
  fileName: string,
  columns: string[],
  rows: Record<string, string>[],
): void {
  const header = columns.map((column) => escapeCell(column)).join(",");
  const body = rows.map((row) => columns.map((column) => escapeCell(row[column] ?? "")).join(","));
  const blob = new Blob([[header, ...body].join("\n")], {
    type: "text/csv;charset=utf-8",
  });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = fileName;
  anchor.click();
  URL.revokeObjectURL(url);
}
