// /settings/pm-schedules — recurring/PM schedule management (Wave 11 lane
// 10 close-out of the lane 9 recurring backend, migration 0056).
//
// Purpose: schedule CRUD over the ticket-template + typed-cadence
// construct (daily/weekly/monthly + day-of-week/month + wall-clock time +
// IANA timezone — the 422 carries the cadence parse error verbatim and
// renders inline), next-run preview per row, enable/disable, and the
// per-schedule RUN LEDGER (GET /pm-schedules/{id}/runs — append-only,
// generated/skipped) with generated tickets clicking through to the real
// ticket. Mutations ride the instant-first envelope.
// Non-purpose: no manual run trigger (the worker owns claiming) and no
// ticket-template editor beyond title/body/priority.
"use client";

import { rejectionText } from "../../../lib/error-copy";
import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  EmptyState,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  StatusTag,
  Table,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";

import { api, useRequireAuth } from "../../../lib/api";
import { ConfirmDestructive, SaveBar, SettingsHeader } from "../../../components/settings/settings-shell";
import { runEnvelopeMutation } from "../../../lib/envelope-write";

type PmSchedule = components["schemas"]["PmSchedule"];
type PmRun = components["schemas"]["PmRun"];
type Board = components["schemas"]["Board"];
type OrgUser = components["schemas"]["OrgUser"];

const DAY_NAMES = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const PRIORITIES = ["p1", "p2", "p3", "p4"];

/** The typed cadence editor state (one shape for create + edit). */
interface CadenceDraft {
  cadence: "daily" | "weekly" | "monthly";
  day_of_week: string;
  day_of_month: string;
  time_of_day: string;
  timezone: string;
}

const DEFAULT_TIMEZONE = typeof Intl !== "undefined" ? Intl.DateTimeFormat().resolvedOptions().timeZone ?? "UTC" : "UTC";

/** The read view carries the cadence FLAT (cadence + time_of_day +
 * day_of_week/day_of_month + timezone) — fold it into the editor draft. */
function cadenceToDraft(schedule: PmSchedule): CadenceDraft {
  return {
    cadence: schedule.cadence,
    day_of_week: schedule.day_of_week == null ? "" : String(schedule.day_of_week),
    day_of_month: schedule.day_of_month == null ? "" : String(schedule.day_of_month),
    time_of_day: schedule.time_of_day.slice(0, 5),
    timezone: schedule.timezone ?? DEFAULT_TIMEZONE,
  };
}

function CadenceEditor({
  draft,
  onChange,
  idPrefix,
}: {
  draft: CadenceDraft;
  onChange(next: CadenceDraft): void;
  idPrefix: string;
}) {
  return (
    <div className="grid gap-2">
      <div className="flex flex-wrap gap-2">
        <div>
          <Label htmlFor={`${idPrefix}-cadence`}>Repeats</Label>
          <Select value={draft.cadence} onValueChange={(v) => onChange({ ...draft, cadence: v as CadenceDraft["cadence"] })}>
            <SelectTrigger id={`${idPrefix}-cadence`} className="w-32">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="daily">Daily</SelectItem>
              <SelectItem value="weekly">Weekly</SelectItem>
              <SelectItem value="monthly">Monthly</SelectItem>
            </SelectContent>
          </Select>
        </div>
        {draft.cadence === "weekly" ? (
          <div>
            <Label htmlFor={`${idPrefix}-dow`}>On</Label>
            <Select value={draft.day_of_week} onValueChange={(v) => onChange({ ...draft, day_of_week: v })}>
              <SelectTrigger id={`${idPrefix}-dow`} className="w-28">
                <SelectValue placeholder="Day" />
              </SelectTrigger>
              <SelectContent>
                {DAY_NAMES.map((d, i) => (
                  <SelectItem key={d} value={String(i)}>
                    {d}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        ) : null}
        {draft.cadence === "monthly" ? (
          <div>
            <Label htmlFor={`${idPrefix}-dom`}>Day of month (1–28)</Label>
            <Input
              id={`${idPrefix}-dom`}
              type="number"
              min="1"
              max="28"
              className="w-28"
              value={draft.day_of_month}
              onChange={(e) => onChange({ ...draft, day_of_month: e.target.value })}
            />
          </div>
        ) : null}
        <div>
          <Label htmlFor={`${idPrefix}-time`}>At (HH:MM)</Label>
          <Input
            id={`${idPrefix}-time`}
            placeholder="08:00"
            className="w-24"
            value={draft.time_of_day}
            onChange={(e) => onChange({ ...draft, time_of_day: e.target.value })}
          />
        </div>
        <div className="min-w-40">
          <Label htmlFor={`${idPrefix}-tz`}>Timezone (IANA)</Label>
          <Input
            id={`${idPrefix}-tz`}
            className="w-52"
            value={draft.timezone}
            onChange={(e) => onChange({ ...draft, timezone: e.target.value })}
          />
        </div>
      </div>
    </div>
  );
}

function cadenceBody(draft: CadenceDraft): components["schemas"]["CadenceSpec"] {
  const time = /^\d{2}:\d{2}$/.test(draft.time_of_day) ? `${draft.time_of_day}:00` : draft.time_of_day;
  return {
    cadence: draft.cadence,
    day_of_week: draft.cadence === "weekly" && draft.day_of_week !== "" ? Number(draft.day_of_week) : null,
    day_of_month: draft.cadence === "monthly" && draft.day_of_month !== "" ? Number(draft.day_of_month) : null,
    time_of_day: time,
    timezone: draft.timezone,
  };
}

export default function PmSchedulesSettingsPage() {
  const auth = useRequireAuth();

  const [schedules, setSchedules] = useState<PmSchedule[] | null>(null);
  const [boards, setBoards] = useState<Board[]>([]);
  const [clients, setClients] = useState<components["schemas"]["Client"][]>([]);
  const [directory, setDirectory] = useState<OrgUser[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);

  // Create / edit dialog
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<PmSchedule | null>(null);
  const [form, setForm] = useState({
    name: "",
    title: "",
    body: "",
    board_id: "",
    client_id: "",
    assignee_id: "",
    priority: "p3",
  });
  const [cadence, setCadence] = useState<CadenceDraft>({
    cadence: "weekly",
    day_of_week: "0",
    day_of_month: "",
    time_of_day: "08:00",
    timezone: DEFAULT_TIMEZONE,
  });
  const [formBusy, setFormBusy] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  // Run ledger dialog
  const [runsFor, setRunsFor] = useState<PmSchedule | null>(null);
  const [runs, setRuns] = useState<PmRun[] | null>(null);
  const [runsError, setRunsError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const [s, b, c, u] = await Promise.all([
      api.api.GET("/api/v1/pm-schedules"),
      api.api.GET("/api/v1/boards"),
      api.api.GET("/api/v1/clients"),
      api.api.GET("/api/v1/org/users", { params: { query: { limit: 200 } } }),
    ]);
    if (s.error !== undefined || s.data === undefined) {
      setLoadError("Failed to load PM schedules.");
      return;
    }
    setSchedules(s.data);
    if (b.data !== undefined) setBoards(b.data);
    if (c.data !== undefined) setClients(c.data);
    if (u.data !== undefined) setDirectory(u.data.users);
    setLoadError(null);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  const [pendingDelete, setPendingDelete] = useState<PmSchedule | null>(null);

  const openCreate = () => {
    setEditing(null);
    setForm({ name: "", title: "", body: "", board_id: boards[0]?.id ?? "", client_id: "", assignee_id: "", priority: "p3" });
    setCadence({ cadence: "weekly", day_of_week: "0", day_of_month: "", time_of_day: "08:00", timezone: DEFAULT_TIMEZONE });
    setFormError(null);
    setDialogOpen(true);
  };

  const openEdit = (schedule: PmSchedule) => {
    setEditing(schedule);
    setForm({
      name: schedule.name,
      title: schedule.title,
      body: schedule.body ?? "",
      board_id: schedule.board_id,
      client_id: schedule.client_id ?? "",
      assignee_id: schedule.assignee_id ?? "",
      priority: schedule.priority,
    });
    setCadence(cadenceToDraft(schedule));
    setFormError(null);
    setDialogOpen(true);
  };

  const save = async () => {
    setFormBusy(true);
    setFormError(null);
    const cadenceSpec = cadenceBody(cadence);
    if (editing === null) {
      if (form.board_id === "") {
        setFormError("Pick the board generated tickets land on.");
        setFormBusy(false);
        return;
      }
      const result = await runEnvelopeMutation<
        components["schemas"]["CreateScheduleRequest"],
        components["schemas"]["ScheduleMutationResponse"]
      >({
        describe: "Create PM schedule",
        optimistic: () => undefined,
        revert: () => undefined,
        buildBody: (base) => ({
          idempotency_key: base.idempotency_key,
          name: form.name.trim(),
          title: form.title.trim(),
          body: form.body.trim() === "" ? null : form.body.trim(),
          board_id: form.board_id,
          client_id: form.client_id === "" ? null : form.client_id,
          assignee_id: form.assignee_id === "" ? null : form.assignee_id,
          priority: form.priority,
          ...cadenceSpec,
        }),
        request: (client, body) => client.POST("/api/v1/pm-schedules", { body }),
      });
      setFormBusy(false);
      if (!result.ok) {
        setFormError(rejectionText(result, "The schedule was rejected."));
        return;
      }
    } else {
      const before = schedules ?? [];
      const result = await runEnvelopeMutation<
        components["schemas"]["UpdateScheduleRequest"],
        components["schemas"]["ScheduleMutationResponse"]
      >({
        describe: "Update PM schedule",
        entityId: editing.id,
        optimistic: () =>
          setSchedules((prev) =>
            (prev ?? []).map((s) =>
              s.id === editing.id
                ? {
                    ...s,
                    name: form.name.trim(),
                    title: form.title.trim(),
                    body: form.body.trim() === "" ? null : form.body.trim(),
                    priority: form.priority as PmSchedule["priority"],
                    cadence: cadenceSpec.cadence as PmSchedule["cadence"],
                    day_of_week: cadenceSpec.day_of_week,
                    day_of_month: cadenceSpec.day_of_month,
                    time_of_day: cadenceSpec.time_of_day,
                    timezone: cadenceSpec.timezone ?? s.timezone,
                    enabled: s.enabled,
                  }
                : s,
            ),
          ),
        revert: () => setSchedules(before),
        buildBody: (base) => ({
          idempotency_key: base.idempotency_key,
          name: form.name.trim(),
          title: form.title.trim(),
          body: form.body.trim() === "" ? null : form.body.trim(),
          priority: form.priority,
          cadence: cadenceSpec,
        }),
        request: (client, body) =>
          client.PATCH("/api/v1/pm-schedules/{id}", { params: { path: { id: editing.id } }, body }),
      });
      setFormBusy(false);
      if (!result.ok) {
        setFormError(rejectionText(result, "The change was rejected."));
        return;
      }
    }
    setDialogOpen(false);
    void refresh();
  };

  const setEnabled = async (schedule: PmSchedule, enabled: boolean) => {
    const before = schedules ?? [];
    const result = await runEnvelopeMutation<
      components["schemas"]["UpdateScheduleRequest"],
      components["schemas"]["ScheduleMutationResponse"]
    >({
      describe: enabled ? "Enable PM schedule" : "Disable PM schedule",
      entityId: schedule.id,
      optimistic: () => setSchedules((prev) => (prev ?? []).map((s) => (s.id === schedule.id ? { ...s, enabled } : s))),
      revert: () => setSchedules(before),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key, enabled }),
      request: (client, body) =>
        client.PATCH("/api/v1/pm-schedules/{id}", { params: { path: { id: schedule.id } }, body }),
    });
    if (!result.ok) {
      setLoadError(rejectionText(result, "The change was rejected."));
      void refresh();
    }
  };

  const remove = async (schedule: PmSchedule) => {
    const before = schedules ?? [];
    const result = await runEnvelopeMutation<
      components["schemas"]["MutationParams"],
      unknown
    >({
      describe: "Delete PM schedule",
      entityId: schedule.id,
      optimistic: () => setSchedules((prev) => (prev ?? []).filter((s) => s.id !== schedule.id)),
      revert: () => setSchedules(before),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key }),
      request: (client, body) =>
        client.DELETE("/api/v1/pm-schedules/{id}", {
          params: { path: { id: schedule.id } },
        }) as Promise<{ data?: unknown; error?: unknown; response: Response }>,
    });
    if (!result.ok) {
      setLoadError(rejectionText(result, "The delete was rejected."));
      void refresh();
    }
  };

  const openRuns = async (schedule: PmSchedule) => {
    setRunsFor(schedule);
    setRuns(null);
    setRunsError(null);
    const { data, error } = await api.api.GET("/api/v1/pm-schedules/{id}/runs", {
      params: { path: { id: schedule.id } },
    });
    if (error !== undefined || data === undefined) {
      setRunsError("Failed to load the run ledger.");
      return;
    }
    setRuns(data);
  };

  if (auth.status !== "authed") return null;

  return (
    <div className="grid gap-4">
      <div className="flex items-start justify-between gap-4">
        <SettingsHeader
          title="PM schedules"
          description="Recurring ticket templates: the worker claims due slots and generates ordinary tickets (audited on their timeline)."
        />
        <Button onClick={openCreate} data-testid="create-pm-schedule">
          New schedule
        </Button>
      </div>

      {loadError !== null ? (
        <p role="alert" className="text-[13px]">
          {loadError}
        </p>
      ) : schedules === null ? (
        <Card>
          <div className="flex justify-center p-6">
            <Spinner />
          </div>
        </Card>
      ) : schedules.length === 0 ? (
        <Card>
          <EmptyState message="No PM schedules yet. Create one to generate recurring tickets (e.g. monthly backup checks)." />
        </Card>
      ) : (
        <Card className="overflow-x-auto">
          <Table>
            <thead>
              <tr>
                <Th>Schedule</Th>
                <Th>Cadence</Th>
                <Th>Next run</Th>
                <Th>Last run</Th>
                <Th>State</Th>
                <Th className="text-right">Actions</Th>
              </tr>
            </thead>
            <tbody>
              {schedules.map((s) => (
                <Tr key={s.id}>
                  <Td className="text-[13px] font-medium">
                    {s.name}
                    <div className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                      {s.title} · {boards.find((b) => b.id === s.board_id)?.name ?? "board"} · {s.priority}
                    </div>
                  </Td>
                  <Td className="text-[12px]">
                    {s.cadence}
                    {s.cadence === "weekly" && s.day_of_week != null ? ` · ${DAY_NAMES[s.day_of_week] ?? "?"}` : ""}
                    {s.cadence === "monthly" && s.day_of_month != null ? ` · day ${s.day_of_month}` : ""}
                    <div className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                      {s.time_of_day.slice(0, 5)} {s.timezone}
                    </div>
                  </Td>
                  <Td className="text-[12px] tabular-nums">{new Date(s.next_run_at).toLocaleString()}</Td>
                  <Td className="text-[12px]">
                    {s.last_error ? (
                      <span title={s.last_error} style={{ color: "var(--m-status-danger-fg)" }}>
                        Failed {s.last_run_at ? new Date(s.last_run_at).toLocaleString() : ""}
                      </span>
                    ) : s.last_run_at ? (
                      new Date(s.last_run_at).toLocaleString()
                    ) : (
                      <span style={{ color: "var(--m-text-muted)" }}>never</span>
                    )}
                  </Td>
                  <Td>
                    <StatusTag tone={s.enabled ? "success" : "neutral"}>{s.enabled ? "enabled" : "paused"}</StatusTag>
                  </Td>
                  <Td className="text-right">
                    <div className="flex justify-end gap-1">
                      <Button size="sm" variant="ghost" onClick={() => void openRuns(s)} data-testid={`pm-runs-${s.name}`}>
                        Runs
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => setEnabled(s, !s.enabled)}>
                        {s.enabled ? "Pause" : "Enable"}
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => openEdit(s)}>
                        Edit
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => setPendingDelete(s)}>
                        Delete
                      </Button>
                    </div>
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        </Card>
      )}

      {/* Create / edit dialog */}
      <Dialog open={dialogOpen} onOpenChange={(o) => { setDialogOpen(o); if (!o) setFormError(null); }}>
        <DialogContent
          title={editing === null ? "New PM schedule" : "Edit PM schedule"}
          description="Generated tickets are ordinary tickets — board, client, priority and assignee set their landing shape."
        >
          <div className="grid gap-3 p-4">
            <div className="grid gap-3 sm:grid-cols-2">
              <div>
                <Label htmlFor="pm-name">Schedule name</Label>
                <Input id="pm-name" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} placeholder="Monthly backup checks" />
              </div>
              <div>
                <Label htmlFor="pm-title">Generated ticket title</Label>
                <Input id="pm-title" value={form.title} onChange={(e) => setForm({ ...form, title: e.target.value })} placeholder="Backup verification — {{site}}" />
              </div>
            </div>
            <div>
              <Label htmlFor="pm-body">Ticket body</Label>
              <Input id="pm-body" value={form.body} onChange={(e) => setForm({ ...form, body: e.target.value })} placeholder="Steps: verify last night's job, rotate tapes, log results." />
            </div>
            <div className="grid gap-3 sm:grid-cols-2">
              <div>
                <Label>Board</Label>
                <Select value={form.board_id} onValueChange={(v) => setForm({ ...form, board_id: v })} disabled={editing !== null}>
                  <SelectTrigger className="w-full" aria-label="Board for generated tickets">
                    <SelectValue placeholder="Pick a board" />
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
                <Label>Priority</Label>
                <Select value={form.priority} onValueChange={(v) => setForm({ ...form, priority: v })}>
                  <SelectTrigger className="w-full" aria-label="Priority">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {PRIORITIES.map((p) => (
                      <SelectItem key={p} value={p}>
                        {p.toUpperCase()}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
            <div className="grid gap-3 sm:grid-cols-2">
              <div>
                <Label>Client (optional)</Label>
                <Select value={form.client_id} onValueChange={(v) => setForm({ ...form, client_id: v })}>
                  <SelectTrigger className="w-full" aria-label="Client">
                    <SelectValue placeholder="Any" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="">Any</SelectItem>
                    {clients.map((c) => (
                      <SelectItem key={c.id} value={c.id}>
                        {c.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div>
                <Label>Assignee (optional)</Label>
                <Select value={form.assignee_id} onValueChange={(v) => setForm({ ...form, assignee_id: v })}>
                  <SelectTrigger className="w-full" aria-label="Assignee">
                    <SelectValue placeholder="Unassigned" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="">Unassigned</SelectItem>
                    {directory.map((u) => (
                      <SelectItem key={u.id} value={u.id}>
                        {u.display_name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
            <CadenceEditor draft={cadence} onChange={setCadence} idPrefix="pm" />
            <SaveBar
              dirty={form.name.trim() !== "" && form.title.trim() !== ""}
              saving={formBusy}
              error={formError}
              onSave={() => void save()}
              saveLabel={editing === null ? "Create schedule" : "Save"}
            />
            <DialogActions>
              <Button variant="ghost" onClick={() => setDialogOpen(false)}>
                Cancel
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>

      {/* Run ledger dialog */}
      <Dialog open={runsFor !== null} onOpenChange={(o) => { if (!o) { setRunsFor(null); setRuns(null); } }}>
        <DialogContent
          title={runsFor !== null ? `Run ledger — ${runsFor.name}` : "Run ledger"}
          description="Append-only: one row per claimed slot. Generated tickets are ordinary tickets linked below."
        >
          <div className="grid max-h-[60vh] gap-3 overflow-auto p-4">
            {runsError !== null ? (
              <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {runsError}
              </p>
            ) : runs === null ? (
              <div className="flex justify-center p-4">
                <Spinner />
              </div>
            ) : runs.length === 0 ? (
              <EmptyState message="No runs yet — the worker records a row per claimed slot." />
            ) : (
              <Table>
                <thead>
                  <tr>
                    <Th>Slot</Th>
                    <Th>Status</Th>
                    <Th>Generated ticket</Th>
                  </tr>
                </thead>
                <tbody>
                  {runs.map((r) => (
                    <Tr key={r.id}>
                      <Td className="text-[12px] tabular-nums">{new Date(r.slot_at).toLocaleString()}</Td>
                      <Td>
                        <StatusTag tone={r.status === "generated" ? "success" : "neutral"}>{r.status}</StatusTag>
                      </Td>
                      <Td className="text-[12px]">
                        {r.ticket_id != null ? (
                          <Link href={`/tickets/${r.ticket_id}`} className="underline decoration-[var(--m-border)] underline-offset-2">
                            open ticket
                          </Link>
                        ) : (
                          <span style={{ color: "var(--m-text-muted)" }}>—</span>
                        )}
                      </Td>
                    </Tr>
                  ))}
                </tbody>
              </Table>
            )}
          </div>
          <DialogActions>
            <Button variant="ghost" onClick={() => setRunsFor(null)}>
              Close
            </Button>
          </DialogActions>
        </DialogContent>
      </Dialog>

<ConfirmDestructive
        open={pendingDelete !== null}
        onOpenChange={(open) => {
          if (!open) setPendingDelete(null);
        }}
        title={`Delete PM schedule "${pendingDelete?.name ?? ""}"?`}
        body="Generated tickets stay. Future runs of this schedule stop permanently."
        confirmLabel="Delete schedule"
        onConfirm={() => {
          if (pendingDelete === null) return;
          const schedule = pendingDelete;
          setPendingDelete(null);
          void remove(schedule);
        }}
      />
    </div>
  );
}
