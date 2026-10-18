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
  Badge,
  Button,
  ColumnChooser,
  DataToolbar,
  DensitySwitch,
  ExportControl,
  FilterBar,
  Spinner,
  SurfaceHeader,
  Table,
  Td,
  Th,
  Tr,
  type FilterClause,
  type FilterField,
} from "@marula-core/ui";
import { formatInTimeZone } from "@marula-core/ui";
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
import { useTimeFormat } from "../../lib/use-time-format";
import { SurfaceTabs } from "../../components/insight/surface-tabs";
import { InsightState } from "../../components/insight/insight-state";
import "../../components/insight/insight-table.css";
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
  const { timeZone } = useTimeFormat();
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
        occurredLabel: formatInTimeZone(ev.occurred_at, timeZone, "dayTime"),
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
    [events, timeZone],
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
          <Badge tone={KIND_TONE[r.actorKind] ?? "neutral"}>{r.actorKind}</Badge>
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
      <SurfaceHeader
        className="m-rise"
        title="Audit trail"
        meta={
          <span
            className="inline-flex h-6 items-center rounded-full px-2.5 text-[12px] font-semibold"
            style={{ background: "var(--m-surface-2)", color: "var(--m-text-secondary)" }}
          >
            Append-only
          </span>
        }
        description={
          "Every security change, administrative action and domain lifecycle event, in order. Rows expand in place to their structured detail. Nothing here can be edited or deleted."
        }
      />

      {/* A rejected appearance.data_surfaces write reverts the user's
          columns and sort; it says so in blush, never red. */}
      {prefs.error !== null && (
        <div
          role="alert"
          className="flex items-center gap-2 rounded-[var(--m-radius-control)] px-3 py-2 text-[13px] font-medium"
          style={{ background: "var(--m-status-due-bg)", color: "var(--m-status-due-fg)" }}
          data-testid="prefs-save-error"
        >
          {prefs.error}
        </div>
      )}

      <DataToolbar
        label="Audit ledger controls"
        views={
          <SurfaceTabs
            tabs={[
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
              // The clause change re-runs the load effect above, whose
              // callback depends on `clauses`. Firing load here too would
              // race it with a stale closure: in the live app the
              // unfiltered response lands last and the table keeps the
              // pre-filter rows, so the chip looks like it did nothing.
              setClauses(next);
            }}
            label="audit filters"
            emptyHint=""
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
            label="Export"
          />
        }
      />

      <div
        className="overflow-x-auto rounded-[16px] border"
        style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      >
          {errorMsg !== null ? (
            <div role="alert">
              <InsightState
                tone="error"
                title="Couldn't load the audit ledger"
                hint={`${errorMsg} Your filters and columns are saved, so nothing is lost.`}
                primary={
                  <Button variant="primary" size="sm" onClick={() => void loadEvents(cursor)}>
                    Retry
                  </Button>
                }
              />
            </div>
          ) : loading ? (
            <div className="flex justify-center p-6">
              <Spinner />
            </div>
          ) : sorted.length === 0 ? (
            clauses.length > 0 ? (
              <InsightState
                title="No events match these filters"
                hint="Try removing one, or widen the match."
                primary={
                  <Button variant="primary" size="sm" onClick={() => setClauses([])}>
                    Clear filters
                  </Button>
                }
              />
            ) : (
              <InsightState
                title="Nothing in the ledger yet"
                hint="Events land here the moment they happen — security changes, settings, domain lifecycle."
              />
            )
          ) : (
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
                                  <code className="">{r.id}</code>
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
                                  className="overflow-x-auto rounded-md p-3 text-[11px]"
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
          <div className="m-insight-footer flex min-h-[52px] flex-wrap items-center gap-3 px-3 pb-2">
            <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              <b className="tabular-nums" style={{ color: "var(--m-text)" }}>
                {sorted.length}
              </b>{" "}
              of{" "}
              <b className="tabular-nums" style={{ color: "var(--m-text)" }}>
                {rows.length}
              </b>{" "}
              events
            </span>
            {nextCursor !== null ? (
              <button
                type="button"
                className="m-insight-pager"
                onClick={() => void loadEvents(nextCursor)}
              >
                Load 50 more
              </button>
            ) : null}
            <span className="flex-1" />
            <span
              className="hidden items-center gap-1.5 lg:flex"
              style={{ color: "var(--m-text-muted)", fontSize: 12 }}
            >
              <kbd className="m-insight-kbd">↵</kbd> inspect
            </span>
            <span className="flex items-center gap-2">
              {/* The switch carries its own visible label — adding one here
                  double-prints "Density". */}
              <DensitySwitch value={prefs.density} onChange={(d) => void prefs.setDensity(d)} />
            </span>
          </div>
      </div>
    </div>
  );
}

