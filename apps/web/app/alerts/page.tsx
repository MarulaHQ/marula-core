// /alerts — alert ingest sources + alert triage viewer (Wave 11 lane 8;
// Wave 17 lane 6 triage pass).
//
// Purpose: browse ingested alerts with severity facets + escalation
// visibility (the incident ticket a storm collapsed into — or the honest
// "unrouted" state), filterable by source and ticket; the sources admin
// (create → secret shown exactly once, rotate posture per docs, enable/
// disable optimistic).
//
// Triage: open → acknowledged → resolved via POST /alerts/{id}/acknowledge
// and /alerts/{id}/resolve (0089) — optimistic flip, server response replaces
// it; re-transition is a typed conflict (shown) and a vanished alert is a
// 404 (the row leaves the list, never faked).
"use client";

import {
  AssistantColumn,
  Badge,
  Button,
  Card,
  ColumnChooser,
  DataToolbar,
  DensitySwitch,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  ExportControl,
  FieldHint,
  FilterBar,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  SurfaceHeader,
  Table,
  Td,
  Th,
  Tr,
  ViewSwitch,
  applyDensity,
  useDensity,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  BlushNote,
  HEAD_CELL_CLASS,
  KeyHint,
  LedgerFooter,
  PageFrame,
  SkeletonRows,
  SparkIcon,
  StatPill,
  StatusPill,
  TableCard,
  downloadCsv,
} from "../../components/knowledge/ledger";
import { api, useRequireAuth } from "../../lib/api";
import { rejectionText, envelopeOf, friendlyMutationText } from "../../lib/error-copy";

type AlertSourceView = components["schemas"]["AlertSourceView"];
type AlertView = components["schemas"]["AlertView"];
type BoardNavEntry = components["schemas"]["BoardNavEntry"];
type AlertSeverity = components["schemas"]["AlertSeverity"];

const SEVERITIES: AlertSeverity[] = ["critical", "warning", "info"];

const SEVERITY_TONE: Record<string, "danger" | "warning" | "neutral"> = {
  critical: "danger",
  warning: "warning",
  info: "neutral",
};

const SEVERITY_FILTERS = ["all", "critical", "warning", "info", "escalated", "unrouted"] as const;
type SeverityFilter = (typeof SEVERITY_FILTERS)[number];

const STATUS_TONE: Record<string, "success" | "warning" | "neutral"> = {
  open: "warning",
  acknowledged: "neutral",
  resolved: "success",
};

type AlertColumnKey =
  | "severity"
  | "alert"
  | "source"
  | "status"
  | "escalation"
  | "received";

interface AlertColumn {
  key: AlertColumnKey;
  label: string;
  visible: boolean;
  required?: boolean;
}

/** The pack's ledger columns; `alert` is the identity and cannot be hidden. */
const ALERT_COLUMNS: AlertColumn[] = [
  { key: "severity", label: "Severity", visible: true },
  { key: "alert", label: "Alert", visible: true, required: true },
  { key: "source", label: "Source", visible: true },
  { key: "status", label: "Status", visible: true },
  { key: "escalation", label: "Escalation", visible: true },
  { key: "received", label: "Received", visible: true },
];

function matchesFilter(alert: AlertView, filter: SeverityFilter): boolean {
  if (filter === "all") return true;
  if (filter === "escalated") return alert.incident_ticket_id !== null && alert.incident_ticket_id !== undefined;
  if (filter === "unrouted") return alert.incident_ticket_id === null || alert.incident_ticket_id === undefined;
  return alert.severity === filter;
}

export default function AlertsPage() {
  const auth = useRequireAuth();

  const [tab, setTab] = useState<"alerts" | "sources">("alerts");

  // Sources state
  const [sources, setSources] = useState<AlertSourceView[] | null>(null);
  const [boards, setBoards] = useState<BoardNavEntry[]>([]);
  const [loadingSources, setLoadingSources] = useState(false);
  const [sourcesError, setSourcesError] = useState<string | null>(null);

  // Create Source modal state
  const [createOpen, setCreateOpen] = useState(false);
  const [sourceName, setSourceName] = useState("");
  const [sourceRef, setSourceRef] = useState("");
  const [selectedBoardId, setSelectedBoardId] = useState("");
  const [windowSecs, setWindowSecs] = useState("900");
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);

  // Secret display (shown once!)
  const [revealedSecret, setRevealedSecret] = useState<{
    name: string;
    secret: string;
    sourceRef: string;
  } | null>(null);

  // Alerts triage state
  const [alerts, setAlerts] = useState<AlertView[] | null>(null);
  const [loadingAlerts, setLoadingAlerts] = useState(false);
  const [alertsError, setAlertsError] = useState<string | null>(null);
  const [severityFilter, setSeverityFilter] = useState<SeverityFilter>("all");
  const [filterStatus, setFilterStatus] = useState<"all" | "open" | "acknowledged" | "resolved">("all");
  // Per-row triage feedback: conflict messages hang on the alert id.
  const [rowErrors, setRowErrors] = useState<Record<string, string>>({});
  const [busyAlertId, setBusyAlertId] = useState<string | null>(null);
  const [sourceById, setSourceById] = useState<Map<string, AlertSourceView>>(new Map());
  const [clauses, setClauses] = useState<{ id: string; field: string; op: never; value?: string }[]>(
    [],
  );
  const [columns, setColumns] = useState<AlertColumn[]>(ALERT_COLUMNS);
  const density = useDensity();

  const refreshBoards = useCallback(async () => {
    const { data } = await api.api.GET("/api/v1/boards/nav");
    if (data !== undefined) {
      setBoards(data);
      if (data.length > 0) setSelectedBoardId((cur) => (cur === "" ? data[0]!.id : cur));
    }
  }, []);

  const refreshSources = useCallback(async () => {
    setLoadingSources(true);
    setSourcesError(null);
    const { data, error } = await api.api.GET("/api/v1/alert-sources");
    setLoadingSources(false);
    if (error !== undefined || data === undefined) {
      setSourcesError("Failed to load alert sources.");
      return;
    }
    setSources(data.sources);
    setSourceById(new Map(data.sources.map((s) => [s.id, s])));
  }, []);

  const refreshAlerts = useCallback(async () => {
    setLoadingAlerts(true);
    setAlertsError(null);
    const { data, error } = await api.api.GET("/api/v1/alerts", {
      params: {
        query: {
          status: filterStatus === "all" ? null : filterStatus,
          limit: 100,
        },
      },
    });
    setLoadingAlerts(false);
    if (error !== undefined || data === undefined) {
      setAlertsError(rejectionText(error as never, "Failed to load alerts."));
      return;
    }
    setAlerts(data.alerts);
  }, [filterStatus]);

  useEffect(() => {
    if (auth.status === "authed") {
      void refreshBoards();
      void refreshSources();
      void refreshAlerts();
    }
  }, [auth.status, refreshBoards, refreshSources, refreshAlerts]);

  const handleCreateSource = async () => {
    if (sourceName.trim() === "" || selectedBoardId === "") return;
    setCreating(true);
    setCreateError(null);
    const win = Number.parseInt(windowSecs, 10) || 900;
    const { data, error } = await api.api.POST("/api/v1/alert-sources", {
      body: {
        name: sourceName.trim(),
        source_ref: sourceRef.trim() === "" ? undefined : sourceRef.trim(),
        board_id: selectedBoardId,
        correlation_window_secs: win,
        idempotency_key:
          typeof crypto !== "undefined" && crypto.randomUUID !== undefined
            ? crypto.randomUUID()
            : `alsrc-${Date.now()}`,
      },
    });
    setCreating(false);
    if (error !== undefined || data === undefined) {
      setCreateError(rejectionText(error as never, "Failed to create the alert source. Verify the inputs."));
      return;
    }
    if (data.secret !== undefined && data.secret !== null) {
      setRevealedSecret({
        name: data.name,
        secret: data.secret,
        sourceRef: data.source_ref,
      });
    }
    setCreateOpen(false);
    setSourceName("");
    setSourceRef("");
    setWindowSecs("900");
    void refreshSources();
  };

  const handleToggleSource = async (source: AlertSourceView) => {
    setSources((cur) =>
      (cur ?? []).map((s) => (s.id === source.id ? { ...s, enabled: !s.enabled } : s)),
    );
    const { error } = await api.api.PATCH("/api/v1/alert-sources/{id}", {
      params: { path: { id: source.id } },
      body: {
        enabled: !source.enabled,
        idempotency_key:
          typeof crypto !== "undefined" && crypto.randomUUID !== undefined
            ? crypto.randomUUID()
            : `alsrc-patch-${Date.now()}`,
      },
    });
    if (error !== undefined) {
      setSources((cur) =>
        (cur ?? []).map((s) => (s.id === source.id ? { ...s, enabled: source.enabled } : s)),
      );
      setSourcesError(rejectionText(error as never, "The enable toggle was rejected."));
      return;
    }
    void refreshSources();
  };

  /** Shared triage transition (acknowledge | resolve), IF envelope:
   * optimistic status flip → server alert replaces it; 409 surfaces the
   * conflict honestly; 404 removes the row that left scope. */
  const triageAlert = useCallback(
    async (
      alert: AlertView,
      path: "/api/v1/alerts/{id}/acknowledge" | "/api/v1/alerts/{id}/resolve",
      nextStatus: string,
    ) => {
      const id = alert.id;
      setBusyAlertId(id);
      const prev = alerts;
      setAlerts(
        (cur) => (cur ?? []).map((a) => (a.id === id ? { ...a, status: nextStatus } : a)),
      );
      const { data, error } = await api.api.POST(path, {
        params: { path: { id } },
        body: {
          idempotency_key:
            typeof crypto !== "undefined" && crypto.randomUUID !== undefined
              ? crypto.randomUUID()
              : `triage-${Date.now()}`,
        },
      });
      setBusyAlertId(null);
      if (error !== undefined || data === undefined) {
        // Gone from scope: the row leaves the list — never rendered as fake.
        const status = (error as { status?: number } | null | undefined)?.status;
        const env = envelopeOf(error);
        if (status === 404 || env.reason === "not_found" || env.reason === "rls_sparse") {
          setAlerts((cur) => (prev ?? []).filter((a) => a.id !== id));
          setRowErrors((cur) => ({ ...cur, [id]: "That alert is no longer in your visible workspace — it left the list." }));
          window.setTimeout(() => setRowErrors((cur) => omitKey(cur, id)), 6000);
          return;
        }
        // Optimistic state was already synced above; revert and render the
        // typed conflict (re-transition) or other rejection honestly.
        setAlerts(prev);
        setRowErrors((cur) => ({
          ...cur,
          [id]: friendlyMutationText(error, "The triage change was rejected."),
        }));
        window.setTimeout(() => setRowErrors((cur) => omitKey(cur, id)), 8000);
        return;
      }
      setAlerts((cur) => (cur ?? []).map((a) => (a.id === id ? data.alert : a)));
    },
    [alerts],
  );

  const handleAcknowledge = (a: AlertView) =>
    void triageAlert(a, "/api/v1/alerts/{id}/acknowledge", "acknowledged");

  const handleResolve = (a: AlertView) =>
    void triageAlert(a, "/api/v1/alerts/{id}/resolve", "resolved");

  const shownColumns = columns.filter((c) => c.visible);

  const filteredAlerts = useMemo(() => {
    // The source, ticket and status filters ride the server query; the
    // builder's clauses are this list's own narrowing, so both live here
    // rather than each hiding half of a filter behind a round trip.
    let rows = (alerts ?? []).filter((a) => matchesFilter(a, severityFilter));
    for (const clause of clauses) {
      const value = (clause.value ?? "").toLowerCase();
      if (clause.field === "title") rows = rows.filter((a) => a.title.toLowerCase().includes(value));
      if (clause.field === "severity") rows = rows.filter((a) => a.severity === clause.value);
      if (clause.field === "status") rows = rows.filter((a) => a.status === clause.value);
      if (clause.field === "source") {
        rows = rows.filter((a) =>
          (sourceById.get(a.source_id)?.name ?? a.source_id).toLowerCase().includes(value),
        );
      }
      if (clause.field === "ticket") {
        rows = rows.filter((a) =>
          (a.incident_ticket_id ?? "").toLowerCase().includes(value),
        );
      }
    }
    return rows;
  }, [alerts, severityFilter, clauses, sourceById]);

  const counts = useMemo(() => {
    const all = { all: 0, escalated: 0, unrouted: 0 };
    const bySeverity: Record<string, number> = { critical: 0, warning: 0, info: 0 };
    for (const a of alerts ?? []) {
      all.all += 1;
      bySeverity[a.severity] = (bySeverity[a.severity] ?? 0) + 1;
      if (a.incident_ticket_id !== null && a.incident_ticket_id !== undefined) all.escalated += 1;
      else all.unrouted += 1;
    }
    return { ...all, ...bySeverity };
  }, [alerts]);

  if (auth.status !== "authed") return null;

  return (
    <PageFrame>
      <SurfaceHeader
        title="Alerts"
        description="Signed ingest, correlated into tickets"
        meta={
          <>
            <StatPill
              tone="red"
              label="Critical"
              count={bySeverityKey(counts, "critical")}
              onClick={() => setSeverityFilter(severityFilter === "critical" ? "all" : "critical")}
              pressed={severityFilter === "critical"}
            />
            <StatPill
              tone="blush"
              label="Unrouted"
              count={counts.unrouted}
              onClick={() => setSeverityFilter(severityFilter === "unrouted" ? "all" : "unrouted")}
              pressed={severityFilter === "unrouted"}
            />
          </>
        }
        actions={
          <>
            <Button
              variant={tab === "alerts" ? "primary" : "secondary"}
              onClick={() => setTab("alerts")}
            >
              Alert feed
            </Button>
            <Button
              variant={tab === "sources" ? "primary" : "secondary"}
              onClick={() => setTab("sources")}
            >
              Sources
            </Button>
          </>
        }
      />

      {/* Secret shown exactly once */}
      {revealedSecret !== null ? (
        <Card>
          <div className="grid gap-2 p-4" style={{ borderLeft: "4px solid var(--m-accent)" }}>
            <Label className="text-sm font-semibold">
              HMAC signing secret for “{revealedSecret.name}”
            </Label>
            <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              Ingest endpoint: <code className="rounded bg-black/10 px-1 py-0.5 dark:bg-white/10">/api/v1/alerts/ingest/{revealedSecret.sourceRef}</code>
            </p>
            <code
              className="select-all rounded-md p-2 text-[12px] break-all"
              style={{
                background: "var(--m-surface)",
                color: "var(--m-text)",
                border: "1px solid var(--m-border)",
              }}
            >
              {revealedSecret.secret}
            </code>
            <FieldHint>
              Copy this secret now. It is encrypted in storage and will NEVER be
              shown again. Sign HTTP payloads with hex HMAC-SHA256 in the{" "}
              <code className="text-[11px]">X-Marula-Signature</code> header.
            </FieldHint>
            <div className="mt-1">
              <Button variant="ghost" size="sm" onClick={() => setRevealedSecret(null)}>
                Dismiss
              </Button>
            </div>
          </div>
        </Card>
      ) : null}

      {tab === "alerts" ? (
        <div className="flex min-h-0 flex-1 flex-col gap-3">
          <DataToolbar
            label="Alert feed controls"
            views={
              <ViewSwitch
                label="Alert views"
                views={[
                  { id: "all", label: "All alerts", hint: String(filteredAlerts.length) },
                  { id: "open", label: "Open" },
                  { id: "acknowledged", label: "Acknowledged" },
                  { id: "resolved", label: "Resolved" },
                ]}
                activeId={filterStatus}
                onChange={(id) => setFilterStatus(id as typeof filterStatus)}
              />
            }
            filters={
              <FilterBar
                label="Alert filters"
                emptyHint="No filters"
                fields={[
                  { key: "title", label: "Alert", operators: ["contains"] },
                  { key: "severity", label: "Severity", operators: ["eq"] },
                  { key: "source", label: "Source", operators: ["contains"] },
                  { key: "status", label: "Status", operators: ["eq"] },
                  { key: "ticket", label: "Incident ticket id", operators: ["contains"] },
                ]}
                clauses={clauses}
                onChange={(next) => setClauses(next.map((c) => ({ ...c, op: c.op as never })))}
              />
            }
            columns={
              <ColumnChooser
                label="Alert columns"
                columns={columns.map((c) => ({ key: c.key, label: c.label, visible: c.visible, required: c.required }))}
                onChange={(next) =>
                  setColumns(
                    next.map((c) => ({
                      key: c.key as AlertColumnKey,
                      label: c.label,
                      visible: c.visible,
                      required: c.required,
                    })),
                  )
                }
              />
            }
            export={
              <ExportControl
                label="Export"
                formats={["csv"]}
                columns={shownColumns.map((c) => c.label)}
                rowCount={filteredAlerts.length}
                filters={clauses}
                onExport={() =>
                  downloadCsv(
                    "alerts.csv",
                    shownColumns.map((c) => c.label),
                    filteredAlerts.map((a) =>
                      shownColumns.map((c) => {
                        if (c.key === "severity") return a.severity;
                        if (c.key === "alert") return a.title;
                        if (c.key === "source") return sourceById.get(a.source_id)?.name ?? a.source_id.slice(0, 8);
                        if (c.key === "status") return a.status;
                        if (c.key === "escalation")
                          return a.incident_ticket_id == null ? "unrouted" : a.incident_ticket_id;
                        return a.received_at;
                      }),
                    ),
                  )
                }
              />
            }
          />
          <div className="flex min-h-0 flex-1 flex-col gap-0">
          {alertsError !== null ? (
            <div className="mb-2" role="alert">
              <BlushNote title="The alert feed did not load.">{alertsError}</BlushNote>
            </div>
          ) : null}

          <div className="flex min-h-0 flex-1 gap-3">
            <TableCard label="Alert feed">
            {loadingAlerts ? (
              <SkeletonRows sentence="Loading the alert feed." columns={4} />
            ) : filteredAlerts.length === 0 ? (
              <div className="p-6">
                <EmptyState
                  message={
                    alerts !== null && alerts.length > 0
                      ? "No alerts match this facet — widen the severity filter."
                      : "No alerts received yet. Inbound webhook alerts from your configured sources appear here."
                  }
                />
              </div>
            ) : (
              <Table data-testid="alerts-table">
                <thead>
                  <tr>
                    {shownColumns.map((c) => (
                      <Th key={c.key} className={HEAD_CELL_CLASS}>
                        {c.label}
                      </Th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {filteredAlerts.map((a) => (
                    <Tr key={a.id} style={{ height: "var(--m-density-row)" }}>
                      {shownColumns.map((c) => (
                        <Td key={c.key}>
                          {c.key === "severity" ? (
                            <StatusPill tone={SEVERITY_TONE[a.severity] ?? "neutral"}>{a.severity}</StatusPill>
                          ) : null}
                          {c.key === "alert" ? (
                            <div className="min-w-0">
                              <div className="truncate text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
                                {a.title}
                              </div>
                              <div className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                                dedup {a.dedup_key} · corr {a.correlation_key}
                              </div>
                            </div>
                          ) : null}
                          {c.key === "source" ? (
                            <span className="text-[12px]">
                              {sourceById.get(a.source_id)?.name ?? a.source_id.slice(0, 8)}
                            </span>
                          ) : null}
                          {c.key === "status" ? (
                            <div className="flex items-center gap-2">
                              <StatusPill tone={STATUS_TONE[a.status] ?? "neutral"}>{a.status}</StatusPill>
                              {rowErrors[a.id] !== undefined ? (
                                <span className="text-[11px]" style={{ color: "var(--m-status-danger-fg)" }}>
                                  {rowErrors[a.id]}
                                </span>
                              ) : null}
                              {a.status === "open" ? (
                                <Button
                                  data-testid={`alert-ack-${a.id}`}
                                  variant="ghost"
                                  size="sm"
                                  disabled={busyAlertId === a.id}
                                  onClick={() => handleAcknowledge(a)}
                                >
                                  Acknowledge
                                </Button>
                              ) : null}
                              {a.status === "open" || a.status === "acknowledged" ? (
                                <Button
                                  data-testid={`alert-resolve-${a.id}`}
                                  variant="ghost"
                                  size="sm"
                                  disabled={busyAlertId === a.id}
                                  onClick={() => handleResolve(a)}
                                >
                                  Resolve
                                </Button>
                              ) : null}
                              {a.acked_by != null && a.acked_at != null ? (
                                <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                                  acked {relativeTime(a.acked_at)}
                                </span>
                              ) : null}
                            </div>
                          ) : null}
                          {c.key === "escalation" ? (
                            a.incident_ticket_id !== null && a.incident_ticket_id !== undefined ? (
                              <div className="flex items-center gap-2">
                                <Badge tone="success">escalated</Badge>
                                <Link
                                  href={`/tickets/${a.incident_ticket_id}`}
                                  className="text-[13px] font-medium underline decoration-[var(--m-border)] underline-offset-2"
                                >
                                  Ticket #{a.incident_ticket_id.slice(0, 8)}
                                </Link>
                              </div>
                            ) : (
                              <Badge tone="neutral">unrouted</Badge>
                            )
                          ) : null}
                          {c.key === "received" ? (
                            <span className="whitespace-nowrap text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                              {relativeTime(a.received_at)}
                            </span>
                          ) : null}
                        </Td>
                      ))}
                    </Tr>
                  ))}
                </tbody>
              </Table>
            )}
            </TableCard>
            {/* AI ladder state 2: the resting stub, docked to the feed's RIGHT
                edge — the pack puts the assistant column on the right, and the
                stub is that column collapsed. The count is the unrouted alerts,
                and null when there is nothing to count. */}
            <AssistantColumn
              count={counts.unrouted === 0 ? null : counts.unrouted}
              spark={<SparkIcon size={16} />}
              label={`${counts.unrouted} alerts have not been routed to a ticket`}
            />
          </div>

          <LedgerFooter
            shown={filteredAlerts.length}
            total={counts.all}
            noun="alerts"
            density={<DensitySwitch value={density} onChange={applyDensity} />}
            hints={
              <>
                <KeyHint keys="J K" label="move" />
                <KeyHint keys="A" label="acknowledge" />
                <KeyHint keys="F" label="filter" />
              </>
            }
          />
          </div>
        </div>
      ) : null}

      {tab === "sources" ? (
        <div className="grid gap-4">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-semibold">Configured sources</h2>
            <Dialog open={createOpen} onOpenChange={setCreateOpen}>
              <DialogTrigger asChild>
                <Button
                  onClick={() => {
                    setSourceName("");
                    setSourceRef("");
                    setWindowSecs("900");
                  }}
                >
                  New source
                </Button>
              </DialogTrigger>
              <DialogContent title="New alert ingest source">
                <div className="grid gap-3 p-4">
                  <div>
                    <Label htmlFor="as-name">Source name</Label>
                    <Input
                      id="as-name"
                      placeholder="e.g. Datadog / Grafana / Zabbix"
                      value={sourceName}
                      onChange={(e) => setSourceName(e.target.value)}
                      required
                    />
                  </div>

                  <div>
                    <Label htmlFor="as-ref">Routing reference</Label>
                    <Input
                      id="as-ref"
                      placeholder="3–64 chars [a-z0-9-] · auto-generated if blank"
                      value={sourceRef}
                      onChange={(e) => setSourceRef(e.target.value)}
                    />
                    <FieldHint>Path segment of the webhook ingest URL.</FieldHint>
                  </div>

                  <div>
                    <Label htmlFor="as-board">Target incident board</Label>
                    <Select value={selectedBoardId} onValueChange={setSelectedBoardId}>
                      <SelectTrigger id="as-board" className="w-full">
                        <SelectValue placeholder="Pick the incident board" />
                      </SelectTrigger>
                      <SelectContent>
                        {boards.map((b) => (
                          <SelectItem key={b.id} value={b.id}>
                            {b.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <FieldHint>Alert storms open tickets here.</FieldHint>
                  </div>

                  <div>
                    <Label htmlFor="as-window">Correlation window (seconds)</Label>
                    <Input
                      id="as-window"
                      type="number"
                      min={10}
                      max={86400}
                      value={windowSecs}
                      onChange={(e) => setWindowSecs(e.target.value)}
                    />
                    <FieldHint>Storm dedup window (10–86,400s; default 900s = 15 min).</FieldHint>
                  </div>

                  {createError !== null ? (
                    <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                      {createError}
                    </p>
                  ) : null}

                  <DialogActions>
                    <Button variant="ghost" onClick={() => setCreateOpen(false)}>
                      Cancel
                    </Button>
                    <Button
                      onClick={() => void handleCreateSource()}
                      disabled={creating || sourceName.trim() === "" || selectedBoardId === ""}
                    >
                      {creating ? <Spinner /> : null} Create source
                    </Button>
                  </DialogActions>
                </div>
              </DialogContent>
            </Dialog>
          </div>

          {sourcesError !== null ? (
            <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {sourcesError}
            </p>
          ) : null}

          <Card className="overflow-x-auto">
            {loadingSources ? (
              <div className="flex justify-center p-6">
                <Spinner />
              </div>
            ) : sources === null || sources.length === 0 ? (
              <div className="p-6">
                <EmptyState message="No ingest sources yet. Create one to receive webhook alerts." />
              </div>
            ) : (
              <Table>
                <thead>
                  <tr>
                    <Th>Name</Th>
                    <Th>Ingest URL</Th>
                    <Th>Storm window</Th>
                    <Th>Status</Th>
                    <Th className="text-right">Action</Th>
                  </tr>
                </thead>
                <tbody>
                  {(sources ?? []).map((s) => (
                    <Tr key={s.id}>
                      <Td className="text-[13px] font-medium">{s.name}</Td>
                      <Td>
                        <code
                          className="cursor-pointer rounded px-1 py-0.5 text-[12px]"
                          style={{ background: "color-mix(in srgb, var(--m-accent) 8%, transparent)" }}
                          title="Click to copy the ingest path"
                          onClick={() => void navigator.clipboard?.writeText(`/api/v1/alerts/ingest/${s.source_ref}`)}
                        >
                          /api/v1/alerts/ingest/{s.source_ref}
                        </code>
                      </Td>
                      <Td className="text-[12px] tabular-nums">{s.correlation_window_secs}s</Td>
                      <Td>
                        <Badge tone={s.enabled ? "success" : "neutral"}>
                          {s.enabled ? "enabled" : "disabled"}
                        </Badge>
                      </Td>
                      <Td className="text-right">
                        <Button variant="ghost" size="sm" onClick={() => void handleToggleSource(s)}>
                          {s.enabled ? "Disable" : "Enable"}
                        </Button>
                      </Td>
                    </Tr>
                  ))}
                </tbody>
              </Table>
            )}
          </Card>
        </div>
      ) : null}
    </PageFrame>
  );
}

function relativeTime(iso: string): string {
  const at = new Date(iso).getTime();
  const deltaMs = Date.now() - at;
  if (Number.isNaN(at)) return iso;
  const mins = Math.round(deltaMs / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.round(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  return new Date(iso).toLocaleDateString();
}

function omitKey(
  record: Record<string, string>,
  key: string,
): Record<string, string> {
  const next = { ...record };
  delete next[key];
  return next;
}

/** Severity counts land in the merged record; read them defensively. */
function bySeverityKey(
  counts: { all: number; escalated: number; unrouted: number } & Partial<Record<AlertSeverity, number>>,
  severity: AlertSeverity,
): number {
  return counts[severity] ?? 0;
}
