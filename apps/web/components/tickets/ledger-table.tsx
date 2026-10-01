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
import { MergeBanner } from "./merge-banner";
import {
  CATEGORY_LABEL,
  relativeTime,
  slaReading,
  type TicketRow,
} from "../../lib/ticket-ops";

/** Rows past this share entrance delay 0, so a full page is not still
 *  revealing while the reader is already on the last row. */
export const STAGGER_LIMIT = 20;

export interface LedgerColumnDef extends ChoosableColumn {
  sortable: boolean;
}

export function TicketTable({
  columns,
  rows,
  sort,
  sla,
  people,
  clients,
  numbersById,
  myId,
  selected,
  onSort,
  onReorderColumns,
  onResizeColumn,
  onToggleRow,
}: {
  columns: LedgerColumnDef[];
  rows: TicketRow[];
  sort: { field: string; dir: "asc" | "desc" } | undefined;
  sla: Record<string, components["schemas"]["TicketSlaStatus"] | null>;
  people: Record<string, string>;
  /** client id -> display name, for the Client column. */
  clients: Record<string, string>;
  /** ticket id -> number, so a merged row can name its survivor. */
  numbersById: Record<string, string>;
  myId: string | null;
  selected: ReadonlySet<string>;
  onSort: (field: string) => void;
  onReorderColumns: (from: number, to: number) => void;
  onResizeColumn: (columnKey: string, width: number) => void;
  onToggleRow: (id: string, on: boolean) => void;
}): React.JSX.Element {
  const allSelected = rows.length > 0 && rows.every((r) => selected.has(r.id));
  return (
    <>
      <Table>
        <thead>
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
              numbersById={numbersById}
              myId={myId}
              selected={selected.has(row.id)}
              onToggle={(on) => onToggleRow(row.id, on)}
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
  numbersById,
  myId,
  selected,
  onToggle,
}: {
  row: TicketRow;
  index: number;
  columns: ChoosableColumn[];
  sla: components["schemas"]["TicketSlaStatus"] | null;
  people: Record<string, string>;
  clients: Record<string, string>;
  numbersById: Record<string, string>;
  myId: string | null;
  selected: boolean;
  onToggle: (on: boolean) => void;
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
  myId,
}: {
  columnKey: string;
  row: TicketRow;
  sla: components["schemas"]["TicketSlaStatus"] | null;
  people: Record<string, string>;
  clients: Record<string, string>;
  myId: string | null;
}): React.JSX.Element | null {
  switch (columnKey) {
    case "number":
      return (
        <span className="text-[12.5px] font-semibold tabular-nums" style={{ color: "var(--m-text)" }}>
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
      if (!row.assignee_id) return <Avatar unassigned aria-label="Unassigned" />;
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
      return (
        <span
          className="text-[12.5px] tabular-nums"
          style={{
            color: reading.warn
              ? reading.tone === "danger"
                ? "var(--m-status-danger-fg)"
                : "var(--m-status-warning-fg)"
              : "var(--m-text-secondary)",
            fontWeight: reading.warn ? 600 : 400,
          }}
        >
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
