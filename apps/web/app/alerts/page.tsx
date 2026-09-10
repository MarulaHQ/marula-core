// /alerts — Alert sources admin & incident / alert viewer (Wave 11 lane 8).
//
// Purpose:
//  - Source admin: list sources, create source (generating secret returned once),
//    rotate secret (create replacement / update secret shown once), enable/disable toggle.
//  - Alerts & incidents: browse ingested alerts, filter by source / incident ticket,
//    severity badges, status tags, and drill-in link to ticket (/tickets/[id]).
//
// Instant-first: mutation envelope for idempotency, inline errors, typed empty states,
// Workbench tokens, light+dark, <=120ms transitions.
"use client";

import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  Table,
  Tag,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";

type AlertSourceView = components["schemas"]["AlertSourceView"];
type AlertView = components["schemas"]["AlertView"];
type BoardNavEntry = components["schemas"]["BoardNavEntry"];
type AlertSeverity = components["schemas"]["AlertSeverity"];

function severityTone(severity: AlertSeverity): "danger" | "warning" | "neutral" {
  switch (severity) {
    case "critical":
      return "danger";
    case "warning":
      return "warning";
    case "info":
      return "neutral";
    default:
      return "neutral";
  }
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

  // Alerts browse state
  const [alerts, setAlerts] = useState<AlertView[] | null>(null);
  const [loadingAlerts, setLoadingAlerts] = useState(false);
  const [alertsError, setAlertsError] = useState<string | null>(null);
  const [filterSourceId, setFilterSourceId] = useState<string>("");
  const [filterTicketId, setFilterTicketId] = useState<string>("");

  const refreshBoards = useCallback(async () => {
    const { data } = await api.api.GET("/api/v1/boards/nav");
    if (data) {
      setBoards(data);
      if (!selectedBoardId && data.length > 0) {
        setSelectedBoardId(data[0]!.id);
      }
    }
  }, [selectedBoardId]);

  const refreshSources = useCallback(async () => {
    setLoadingSources(true);
    setSourcesError(null);
    const { data, error } = await api.api.GET("/api/v1/alert-sources");
    setLoadingSources(false);
    if (error !== undefined || !data) {
      setSourcesError("Failed to load alert sources.");
      return;
    }
    setSources(data.sources);
  }, []);

  const refreshAlerts = useCallback(async () => {
    setLoadingAlerts(true);
    setAlertsError(null);
    const { data, error } = await api.api.GET("/api/v1/alerts", {
      params: {
        query: {
          source_id: filterSourceId ? filterSourceId : undefined,
          ticket_id: filterTicketId ? filterTicketId : undefined,
          limit: 100,
        },
      },
    });
    setLoadingAlerts(false);
    if (error !== undefined || !data) {
      setAlertsError("Failed to load alerts.");
      return;
    }
    setAlerts(data.alerts);
  }, [filterSourceId, filterTicketId]);

  useEffect(() => {
    if (auth.status === "authed") {
      void refreshBoards();
      void refreshSources();
      void refreshAlerts();
    }
  }, [auth.status, refreshBoards, refreshSources, refreshAlerts]);

  const handleCreateSource = async () => {
    if (!sourceName.trim() || !selectedBoardId) return;
    setCreating(true);
    setCreateError(null);
    const win = parseInt(windowSecs, 10) || 900;
    const idempotencyKey =
      typeof crypto !== "undefined" && crypto.randomUUID
        ? crypto.randomUUID()
        : `alsrc-${Date.now()}`;

    const { data, error } = await api.api.POST("/api/v1/alert-sources", {
      body: {
        name: sourceName.trim(),
        source_ref: sourceRef.trim() ? sourceRef.trim() : undefined,
        board_id: selectedBoardId,
        correlation_window_secs: win,
        idempotency_key: idempotencyKey,
      },
    });
    setCreating(false);
    if (error !== undefined || !data) {
      setCreateError("Failed to create alert source. Please verify inputs.");
      return;
    }

    if (data.secret) {
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
    const idempotencyKey =
      typeof crypto !== "undefined" && crypto.randomUUID
        ? crypto.randomUUID()
        : `alsrc-patch-${Date.now()}`;

    await api.api.PATCH("/api/v1/alert-sources/{id}", {
      params: { path: { id: source.id } },
      body: {
        enabled: !source.enabled,
        idempotency_key: idempotencyKey,
      },
    });
    void refreshSources();
  };

  if (auth.status !== "authed") return null;

  return (
    <div className="grid gap-6 max-w-5xl mx-auto p-6">
      <header className="flex flex-wrap items-center justify-between gap-4">
        <div>
          <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
            Alerts & Ingest Sources
          </h1>
          <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            HMAC-signed machine-to-machine alert ingest, storm correlation into incident tickets, and source administration.
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            variant={tab === "alerts" ? "primary" : "secondary"}
            onClick={() => setTab("alerts")}
          >
            Alerts Feed
          </Button>
          <Button
            variant={tab === "sources" ? "primary" : "secondary"}
            onClick={() => setTab("sources")}
          >
            Alert Sources
          </Button>
        </div>
      </header>

      {/* Secret Shown Once Banner */}
      {revealedSecret !== null && (
        <Card>
          <div className="p-4 grid gap-2" style={{ borderLeft: "4px solid var(--m-accent)" }}>
            <Label className="font-semibold text-sm">
              HMAC Signing Secret for “{revealedSecret.name}”
            </Label>
            <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              Ingest endpoint: <code className="px-1 py-0.5 rounded bg-black/10 dark:bg-white/10">/api/v1/alerts/ingest/{revealedSecret.sourceRef}</code>
            </p>
            <code
              className="text-[12px] font-mono break-all rounded-[2px] p-2 select-all"
              style={{
                background: "var(--m-surface)",
                color: "var(--m-text)",
                border: "1px solid var(--m-border)",
              }}
            >
              {revealedSecret.secret}
            </code>
            <FieldHint>
              Copy this secret now. It is encrypted in storage and will NEVER be shown again. Sign HTTP payloads using hex HMAC-SHA256 in the <code className="text-[11px]">X-Meridian-Signature</code> header.
            </FieldHint>
            <div className="mt-1">
              <Button variant="ghost" size="sm" onClick={() => setRevealedSecret(null)}>
                Dismiss
              </Button>
            </div>
          </div>
        </Card>
      )}

      {tab === "alerts" && (
        <div className="grid gap-4">
          <Card className="p-4">
            <div className="flex flex-wrap items-end gap-3">
              <div className="w-56">
                <Label htmlFor="filter-source">Filter by Source</Label>
                <Select value={filterSourceId} onValueChange={(v) => setFilterSourceId(v)}>
                  <SelectTrigger id="filter-source" className="w-full">
                    <SelectValue placeholder="All sources" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="">All sources</SelectItem>
                    {sources?.map((s) => (
                      <SelectItem key={s.id} value={s.id}>
                        {s.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="w-64">
                <Label htmlFor="filter-ticket">Ticket ID</Label>
                <Input
                  id="filter-ticket"
                  placeholder="UUID or leave blank"
                  value={filterTicketId}
                  onChange={(e) => setFilterTicketId(e.target.value)}
                />
              </div>

              <Button variant="secondary" onClick={() => void refreshAlerts()}>
                Filter
              </Button>
            </div>
          </Card>

          {alertsError && (
            <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {alertsError}
            </p>
          )}

          <Card className="overflow-x-auto">
            {loadingAlerts ? (
              <div className="p-6 flex justify-center">
                <Spinner />
              </div>
            ) : alerts === null || alerts.length === 0 ? (
              <div className="p-6">
                <EmptyState message="No alerts received yet. Incoming webhook alerts from your configured sources will appear here." />
              </div>
            ) : (
              <Table>
                <thead>
                  <tr>
                    <Th>Severity</Th>
                    <Th>Title / Dedup Key</Th>
                    <Th>Correlation Key</Th>
                    <Th>Incident Ticket</Th>
                    <Th>Timestamp</Th>
                  </tr>
                </thead>
                <tbody>
                  {alerts.map((a) => (
                    <Tr key={a.id}>
                      <Td>
                        <Tag tone={severityTone(a.severity as AlertSeverity)}>{a.severity}</Tag>
                      </Td>
                      <Td>
                        <div className="font-medium text-[13px]">{a.title}</div>
                        <div className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                          dedup: {a.dedup_key}
                        </div>
                      </Td>
                      <Td>
                        <code className="text-[12px] font-mono">{a.correlation_key}</code>
                      </Td>
                      <Td>
                        {a.incident_ticket_id ? (
                          <Link
                            href={`/tickets/${a.incident_ticket_id}`}
                            className="text-[13px] font-medium underline decoration-[var(--m-border)] underline-offset-2 hover:text-[var(--m-accent)]"
                          >
                            Ticket #{a.incident_ticket_id.slice(0, 8)}
                          </Link>
                        ) : (
                          <span style={{ color: "var(--m-text-muted)" }}>—</span>
                        )}
                      </Td>
                      <Td className="text-[12px] whitespace-nowrap" style={{ color: "var(--m-text-secondary)" }}>
                        {new Date(a.received_at).toLocaleString()}
                      </Td>
                    </Tr>
                  ))}
                </tbody>
              </Table>
            )}
          </Card>
        </div>
      )}

      {tab === "sources" && (
        <div className="grid gap-4">
          <div className="flex justify-between items-center">
            <h2 className="text-sm font-semibold">Configured Sources</h2>
            <Dialog open={createOpen} onOpenChange={setCreateOpen}>
              <DialogTrigger asChild>
                <Button>Create alert source</Button>
              </DialogTrigger>
              <DialogContent title="Create Alert Ingest Source">
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
                    <Label htmlFor="as-ref">Routing reference (slug)</Label>
                    <Input
                      id="as-ref"
                      placeholder="Optional: 3-64 chars [a-z0-9-]"
                      value={sourceRef}
                      onChange={(e) => setSourceRef(e.target.value)}
                    />
                    <FieldHint>Used in the webhook ingest URL path. Auto-generated if blank.</FieldHint>
                  </div>

                  <div>
                    <Label htmlFor="as-board">Target Incident Board</Label>
                    <Select value={selectedBoardId} onValueChange={setSelectedBoardId}>
                      <SelectTrigger id="as-board" className="w-full">
                        <SelectValue placeholder="Pick target board" />
                      </SelectTrigger>
                      <SelectContent>
                        {boards.map((b) => (
                          <SelectItem key={b.id} value={b.id}>
                            {b.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>

                  <div>
                    <Label htmlFor="as-window">Correlation Window (seconds)</Label>
                    <Input
                      id="as-window"
                      type="number"
                      min={10}
                      max={86400}
                      value={windowSecs}
                      onChange={(e) => setWindowSecs(e.target.value)}
                    />
                    <FieldHint>Alert storm deduplication window (10 - 86,400s; default 900s = 15m).</FieldHint>
                  </div>

                  {createError && (
                    <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                      {createError}
                    </p>
                  )}

                  <DialogActions>
                    <Button variant="ghost" onClick={() => setCreateOpen(false)}>
                      Cancel
                    </Button>
                    <Button onClick={() => void handleCreateSource()} disabled={creating || !sourceName.trim()}>
                      {creating ? <Spinner /> : null} Save Source
                    </Button>
                  </DialogActions>
                </div>
              </DialogContent>
            </Dialog>
          </div>

          {sourcesError && (
            <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {sourcesError}
            </p>
          )}

          <Card className="overflow-x-auto">
            {loadingSources ? (
              <div className="p-6 flex justify-center">
                <Spinner />
              </div>
            ) : sources === null || sources.length === 0 ? (
              <div className="p-6">
                <EmptyState message="No alert sources configured yet. Create a source to receive automated alerts." />
              </div>
            ) : (
              <Table>
                <thead>
                  <tr>
                    <Th>Name</Th>
                    <Th>Ingest Ref</Th>
                    <Th>Storm Window</Th>
                    <Th>Status</Th>
                    <Th className="text-right">Action</Th>
                  </tr>
                </thead>
                <tbody>
                  {sources.map((s) => (
                    <Tr key={s.id}>
                      <Td className="font-medium text-[13px]">{s.name}</Td>
                      <Td>
                        <code className="text-[12px] font-mono px-1 py-0.5 rounded bg-black/5 dark:bg-white/5">
                          {s.source_ref}
                        </code>
                      </Td>
                      <Td className="text-[12px]">{s.correlation_window_secs}s</Td>
                      <Td>
                        <Tag tone={s.enabled ? "success" : "neutral"}>
                          {s.enabled ? "Enabled" : "Disabled"}
                        </Tag>
                      </Td>
                      <Td className="text-right">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => void handleToggleSource(s)}
                        >
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
      )}
    </div>
  );
}
