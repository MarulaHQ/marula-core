// /tickets — the ticket ledger, rebuilt as a Marula data surface.
//
// Purpose: the icon rail's Tickets target. A list a technician works FROM, not
// a dump of rows — so it carries the four data-surface controls (ViewSwitch,
// FilterBar, ColumnChooser, ExportControl) on one DataToolbar, bulk selection
// for the actions a person takes on twenty tickets at once, and the two facts a
// queue is judged on per row: who owns it and how long it has left.
//
// Three rules the surface obeys:
//  1. The view, the columns, the filters and the sort are the user's. They live
//     in `appearance.data_surfaces` under `tickets.list` through
//     `useDataSurfacePrefs`, so a column order survives a reload and a second
//     device.
//  2. A row is never a checkbox-only target. Selection is a leading column and
//     the row itself stays one link to the workbench.
//  3. Nothing invents a deadline. The countdown prefers the SLA clock the
//     server reports and falls back to the ticket's own due date, and a ticket
//     with neither reads "—" rather than claiming zero.
//
// Non-purpose: no editing of ticket content. The workbench owns every write
// path except the two bulk acts below, which are one PATCH each and report
// exactly how many rows took.
"use client";

import {
  Badge,
  Button,
  Card,
  ColumnChooser,
  DataToolbar,
  EmptyState,
  ExportControl,
  FilterBar,
  Spinner,
  ViewSwitch,
  type ChoosableColumn,
  type ExportRequest,
  type FilterClause,
  type ViewOption,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";
import { useDataSurfacePrefs, type SavedSurfaceView } from "../../lib/data-surface-prefs";
import {
  TicketTable,
  type LedgerColumnDef,
} from "../../components/tickets/ledger-table";
import { useTicketSlaMap } from "../../lib/ticket-sla";
import {
  CATEGORY_LABEL,
  TICKET_FILTER_FIELDS,
  matchesClauses,
  priorityBadge,
  relativeTime,
  slaReading,
  sortTickets,
  statusTone,
  type TicketCategory,
  type TicketRow,
} from "../../lib/ticket-ops";

/** The D12 key this surface's display state is saved under. */
const SURFACE = "tickets.list";
/** One page. Deep pages go through the keyset cursor, which a bulk-selection
 *  surface cannot act on without walking every id first. */
const PAGE_LIMIT = 50;

const VIEWS: ViewOption[] = [
  { id: "open", label: "Open" },
  { id: "in_progress", label: "In progress" },
  { id: "waiting", label: "Waiting" },
  { id: "closed", label: "Closed" },
  { id: "all", label: "All" },
];

type ColumnDef = ChoosableColumn & { sortable: boolean };

const COLUMN_DEFS: ColumnDef[] = [
  { key: "number", label: "Number", visible: true, required: true, width: 116, sortable: true },
  { key: "title", label: "Subject", visible: true, required: true, width: 320, sortable: true },
  { key: "category", label: "Status", visible: true, width: 116, sortable: true },
  { key: "priority", label: "Priority", visible: true, width: 88, sortable: true },
  { key: "assignee", label: "Assignee", visible: true, width: 150, sortable: false },
  { key: "sla", label: "SLA", visible: true, width: 116, sortable: false },
  { key: "updated_at", label: "Updated", visible: true, width: 104, sortable: true },
];

const DEFAULT_COLUMNS: ChoosableColumn[] = COLUMN_DEFS.map((c) => ({
  key: c.key,
  label: c.label,
  visible: c.visible,
  width: c.width,
  required: c.required,
}));

/** Merge the surface's own defaults with whatever the user saved. A column the
 *  surface has since dropped stops being offered instead of becoming an
 *  unremovable ghost, and a column added since they last saved appears. */
function reconcileColumns(saved: SavedSurfaceView["columns"]): ChoosableColumn[] {
  if (saved.length === 0) return DEFAULT_COLUMNS;
  const byKey = new Map(saved.map((c) => [c.key, c]));
  const known = COLUMN_DEFS.map((def) => {
    const prior = byKey.get(def.key);
    return {
      key: def.key,
      label: def.label,
      visible: prior?.visible ?? def.visible,
      width: prior?.width ?? def.width,
      required: def.required,
    };
  });
  return known;
}

function clausesFromSaved(saved: { field: string; op: string; value?: string }[]): FilterClause[] {
  const allowed = new Set(TICKET_FILTER_FIELDS.map((f) => f.key));
  return saved
    .filter((f) => allowed.has(f.field))
    .map((f) => ({
      id: `${f.field}:${f.op}:${f.value ?? ""}`,
      field: f.field,
      op: f.op as FilterClause["op"],
      value: f.value,
    }));
}

function toSavedFilters(clauses: FilterClause[]): { field: string; op: string; value?: string }[] {
  return clauses.map((c) => ({ field: c.field, op: c.op, value: c.value }));
}

function csvCell(value: string): string {
  return /[",\n]/.test(value) ? `"${value.replace(/"/g, '""')}"` : value;
}

export default function TicketsListPage() {
  const auth = useRequireAuth();
  const prefs = useDataSurfacePrefs();
  const saved = prefs.view(SURFACE);

  const [view, setView] = useState<string>(saved.view ?? "open");
  const [rows, setRows] = useState<TicketRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [people, setPeople] = useState<Record<string, string>>({});
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [bulkBusy, setBulkBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [bulkPriority, setBulkPriority] = useState<components["schemas"]["Priority"]>("p2");
  const [reloadKey, setReloadKey] = useState(0);

  const columns = useMemo(() => reconcileColumns(saved.columns), [saved.columns]);
  const clauses = useMemo(() => clausesFromSaved(saved.filters), [saved.filters]);
  const sort = saved.sort;
  const visible = useMemo(() => columns.filter((c) => c.visible), [columns]);
  const visibleDefs = useMemo<LedgerColumnDef[]>(
    () =>
      visible.map((c) => ({
        ...c,
        sortable: COLUMN_DEFS.find((d) => d.key === c.key)?.sortable === true,
      })),
    [visible],
  );

  // --- data ---------------------------------------------------------------
  const refresh = useCallback(
    async (activeView: string) => {
      setRows(null);
      setError(null);
      const query =
        activeView === "all"
          ? { limit: PAGE_LIMIT }
          : { category: activeView as TicketCategory, limit: PAGE_LIMIT };
      const result = await api.api.GET("/api/v1/tickets", { params: { query } });
      if (result.error !== undefined || result.data === undefined) {
        setError("The ticket list did not load. Check your connection and retry.");
        setRows([]);
        return;
      }
      const next = [...result.data].sort((a, b) => b.updated_at.localeCompare(a.updated_at));
      setRows(next);
    },
    [],
  );

  useEffect(() => {
    if (auth.status === "authed") void refresh(view);
  }, [auth.status, view, refresh, reloadKey]);

  // The SLA clock the customer is promised, per row. The read itself is shared
  // and cached by ticket id (lib/ticket-sla), so this page pays one capped
  // fan-out per ticket per SESSION rather than fifty calls every time the
  // ledger is opened, and the workbench pays none for a ticket already seen.
  const slaIds = useMemo(() => (rows ?? []).map((r) => r.id), [rows]);
  // A merged row links to its survivor, which is usually a row already on
  // this page — so the banner can name it instead of claiming it is out of
  // scope.
  const numbersById = useMemo(() => {
    const map: Record<string, string> = {};
    for (const row of rows ?? []) map[row.id] = row.number;
    return map;
  }, [rows]);
  const { clocks: sla } = useTicketSlaMap(slaIds);

  // Assignee names. The staff directory is manager-scoped, so its absence
  // degrades the column to "You" / "Assigned" rather than breaking the surface.
  useEffect(() => {
    if (auth.status !== "authed") return;
    let cancelled = false;
    void (async () => {
      try {
        const res = await api.api.GET("/api/v1/org/users", {
          params: { query: { limit: 200 } },
        });
        if (cancelled || res.data === undefined) return;
        const map: Record<string, string> = {};
        for (const user of res.data.users) map[user.id] = user.display_name;
        setPeople(map);
      } catch {
        if (!cancelled) setPeople({});
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [auth.status]);

  // --- derived rows -------------------------------------------------------
  const filtered = useMemo(() => {
    if (rows === null) return null;
    return sortTickets(rows.filter((r) => matchesClauses(r, clauses)), sort);
  }, [rows, clauses, sort]);

  const shown = filtered ?? [];

  // A selection that outlives its row would let a bulk act on a ticket that is
  // no longer on screen, so it is pruned against what is actually rendered.
  const liveIds = useMemo(() => new Set(shown.map((r) => r.id)), [shown]);
  useEffect(() => {
    setSelected((prev) => {
      const next = new Set([...prev].filter((id) => liveIds.has(id)));
      return next.size === prev.size ? prev : next;
    });
  }, [liveIds]);

  const allSelected = shown.length > 0 && shown.every((r) => selected.has(r.id));

  const toggleAll = useCallback(() => {
    setSelected(allSelected ? new Set() : new Set(shown.map((r) => r.id)));
  }, [allSelected, shown]);

  const toggleOne = useCallback((id: string, on: boolean) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });
  }, []);

  // --- persistence --------------------------------------------------------
  const persist = useCallback(
    (patch: {
      view?: string;
      columns?: SavedSurfaceView["columns"];
      filters?: SavedSurfaceView["filters"];
      sort?: SavedSurfaceView["sort"];
    }) => {
      void prefs.save(SURFACE, {
        view: patch.view ?? view,
        columns: patch.columns ?? columns.map((c) => ({ key: c.key, visible: c.visible, width: c.width })),
        filters: patch.filters ?? saved.filters,
        sort: patch.sort ?? saved.sort,
      });
    },
    [prefs, view, columns, saved.filters, saved.sort],
  );

  const onView = useCallback(
    (id: string) => {
      setView(id);
      setSelected(new Set());
      persist({ view: id });
    },
    [persist],
  );

  const onColumns = useCallback(
    (next: ChoosableColumn[]) => {
      persist({ columns: next.map((c) => ({ key: c.key, visible: c.visible, width: c.width })) });
    },
    [persist],
  );

  const onClauses = useCallback(
    (next: FilterClause[]) => {
      persist({ filters: toSavedFilters(next) });
    },
    [persist],
  );

  const onSort = useCallback(
    (field: string) => {
      const dir = sort?.field === field && sort.dir === "asc" ? "desc" : "asc";
      persist({ sort: { field, dir } });
    },
    [persist, sort],
  );

  // --- bulk ---------------------------------------------------------------
  const runBulk = useCallback(
    async (
      patch: {
        priority?: components["schemas"]["Priority"];
        assignee_id?: string | null;
      },
      verb: string,
    ) => {
      const ids = [...selected];
      if (ids.length === 0) return;
      setBulkBusy(true);
      setNotice(null);
      let ok = 0;
      const failures: string[] = [];
      await Promise.all(
        ids.map(async (id) => {
          try {
            const res = await api.api.PATCH("/api/v1/tickets/{id}", {
              params: { path: { id } },
              body: patch,
            });
            if (res.error === undefined) ok += 1;
            else failures.push(id);
          } catch {
            failures.push(id);
          }
        }),
      );
      setBulkBusy(false);
      setNotice(
        failures.length === 0
          ? `${verb} on ${ok} ${ok === 1 ? "ticket" : "tickets"}.`
          : `${verb} on ${ok} of ${ids.length}. ${failures.length} could not be saved — retry those rows.`,
      );
      setSelected(new Set());
      setReloadKey((k) => k + 1);
    },
    [selected],
  );

  // --- export -------------------------------------------------------------
  const onExport = useCallback(
    (request: ExportRequest) => {
      const header = request.columns;
      const lines = [header.map(csvCell).join(",")];
      for (const row of shown) {
        lines.push(header.map((key) => csvCell(exportValue(row, key, sla, people, auth.me?.user_id ?? null))).join(","));
      }
      const blob = new Blob([lines.join("\n")], { type: "text/csv;charset=utf-8" });
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = `tickets-${view}-${new Date().toISOString().slice(0, 10)}.csv`;
      document.body.appendChild(anchor);
      anchor.click();
      anchor.remove();
      URL.revokeObjectURL(url);
      setNotice(`Exported ${shown.length} ${shown.length === 1 ? "row" : "rows"} as CSV.`);
    },
    [shown, view, sla, people, auth.me],
  );

  if (auth.status !== "authed") return null;

  const mergedCount = shown.filter((r) => r.merged_into_ticket_id !== null && r.merged_into_ticket_id !== undefined).length;

  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-4 p-4 md:p-6">
      <div className="min-w-0">
        <h1 className="text-lg font-semibold">Tickets</h1>
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Everything you can reach, newest activity first. Open a row to work it in the
          workbench.
        </p>
      </div>

      <DataToolbar
        label="Ticket list controls"
        views={<ViewSwitch views={VIEWS} activeId={view} onChange={onView} label="Ticket views" />}
        filters={
          <FilterBar
            fields={TICKET_FILTER_FIELDS}
            clauses={clauses}
            onChange={onClauses}
            label="Ticket filters"
            emptyHint="No filters"
          />
        }
        columns={<ColumnChooser columns={columns} onChange={onColumns} label="Ticket columns" />}
        export={
          <ExportControl
            columns={visible.map((c) => c.key)}
            rowCount={shown.length}
            sort={sort}
            filters={clauses}
            onExport={onExport}
          />
        }
        trailing={
          selected.size > 0 ? (
            <div className="m-bulk-bar" role="group" aria-label="Bulk actions">
              <span className="m-bulk-count">
                {selected.size} selected
              </span>
              <select
                className="m-input"
                aria-label="Priority to set on the selected tickets"
                value={bulkPriority}
                onChange={(e) => setBulkPriority(e.target.value as components["schemas"]["Priority"])}
              >
                <option value="p1">P1</option>
                <option value="p2">P2</option>
                <option value="p3">P3</option>
                <option value="p4">P4</option>
              </select>
              <Button
                size="sm"
                variant="secondary"
                disabled={bulkBusy}
                onClick={() => void runBulk({ priority: bulkPriority }, `Set ${bulkPriority.toUpperCase()}`)}
              >
                Set priority
              </Button>
              <Button
                size="sm"
                variant="secondary"
                disabled={bulkBusy || auth.me === null}
                onClick={() =>
                  auth.me !== null &&
                  void runBulk({ assignee_id: auth.me.user_id }, "Assigned to you")
                }
              >
                Assign to me
              </Button>
              <Button size="sm" variant="ghost" disabled={bulkBusy} onClick={() => setSelected(new Set())}>
                Clear
              </Button>
            </div>
          ) : null
        }
      />

      {notice !== null ? (
        <p className="text-[13px]" role="status" style={{ color: "var(--m-text-secondary)" }}>
          {notice}
        </p>
      ) : null}
      {prefs.error !== null ? (
        <p className="text-[13px]" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
          {prefs.error}
        </p>
      ) : null}

      {rows === null ? (
        <Card>
          <div className="p-4">
            <Spinner label="Loading tickets" />
          </div>
        </Card>
      ) : error !== null ? (
        <Card>
          <div className="flex items-center justify-between gap-3 p-4">
            <p role="alert" className="text-[13px]">
              {error}
            </p>
            <Button variant="secondary" size="sm" onClick={() => void refresh(view)}>
              Retry
            </Button>
          </div>
        </Card>
      ) : shown.length === 0 ? (
        <Card>
          <div className="p-6">
            <EmptyState
              message={
                clauses.length > 0
                  ? "No tickets match these filters. Remove one to widen the search."
                  : view === "all"
                    ? "There are no tickets yet."
                    : `No ${CATEGORY_LABEL[view as TicketCategory]?.toLowerCase() ?? ""} tickets in this view.`
              }
              action={
                clauses.length > 0 ? (
                  <Button size="sm" variant="secondary" onClick={() => onClauses([])}>
                    Clear filters
                  </Button>
                ) : undefined
              }
            />
          </div>
        </Card>
      ) : (
        <Card className="overflow-hidden">
          <TicketTable
            columns={visibleDefs}
            rows={shown}
            sort={sort}
            sla={sla}
            people={people}
            numbersById={numbersById}
            myId={auth.me?.user_id ?? null}
            selected={selected}
            onSort={onSort}
            onReorderColumns={(from, to) => {
              const next = columns.slice();
              const [moved] = next.splice(from, 1);
              if (moved !== undefined) next.splice(to, 0, moved);
              onColumns(next);
            }}
            onResizeColumn={(key, width) =>
              onColumns(columns.map((c) => (c.key === key ? { ...c, width } : c)))
            }
            onToggleRow={toggleOne}
          />
          <p className="m-table-foot">
            {shown.length === PAGE_LIMIT
              ? `Showing the ${PAGE_LIMIT} most recent. Narrow the view to reach older tickets.`
              : `${shown.length} ${shown.length === 1 ? "ticket" : "tickets"}`}
            {mergedCount > 0 ? ` · ${mergedCount} merged` : ""}
          </p>
        </Card>
      )}
    </div>
  );
}

// --- one row ---------------------------------------------------------------

function exportValue(
  row: TicketRow,
  key: string,
  sla: Record<string, components["schemas"]["TicketSlaStatus"] | null>,
  people: Record<string, string>,
  myId: string | null,
): string {
  switch (key) {
    case "category":
      return CATEGORY_LABEL[row.category] ?? row.category;
    case "priority":
      return row.priority.toUpperCase();
    case "assignee":
      if (!row.assignee_id) return "Unassigned";
      return people[row.assignee_id] ?? (row.assignee_id === myId ? "You" : "Assigned");
    case "sla":
      return slaReading(row, sla[row.id] ?? null)?.label ?? "";
    case "updated_at":
      return row.updated_at;
    default: {
      const raw = (row as unknown as Record<string, unknown>)[key];
      return raw === null || raw === undefined ? "" : String(raw);
    }
  }
}
