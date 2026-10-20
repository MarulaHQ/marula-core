"use client";
/**
 * ExportControl — Wave 41 lane 2b, D12: export exactly what is on screen.
 *
 * Purpose: hand the user a file that matches the table they are looking at —
 * the same filter, the same sort, the same column order, the same visible
 * columns — and say how many rows are in it BEFORE they ask. An export whose
 * row count is a surprise is how a user mails a 40,000-row ledger to a client
 * by accident, so the count sits on the control, not behind it.
 *
 * The async path is not a nicety. A ledger past a few thousand rows cannot be
 * built in the browser without the tab going away, so the same control takes
 * a job: it starts one, reports progress, and hands back a reference. The
 * caller owns the job; this owns the moment before it starts.
 *
 * Non-purpose: this builds no file. It reports the request and renders the
 * two states a caller must handle — ready, and working.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { cn } from "../lib/cn";
import { formatNumber } from "../lib/number";
import { Icon } from "../kit";

export type ExportFormat = "csv" | "xlsx";

export interface ExportRequest {
  format: ExportFormat;
  /** Row count of the CURRENT view, as the caller knows it. */
  rowCount: number;
  /** The visible columns, in the order the table is showing them. */
  columns: string[];
  sort?: { field: string; dir: "asc" | "desc" };
  filters?: unknown;
}

/** Human wording for a sort field the caller passes as a schema key. */
function readableField(field: string): string {
  const words = field.replace(/[_-]+/g, " ").trim();
  return words.length === 0 ? field : words.charAt(0).toUpperCase() + words.slice(1);
}

/** How many filters the caller handed us, when that can be counted at all. */
function countFilters(filters: unknown): number | null {
  if (filters === undefined || filters === null) return 0;
  if (Array.isArray(filters)) return filters.length;
  if (typeof filters === "object") return Object.keys(filters).length;
  return null;
}

export interface ExportControlProps {
  /** Columns the export will contain, in table order. */
  columns: string[];
  /** Rows the export will contain, given the current filter. */
  rowCount: number;
  sort?: { field: string; dir: "asc" | "desc" };
  /** Human label for the sort field; falls back to the field key. */
  sortLabel?: string;
  filters?: unknown;
  onExport: (request: ExportRequest) => void | Promise<void>;
  /** Above this many rows the export goes through the async path, and the
   * control says so rather than pretending the browser can hold it. */
  asyncThreshold?: number;
  label?: string;
  className?: string;
  formats?: ExportFormat[];
}

export function ExportControl({
  columns,
  rowCount,
  sort,
  sortLabel,
  filters,
  onExport,
  asyncThreshold = 5000,
  label = "Export",
  className,
  formats = ["csv", "xlsx"],
}: ExportControlProps): React.JSX.Element {
  const [working, setWorking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const isAsync = rowCount > asyncThreshold;

  /* What the file will hold, in the order a reader checks it: rows, filters,
     sort, then the columns. The control already received every part; stating
     only the rows made the menu a count rather than a description of the set. */
  const filterCount = countFilters(filters);
  const setSummary = [
    `${formatNumber(rowCount)} ${rowCount === 1 ? "row" : "rows"}`,
    filterCount === null
      ? null
      : filterCount === 0
        ? "no filters"
        : `${filterCount} ${filterCount === 1 ? "filter" : "filters"}`,
    sort === undefined ? null : `sorted by ${sortLabel ?? readableField(sort.field)}, ${sort.dir === "asc" ? "ascending" : "descending"}`,
    `${columns.length} ${columns.length === 1 ? "column" : "columns"}`,
  ]
    .filter((part): part is string => part !== null)
    .join(" · ");

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") setOpen(false);
    };
    const onPointer = (e: MouseEvent): void => {
      if (root.current && !root.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onPointer);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onPointer);
    };
  }, [open]);

  const run = useCallback(
    async (format: ExportFormat) => {
      if (columns.length === 0) {
        setError("No columns are visible, so there is nothing to export.");
        return;
      }
      setError(null);
      setWorking(true);
      try {
        await onExport({ format, rowCount, columns, sort, filters });
      } catch (e) {
        setError(e instanceof Error ? e.message : "The export did not finish.");
      } finally {
        setWorking(false);
        setOpen(false);
      }
    },
    [columns, filters, onExport, rowCount, sort],
  );

  return (
    <div className={cn("m-export", className)} ref={root}>
      <button
        type="button"
        className="m-toolbar-button"
        aria-expanded={open}
        aria-haspopup="menu"
        disabled={working}
        onClick={() => setOpen((v) => !v)}
      >
        <Icon name="download" size={14} />
        {working ? "Working…" : label}
        <span className="m-toolbar-count" aria-hidden="true">
          {formatNumber(rowCount)}
        </span>
      </button>
      {open ? (
        <div className="m-export-menu" role="menu" aria-label={`${label} to a file`}>
          <p className="m-export-count" role="note">
            {setSummary} — exactly what is on screen.
          </p>
          {formats.map((format) => (
            <button
              key={format}
              type="button"
              role="menuitem"
              className="m-export-item"
              disabled={working}
              onClick={() => void run(format)}
            >
              {working ? "Working…" : `${format.toUpperCase()}`}
            </button>
          ))}
          {isAsync ? (
            <p className="m-export-note">
              Over {formatNumber(asyncThreshold)} rows, so this runs in the background and lands
              in your notifications.
            </p>
          ) : null}
          {error ? (
            <p className="m-export-error" role="alert">
              {error}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
