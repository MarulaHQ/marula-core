// The ledger's table: header controls, rows, cells and the merged-row banner.
//
// Purpose: this is the whole visual contract of the ticket ledger in one
// presentational module. It takes rows and callbacks and owns no data, no
// fetching and no persistence — which is what lets it be rendered in
// isolation (a screenshot pass, a visual regression fixture) without standing
// up the API, and what keeps `app/tickets/page.tsx` about the surface's state
// rather than about markup.
//
// The header is where the four data-surface controls meet the table: column
// reorder and resize are delegated to `Th`, sorting is a control rendered HERE
// because `Th` deliberately leaves a bare Arrow to the surface (docs: a
// primitive that swallowed it would break every sortable table).
"use client";

import {
  BulkHeaderRow,
  Checkbox,
  Table,
  Td,
  Th,
  Tr,
  type ChoosableColumn,
} from "@marula-core/ui";
import { Avatar, Pill, PriorityChip, type Priority } from "@marula-core/ui/kit";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import type { ReactNode } from "react";
import { MergeBanner } from "./merge-banner";
import {
  CATEGORY_LABEL,
  relativeTime,
  slaReading,
  type TicketRow,
} from "../../lib/ticket-ops";
import "./ledger-chrome.css";

/** Rows past this share entrance delay 0, so a full page is not still
 *  revealing while the reader is already on the last row. */
export const STAGGER_LIMIT = 20;

export interface LedgerColumnDef extends ChoosableColumn {
  sortable: boolean;
}

/**
 * Name lookups for the pack's "available but hidden" columns (Contact, Board,
 * Type). The surface loads each map only while its column is shown, so the
 * ledger never pays a read for a column nobody turned on.
 */
export interface LedgerNameMaps {
  /** contact id -> contact name, for the Contact column. */
  contacts?: Record<string, string>;
  /** board id -> board name, for the Board column. */
  boards?: Record<string, string>;
  /** taxonomy type id -> type name, for the Type column. */
  types?: Record<string, string>;
}

/**
 * The selection bar, rendered IN PLACE of the column header while rows are
 * selected (pack state 05). The placement is the law: the layout never jumps
 * because the bar occupies exactly the header's row. The markup and the gold
 * tint live in the kit now (`BulkHeaderRow` / `BulkActionBar`) — this interface
 * is the surface's half of the contract: how many rows, how to clear them, and
 * the controls the surface that owns the writes supplies.
 */
export interface LedgerBulkBar {
  /** How many rows are selected. */
  count: number;
  /** Clears the selection. The kit renders the button, last on the bar. */
  onClear: () => void;
  /** The surface's merge path, when it has one. */
  onMerge?: () => void;
  /** The controls, owned by the surface that owns the writes. */
  children: ReactNode;
}

export function TicketTable({
  columns,
  rows,
  sort,
  sla,
  people,
  clients,
  names,
  bulk,
  numbersById,
  myId,
  selected,
  onSort,
  onReorderColumns,
  onResizeColumn,
  onToggleRow,
  onTake,
}: {
  columns: LedgerColumnDef[];
  rows: TicketRow[];
  sort: { field: string; dir: "asc" | "desc" } | undefined;
  sla: Record<string, components["schemas"]["TicketSlaStatus"] | null>;
  people: Record<string, string>;
  /** client id -> display name, for the Client column. */
  clients: Record<string, string>;
  /** Names for the optional hidden columns; absent until a column shows. */
  names?: LedgerNameMaps;
  /** While rows are selected, the bar that replaces the column header. */
  bulk?: LedgerBulkBar;
  /** ticket id -> number, so a merged row can name its survivor. */
  numbersById: Record<string, string>;
  myId: string | null;
  selected: ReadonlySet<string>;
  onSort: (field: string) => void;
  onReorderColumns: (from: number, to: number) => void;
  onResizeColumn: (columnKey: string, width: number) => void;
  onToggleRow: (id: string, on: boolean) => void;
  /** Offered per row on unassigned rows: the one-click Take it. */
  onTake?: (row: TicketRow) => void;
}): React.JSX.Element {
  const allSelected = rows.length > 0 && rows.every((r) => selected.has(r.id));
  return (
    <>
      <Table>
        {/* Column widths live on a colgroup, not only on the header cells: the
            bulk bar REPLACES the header row, and with auto table layout the
            widths the header carried vanish with it — the columns then share the
            row equally and every cell wraps. The colgroup keeps the geometry
            whether or not a row is selected. */}
        <colgroup>
          <col style={{ width: 40 }} />
          {columns.map((column) => (
            <col key={column.key} style={{ width: column.width }} />
          ))}
        </colgroup>
        <thead>
          {bulk !== undefined ? (
            <BulkHeaderRow
              columnCount={columns.length + 1}
              count={bulk.count}
              noun="ticket"
              onClear={bulk.onClear}
              onMerge={bulk.onMerge}
              testId="ledger-bulk-bar"
              leading={
                /* The pack keeps the select column's checkbox on the bar's own
                   row, so the column does not close up under the pointer. */
                <Checkbox
                  checked
                  onCheckedChange={() => bulk.onClear()}
                  aria-label="Deselect all tickets"
                />
              }
            >
              {bulk.children}
            </BulkHeaderRow>
          ) : (
            <tr>
              <Th className="w-10">
                <Checkbox
                  checked={allSelected}
                  onCheckedChange={(on) => {
                    for (const row of rows) onToggleRow(row.id, on === true);
                  }}
                  aria-label={allSelected ? "Clear selection" : `Select all ${rows.length} tickets`}
                />
              </Th>
              {columns.map((column, index) => {
                const active = sort?.field === column.key;
                const canSort = column.sortable;
                return (
                  <Th
                    key={column.key}
                    columnKey={column.key}
                    width={column.width}
                    index={index}
                    columnCount={columns.length}
                    sortable={canSort}
                    resizable={column.width !== undefined}
                    onReorder={onReorderColumns}
                    onResize={onResizeColumn}
                    aria-sort={
                      canSort
                        ? active
                          ? sort?.dir === "asc"
                            ? "ascending"
                            : "descending"
                          : "none"
                        : undefined
                    }
                  >
                    {canSort ? (
                      <button type="button" className="m-th-sort" onClick={() => onSort(column.key)}>
                        {column.label}
                        <span className="m-th-sort-mark" aria-hidden="true">
                          {active ? (sort?.dir === "asc" ? "\u25b2" : "\u25bc") : "\u2195"}
                        </span>
                      </button>
                    ) : (
                      column.label
                    )}
                  </Th>
                );
              })}
            </tr>
          )}
        </thead>
        <tbody className="m-enter">
          {rows.map((row, index) => (
            <TicketRowView
              key={row.id}
              row={row}
              index={index}
              columns={columns}
              sla={sla[row.id] ?? null}
              people={people}
              clients={clients}
              names={names}
              numbersById={numbersById}
              myId={myId}
              selected={selected.has(row.id)}
              onToggle={(on) => onToggleRow(row.id, on)}
              onTake={onTake}
            />
          ))}
        </tbody>
      </Table>
    </>
  );
}

export function TicketRowView({
  row,
  index,
  columns,
  sla,
  people,
  clients,
  names,
  numbersById,
  myId,
  selected,
  onToggle,
  onTake,
}: {
  row: TicketRow;
  index: number;
  columns: ChoosableColumn[];
  sla: components["schemas"]["TicketSlaStatus"] | null;
  people: Record<string, string>;
  clients: Record<string, string>;
  names?: LedgerNameMaps;
  numbersById: Record<string, string>;
  myId: string | null;
  selected: boolean;
  onToggle: (on: boolean) => void;
  onTake?: (row: TicketRow) => void;
}): React.JSX.Element {
  const survivor = row.merged_into_ticket_id ?? null;
  // Rows past the stagger limit share delay 0, so a full page does not still
  // be revealing while the reader is already on the last row. The delay is set
  // here rather than derived from a custom property because only this surface
  // knows how many rows it rendered.
  const entrance = { animationDelay: `${index < STAGGER_LIMIT ? index * 12 : 0}ms` };
  return (
    <>
      <Tr selected={selected} style={entrance} data-testid="ticket-row">
        <Td>
          <Checkbox
            checked={selected}
            onCheckedChange={(on) => onToggle(on === true)}
            aria-label={`Select ${row.number}`}
          />
        </Td>
        {columns.map((column) => (
          <Td key={column.key}>
            <TicketCell
              columnKey={column.key}
              row={row}
              sla={sla}
              people={people}
              clients={clients}
              names={names}
              onTake={onTake}
              myId={myId}
            />
          </Td>
        ))}
      </Tr>
      {survivor !== null ? (
        <tr className="m-merge-row" style={entrance}>
          <td colSpan={columns.length + 1}>
            <MergeBanner
              survivorId={survivor}
              survivorLabel={numbersById[survivor] ?? null}
            />
          </td>
        </tr>
      ) : null}
    </>
  );
}

export function TicketCell({
  columnKey,
  row,
  sla,
  people,
  clients,
  names,
  onTake,
  myId,
}: {
  columnKey: string;
  row: TicketRow;
  sla: components["schemas"]["TicketSlaStatus"] | null;
  people: Record<string, string>;
  clients: Record<string, string>;
  names?: LedgerNameMaps;
  /** Present when the surface offers one-click Take it on unassigned rows. */
  onTake?: (row: TicketRow) => void;
  myId: string | null;
}): React.JSX.Element | null {
  switch (columnKey) {
    case "number":
      return (
        <span
          className="whitespace-nowrap text-[12.5px] font-semibold tabular-nums"
          style={{ color: "var(--m-text)" }}
        >
          {row.number}
        </span>
      );
    case "title":
      return (
        <span className="flex min-w-0 items-center gap-2">
          <Link href={`/tickets/${row.id}`} className="min-w-0 truncate no-underline" style={{ color: "var(--m-text)" }}>
            {row.title}
          </Link>
          {row.merged_into_ticket_id ? (
            <span className="m-merged-flag">merged</span>
          ) : null}
        </span>
      );
    case "category":
      return <Pill tone={PILL_TONE[row.category] ?? "neutral"}>{CATEGORY_LABEL[row.category] ?? row.category}</Pill>;
    case "priority":
      return <PriorityChip level={row.priority.toUpperCase() as Priority} />;
    case "assignee": {
      if (!row.assignee_id) {
        // The pack: an unassigned row offers one-click Take it, the queue a
        // dispatcher works from in a single motion (D12 — the action rides
        // the row, not a toolbar). One PATCH, and the row becomes "You".
        if (onTake === undefined) return <Avatar unassigned aria-label="Unassigned" />;
        return (
          <div className="flex min-w-0 items-center gap-2">
            <Avatar unassigned aria-label="Unassigned" />
            <button
              type="button"
              className="m-take-it"
              onClick={() => onTake(row)}
              aria-label={`Take ${row.number}`}
            >
              Take it
            </button>
          </div>
        );
      }
      const name = people[row.assignee_id] ?? (row.assignee_id === myId ? "You" : "Assigned");
      return (
        <span className="flex min-w-0 items-center gap-2">
          <Avatar initials={initialsOf(name)} />
          <span className="truncate text-[13px]">{name}</span>
        </span>
      );
    }
    case "client": {
      const name = row.client_id ? clients[row.client_id] : undefined;
      return (
        <span className="truncate text-[13px]" style={{ color: name ? "var(--m-text)" : "var(--m-text-muted)" }}>
          {name ?? "\u2014"}
        </span>
      );
    }
    case "sla": {
      const reading = slaReading(row, sla);
      if (reading === null) {
        return (
          <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
            &mdash;
          </span>
        );
      }
      // Colour is never alone (DESIGN-NOTES-tickets §9): a breached clock says
      // "Breached 1h 05m" with an alert icon in the red pair, an at-risk one
      // reads "42m left" in the warning pair, and a paused clock says "Paused"
      // with a pause icon. Calm stays plain text.
      if (sla?.paused === true) {
        return (
          <span className="m-sla-pill" style={{ background: "var(--m-status-warning-bg)", color: "var(--m-status-warning-fg)" }}>
            <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" aria-hidden="true">
              <path d="M9 5v14M15 5v14" strokeLinecap="round" />
            </svg>
            Paused
          </span>
        );
      }
      if (reading.tone === "danger") {
        const span = reading.label.replace(/\s*over$/, "");
        const applied = sla?.applied === true;
        return (
          <span className="m-sla-pill" style={{ background: "var(--m-status-danger-bg)", color: "var(--m-status-danger-fg)" }}>
            <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" aria-hidden="true">
              <path d="M12 3 2.5 20h19L12 3Z" strokeLinejoin="round" />
              <path d="M12 10v4M12 17.2v.1" strokeLinecap="round" />
            </svg>
            {applied ? `Breached ${span}` : `Overdue ${span}`}
          </span>
        );
      }
      if (reading.tone === "warning") {
        return (
          <span className="m-sla-pill" style={{ background: "var(--m-status-warning-bg)", color: "var(--m-status-warning-fg)" }}>
            {reading.label}
          </span>
        );
      }
      return (
        <span className="text-[12.5px] tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
          {reading.label}
        </span>
      );
    }
    case "updated_at":
      return (
        <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
          {relativeTime(row.updated_at)}
        </span>
      );
    // The pack's available-but-hidden four. Each reads "—" when its name map
    // has not loaded (the surface loads a map only while its column shows),
    // so an unresolved id never renders as a raw uuid.
    case "contact": {
      const name = row.contact_id !== null && row.contact_id !== undefined
        ? names?.contacts?.[row.contact_id]
        : undefined;
      return (
        <span className="truncate text-[13px]" style={{ color: name ? "var(--m-text)" : "var(--m-text-muted)" }}>
          {name ?? "\u2014"}
        </span>
      );
    }
    case "board": {
      const name = names?.boards?.[row.board_id];
      return (
        <span className="truncate text-[13px]" style={{ color: name ? "var(--m-text)" : "var(--m-text-muted)" }}>
          {name ?? "\u2014"}
        </span>
      );
    }
    case "type": {
      const name = row.type_id !== null && row.type_id !== undefined
        ? names?.types?.[row.type_id]
        : undefined;
      return (
        <span className="truncate text-[13px]" style={{ color: name ? "var(--m-text)" : "var(--m-text-muted)" }}>
          {name ?? "\u2014"}
        </span>
      );
    }
    case "created_at":
      return (
        <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
          {relativeTime(row.created_at)}
        </span>
      );
    default:
      return null;
  }
}

/** Status category -> the kit Pill tone. Colour is never the only signal: the word is always shown. */
const PILL_TONE: Record<string, "new" | "progress" | "waiting" | "resolved"> = {
  open: "new",
  in_progress: "progress",
  waiting: "waiting",
  closed: "resolved",
};

/** "Fiona Owner" -> "FO". One token is not an initial; fall back to its first letter. */
function initialsOf(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  if (parts.length === 1) return parts[0]!.slice(0, 1).toUpperCase();
  return (parts[0]![0]! + parts[parts.length - 1]![0]!).toUpperCase();
}
