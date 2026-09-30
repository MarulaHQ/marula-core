// /audit — the audit-trail ledger as a data-surface template (Wave 41 lane 11).
//
// Purpose: keyset-paged, filterable append-only ledger in the TL frame —
// one toolbar row (views + filter chips left, Columns + Export right), a
// 16px-radius bordered table, the 52px footer with density. Rows expand in
// place to the full structured detail. **No bulk actions** — the selection
// bulk bar does not exist here; the pack is explicit that audit and
// provenance take the view switch, column chooser, filter bar and export
// control and nothing else. Validation trouble renders blush
// (`--m-status-due`), never red; red is reserved for breach.
//
// Non-purpose: no editing (append-only), no selection model.
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
  ViewSwitch,
  type FilterClause,
  type FilterField,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { Fragment, useCallback, useEffect, useMemo, useState } from "react";
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
type AuditEvent = components["schemas"]["AuditEvent"];

const SURFACE_KEY = "audit.ledger";

/** The serialized `Actor` union renders as `kind · short-id`, never raw JSON. */
function actorLabel(actor: AuditEvent["actor"] | null | undefined): string | null {
  if (actor === null || actor === undefined) return null;
  if (typeof actor !== "object") return String(actor);
  if (!("id" in actor) || typeof actor.id !== "string") return actor.kind;
  return `${actor.kind} ${actor.id.length > 64 ? actor.id.slice(0, 12) : actor.id}`;
}

function kindOfActor(actor: AuditEvent["actor"]): string {
  if (typeof actor === "object" && actor !== null && "kind" in actor) return String(actor.kind);
  return "other";
}

/** Colour is never alone: the kind tag carries the word, tone is quiet. */
const KIND_TONE: Record<string, "success" | "neutral" | "warning" | "danger"> = {
  human: "neutral",
  agent: "success",
  system: "neutral",
  api_key: "warning",
};

type AuditRow = {
  id: string;
  occurred: string;
  occurredLabel: string;
  action: string;
  actorKind: string;
  actorText: string | null;
  entityText: string;
  ip: string | null;
  detail: unknown;
};

export default function AuditPage() {
  const auth = useRequireAuth();
  const prefs = useDataSurfacePrefs();

  const [events, setEvents] = useState<AuditEvent[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const [view, setView] = useState<"recent" | "agents">("recent");
  const [clauses, setClauses] = useState<FilterClause[]>([]);
  const [expandedId, setExpandedId] = useState<string | null>(null);

  // Cursors
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [cursor, setCursor] = useState<string | null>(null);

  const FILTER_FIELDS: FilterField[] = useMemo(
    () => [
      { key: "action", label: "Action", operators: ["contains"] },
      { key: "actor", label: "Actor", operators: ["contains"] },
      { key: "entity", label: "Entity", operators: ["contains"] },
    ],
    [],
  );

  const rows: AuditRow[] = useMemo(
    () =>
      (events ?? []).map((ev) => ({
        id: ev.id,
        occurred: ev.occurred_at,
        occurredLabel: new Date(ev.occurred_at).toLocaleString("en-GB", {
          day: "numeric",
          month: "short",
          hour: "2-digit",
          minute: "2-digit",
        }),
        action: ev.action,
        actorKind: kindOfActor(ev.actor),
        actorText: actorLabel(ev.actor),
        entityText:
          ev.entity_type !== null && ev.entity_type !== undefined
            ? `${ev.entity_type}${ev.entity_id ? ` · ${ev.entity_id.slice(0, 8)}` : ""}`
            : "—",
        ip: ev.ip ?? null,
        detail: ev.detail,
      })),
    [events],
  );

  const columns: readonly InsightColumn<AuditRow>[] = useMemo(
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
        key: "kind",
        label: "Kind",
        sortValue: (r) => r.actorKind,
        render: (r) => (
          <Tag tone={KIND_TONE[r.actorKind] ?? "neutral"}>{r.actorKind}</Tag>
        ),
      },
      {
        key: "action",
        label: "Event",
        required: true,
        sortValue: (r) => r.action,
        render: (r) => (
          <span className="truncate text-[13px] font-medium" style={{ color: "var(--m-text)" }}>
            {r.action}
          </span>
        ),
      },
      {
        key: "actor",
        label: "Actor",
        sortValue: (r) => r.actorText ?? "",
        render: (r) => (
          <span className="truncate text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            {r.actorText ?? "—"}
          </span>
        ),
      },
      {
        key: "entity",
        label: "Entity",
        render: (r) => (
          <span className="truncate text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            {r.entityText}
          </span>
        ),
      },
    ],
    [],
  );

  const table = useSurfaceTableColumns<AuditRow>(SURFACE_KEY, prefs, columns);

  // Server filter inputs ride the filter clauses: contains on action.
  const loadEvents = useCallback(
    async (targetCursor: string | null) => {
      setLoading(true);
      setErrorMsg(null);
      const actionClause = clauses.find((c) => c.field === "action");
      const actorClause = clauses.find((c) => c.field === "actor");
      const { data, error } = await api.api.GET("/api/v1/audit-events", {
        params: {
          query: {
            action: actionClause?.value === undefined || actionClause.value === "" ? undefined : actionClause.value,
            actor: actorClause?.value === undefined || actorClause.value === "" ? undefined : actorClause.value,
            cursor: targetCursor ?? undefined,
            limit: 50,
          },
        },
      });
      setLoading(false);
      if (error !== undefined || data === undefined) {
        setErrorMsg(rejectionText(error as never, "The audit ledger did not load."));
        return;
      }
      // Pre-guard (lane 10 sweep): a resolving read can carry a malformed
      // payload — render an honest empty ledger, never a crash.
      setEvents(Array.isArray(data.events) ? data.events : []);
      setCursor(targetCursor);
      setNextCursor(data.next_cursor ?? null);
      setExpandedId(null);
    },
    [clauses],
  );

  useEffect(() => {
    if (auth.status === "authed") void loadEvents(null);
  }, [auth.status, loadEvents]);

  // Client-side view + remaining clauses.
  const viewFiltered = useMemo(() => {
    const base = view === "agents" ? rows.filter((r) => r.actorKind === "agent") : rows;
    if (clauses.length === 0) return base;
    return base.filter((r) =>
      clauses.every((c) => {
        if (c.field === "action" || c.field === "actor") return true; // handled server-side
        if (c.field === "entity")
          return c.op === "contains"
            ? r.entityText.toLowerCase().includes((c.value ?? "").toLowerCase())
            : r.entityText.toLowerCase() === (c.value ?? "").toLowerCase();
        return true;
      }),
    );
  }, [rows, view, clauses]);

  const sorted = useMemo(
    () => sortRows(viewFiltered, "audit", columns, table.saved.sort),
    [viewFiltered, columns, table.saved.sort],
  );

  const handleExport = useCallback(() => {
    const csv = toCsv(table.keys, columns, sorted, (col, row) =>
      col.key === "when"
        ? row.occurredLabel
        : col.key === "kind"
          ? row.actorKind
          : col.key === "action"
            ? row.action
            : col.key === "actor"
              ? (row.actorText ?? "")
              : row.entityText,
    );
    downloadCsv("marula-audit.csv", csv);
  }, [table.keys, columns, sorted]);

  if (auth.status !== "authed") return null;

  return (
    <div className="grid gap-4 p-6">
      <header className="m-rise flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-3">
          <h1
            className="text-[26px] font-semibold tracking-[-0.01em]"
            style={{ color: "var(--m-text)" }}
          >
            Audit trail
          </h1>
          <span
            className="inline-flex h-6 items-center rounded-full px-2.5 text-[12px] font-semibold"
            style={{ background: "var(--m-surface-2)", color: "var(--m-text-secondary)" }}
          >
            Append-only
          </span>
        </div>
        <p className="max-w-md text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Every security change, administrative action and domain lifecycle event, in order.
          Rows expand in place to their structured detail. Nothing here can be edited or
          deleted.
        </p>
      </header>

      <DataToolbar
        label="Audit ledger controls"
        views={
          <ViewSwitch
            views={[
              { id: "recent", label: "Recent", hint: String(rows.length) },
              { id: "agents", label: "Agent events", hint: String(rows.filter((r) => r.actorKind === "agent").length) },
            ]}
            activeId={view}
            onChange={(id) => setView(id as "recent" | "agents")}
            label="Audit views"
          />
        }
        filters={
          <FilterBar
            fields={FILTER_FIELDS}
            clauses={clauses}
            onChange={(next) => {
              const actionChanged =
                next.filter((c) => c.field === "action").length !==
                clauses.filter((c) => c.field === "action").length;
              const actorChanged =
                next.filter((c) => c.field === "actor").length !==
                clauses.filter((c) => c.field === "actor").length;
              setClauses(next);
              if (actionChanged || actorChanged) void loadEvents(null);
            }}
            label="audit filters"
            emptyHint="No filters"
          />
        }
        columns={
          <ColumnChooser
            columns={table.chooser}
            onChange={table.onChooser}
            label="Audit columns"
          />
        }
        export={
          <ExportControl
            columns={table.keys}
            rowCount={sorted.length}
            sort={table.saved.sort}
            onExport={() => handleExport()}
            label="Export audit ledger"
          />
        }
      />

      {errorMsg !== null ? (
        <div
          role="alert"
          className="rounded-[16px] px-4 py-3 text-[13px] font-medium"
          style={{ background: "var(--m-status-due-bg)", color: "var(--m-status-due-fg)" }}
        >
          {errorMsg}
        </div>
      ) : (
        <div
          className="overflow-x-auto rounded-[16px] border"
          style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
        >
          {loading ? (
            <div className="flex justify-center p-6">
              <Spinner />
            </div>
          ) : sorted.length === 0 ? (
            <div className="p-6">
              <EmptyStateCopy loading={false} hasFilters={clauses.length > 0} />
            </div>
          ) : (
            <Table>
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
                {sorted.map((r) => {
                  const expanded = expandedId === r.id;
                  return (
                    <Fragment key={r.id}>
                      <Tr
                        data-testid={`audit-row-${r.id}`}
                        className="cursor-pointer"
                        onClick={() => setExpandedId(expanded ? null : r.id)}
                      >
                        {table.keys.map((key) => {
                          const col = columns.find((c) => c.key === key);
                          if (col === undefined) return null;
                          return <Td key={key}>{col.render(r)}</Td>;
                        })}
                      </Tr>
                      {expanded ? (
                        <Tr>
                          <Td className="p-0" />
                          <Td colSpan={(table.keys.length - 1) as never} className="border-b p-3 align-top">
                            <div
                              className="grid gap-2 rounded-md border p-3 text-[12px]"
                              style={{ borderColor: "var(--m-border)" }}
                            >
                              <div className="grid gap-1 md:grid-cols-2">
                                <span>
                                  <span className="font-semibold">Event id:</span>{" "}
                                  <code className="font-mono">{r.id}</code>
                                </span>
                                <span>
                                  <span className="font-semibold">Actor:</span> {r.actorText ?? "—"}
                                </span>
                                <span>
                                  <span className="font-semibold">Entity:</span> {r.entityText}
                                </span>
                                <span>
                                  <span className="font-semibold">IP:</span> {r.ip ?? "—"}
                                </span>
                              </div>
                              <div>
                                <span className="mb-1 block font-semibold">Payload</span>
                                <pre
                                  className="overflow-x-auto rounded-md p-3 font-mono text-[11px]"
                                  style={{
                                    background: "var(--m-surface-2)",
                                    border: "1px solid var(--m-border)",
                                  }}
                                >
                                  {JSON.stringify(r.detail, null, 2)}
                                </pre>
                              </div>
                            </div>
                          </Td>
                        </Tr>
                      ) : null}
                    </Fragment>
                  );
                })}
              </tbody>
            </Table>
          )}
          <div className="flex min-h-[52px] flex-wrap items-center gap-3 px-3 pb-2">
            <span className="flex items-center gap-2 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              <span className="tabular-nums">
                <b style={{ color: "var(--m-text)" }}>{sorted.length}</b> of{" "}
                <b style={{ color: "var(--m-text)" }}>{rows.length}</b> events
              </span>
              {cursor !== null ? (
                <span style={{ color: "var(--m-text-muted)" }}>continued</span>
              ) : null}
              <span className="flex-1" />
              <Button
                variant="ghost"
                size="sm"
                disabled={cursor === null}
                onClick={() => void loadEvents(null)}
              >
                First page
              </Button>
              <Button
                variant="secondary"
                size="sm"
                disabled={nextCursor === null}
                onClick={() => void loadEvents(nextCursor)}
              >
                More events
              </Button>
            </span>
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

function EmptyStateCopy({ loading, hasFilters }: { loading: boolean; hasFilters: boolean }): React.JSX.Element {
  if (loading) return <Spinner />;
  return (
    <div className="grid place-items-center gap-2 py-10 text-center">
      <svg width="48" height="48" viewBox="0 0 48 48" fill="none" aria-hidden="true">
        <rect x="8" y="6" width="32" height="36" rx="4" stroke="var(--m-bark, #7c746a)" strokeWidth="2.5" />
        <path d="M15 16h18M15 23h18M15 30h10" stroke="var(--m-bark, #7c746a)" strokeWidth="2.5" strokeLinecap="round" />
      </svg>
      <p className="text-[15px] font-semibold" style={{ color: "var(--m-text)" }}>
        Nothing in the ledger yet
      </p>
      <p className="max-w-sm text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
        {hasFilters
          ? "No events match these filters. Widen the window or remove a filter."
          : "Events land here the moment they happen — security changes, settings, domain lifecycle."}
      </p>
    </div>
  );
}
