// /boards — board index, composed from the data-surface template.
//
// Purpose: the icon rail's Boards target. Wave 44 lane 2 (qa-p44-ui-tickets-boards)
// found this surface a nav-pass card list, so it is drawn the way the pack draws
// EVERY list: the Tickets list frame (the derivation row for /boards reads "from
// the data-surface template"), which means one toolbar row with the view switch
// left and Columns/Filter/Export right, a table with a density-respecting footer,
// the same two empty shapes and the same load-error shape as its sibling.
//
// What it does NOT carry: the boards surface has no create flow in this product
// (creating a board is an admin act under Settings), so the title row's primary
// slot stays empty rather than shipping a dead button. Board administration and
// the board tree stay on BoardNav, exactly as before.
//
// The view, the columns and the sort are the caller's, saved per user under
// `appearance.data_surfaces` → `boards.index` through `useDataSurfacePrefs`, so
// they survive a reload and a second device (D12).
"use client";

import {
  Badge,
  Button,
  Card,
  ColumnChooser,
  DataToolbar,
  DensitySwitch,
  EmptyState,
  ExportControl,
  FilterChips,
  FilterTrigger,
  Spinner,
  SurfaceHeader,
  type ChoosableColumn,
  type ExportRequest,
  type FilterClause,
  type ViewOption,
  SurfaceFooter,
} from "@marula-core/ui";
import {
  Segmented,
  SegmentedCount,
  SegmentedItem,
} from "@marula-core/ui/kit";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { api, useRequireAuth } from "../../lib/api";
import { useDataSurfacePrefs, type SavedSurfaceView } from "../../lib/data-surface-prefs";
import { Table, Td, Th, Tr } from "@marula-core/ui";
import { CATEGORY_LABEL, relativeTime } from "../../lib/ticket-ops";

type NavEntry = components["schemas"]["BoardNavEntry"];
type Group = { id: string; name: string; position: number };

/** The D12 key this surface's display state is saved under. */
const SURFACE = "boards.index";

/** The template's three named views, in the board's own terms: everything the
 *  caller can reach, the boards they hold a role on, and the starred ones. */
const VIEWS: ViewOption[] = [
  { id: "all", label: "All boards" },
  { id: "mine", label: "My boards" },
  { id: "favorites", label: "Favorites" },
];

const COLUMN_DEFS = [
  { key: "name", label: "Board", visible: true, required: true, width: 260, sortable: true },
  { key: "group", label: "Group", visible: true, width: 150, sortable: true },
  { key: "description", label: "Description", visible: true, width: 320, sortable: false },
  { key: "role", label: "Your role", visible: true, width: 120, sortable: true },
  { key: "default", label: "Default", visible: false, width: 90, sortable: true },
];

/** The filterable fields a board list honestly has: its name and its group. */
const FILTER_FIELDS = [
  { key: "name", label: "Board", operators: ["contains"] as const },
  { key: "group_name", label: "Group", operators: ["eq", "ne"] as const },
];

export default function BoardsIndexPage() {
  const auth = useRequireAuth();
  const prefs = useDataSurfacePrefs();
  const saved = prefs.view(SURFACE);

  const [view, setView] = useState<string>(() => {
    const v = saved.view;
    return v === "mine" || v === "favorites" ? v : "all";
  });
  const [rows, setRows] = useState<NavEntry[] | null>(null);
  const [groups, setGroups] = useState<Group[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [reloadKey, setReloadKey] = useState(0);
  // The filter builder's open state, owned by the page so the middle chips'
  // `+ Add filter` and the right-hand Filter trigger open one builder.
  const [filterOpen, setFilterOpen] = useState(false);

  const columns = useMemo<ChoosableColumn[]>(() => {
    if (saved.columns.length === 0) {
      return COLUMN_DEFS.map((c) => ({ key: c.key, label: c.label, visible: c.visible, width: c.width, required: c.required }));
    }
    const byKey = new Map(saved.columns.map((c) => [c.key, c]));
    return COLUMN_DEFS.map((def) => ({
      key: def.key,
      label: def.label,
      visible: byKey.get(def.key)?.visible ?? def.visible,
      width: byKey.get(def.key)?.width ?? def.width,
      required: def.required,
    }));
  }, [saved.columns]);

  const clauses = useMemo<FilterClause[]>(
    () =>
      saved.filters
        .filter((f) => FILTER_FIELDS.some((x) => x.key === f.field))
        .map((f) => ({
          id: `${f.field}:${f.op}:${f.value ?? ""}`,
          field: f.field,
          op: f.op as FilterClause["op"],
          value: f.value,
        })),
    [saved.filters],
  );

  const refresh = useCallback(async () => {
    setRows(null);
    setError(null);
    const nav = await api.api.GET("/api/v1/boards/nav");
    if (nav.error !== undefined || nav.data === undefined) {
      setError("Failed to load boards. Check your connection and retry.");
      setRows([]);
      return;
    }
    setRows(nav.data);
    const gs = await api.api.GET("/api/v1/board-groups");
    if (gs.error === undefined && gs.data !== undefined) setGroups([...gs.data]);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh, reloadKey]);

  const persist = useCallback(
    (patch: { view?: string; columns?: SavedSurfaceView["columns"] }) => {
      void prefs.save(SURFACE, {
        view: patch.view ?? view,
        columns: patch.columns ?? columns.map((c) => ({ key: c.key, visible: c.visible, width: c.width })),
        filters: saved.filters,
        sort: saved.sort,
      });
    },
    [prefs, view, columns, saved.filters, saved.sort],
  );

  const onView = useCallback(
    (id: string) => {
      setView(id);
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
      void prefs.save(SURFACE, {
        view,
        columns: columns.map((c) => ({ key: c.key, visible: c.visible, width: c.width })),
        filters: next.map((c) => ({ field: c.field, op: c.op, value: c.value })),
        sort: saved.sort,
      });
    },
    [prefs, view, columns, saved.sort],
  );

  const sort = saved.sort ?? { field: "name", dir: "asc" };

  const shown = useMemo(() => {
    if (rows === null) return [];
    let base = rows;
    if (view === "mine") base = base.filter((r) => r.my_role !== null && r.my_role !== undefined);
    else if (view === "favorites") base = base.filter((r) => r.favorite);
    const matched = base.filter((r) =>
      clauses.every((c) => {
        const actual = String((r as unknown as Record<string, unknown>)[c.field] ?? "");
        const value = (c.value ?? "").trim();
        if (c.op === "eq") return actual === value;
        if (c.op === "ne") return actual !== value;
        return actual.toLowerCase().includes(value.toLowerCase());
      }),
    );
    return [...matched].sort((a, b) => {
      const dir = sort?.dir === "asc" ? 1 : -1;
      const av = String((a as unknown as Record<string, unknown>)[sort?.field ?? "name"] ?? "");
      const bv = String((b as unknown as Record<string, unknown>)[sort?.field ?? "name"] ?? "");
      return av.localeCompare(bv) * dir;
    });
  }, [rows, view, clauses, sort]);

  const viewCounts = useMemo(
    () => ({
      all: rows?.length ?? 0,
      mine: rows?.filter((r) => r.my_role !== null && r.my_role !== undefined).length ?? 0,
      favorites: rows?.filter((r) => r.favorite).length ?? 0,
    }),
    [rows],
  );

  const onExport = useCallback(
    (request: ExportRequest) => {
      const keys = request.columns.length > 0 ? request.columns : visible.map((c) => c.key);
      const label = (row: NavEntry, key: string): string => {
        switch (key) {
          case "name": return row.name;
          case "group": return row.group_name ?? "Ungrouped";
          case "description": return row.description ?? "";
          case "role": return row.my_role ?? "";
          case "default": return row.is_default ? "default" : "";
          default: return "";
        }
      };
      const csvCell = (v: string): string => (/[",\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v);
      const lines = [keys.map(csvCell).join(",")];
      for (const row of shown) lines.push(keys.map((k) => csvCell(label(row, k))).join(","));
      const blob = new Blob([lines.join("\n")], { type: "text/csv;charset=utf-8" });
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = `boards-${view}-${new Date().toISOString().slice(0, 10)}.csv`;
      document.body.appendChild(anchor);
      anchor.click();
      anchor.remove();
      URL.revokeObjectURL(url);
    },
    [shown, view],
  );

  const visible = useMemo(() => columns.filter((c) => c.visible), [columns]);

  if (auth.status !== "authed") return null;

  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-4 p-4 md:p-6">
      <SurfaceHeader
        title="Boards"
        meta={
          groups.length > 0 ? (
            <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              {rows?.length ?? 0} {rows?.length === 1 ? "board" : "boards"} in{" "}
              {groups.length} {groups.length === 1 ? "group" : "groups"}
            </span>
          ) : null
        }
      />

      <DataToolbar
        label="Board list controls"
        views={
          <Segmented type="single" value={view} onValueChange={(v) => v && onView(v)} aria-label="Board views">
            {VIEWS.map((v) => (
              <SegmentedItem key={v.id} value={v.id}>
                {v.label}
                <SegmentedCount>{viewCounts[v.id as keyof typeof viewCounts]}</SegmentedCount>
              </SegmentedItem>
            ))}
          </Segmented>
        }
        chips={
          <FilterChips
            fields={FILTER_FIELDS.map((f) => ({ key: f.key, label: f.label, operators: [...f.operators] }))}
            clauses={clauses}
            onChange={onClauses}
            label="Board filters"
            emptyHint="No filters"
            onAdd={() => setFilterOpen(true)}
          />
        }
        filter={
          <FilterTrigger
            fields={FILTER_FIELDS.map((f) => ({ key: f.key, label: f.label, operators: [...f.operators] }))}
            clauses={clauses}
            onChange={onClauses}
            label="Board filters"
            open={filterOpen}
            onOpenChange={setFilterOpen}
          />
        }
        columns={<ColumnChooser columns={columns} onChange={onColumns} label="Board columns" />}
        export={<ExportControl columns={visible.map((c) => c.key)} rowCount={shown.length} sort={sort} filters={clauses} onExport={onExport} />}
      />

      {rows === null ? (
        <Card>
          <div className="p-4">
            <Spinner label="Loading boards" />
          </div>
        </Card>
      ) : error !== null ? (
        <Card>
          <div className="p-6">
            <EmptyState
              message="Couldn't load boards. Nothing else on this page is affected."
              illustration={<ErrorMark />}
              action={
                <Button size="sm" variant="primary" onClick={() => void refresh()}>
                  Retry
                </Button>
              }
            />
          </div>
        </Card>
      ) : shown.length === 0 ? (
        <Card>
          <div className="p-6">
            {clauses.length > 0 ? (
              <EmptyState
                message="No boards match these filters. Remove one to widen the search."
                illustration={<MagnifierMark />}
                action={
                  <Button size="sm" variant="secondary" onClick={() => onClauses([])}>
                    Clear filters
                  </Button>
                }
              />
            ) : (
              <EmptyState
                message={
                  view === "favorites"
                    ? "No favorites yet. Star a board from its page to pin it here."
                    : view === "mine"
                      ? "You hold no board role yet. Ask an admin for one, or look at All boards."
                      : "No boards yet. Once the org creates one it appears here."
                }
                illustration={<SparklesMark />}
                action={
                  view === "mine" && viewCounts.all > 0 ? (
                    <Button size="sm" variant="secondary" onClick={() => onView("all")}>
                      See all boards ({viewCounts.all})
                    </Button>
                  ) : undefined
                }
              />
            )}
          </div>
        </Card>
      ) : (
        <Card className="overflow-hidden">
          <Table>
            <thead>
              <tr>
                {visible.map((column) => (
                  <Th key={column.key} width={column.width}>
                    {column.label}
                  </Th>
                ))}
              </tr>
            </thead>
            <tbody>
              {shown.map((entry) => (
                <tr key={entry.id}>
                  {visible.map((column) => (
                    <Td key={column.key}>
                      <BoardCell entry={entry} columnKey={column.key} />
                    </Td>
                  ))}
                </tr>
              ))}
            </tbody>
          </Table>
          <SurfaceFooter
            shown={shown.length}
            total={shown.length === (rows?.length ?? 0) ? undefined : (rows?.length ?? 0)}
            noun="board"
            density={<DensitySwitch value={prefs.density} onChange={prefs.setDensity} variant="icons" label="Row density" />}
          />
        </Card>
      )}
    </div>
  );
}

function BoardCell({ entry, columnKey }: { entry: NavEntry; columnKey: string }): ReactNode {
  switch (columnKey) {
    case "name":
      return (
        <span className="flex min-w-0 items-center gap-2">
          <Link href={`/boards/${entry.id}`} className="truncate no-underline" style={{ color: "var(--m-text)" }}>
            {entry.name}
          </Link>
          {entry.favorite ? (
            <span aria-label="Favorite" style={{ color: "var(--m-accent)" }} title="You starred this board">
              ★
            </span>
          ) : null}
        </span>
      );
    case "group":
      return (
        <span className="truncate text-[13px]" style={{ color: entry.group_name ? "var(--m-text)" : "var(--m-text-muted)" }}>
          {entry.group_name ?? "Ungrouped"}
        </span>
      );
    case "description":
      return (
        <span className="truncate text-[13px]" style={{ color: entry.description ? "var(--m-text-secondary)" : "var(--m-text-muted)" }}>
          {entry.description ?? "No description"}
        </span>
      );
    case "role":
      return entry.my_role ? <Badge tone="neutral">{entry.my_role}</Badge> : <span style={{ color: "var(--m-text-muted)" }}>—</span>;
    case "default":
      return entry.is_default ? <Badge tone="neutral">default</Badge> : <span style={{ color: "var(--m-text-muted)" }}>—</span>;
    default:
      return null;
  }
}

// The three marks, the same family the tickets ledger uses (a magnifier, a
// sparkles cluster and a cross for "we could not load it" — the cross wears the
// status danger pair, because a failed read is an error message, docs/theme.md
// §3.4).
function EmptyMarkBase({ children }: { children: ReactNode }): React.JSX.Element {
  return (
    <svg width="48" height="48" viewBox="0 0 48 48" fill="none" stroke="var(--m-text-secondary)" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {children}
    </svg>
  );
}

function MagnifierMark(): React.JSX.Element {
  return (
    <EmptyMarkBase>
      <circle cx="21" cy="21" r="12" />
      <path d="m30 30 9 9" />
      <path d="M16 21h10" />
    </EmptyMarkBase>
  );
}

function SparklesMark(): React.JSX.Element {
  return (
    <EmptyMarkBase>
      <path d="M24 12c.8 4.5 3.5 7.2 8 8-4.5.8-7.2 3.5-8 8-.8-4.5-3.5-7.2-8-8 4.5-.8 7.2-3.5 8-8Z" />
      <path d="M35 32c.5 2.6 2 4.1 4.6 4.6-2.6.5-4.1 2-4.6 4.6-.5-2.6-2-4.1-4.6-4.6 2.6-.5 4.1-2 4.6-4.6Z" />
    </EmptyMarkBase>
  );
}

function ErrorMark(): React.JSX.Element {
  return (
    <svg width="48" height="48" viewBox="0 0 48 48" fill="none" stroke="var(--m-status-danger-fg)" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
      <circle cx="24" cy="24" r="13" />
      <path d="M24 16v9" />
      <path d="M24 29v.4" />
    </svg>
  );
}
