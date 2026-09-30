// CSV export for a data surface — the small case, done in the browser.
//
// Purpose: a data surface exports EXACTLY the view on screen: the visible
// columns, in the table's order, over the rows the filter produced. The
// ExportControl already says what it will do (rows, columns); this makes the
// file match that sentence, with no server round trip and no invented totals.
//
// Non-purpose: not the async report path (over 5,000 rows, where the control
// hands off to a background report — lib/reports.ts), and not xlsx.
import type { ExportRequest } from "@marula-core/ui";

/** RFC 4180 quoting: a value is quoted when it holds a delimiter, a quote or a
 *  newline, and an embedded quote is doubled. */
export function toCsv(header: readonly string[], rows: readonly (readonly string[])[]): string {
  const cell = (value: string): string => {
    if (!/[",\n\r]/.test(value)) return value;
    return `"${value.replace(/"/g, '""')}"`;
  };
  const lines = [header.map(cell).join(","), ...rows.map((row) => row.map(cell).join(","))];
  return lines.join("\r\n");
}

/** Build the CSV and hand it to the browser as a download. */
export function downloadRowsCsv(
  filename: string,
  header: readonly string[],
  rows: readonly (readonly string[])[],
): void {
  const blob = new Blob([`﻿${toCsv(header, rows)}`], {
    type: "text/csv;charset=utf-8",
  });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.click();
  URL.revokeObjectURL(url);
}

/** The ExportControl's handler, shaped to what the surface already has. */
export function csvExporter(
  filename: string,
  header: () => readonly string[],
  rows: () => readonly (readonly string[])[],
): (request: ExportRequest) => Promise<void> {
  return async () => {
    downloadRowsCsv(filename, header(), rows());
  };
}
