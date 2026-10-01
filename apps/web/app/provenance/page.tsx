// /provenance — the write trail as a data-surface template (Wave 41 lane 11).
//
// Purpose: who changed what, field by field, across every applied write, in
// the TL frame — one toolbar row (views + filter chips left, Columns +
// Export right), a 16px-radius bordered table, the 52px footer with
// density. **No bulk actions**: the pack takes the view switch, column
// chooser, filter bar and export control on provenance and audit, and
// explicitly rules out the selection bar. This is a read model; undo lives on
// the ticket surface, not here.
//
// Non-purpose: no editing, no reverting. Before → after renders in mono, the
// before value muted and the after value in ink — colour is a hint, the
// words are the content.
"use client";

import {
  Button,
  ColumnChooser,
  DataToolbar,
  DensitySwitch,
  ExportControl,
  FilterBar,
  Spinner,
  Table,
  Tag,
  Td,
  Th,
  Tr,
  type FilterClause,
  type FilterField,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";
import { rejectionText } from "../../lib/error-copy";
import {
  downloadCsv,
  toCsv,
  sortRows,
  toggleSort,
  useSurfaceTableColumns,
  type InsightColumn,
} from "../../lib/insight-table";
import { useDataSurfacePrefs } from "../../lib/data-surface-prefs";
import { SurfaceTabs } from "../../components/insight/surface-tabs";
import { InsightState } from "../../components/insight/insight-state";
import "../../components/insight/insight-table.css";

type ProvenanceEvent = components["schemas"]["ProvenanceEvent"];

const SURFACE_KEY = "provenance.trail";

/** The actor word carries the colour; the tag is never colour alone. */
const ACTOR_TONE: Record<string, "success" | "neutral" | "warning" | "danger"> = {
  human: "neutral",
  agent: "success",
  import: "warning",
};

/** Values render as readable short text with the full value on title. */
function valueCell(value: unknown): string {
  if (value === null || value === undefined) return "∅";
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

function entityHref(e: { entity: string; entity_id: string }): string | null {
  if (e.entity === "ticket") return `/tickets/${e.entity_id}`;
  return null;
}

type ProvRow = {
  key: string;
  occurred: string;
  occurredLabel: string;
  actorKind: string;
  entity: string;
  op: string;
  entityId: string;
  href: string | null;
  field: string;
  before: string;
  after: string;
  seq: number;
};

export default function ProvenancePage() {
  const auth = useRequireAuth();
  const prefs = useDataSurfacePrefs();

  const [events, setEvents] = useState<ProvenanceEvent[] | null>(null);
  const [cursor, setCursor] = useState<string | null>(null);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [view, setView] = useState<"all" | "agent" | "imports">("all");
  const [clauses, setClauses] = useState<FilterClause[]>([]);

  const FILTER_FIELDS: FilterField[] = useMemo(
    () => [
      { key: "field", label: "Field", operators: ["contains"] },
      { key: "entity", label: "Entity", operators: ["contains"] },
      { key: "actor", label: "Actor", operators: ["eq"] },
    ],
    [],
  );

  const load = useCallback(
    async (opts: { cursor: string | null }) => {
      setLoadError(null);
      const fieldClause = clauses.find((c) => c.field === "field");
      const entityClause = clauses.find((c) => c.field === "entity");
      const { data, error } = await api.api.GET("/api/v1/provenance", {
        params: {
          query: {
            field: fieldClause?.value === undefined || fieldClause.value === "" ? null : fieldClause.value,
            entity: entityClause?.value === undefined || entityClause.value === "" ? null : entityClause.value,
            cursor: opts.cursor,
            limit: 50,
          },
        },
      });
      if (error === undefined && data !== undefined) {
        // Pre-guard (lane 10 sweep): a resolving read can carry a malformed
        // payload — render an honest empty trail, never a crash.
        setEvents(Array.isArray(data.events) ? data.events : []);
        setCursor(opts.cursor);
        setNextCursor(data.next_cursor ?? null);
        return;
      }
      setLoadError(rejectionText(error as never, "The write trail did not load."));
    },
    [clauses],
  );

  useEffect(() => {
    if (auth.status === "authed") void load({ cursor: null });
  }, [auth.status, clauses, load]);

  const rows: ProvRow[] = useMemo(
    () =>
      (events ?? []).map((e) => ({
        key: `${e.write_id ?? e.idempotency_key ?? e.occurred_at}-${e.seq}-${e.field}`,
        occurred: e.occurred_at,
        occurredLabel: new Date(e.occurred_at).toLocaleString("en-GB", {
          day: "numeric",
          month: "short",
          hour: "2-digit",
          minute: "2-digit",
        }),
        actorKind: e.actor_kind,
        entity: e.entity,
        op: e.op,
        entityId: e.entity_id.slice(0, 8),
        href: entityHref(e),
        field: e.field,
        before: valueCell(e.before),
        after: valueCell(e.after),
        seq: e.seq,
      })),
    [events],
  );

  const columns: readonly InsightColumn<ProvRow>[] = useMemo(
    () => [
      {
        key: "when",
        label: "When",
        sortValue: (r) => r.occurred,
        render: (r) => (
          <span className="whitespace-nowrap text-[12px] tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
            {r.occurredLabel}
          </span>
        ),
      },
      {
        key: "actor",
        label: "Actor",
        sortValue: (r) => r.actorKind,
        render: (r) => <Tag tone={ACTOR_TONE[r.actorKind] ?? "neutral"}>{r.actorKind}</Tag>,
      },
      {
        key: "record",
        label: "Record",
        sortValue: (r) => `${r.entity} ${r.op} ${r.entityId}`,
        render: (r) =>
          r.href !== null ? (
            <Link
              href={r.href}
              className="text-[13px] underline decoration-[var(--m-border)] underline-offset-2 hover:decoration-[var(--m-accent)]"
              style={{ color: "var(--m-text)" }}
            >
              {r.entity} {r.op} · {r.entityId}
            </Link>
          ) : (
            <span className="text-[13px]" style={{ color: "var(--m-text)" }}>
              {r.entity} {r.op} · {r.entityId}
            </span>
          ),
      },
      {
        key: "field",
        label: "Field",
        required: true,
        sortValue: (r) => r.field,
        render: (r) => <span className="font-mono text-[12px]">{r.field}</span>,
      },
      {
        key: "change",
        label: "Before → After",
        render: (r) => (
          <span className="flex min-w-0 items-center gap-2 font-mono text-[11px]">
            <span
              className="max-w-56 truncate rounded px-1.5 py-0.5"
              style={{ background: "var(--m-surface-2)", color: "var(--m-text-secondary)" }}
              title={`before: ${r.before}`}
            >
              {r.before}
            </span>
            <span aria-hidden="true" style={{ color: "var(--m-text-muted)" }}>→</span>
            <span
              className="max-w-56 truncate rounded px-1.5 py-0.5"
              style={{ background: "var(--m-accent-soft)", color: "var(--m-text)" }}
              title={`after: ${r.after}`}
            >
              {r.after}
            </span>
          </span>
        ),
      },
      {
        key: "seq",
        label: "Seq",
        sortValue: (r) => r.seq,
        render: (r) => (
 <span className="text-right text-[11px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
            {r.seq}
          </span>
        ),
      },
    ],
    [],
  );

  const table = useSurfaceTableColumns<ProvRow>(SURFACE_KEY, prefs, columns);

  const viewFiltered = useMemo(() => {
    const base =
      view === "agent"
        ? rows.filter((r) => r.actorKind === "agent")
        : view === "imports"
          ? rows.filter((r) => r.actorKind === "import")
          : rows;
    if (clauses.length === 0) return base;
    return base.filter((r) => {
      const actorClause = clauses.find((c) => c.field === "actor");
      if (actorClause !== undefined && actorClause.value !== undefined) {
        if (actorClause.value !== "any" && r.actorKind !== actorClause.value) return false;
      }
      return true;
    });
  }, [rows, view, clauses]);

  const sorted = useMemo(
    () => sortRows(viewFiltered, "prov", columns, table.saved.sort),
    [viewFiltered, columns, table.saved.sort],
  );

  const handleExport = useCallback(() => {
    const csv = toCsv(table.keys, columns, sorted, (col, r) => {
      if (col.key === "when") return r.occurredLabel;
      if (col.key === "actor") return r.actorKind;
      if (col.key === "record") return `${r.entity} ${r.op} ${r.entityId}`;
      if (col.key === "change") return `${r.before} -> ${r.after}`;
      if (col.key === "field") return r.field;
      return String(r.seq);
    });
    downloadCsv("marula-provenance.csv", csv);
  }, [table.keys, columns, sorted]);

  if (auth.status !== "authed") return null;

  return (
    <div className="grid gap-4 p-6">
      <header className="m-rise grid gap-1.5">
        <div className="flex flex-wrap items-center gap-3">
          <h1
            className="text-[26px] font-semibold tracking-[-0.01em]"
            style={{ color: "var(--m-text)" }}
          >
            Provenance
          </h1>
          <span
            className="inline-flex h-6 items-center rounded-full px-2.5 text-[12px] font-semibold"
            style={{ background: "var(--m-surface-2)", color: "var(--m-text-secondary)" }}
          >
            Read-only
          </span>
        </div>
        <p className="max-w-2xl text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Who changed what, field by field, across every applied write — including agent
          sessions and imports. It never rewrites history.
        </p>
      </header>

      <DataToolbar
        label="Write trail controls"
        views={
          <SurfaceTabs
            tabs={[
              { id: "all", label: "All writes", hint: String(rows.length) },
              { id: "agent", label: "Agent", hint: String(rows.filter((r) => r.actorKind === "agent").length) },
              { id: "imports", label: "Imports", hint: String(rows.filter((r) => r.actorKind === "import").length) },
            ]}
            activeId={view}
            onChange={(id) => setView(id as "all" | "agent" | "imports")}
            label="Provenance views"
          />
        }
        filters={
          <FilterBar
            fields={FILTER_FIELDS}
            clauses={clauses}
            onChange={(next) => {
              const serverChanged =
                next.filter((c) => c.field === "field").length !==
                  clauses.filter((c) => c.field === "field").length ||
                next.filter((c) => c.field === "entity").length !==
                  clauses.filter((c) => c.field === "entity").length;
              setClauses(next);
              if (serverChanged) void load({ cursor: null });
            }}
            label="provenance filters"
            emptyHint=""
          />
        }
        columns={<ColumnChooser columns={table.chooser} onChange={table.onChooser} label="Provenance columns" />}
        export={
          <ExportControl
            columns={table.keys}
            rowCount={sorted.length}
            sort={table.saved.sort}
            onExport={() => handleExport()}
            label="Export"
          />
        }
      />

      {loadError !== null ? (
        <div
          role="alert"
          className="rounded-[16px] border"
          style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
        >
          <InsightState
            tone="error"
            title="Couldn't load the write trail"
            hint={`${loadError} Your filters and columns are saved, so nothing is lost.`}
            primary={
              <Button variant="primary" size="sm" onClick={() => void load({ cursor: null })}>
                Retry
              </Button>
            }
          />
        </div>
      ) : events === null ? (
        <div className="flex justify-center py-10">
          <Spinner />
        </div>
      ) : sorted.length === 0 ? (
        <div
          className="rounded-[16px] border"
          style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
        >
          <InsightState
            title="No writes match these filters"
            hint="Widen the window, or clear a filter to see the whole trail."
            primary={
              <Button variant="primary" size="sm" onClick={() => setClauses([])}>
                Clear filters
              </Button>
            }
          />
        </div>
      ) : (
        <div
          className="overflow-x-auto rounded-[16px] border"
          style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
        >
          <Table className="m-insight-table">
            <thead>
              <tr>
                {table.keys.map((key) => {
                  const col = columns.find((c) => c.key === key);
                  if (col === undefined) return null;
                  const active = table.saved.sort?.field === key;
                  return (
                    <Th
                      key={key}
                      columnKey={key}
                      sortable={false}
                      resizable={false}
                      aria-sort={active ? (table.saved.sort?.dir === "asc" ? "ascending" : "descending") : undefined}
                      className={col.sortValue !== undefined ? "cursor-pointer select-none" : undefined}
                      onClick={
                        col.sortValue !== undefined
                          ? () => table.save(toggleSort(table.saved, key))
                          : undefined
                      }
                    >
                      <span className="inline-flex items-center gap-1">
                        {col.label}
                        {active ? <span aria-hidden="true">{table.saved.sort?.dir === "asc" ? "↑" : "↓"}</span> : null}
                      </span>
                    </Th>
                  );
                })}
              </tr>
            </thead>
            <tbody>
              {sorted.map((r) => (
                <Tr key={r.key}>
                  {table.keys.map((key) => {
                    const col = columns.find((c) => c.key === key);
                    if (col === undefined) return null;
                    return <Td key={key}>{col.render(r)}</Td>;
                  })}
                </Tr>
              ))}
            </tbody>
          </Table>
          <div className="m-insight-footer flex min-h-[52px] flex-wrap items-center gap-3 px-3 pb-2">
            <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              <b className="tabular-nums" style={{ color: "var(--m-text)" }}>
                {sorted.length}
              </b>{" "}
              of{" "}
              <b className="tabular-nums" style={{ color: "var(--m-text)" }}>
                {rows.length}
              </b>{" "}
              writes
            </span>
            {nextCursor !== null ? (
              <button
                type="button"
                className="m-insight-pager"
                onClick={() => void load({ cursor: nextCursor })}
              >
                Load 50 more
              </button>
            ) : null}
            <span className="flex-1" />
            <span className="flex items-center gap-2">
              <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                Density
              </span>
              <DensitySwitch value={prefs.density} onChange={(d) => void prefs.setDensity(d)} />
            </span>
          </div>
        </div>
      )}
    </div>
  );
}
