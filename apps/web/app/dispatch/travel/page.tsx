// /dispatch/travel — travel & mileage draft entry (Wave 9 lane 9
// close-out).
//
// Purpose: staff entry of travel/mileage drafts against a ticket
// (GET/POST /dispatch/travel-drafts), in-place editing of draft rows
// (PATCH), submit (POST .../submit — turns the draft into a time entry)
// and delete. Mutations ride the instant-first envelope: optimistic row
// patch, idempotency key, authoritative reconcile from the IF2 envelope.
//
// Non-purpose: no route/distance computation (the driver states the
// odometer distance); submitted drafts are immutable here — they live on
// the time-entry surface.
"use client";

import { rejectionText } from "../../../lib/error-copy";
import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
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
  Tag,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";

type TravelDraftView = components["schemas"]["TravelDraftView"] & {
  /** Wave 32 lane 7 links; the SDK gains them on the next openapi re-pin. */
  appointment_id?: string | null;
  expense_id?: string | null;
};
type TravelDraftMutationResponse = components["schemas"]["TravelDraftMutationResponse"];

/** Reconciliation view (Wave 32 lane 7) — local shape until the SDK
 *  re-pin picks up the new endpoint. */
type ReconciliationRow = {
  user_id: string;
  display_name: string;
  draft_count: number;
  submitted_count: number;
  expensed_count: number;
  distance: number;
  distance_unit: string;
  minutes: number;
  expense_minor: number;
  reimbursement_minor: number;
  billable_minor: number;
};
type ReconciliationView = {
  currency: string;
  mileage_rate_minor: number;
  mileage_rate_unit: string;
  reimburse_techs: boolean;
  rows: ReconciliationRow[];
  totals: ReconciliationRow;
};

function formatMinor(minor: number, currency: string): string {
  return new Intl.NumberFormat(undefined, {
    style: "currency",
    currency,
  }).format(minor / 100);
}

function formatMinutes(minutes: number): string {
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return hours > 0 ? `${hours}h ${rest}m` : `${rest}m`;
}

type FormState = {
  ticket_id: string;
  started_at: string;
  ended_at: string;
  distance: string;
  distance_unit: string;
  vehicle: string;
  billable: boolean;
  notes: string;
};

function emptyForm(): FormState {
  return {
    ticket_id: "",
    started_at: "",
    ended_at: "",
    distance: "",
    distance_unit: "km",
    vehicle: "",
    billable: true,
    notes: "",
  };
}

function toIso(local: string): string {
  return local === "" ? "" : new Date(local).toISOString();
}

export default function TravelDraftsPage() {
  const auth = useRequireAuth();
  const [drafts, setDrafts] = useState<TravelDraftView[] | null>(null);
  const [rollup, setRollup] = useState<ReconciliationView | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [statusFilter, setStatusFilter] = useState<"all" | "draft" | "submitted">("all");
  const [actionError, setActionError] = useState<string | null>(null);
  const [formOpen, setFormOpen] = useState(false);
  const [form, setForm] = useState<FormState>(emptyForm);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [submittingId, setSubmittingId] = useState<string | null>(null);
  const [expensingId, setExpensingId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const { data, error } = await api.api.GET("/api/v1/dispatch/travel-drafts");
    if (error !== undefined || data === undefined) {
      setLoadError("Failed to load travel drafts.");
      return;
    }
    setDrafts(data);
    setLoadError(null);
    const to = new Date();
    const from = new Date(to.getTime() - 14 * 24 * 60 * 60 * 1000);
    const reconciliation = (await api.api.GET(
      "/api/v1/dispatch/reconciliation" as never,
      {
        params: { query: { from: from.toISOString(), to: to.toISOString() } },
      } as never,
    )) as { data?: unknown; error?: unknown };
    if (reconciliation.error === undefined && reconciliation.data !== undefined) {
      setRollup(reconciliation.data as ReconciliationView);
    }
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  // Time-entry context entry point (Wave 32 lane 7): the approvals queue
  // and ticket surfaces link here with ?ticket_id=…, so logging the
  // trip's actual mileage starts from the entry that prompted it. Read
  // from `window` in the effect (client-only) so the page keeps
  // prerendering without a Suspense boundary.
  useEffect(() => {
    const ticketId = new URLSearchParams(window.location.search).get("ticket_id");
    if (ticketId !== null && ticketId.trim() !== "") {
      setForm({ ...emptyForm(), ticket_id: ticketId.trim() });
      setFormError(null);
      setFormOpen(true);
    }
  }, []);

  const openCreate = () => {
    setEditingId(null);
    setForm(emptyForm());
    setFormError(null);
    setFormOpen(true);
  };

  const openEdit = (d: TravelDraftView) => {
    setEditingId(d.id);
    setForm({
      ticket_id: d.ticket_id,
      started_at: d.started_at.slice(0, 16),
      ended_at: d.ended_at.slice(0, 16),
      distance: String(d.distance),
      distance_unit: d.distance_unit,
      vehicle: d.vehicle,
      billable: d.billable,
      notes: d.notes ?? "",
    });
    setFormError(null);
    setFormOpen(true);
  };

  const saveDraft = async () => {
    if (form.ticket_id.trim() === "" || form.started_at === "" || form.ended_at === "") {
      setFormError("Ticket, start, and end are required.");
      return;
    }
    const distance = Number(form.distance);
    if (!Number.isFinite(distance) || distance <= 0) {
      setFormError("Enter a distance greater than zero.");
      return;
    }
    setBusy(true);
    setFormError(null);
    const bodyFields = {
      ticket_id: form.ticket_id.trim(),
      started_at: toIso(form.started_at),
      ended_at: toIso(form.ended_at),
      distance,
      distance_unit: form.distance_unit,
      vehicle: form.vehicle.trim(),
      billable: form.billable,
      notes: form.notes.trim() === "" ? null : form.notes.trim(),
    };
    const tempId = editingId ?? `pending:${form.started_at}`;
    const optimisticRow: TravelDraftView = {
      id: tempId,
      user_id: "me",
      ticket_id: bodyFields.ticket_id,
      started_at: bodyFields.started_at,
      ended_at: bodyFields.ended_at,
      distance,
      distance_unit: bodyFields.distance_unit,
      vehicle: bodyFields.vehicle,
      billable: bodyFields.billable,
      notes: bodyFields.notes,
      status: "draft",
      submitted_time_entry_id: null,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };
    const previous = drafts;
    if (editingId !== null) {
      setDrafts((prev) =>
        (prev ?? []).map((d) => (d.id === editingId ? optimisticRow : d)),
      );
    } else {
      setDrafts((prev) => [optimisticRow, ...(prev ?? [])]);
    }
    const result = editingId !== null
      ? await runEnvelopeMutation<
          components["schemas"]["PatchTravelDraftRequest"],
          TravelDraftMutationResponse
        >({
          describe: "Update travel draft",
          entityId: editingId,
          optimistic: () => {},
          revert: () => setDrafts(previous),
          buildBody: (base) => ({ ...bodyFields, idempotency_key: base.idempotency_key }),
          request: (client, body) =>
            client.PATCH("/api/v1/dispatch/travel-drafts/{id}", {
              params: { path: { id: editingId } },
              body,
            }),
          versions: (response) => [[response.id, response.mutation.version]],
        })
      : await runEnvelopeMutation<
          components["schemas"]["CreateTravelDraftRequest"],
          TravelDraftMutationResponse
        >({
          describe: "Add travel draft",
          optimistic: () => {},
          revert: () => setDrafts(previous),
          buildBody: (base) => ({ ...bodyFields, idempotency_key: base.idempotency_key }),
          request: (client, body) =>
            client.POST("/api/v1/dispatch/travel-drafts", { body }),
          versions: (response) => [[response.id, response.mutation.version]],
        });
    setBusy(false);
    if (!result.ok) {
      setFormError(rejectionText(result, "The draft was rejected."));
      return;
    }
    setFormOpen(false);
    void refresh();
  };

  const submitDraft = async (d: TravelDraftView) => {
    setActionError(null);
    setSubmittingId(d.id);
    const previous = drafts;
    setDrafts((prev) =>
      (prev ?? []).map((row) => (row.id === d.id ? { ...row, status: "submitted" } : row)),
    );
    const result = await runEnvelopeMutation<
      components["schemas"]["SubmitTravelDraftRequest"],
      components["schemas"]["TimeEntryMutationResponse"]
    >({
      describe: "Submit travel draft",
      entityId: d.id,
      optimistic: () => {},
      revert: () => setDrafts(previous),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key }),
      request: (client, body) =>
        client.POST("/api/v1/dispatch/travel-drafts/{id}/submit", {
          params: { path: { id: d.id } },
          body,
        }),
    });
    setSubmittingId(null);
    if (!result.ok) {
      setActionError(rejectionText(result, "Submit was rejected."));
      return;
    }
    void refresh();
  };

  const deleteDraft = async (d: TravelDraftView) => {
    setActionError(null);
    const previous = drafts;
    setDrafts((prev) => (prev ?? []).filter((row) => row.id !== d.id));
    const { error } = await api.api.DELETE("/api/v1/dispatch/travel-drafts/{id}", {
      params: { path: { id: d.id } },
    });
    if (error !== undefined) {
      setDrafts(previous);
      setActionError("Delete failed — the draft is still in the list.");
      return;
    }
    void refresh();
  };

  const createExpense = async (d: TravelDraftView) => {
    setActionError(null);
    setExpensingId(d.id);
    const { error } = await api.api.POST(
      "/api/v1/dispatch/travel-drafts/{id}/expense" as never,
      {
        params: { path: { id: d.id } },
        body: { idempotency_key: crypto.randomUUID() },
      } as never,
    );
    setExpensingId(null);
    if (error !== undefined) {
      setActionError("The reimbursement expense was rejected.");
      return;
    }
    void refresh();
  };

  if (auth.status !== "authed") return null;

  const visible = (drafts ?? [])
    .filter((d) => statusFilter === "all" || d.status === statusFilter)
    .sort((a, b) => Date.parse(a.started_at) - Date.parse(b.started_at));

  // Trip totals for the ledger footer: distance per unit plus the billable
  // split, so the day's sequence reads as one coherent route record.
  const totals = visible.reduce(
    (acc, d) => {
      acc.units[d.distance_unit] = (acc.units[d.distance_unit] ?? 0) + d.distance;
      if (d.billable) acc.billable += d.distance;
      return acc;
    },
    { units: {} as Record<string, number>, billable: 0 },
  );

  return (
    <div className="grid gap-6 p-4 md:p-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-lg font-semibold">Travel &amp; mileage</h1>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Draft a trip against a ticket, then submit it to become a time entry.
            Submitted drafts are immutable.
          </p>
        </div>
        <div className="flex gap-2">
          <Select value={statusFilter} onValueChange={(v) => setStatusFilter(v as typeof statusFilter)}>
            <SelectTrigger aria-label="Filter by status" className="w-36">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All</SelectItem>
              <SelectItem value="draft">Draft</SelectItem>
              <SelectItem value="submitted">Submitted</SelectItem>
            </SelectContent>
          </Select>
          <Button onClick={openCreate}>Add draft</Button>
        </div>
      </div>

      {actionError !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {actionError}
        </p>
      ) : null}

      <Card>
        {loadError !== null ? (
          <div className="p-4">
            <p role="alert" className="text-[13px]">{loadError}</p>
          </div>
        ) : drafts === null ? (
          <div className="p-4">
            <Spinner />
          </div>
        ) : visible.length === 0 ? (
          <div className="p-4">
            <EmptyState
              message="No travel drafts here yet. Log a trip against a ticket to start a mileage record."
              action={<Button onClick={openCreate}>Add draft</Button>}
            />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>#</Th>
                <Th>Ticket</Th>
                <Th>When</Th>
                <Th>Vehicle</Th>
                <Th className="text-right">Distance</Th>
                <Th>Billable</Th>
                <Th>Status</Th>
                <Th className="text-right">Actions</Th>
              </tr>
            </thead>
            <tbody>
              {visible.map((d, i) => (
                <Tr key={d.id}>
                  <Td>
                    <span
                      aria-label={`Trip ${i + 1} of ${visible.length}`}
                      title={`Trip ${i + 1} of ${visible.length}`}
                      className="inline-grid h-5 w-5 place-items-center rounded-full text-[10px] font-semibold tabular-nums"
                      style={{ background: "var(--m-accent-soft)", color: "var(--m-accent)" }}
                    >
                      {i + 1}
                    </span>
                  </Td>
                  <Td className="tabular-nums">{d.ticket_id.slice(0, 8)}…</Td>
                  <Td>
                    {new Date(d.started_at).toLocaleString()} →{" "}
                    {new Date(d.ended_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
                  </Td>
                  <Td>{d.vehicle}</Td>
                  <Td className="text-right tabular-nums">
                    {d.distance} {d.distance_unit}
                  </Td>
                  <Td>{d.billable ? "yes" : "no"}</Td>
                  <Td>
                    <StatusTag tone={d.status === "submitted" ? "success" : "neutral"}>{d.status}</StatusTag>
                  </Td>
                  <Td className="text-right">
                    {d.status === "draft" ? (
                      <span className="flex justify-end gap-2">
                        <Button variant="ghost" size="sm" onClick={() => openEdit(d)}>
                          Edit
                        </Button>
                        <Button
                          variant="secondary"
                          size="sm"
                          onClick={() => void submitDraft(d)}
                          disabled={submittingId === d.id}
                        >
                          {submittingId === d.id ? <Spinner /> : null} Submit
                        </Button>
                        <Button variant="ghost" size="sm" onClick={() => void deleteDraft(d)}>
                          Delete
                        </Button>
                      </span>
                    ) : (
                      <span className="flex justify-end gap-2">
                        {d.expense_id ? (
                          <StatusTag tone="success">expensed</StatusTag>
                        ) : (
                          <Button
                            variant="secondary"
                            size="sm"
                            onClick={() => void createExpense(d)}
                            disabled={expensingId === d.id}
                          >
                            {expensingId === d.id ? <Spinner /> : null} Expense
                          </Button>
                        )}
                      </span>
                    )}
                  </Td>
                </Tr>
              ))}
                </tbody>
              </Table>
            )}
            {visible.length > 0 && (
              <div
                className="flex flex-wrap items-center gap-4 border-t px-4 py-2 text-[11px] tabular-nums"
                style={{ borderColor: "var(--m-border)", color: "var(--m-text-secondary)" }}
                data-testid="travel-totals"
              >
                <span>{visible.length} trips</span>
                {Object.entries(totals.units).map(([unit, total]) => (
                  <span key={unit}>
                    {total.toFixed(1)} {unit} total
                  </span>
                ))}
                <span>
                  {totals.billable.toFixed(1)} billable
                </span>
                <span style={{ color: "var(--m-text-muted)" }}>
                  ordered by start time
                </span>
              </div>
            )}
          </Card>

      <Card data-testid="reconciliation-card">
        <div className="border-b px-4 py-3" style={{ borderColor: "var(--m-border)" }}>
          <h2 className="text-sm font-semibold">Reconciliation &amp; reimbursements</h2>
          <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            Last 14 days of logged travel, priced at the org mileage rate.
            Creating an expense books the reimbursement against the ticket.
          </p>
        </div>
        {rollup === null ? (
          <div className="p-4">
            <Spinner />
          </div>
        ) : rollup.rows.length === 0 ? (
          <div className="p-4">
            <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              No travel logged in the last 14 days.
            </p>
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Technician</Th>
                <Th className="text-right">Trips</Th>
                <Th className="text-right">Distance</Th>
                <Th className="text-right">Time</Th>
                <Th className="text-right">Expenses</Th>
                <Th className="text-right">Reimbursable</Th>
              </tr>
            </thead>
            <tbody>
              {rollup.rows.map((row) => (
                <Tr key={row.user_id}>
                  <Td>{row.display_name}</Td>
                  <Td className="text-right tabular-nums">
                    {row.draft_count}
                    {row.submitted_count > 0 ? ` · ${row.submitted_count} submitted` : ""}
                  </Td>
                  <Td className="text-right tabular-nums">
                    {row.distance.toFixed(1)} {row.distance_unit}
                  </Td>
                  <Td className="text-right tabular-nums">{formatMinutes(row.minutes)}</Td>
                  <Td className="text-right tabular-nums">
                    {formatMinor(row.expense_minor, rollup.currency)}
                  </Td>
                  <Td className="text-right tabular-nums">
                    {formatMinor(row.reimbursement_minor, rollup.currency)}
                  </Td>
                </Tr>
              ))}
            </tbody>
            <tfoot>
              <tr
                className="border-t text-[12px] tabular-nums"
                style={{ borderColor: "var(--m-border)", color: "var(--m-text-secondary)" }}
              >
                <Td>Total</Td>
                <Td className="text-right">{rollup.totals.draft_count}</Td>
                <Td className="text-right">
                  {rollup.totals.distance.toFixed(1)} {rollup.totals.distance_unit}
                </Td>
                <Td className="text-right">{formatMinutes(rollup.totals.minutes)}</Td>
                <Td className="text-right">
                  {formatMinor(rollup.totals.expense_minor, rollup.currency)}
                </Td>
                <Td className="text-right">
                  {formatMinor(rollup.totals.reimbursement_minor, rollup.currency)}
                </Td>
              </tr>
            </tfoot>
          </Table>
        )}
        {rollup !== null ? (
          <div
            className="border-t px-4 py-2 text-[11px] tabular-nums"
            style={{ borderColor: "var(--m-border)", color: "var(--m-text-muted)" }}
          >
            Rate {formatMinor(rollup.mileage_rate_minor, rollup.currency)} per{" "}
            {rollup.mileage_rate_unit}
            {rollup.reimburse_techs ? "" : " · technician reimbursement off"}
          </div>
        ) : null}
      </Card>

      <Dialog open={formOpen} onOpenChange={(o) => { setFormOpen(o); if (!o) setFormError(null); }}>
        <DialogContent
          title={editingId !== null ? "Edit travel draft" : "Add travel draft"}
          description="Distance is what the odometer says — the API stores it with the trip window against the ticket."
        >
          <div className="grid gap-3 p-4">
            <div>
              <Label htmlFor="td-ticket">Ticket id</Label>
              <Input
                id="td-ticket"
                value={form.ticket_id}
                onChange={(e) => setForm({ ...form, ticket_id: e.target.value })}
                placeholder="Ticket uuid"
              />
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <Label htmlFor="td-start">Started</Label>
                <Input
                  id="td-start"
                  type="datetime-local"
                  value={form.started_at}
                  onChange={(e) => setForm({ ...form, started_at: e.target.value })}
                />
              </div>
              <div>
                <Label htmlFor="td-end">Ended</Label>
                <Input
                  id="td-end"
                  type="datetime-local"
                  value={form.ended_at}
                  onChange={(e) => setForm({ ...form, ended_at: e.target.value })}
                />
              </div>
            </div>
            <div className="grid grid-cols-3 gap-3">
              <div>
                <Label htmlFor="td-distance">Distance</Label>
                <Input
                  id="td-distance"
                  type="number"
                  min={0}
                  step="0.1"
                  value={form.distance}
                  onChange={(e) => setForm({ ...form, distance: e.target.value })}
                />
              </div>
              <div>
                <Label htmlFor="td-unit">Unit</Label>
                <Select
                  value={form.distance_unit}
                  onValueChange={(v) => setForm({ ...form, distance_unit: v })}
                >
                  <SelectTrigger id="td-unit" aria-label="Distance unit">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="km">km</SelectItem>
                    <SelectItem value="mi">mi</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div>
                <Label htmlFor="td-vehicle">Vehicle</Label>
                <Input
                  id="td-vehicle"
                  value={form.vehicle}
                  onChange={(e) => setForm({ ...form, vehicle: e.target.value })}
                />
              </div>
            </div>
            <div className="flex items-center gap-4">
              <label className="flex items-center gap-2 text-[13px]">
                <input
                  type="checkbox"
                  checked={form.billable}
                  onChange={(e) => setForm({ ...form, billable: e.target.checked })}
                />
                Billable
              </label>
              <div className="flex-1">
                <Label htmlFor="td-notes">Notes</Label>
                <Input
                  id="td-notes"
                  value={form.notes}
                  onChange={(e) => setForm({ ...form, notes: e.target.value })}
                />
              </div>
            </div>
            {formError !== null ? (
              <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {formError}
              </p>
            ) : null}
            <DialogActions>
              <Button variant="ghost" onClick={() => setFormOpen(false)}>
                Cancel
              </Button>
              <Button onClick={() => void saveDraft()} disabled={busy}>
                {busy ? <Spinner /> : null} Save draft
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
