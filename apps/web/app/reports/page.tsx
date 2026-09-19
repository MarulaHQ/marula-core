// /reports — Report builder v1, saved reports, and scheduled delivery
// (Wave 14 lane 4 R6; builds on the Wave 11 lane 8 R3/R4 surfaces).
//
// Purpose:
//  - R6 builder: catalog-driven thin report builder (view → measures →
//    group-by → filters → viz) with compile-on-demand preview and
//    save/share (org/self scope).
//  - Saved reports: run (compiled server-side from the stored spec),
//    edit-in-builder, publish/unpublish, delete.
//  - R4 schedules: schedule CRUD + delivery history (unchanged surface).
//
// Instant-first mutations, typed reason-code copy (never raw JSON),
// Workbench tokens, light+dark, ≤120ms transitions, responsive.
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
import { useCallback, useEffect, useRef, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { api, useRequireAuth } from "../../lib/api";
import { friendlyMutationText } from "../../lib/error-copy";
import { ReportDeliveryHistory } from "../../components/report-delivery-history";
import { ReportBuilder } from "../../components/report-builder";
import { VIEW_LABELS } from "../../lib/report-builder";
import { isManagerPermissions } from "../../lib/dashboards";
import { decodeSpecFromQuery } from "../../lib/report-spec-codec";

type DeepLinkSeed = { spec: components["schemas"]["ReportSpec"]; from?: string; to?: string; nonce: number } | null;

type ReportDefinitionView = components["schemas"]["ReportDefinitionView"];
type ReportScheduleView = components["schemas"]["ReportScheduleView"];
type ScheduleCadence = components["schemas"]["ScheduleCadence"];

type Tab = "builder" | "saved" | "schedules";

export default function ReportsPage() {
  const auth = useRequireAuth();
  const canPublish =
    auth.status === "authed" && isManagerPermissions(auth.me?.permissions ?? []);

  const [tab, setTab] = useState<Tab>("builder");

  // Deep link: /reports?spec=<encoded> lands on the Builder tab with the
  // linked spec seeded and auto-run once (Wave 23 lane 1, items 2+3).
  const router = useRouter();
  const searchParams = useSearchParams();
  const [seedSpec, setSeedSpec] = useState<DeepLinkSeed>(null);
  const landedRef = useRef<string | null>(null);
  useEffect(() => {
    const raw = searchParams.get("spec");
    if (raw === null || landedRef.current === raw) return;
    const decoded = decodeSpecFromQuery(raw);
    if (decoded === null) return;
    landedRef.current = raw;
    setSeedSpec({ ...decoded, nonce: Date.now() });
    setTab("builder");
    router.replace("/reports", { unstable_noScroll: true } as never);
  }, [router, searchParams]);

  // Catalog (R6 builder field lists — served from the Rust catalog).
  const [catalog, setCatalog] = useState<components["schemas"]["CatalogView"][] | null>(null);
  const [catalogError, setCatalogError] = useState<string | null>(null);

  // Definitions state
  const [definitions, setDefinitions] = useState<ReportDefinitionView[] | null>(null);
  const [loadingDefs, setLoadingDefs] = useState(false);
  const [defsError, setDefsError] = useState<string | null>(null);
  const [editingDef, setEditingDef] = useState<ReportDefinitionView | null>(null);
  const [runToken, setRunToken] = useState<{ def: ReportDefinitionView; nonce: number } | null>(null);
  const [rowBusyId, setRowBusyId] = useState<string | null>(null);
  const [rowError, setRowError] = useState<{ id: string; text: string } | null>(null);

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
  const [schedRecipients, setSchedRecipients] = useState<string[]>([]);
  const [orgUsers, setOrgUsers] = useState<{ id: string; display_name: string; email: string }[] | null>(null);
  const [savingSched, setSavingSched] = useState(false);
  const [schedFormError, setSchedFormError] = useState<string | null>(null);

  const refreshDefinitions = useCallback(async () => {
    setLoadingDefs(true);
    setDefsError(null);
    const { data, error } = await api.api.GET("/api/v1/report-definitions");
    setLoadingDefs(false);
    if (error !== undefined || !data) {
      setDefsError(friendlyMutationText(error, "Report definitions could not be loaded."));
      return;
    }
    setDefinitions(data.definitions);
  }, []);

  const refreshSchedules = useCallback(async () => {
    setLoadingSchedules(true);
    setSchedulesError(null);
    const { data, error } = await api.api.GET("/api/v1/report-schedules");
    setLoadingSchedules(false);
    if (error !== undefined || !data) {
      setSchedulesError(friendlyMutationText(error, "Report schedules could not be loaded."));
      return;
    }
    setSchedules(data.schedules);
  }, []);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refreshDefinitions();
    void refreshSchedules();
    setCatalogError(null);
    void api.api
      .GET("/api/v1/report-catalog")
      .then(({ data, error }) => {
        if (error !== undefined || !data) {
          setCatalogError("The report field catalog could not be loaded.");
          return;
        }
        setCatalog(data.views);
      })
      .catch(() => setCatalogError("The report field catalog could not be loaded."));
  }, [auth.status, refreshDefinitions, refreshSchedules]);

  const handleDeleteDefinition = async (id: string) => {
    setRowBusyId(id);
    setRowError(null);
    const { error } = await api.api.DELETE("/api/v1/report-definitions/{id}", {
      params: { path: { id } },
    });
    setRowBusyId(null);
    if (error !== undefined) {
      setRowError({ id, text: friendlyMutationText(error, "The report could not be deleted.") });
      return;
    }
    if (editingDef?.id === id) setEditingDef(null);
    void refreshDefinitions();
  };

  /** Share = flip the definition's visibility scope (full-replace PUT). */
  const handleShare = async (def: ReportDefinitionView, scope: "org" | "self") => {
    setRowBusyId(def.id);
    setRowError(null);
    const idempotencyKey =
      typeof crypto !== "undefined" && crypto.randomUUID ? crypto.randomUUID() : `share-${Date.now()}`;
    const { error } = await api.api.PUT("/api/v1/report-definitions/{id}", {
      params: { path: { id: def.id } },
      body: {
        name: def.name,
        description: def.description ?? null,
        spec: def.spec,
        scope,
        idempotency_key: idempotencyKey,
      },
    });
    setRowBusyId(null);
    if (error !== undefined) {
      setRowError({
        id: def.id,
        text: friendlyMutationText(
          error,
          scope === "org"
            ? "This report could not be published org-wide."
            : "This report could not be made private.",
        ),
      });
      return;
    }
    void refreshDefinitions();
  };

  const handleSaveSchedule = async () => {
    if (!schedName.trim() || !schedDefId) return;
    setSavingSched(true);
    setSchedFormError(null);
    const idempotencyKey =
      typeof crypto !== "undefined" && crypto.randomUUID ? crypto.randomUUID() : `rsched-${Date.now()}`;
    const { error } = await api.api.POST("/api/v1/report-schedules", {
      body: {
        name: schedName.trim(),
        report_definition_id: schedDefId,
        cadence: schedCadence,
        time_of_day: schedTime,
        timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
        recipients: schedRecipients,
        idempotency_key: idempotencyKey,
      },
    });
    setSavingSched(false);
    if (error !== undefined) {
      setSchedFormError(friendlyMutationText(error, "The schedule could not be created."));
      return;
    }
    setScheduleModalOpen(false);
    setSchedName("");
    setSchedRecipients([]);
    void refreshSchedules();
  };

  const handleDeleteSchedule = async (id: string) => {
    const { error } = await api.api.DELETE("/api/v1/report-schedules/{id}", {
      params: { path: { id } },
    });
    if (error !== undefined) {
      setSchedulesError(friendlyMutationText(error, "The schedule could not be deleted."));
      return;
    }
    void refreshSchedules();
  };

  if (auth.status !== "authed") return null;

  return (
    <div className="grid gap-6 max-w-6xl mx-auto p-4 md:p-6">
      <header className="flex flex-wrap items-center justify-between gap-4">
        <div>
          <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
            Reports
          </h1>
          <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Build reports over the operational read model, run them on demand, or schedule delivery.
          </p>
        </div>

        <div className="flex items-center gap-2" role="tablist" aria-label="Report sections">
          <Button variant={tab === "builder" ? "primary" : "secondary"} onClick={() => setTab("builder")} aria-selected={tab === "builder"} role="tab">
            Builder
          </Button>
          <Button variant={tab === "saved" ? "primary" : "secondary"} onClick={() => setTab("saved")} aria-selected={tab === "saved"} role="tab">
            Saved reports
          </Button>
          <Button variant={tab === "schedules" ? "primary" : "secondary"} onClick={() => setTab("schedules")} aria-selected={tab === "schedules"} role="tab">
            Schedules
          </Button>
        </div>
      </header>

      {tab === "builder" && (
        <div className="grid gap-4">
          {catalogError !== null ? (
            <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {catalogError}
            </p>
          ) : null}
          <ReportBuilder
            catalog={catalog}
            editingDef={editingDef}
            canPublish={canPublish}
            runToken={runToken}
            seedSpec={seedSpec}
            onSaved={() => {
              setEditingDef(null);
              void refreshDefinitions();
            }}
            onCancelEdit={() => {
              setEditingDef(null);
              setRunToken(null);
            }}
          />
        </div>
      )}

      {tab === "saved" && (
        <div className="grid gap-4">
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
                <EmptyState message="No saved reports yet. Build one in the Builder tab, then save it here." />
              </div>
            ) : (
              <Table>
                <thead>
                  <tr>
                    <Th>Name</Th>
                    <Th>Data</Th>
                    <Th>Visibility</Th>
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
                      <Td className="text-[12px]">{VIEW_LABELS[d.spec.view] ?? d.spec.view}</Td>
                      <Td>
                        <Tag tone={d.scope === "org" ? "neutral" : "warning"}>
                          {d.scope === "org" ? "Org-wide" : "Private"}
                        </Tag>
                      </Td>
                      <Td className="text-[12px] whitespace-nowrap" style={{ color: "var(--m-text-secondary)" }}>
                        {new Date(d.updated_at).toLocaleDateString()}
                      </Td>
                      <Td className="text-right">
                        <div className="flex justify-end flex-wrap gap-1">
                          {rowBusyId === d.id ? <Spinner /> : null}
                          <Button
                            size="sm"
                            variant="secondary"
                            onClick={() => {
                              setEditingDef(d);
                              setRunToken({ def: d, nonce: Date.now() });
                              setTab("builder");
                            }}
                          >
                            Run
                          </Button>
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={() => {
                              setEditingDef(d);
                              setTab("builder");
                            }}
                          >
                            Edit
                          </Button>
                          {canPublish && d.scope === "self" ? (
                            <Button size="sm" variant="ghost" onClick={() => void handleShare(d, "org")}>
                              Publish
                            </Button>
                          ) : null}
                          {canPublish && d.scope === "org" ? (
                            <Button size="sm" variant="ghost" onClick={() => void handleShare(d, "self")}>
                              Unpublish
                            </Button>
                          ) : null}
                          <Button size="sm" variant="ghost" onClick={() => void handleDeleteDefinition(d.id)}>
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
          {rowError !== null ? (
            <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {rowError.text}
            </p>
          ) : null}
        </div>
      )}

      {tab === "schedules" && (
        <div className="grid gap-4">
          <div className="flex justify-between items-center flex-wrap gap-2">
            <h2 className="text-sm font-semibold">Delivery schedules</h2>
            <Dialog
              open={scheduleModalOpen}
              onOpenChange={(open) => {
                setScheduleModalOpen(open);
                if (open && orgUsers === null) {
                  void api.api
                    .GET("/api/v1/org/users")
                    .then(({ data }) => {
                      if (data) {
                        setOrgUsers(
                          data.users
                            .filter((u) => u.is_active)
                            .map((u) => ({ id: u.id, display_name: u.display_name, email: u.email })),
                        );
                      }
                    })
                    .catch(() => setOrgUsers([]));
                }
              }}
            >
              <DialogTrigger asChild>
                <Button>Create schedule</Button>
              </DialogTrigger>
              <DialogContent title="Create delivery schedule">
                <div className="grid gap-3 p-4">
                  <div>
                    <Label htmlFor="sch-name">Schedule name</Label>
                    <Input
                      id="sch-name"
                      placeholder="e.g. Daily executive digest"
                      value={schedName}
                      onChange={(e) => setSchedName(e.target.value)}
                      required
                    />
                  </div>

                  <div>
                    <Label htmlFor="sch-def">Report</Label>
                    <Select value={schedDefId} onValueChange={setSchedDefId}>
                      <SelectTrigger id="sch-def" className="w-full">
                        <SelectValue placeholder="Pick a saved report" />
                      </SelectTrigger>
                      <SelectContent>
                        {(definitions ?? []).map((d) => (
                          <SelectItem key={d.id} value={d.id}>
                            {d.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <FieldHint>Scheduled delivery emails the report as of each run.</FieldHint>
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
                      <Label htmlFor="sch-time">Time of day (UTC)</Label>
                      <Input
                        id="sch-time"
                        value={schedTime}
                        onChange={(e) => setSchedTime(e.target.value)}
                        placeholder="08:00"
                      />
                    </div>
                  </div>

                  <div>
                    <Label htmlFor="sch-recip">Recipients</Label>
                    <div className="grid gap-1 max-h-40 overflow-y-auto border p-2" style={{ borderColor: "var(--m-border)" }}>
                      {orgUsers === null ? (
                        <Spinner />
                      ) : orgUsers.length === 0 ? (
                        <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                          No staff users found.
                        </span>
                      ) : (
                        orgUsers.map((u) => (
                          <label key={u.id} className="flex items-center gap-2 text-[13px]">
                            <input
                              type="checkbox"
                              checked={schedRecipients.includes(u.id)}
                              onChange={(e) =>
                                setSchedRecipients((prev) =>
                                  e.target.checked ? [...prev, u.id] : prev.filter((id) => id !== u.id),
                                )
                              }
                            />
                            {u.display_name} <span style={{ color: "var(--m-text-muted)" }}>{u.email}</span>
                          </label>
                        ))
                      )}
                    </div>
                    <FieldHint>Delivery goes to org staff — pick at least one recipient.</FieldHint>
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
                    <Button
                      onClick={() => void handleSaveSchedule()}
                      disabled={savingSched || !schedName.trim() || schedRecipients.length === 0}
                    >
                      {savingSched ? <Spinner /> : null} Save schedule
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
                <EmptyState message="No delivery schedules yet. Save a report first, then schedule it here." />
              </div>
            ) : (
              <Table>
                <thead>
                  <tr>
                    <Th>Name</Th>
                    <Th>Report</Th>
                    <Th>Cadence</Th>
                    <Th>Next run</Th>
                    <Th>Last run</Th>
                    <Th>Recipients</Th>
                    <Th className="text-right">Actions</Th>
                  </tr>
                </thead>
                <tbody>
                  {schedules.map((s) => (
                    <Tr key={s.id}>
                      <Td className="font-medium text-[13px]">{s.name}</Td>
                      <Td className="text-[12px]">{s.definition_name}</Td>
                      <Td>
                        <Tag tone="neutral">
                          {s.cadence} @ {s.time_of_day}
                        </Tag>
                      </Td>
                      <Td className="text-[12px] whitespace-nowrap" style={{ color: "var(--m-text-secondary)" }}>
                        {s.next_run_at ? new Date(s.next_run_at).toLocaleString() : "—"}
                      </Td>
                      <Td className="text-[12px] whitespace-nowrap">
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
                        {s.recipients.length} recipient{s.recipients.length === 1 ? "" : "s"}
                      </Td>
                      <Td className="text-right">
                        <div className="flex justify-end gap-1">
                          <ReportDeliveryHistory scheduleId={s.id} scheduleName={s.name} />
                          <Button variant="ghost" size="sm" onClick={() => void handleDeleteSchedule(s.id)}>
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
    </div>
  );
}
