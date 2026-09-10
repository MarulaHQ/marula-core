// /reports — Report definitions, runner, and scheduled delivery (Wave 11 lane 8).
//
// Purpose:
//  - R3 report definitions UI: stored-spec CRUD, run report on demand, share (org/self scope).
//  - R4 schedules UI: schedule CRUD (cadence: daily/weekly/monthly, time_of_day, recipients)
//    and preview next run / delivery history.
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
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";
import { ReportDeliveryHistory } from "../../components/report-delivery-history";

type ReportDefinitionView = components["schemas"]["ReportDefinitionView"];
type ReportScheduleView = components["schemas"]["ReportScheduleView"];
type DefinitionScope = components["schemas"]["DefinitionScope"];
type ScheduleCadence = components["schemas"]["ScheduleCadence"];
type ViewId = components["schemas"]["ViewId"];

const REPORT_KINDS: { kind: ViewId; label: string }[] = [
  { kind: "tickets", label: "Tickets" },
  { kind: "time_entries", label: "Time Entries" },
  { kind: "sla_outcomes", label: "SLA Outcomes" },
  { kind: "gap_days", label: "Gap Days" },
  { kind: "first_response", label: "First Response Time" },
];

export default function ReportsPage() {
  const auth = useRequireAuth();

  const [tab, setTab] = useState<"definitions" | "schedules" | "run">("definitions");

  // Definitions state
  const [definitions, setDefinitions] = useState<ReportDefinitionView[] | null>(null);
  const [loadingDefs, setLoadingDefs] = useState(false);
  const [defsError, setDefsError] = useState<string | null>(null);

  // Create/Edit definition modal
  const [defModalOpen, setDefModalOpen] = useState(false);
  const [editingDefId, setEditingDefId] = useState<string | null>(null);
  const [defName, setDefName] = useState("");
  const [defDescription, setDefDescription] = useState("");
  const [defKind, setDefKind] = useState<ViewId>("tickets");
  const [defScope, setDefScope] = useState<DefinitionScope>("org");
  const [savingDef, setSavingDef] = useState(false);
  const [defFormError, setDefFormError] = useState<string | null>(null);

  // Schedules state
  const [schedules, setSchedules] = useState<ReportScheduleView[] | null>(null);
  const [loadingSchedules, setLoadingSchedules] = useState(false);
  const [schedulesError, setSchedulesError] = useState<string | null>(null);

  // Create schedule modal
  const [scheduleModalOpen, setScheduleModalOpen] = useState(false);
  const [schedName, setSchedName] = useState("");
  const [schedDefId, setSchedDefId] = useState("");
  const [schedCadence, setSchedCadence] = useState<ScheduleCadence>("daily");
  const [schedTime, setSchedTime] = useState("08:00");
  const [schedRecipients, setSchedRecipients] = useState("");
  const [savingSched, setSavingSched] = useState(false);
  const [schedFormError, setSchedFormError] = useState<string | null>(null);

  // Run on-demand report state
  const [selectedRunDef, setSelectedRunDef] = useState<ReportDefinitionView | null>(null);
  const [runLoading, setRunLoading] = useState(false);
  const [runResult, setRunResult] = useState<any | null>(null);
  const [runError, setRunError] = useState<string | null>(null);

  const refreshDefinitions = useCallback(async () => {
    setLoadingDefs(true);
    setDefsError(null);
    const { data, error } = await api.api.GET("/api/v1/report-definitions");
    setLoadingDefs(false);
    if (error !== undefined || !data) {
      setDefsError("Failed to load report definitions.");
      return;
    }
    setDefinitions(data.definitions);
    if (!schedDefId && data.definitions.length > 0) {
      setSchedDefId(data.definitions[0]!.id);
    }
  }, [schedDefId]);

  const refreshSchedules = useCallback(async () => {
    setLoadingSchedules(true);
    setSchedulesError(null);
    const { data, error } = await api.api.GET("/api/v1/report-schedules");
    setLoadingSchedules(false);
    if (error !== undefined || !data) {
      setSchedulesError("Failed to load report schedules.");
      return;
    }
    setSchedules(data.schedules);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") {
      void refreshDefinitions();
      void refreshSchedules();
    }
  }, [auth.status, refreshDefinitions, refreshSchedules]);

  const handleSaveDefinition = async () => {
    if (!defName.trim()) return;
    setSavingDef(true);
    setDefFormError(null);
    const idempotencyKey =
      typeof crypto !== "undefined" && crypto.randomUUID
        ? crypto.randomUUID()
        : `rdef-${Date.now()}`;

    const spec = {
      view: defKind,
      measures: [{ agg: "count" as const }],
      viz: "table" as const,
    };

    if (editingDefId) {
      const { error } = await api.api.PUT("/api/v1/report-definitions/{id}", {
        params: { path: { id: editingDefId } },
        body: {
          name: defName.trim(),
          description: defDescription.trim() ? defDescription.trim() : null,
          spec,
          scope: defScope,
          idempotency_key: idempotencyKey,
        },
      });
      setSavingDef(false);
      if (error) {
        setDefFormError("Failed to update report definition.");
        return;
      }
    } else {
      const { error } = await api.api.POST("/api/v1/report-definitions", {
        body: {
          name: defName.trim(),
          description: defDescription.trim() ? defDescription.trim() : null,
          spec,
          scope: defScope,
          idempotency_key: idempotencyKey,
        },
      });
      setSavingDef(false);
      if (error) {
        setDefFormError("Failed to create report definition.");
        return;
      }
    }

    setDefModalOpen(false);
    setEditingDefId(null);
    setDefName("");
    setDefDescription("");
    void refreshDefinitions();
  };

  const handleDeleteDefinition = async (id: string) => {
    await api.api.DELETE("/api/v1/report-definitions/{id}", {
      params: { path: { id } },
    });
    void refreshDefinitions();
  };

  const handleSaveSchedule = async () => {
    if (!schedName.trim() || !schedDefId) return;
    setSavingSched(true);
    setSchedFormError(null);
    const idempotencyKey =
      typeof crypto !== "undefined" && crypto.randomUUID
        ? crypto.randomUUID()
        : `rsched-${Date.now()}`;

    const recipientList = schedRecipients
      .split(",")
      .map((s) => s.trim())
      .filter((s) => s.length > 0);

    const { error } = await api.api.POST("/api/v1/report-schedules", {
      body: {
        name: schedName.trim(),
        report_definition_id: schedDefId,
        cadence: schedCadence,
        time_of_day: schedTime,
        timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
        recipients: recipientList,
        idempotency_key: idempotencyKey,
      },
    });
    setSavingSched(false);
    if (error) {
      setSchedFormError("Failed to create report schedule. Verify parameters.");
      return;
    }

    setScheduleModalOpen(false);
    setSchedName("");
    setSchedRecipients("");
    void refreshSchedules();
  };

  const handleDeleteSchedule = async (id: string) => {
    await api.api.DELETE("/api/v1/report-schedules/{id}", {
      params: { path: { id } },
    });
    void refreshSchedules();
  };

  const handleRunReport = async (def: ReportDefinitionView) => {
    setSelectedRunDef(def);
    setTab("run");
    setRunLoading(true);
    setRunError(null);
    setRunResult(null);

    const { data, error } = await api.api.GET("/api/v1/reports/{kind}", {
      params: {
        path: { kind: def.spec.view },
        query: { limit: 50 },
      },
    });
    setRunLoading(false);
    if (error !== undefined || !data) {
      setRunError("Failed to run report.");
      return;
    }
    setRunResult(data);
  };

  if (auth.status !== "authed") return null;

  return (
    <div className="grid gap-6 max-w-5xl mx-auto p-6">
      <header className="flex flex-wrap items-center justify-between gap-4">
        <div>
          <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
            Reports & Scheduled Deliveries
          </h1>
          <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Stored report definition CRUD, compile-on-demand execution, and automated delivery schedules.
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            variant={tab === "definitions" ? "primary" : "secondary"}
            onClick={() => setTab("definitions")}
          >
            Definitions (R3)
          </Button>
          <Button
            variant={tab === "schedules" ? "primary" : "secondary"}
            onClick={() => setTab("schedules")}
          >
            Schedules (R4)
          </Button>
          {selectedRunDef && (
            <Button
              variant={tab === "run" ? "primary" : "secondary"}
              onClick={() => setTab("run")}
            >
              Run: {selectedRunDef.name}
            </Button>
          )}
        </div>
      </header>

      {tab === "definitions" && (
        <div className="grid gap-4">
          <div className="flex justify-between items-center">
            <h2 className="text-sm font-semibold">Saved Report Definitions</h2>
            <Dialog open={defModalOpen} onOpenChange={setDefModalOpen}>
              <DialogTrigger asChild>
                <Button
                  onClick={() => {
                    setEditingDefId(null);
                    setDefName("");
                    setDefDescription("");
                    setDefKind("tickets");
                    setDefScope("org");
                  }}
                >
                  Create definition
                </Button>
              </DialogTrigger>
              <DialogContent title={editingDefId ? "Edit Report Definition" : "Create Report Definition"}>
                <div className="grid gap-3 p-4">
                  <div>
                    <Label htmlFor="rd-name">Definition Name</Label>
                    <Input
                      id="rd-name"
                      placeholder="e.g. Monthly Billable Utilization"
                      value={defName}
                      onChange={(e) => setDefName(e.target.value)}
                      required
                    />
                  </div>

                  <div>
                    <Label htmlFor="rd-desc">Description</Label>
                    <Input
                      id="rd-desc"
                      placeholder="Optional summary"
                      value={defDescription}
                      onChange={(e) => setDefDescription(e.target.value)}
                    />
                  </div>

                  <div>
                    <Label htmlFor="rd-kind">Report View / Kind</Label>
                    <Select value={defKind} onValueChange={(v) => setDefKind(v as ViewId)}>
                      <SelectTrigger id="rd-kind" className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {REPORT_KINDS.map((k) => (
                          <SelectItem key={k.kind} value={k.kind}>
                            {k.label}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>

                  <div>
                    <Label htmlFor="rd-scope">Sharing Scope</Label>
                    <Select value={defScope} onValueChange={(v) => setDefScope(v as DefinitionScope)}>
                      <SelectTrigger id="rd-scope" className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="org">Org-wide (All staff)</SelectItem>
                        <SelectItem value="self">Private to me</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>

                  {defFormError && (
                    <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                      {defFormError}
                    </p>
                  )}

                  <DialogActions>
                    <Button variant="ghost" onClick={() => setDefModalOpen(false)}>
                      Cancel
                    </Button>
                    <Button onClick={() => void handleSaveDefinition()} disabled={savingDef || !defName.trim()}>
                      {savingDef ? <Spinner /> : null} Save Definition
                    </Button>
                  </DialogActions>
                </div>
              </DialogContent>
            </Dialog>
          </div>

          {defsError && (
            <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {defsError}
            </p>
          )}

          <Card className="overflow-x-auto">
            {loadingDefs ? (
              <div className="p-6 flex justify-center">
                <Spinner />
              </div>
            ) : definitions === null || definitions.length === 0 ? (
              <div className="p-6">
                <EmptyState message="No report definitions yet. Create one to run or schedule automated reports." />
              </div>
            ) : (
              <Table>
                <thead>
                  <tr>
                    <Th>Name</Th>
                    <Th>Report Kind</Th>
                    <Th>Scope</Th>
                    <Th>Updated</Th>
                    <Th className="text-right">Actions</Th>
                  </tr>
                </thead>
                <tbody>
                  {definitions.map((d) => (
                    <Tr key={d.id}>
                      <Td>
                        <div className="font-medium text-[13px]">{d.name}</div>
                        {d.description && (
                          <div className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                            {d.description}
                          </div>
                        )}
                      </Td>
                      <Td className="text-[12px]">
                        <code>{d.spec.view}</code>
                      </Td>
                      <Td>
                        <Tag tone={d.scope === "org" ? "neutral" : "warning"}>
                          {d.scope === "org" ? "Org" : "Self"}
                        </Tag>
                      </Td>
                      <Td className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                        {new Date(d.updated_at).toLocaleDateString()}
                      </Td>
                      <Td className="text-right">
                        <div className="flex justify-end gap-1">
                          <Button size="sm" variant="secondary" onClick={() => void handleRunReport(d)}>
                            Run
                          </Button>
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={() => {
                              setEditingDefId(d.id);
                              setDefName(d.name);
                              setDefDescription(d.description ?? "");
                              setDefKind(d.spec.view as ViewId);
                              setDefScope(d.scope);
                              setDefModalOpen(true);
                            }}
                          >
                            Edit
                          </Button>
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={() => void handleDeleteDefinition(d.id)}
                          >
                            Delete
                          </Button>
                        </div>
                      </Td>
                    </Tr>
                  ))}
                </tbody>
              </Table>
            )}
          </Card>
        </div>
      )}

      {tab === "schedules" && (
        <div className="grid gap-4">
          <div className="flex justify-between items-center">
            <h2 className="text-sm font-semibold">Delivery Schedules (R4)</h2>
            <Dialog open={scheduleModalOpen} onOpenChange={setScheduleModalOpen}>
              <DialogTrigger asChild>
                <Button>Create schedule</Button>
              </DialogTrigger>
              <DialogContent title="Create Delivery Schedule">
                <div className="grid gap-3 p-4">
                  <div>
                    <Label htmlFor="sch-name">Schedule Name</Label>
                    <Input
                      id="sch-name"
                      placeholder="e.g. Daily Executive Digest"
                      value={schedName}
                      onChange={(e) => setSchedName(e.target.value)}
                      required
                    />
                  </div>

                  <div>
                    <Label htmlFor="sch-def">Report Definition</Label>
                    <Select value={schedDefId} onValueChange={setSchedDefId}>
                      <SelectTrigger id="sch-def" className="w-full">
                        <SelectValue placeholder="Pick a report definition" />
                      </SelectTrigger>
                      <SelectContent>
                        {definitions?.map((d) => (
                          <SelectItem key={d.id} value={d.id}>
                            {d.name} ({d.spec.view})
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>

                  <div className="grid grid-cols-2 gap-2">
                    <div>
                      <Label htmlFor="sch-cadence">Cadence</Label>
                      <Select value={schedCadence} onValueChange={(v) => setSchedCadence(v as ScheduleCadence)}>
                        <SelectTrigger id="sch-cadence" className="w-full">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="daily">Daily</SelectItem>
                          <SelectItem value="weekly">Weekly</SelectItem>
                          <SelectItem value="monthly">Monthly</SelectItem>
                        </SelectContent>
                      </Select>
                    </div>

                    <div>
                      <Label htmlFor="sch-time">Time of Day (HH:MM)</Label>
                      <Input
                        id="sch-time"
                        value={schedTime}
                        onChange={(e) => setSchedTime(e.target.value)}
                        placeholder="08:00"
                      />
                    </div>
                  </div>

                  <div>
                    <Label htmlFor="sch-recip">Recipient User IDs (comma-separated UUIDs)</Label>
                    <Input
                      id="sch-recip"
                      placeholder="UUID1, UUID2"
                      value={schedRecipients}
                      onChange={(e) => setSchedRecipients(e.target.value)}
                    />
                    <FieldHint>Staff members in your org who will receive the delivery.</FieldHint>
                  </div>

                  {schedFormError && (
                    <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                      {schedFormError}
                    </p>
                  )}

                  <DialogActions>
                    <Button variant="ghost" onClick={() => setScheduleModalOpen(false)}>
                      Cancel
                    </Button>
                    <Button onClick={() => void handleSaveSchedule()} disabled={savingSched || !schedName.trim()}>
                      {savingSched ? <Spinner /> : null} Save Schedule
                    </Button>
                  </DialogActions>
                </div>
              </DialogContent>
            </Dialog>
          </div>

          {schedulesError && (
            <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {schedulesError}
            </p>
          )}

          <Card className="overflow-x-auto">
            {loadingSchedules ? (
              <div className="p-6 flex justify-center">
                <Spinner />
              </div>
            ) : schedules === null || schedules.length === 0 ? (
              <div className="p-6">
                <EmptyState message="No delivery schedules configured yet. Schedule automated report mailings above." />
              </div>
            ) : (
              <Table>
                <thead>
                  <tr>
                    <Th>Schedule Name</Th>
                    <Th>Definition</Th>
                    <Th>Cadence</Th>
                    <Th>Next Run</Th>
                    <Th>Last Run</Th>
                    <Th>Recipients</Th>
                    <Th className="text-right">Action</Th>
                  </tr>
                </thead>
                <tbody>
                  {schedules.map((s) => (
                    <Tr key={s.id}>
                      <Td className="font-medium text-[13px]">{s.name}</Td>
                      <Td className="text-[12px]">
                        {s.definition_name} <span style={{ color: "var(--m-text-muted)" }}>({s.definition_kind})</span>
                      </Td>
                      <Td>
                        <Tag tone="neutral">{s.cadence} @ {s.time_of_day}</Tag>
                      </Td>
                      <Td className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                        {s.next_run_at ? new Date(s.next_run_at).toLocaleString() : "—"}
                      </Td>
                      <Td className="text-[12px]">
                        {s.last_error ? (
                          <span title={s.last_error} style={{ color: "var(--m-status-danger-fg)" }}>
                            Failed {s.last_run_at ? new Date(s.last_run_at).toLocaleString() : ""}
                          </span>
                        ) : s.last_run_at ? (
                          <span style={{ color: "var(--m-text-secondary)" }}>
                            {new Date(s.last_run_at).toLocaleString()}
                          </span>
                        ) : (
                          "—"
                        )}
                      </Td>
                      <Td className="text-[12px]">
                        {s.recipients.length} user{s.recipients.length === 1 ? "" : "s"}
                      </Td>
                      <Td className="text-right">
                        <div className="flex justify-end gap-1">
                          <ReportDeliveryHistory scheduleId={s.id} scheduleName={s.name} />
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => void handleDeleteSchedule(s.id)}
                          >
                            Delete
                          </Button>
                        </div>
                      </Td>
                    </Tr>
                  ))}
                </tbody>
              </Table>
            )}
          </Card>
        </div>
      )}

      {tab === "run" && selectedRunDef && (
        <div className="grid gap-4">
          <div className="flex items-center justify-between">
            <div>
              <h2 className="text-sm font-semibold">Report Output: {selectedRunDef.name}</h2>
              <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                Kind: <code>{selectedRunDef.spec.view}</code>
              </p>
            </div>
            <Button variant="secondary" onClick={() => void handleRunReport(selectedRunDef)}>
              Re-run
            </Button>
          </div>

          {runError && (
            <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {runError}
            </p>
          )}

          <Card className="p-4 overflow-x-auto">
            {runLoading ? (
              <div className="p-6 flex justify-center">
                <Spinner />
              </div>
            ) : runResult ? (
              <div className="grid gap-4">
                {runResult.summary && (
                  <div className="p-3 rounded bg-black/5 dark:bg-white/5 text-[12px]">
                    <div className="font-semibold mb-1">Summary</div>
                    <pre className="overflow-x-auto">{JSON.stringify(runResult.summary, null, 2)}</pre>
                  </div>
                )}
                <div>
                  <div className="font-semibold text-xs mb-2">Rows ({runResult.rows?.length ?? 0})</div>
                  {runResult.rows && runResult.rows.length > 0 ? (
                    <Table>
                      <thead>
                        <tr>
                          {Object.keys(runResult.rows[0] || {}).map((k) => (
                            <Th key={k}>{k}</Th>
                          ))}
                        </tr>
                      </thead>
                      <tbody>
                        {runResult.rows.map((row: any, i: number) => (
                          <Tr key={i}>
                            {Object.values(row).map((val: any, j: number) => (
                              <Td key={j} className="text-[12px]">
                                {typeof val === "object" ? JSON.stringify(val) : String(val ?? "—")}
                              </Td>
                            ))}
                          </Tr>
                        ))}
                      </tbody>
                    </Table>
                  ) : (
                    <EmptyState message="No rows matched the query window." />
                  )}
                </div>
              </div>
            ) : (
              <EmptyState message="Click Run to fetch the latest report data." />
            )}
          </Card>
        </div>
      )}
    </div>
  );
}
