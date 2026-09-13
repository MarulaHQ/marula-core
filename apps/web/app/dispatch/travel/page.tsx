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

type TravelDraftView = components["schemas"]["TravelDraftView"];
type TravelDraftMutationResponse = components["schemas"]["TravelDraftMutationResponse"];

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
  const [loadError, setLoadError] = useState<string | null>(null);
  const [statusFilter, setStatusFilter] = useState<"all" | "draft" | "submitted">("all");
  const [actionError, setActionError] = useState<string | null>(null);
  const [formOpen, setFormOpen] = useState(false);
  const [form, setForm] = useState<FormState>(emptyForm);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [submittingId, setSubmittingId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const { data, error } = await api.api.GET("/api/v1/dispatch/travel-drafts");
    if (error !== undefined || data === undefined) {
      setLoadError("Failed to load travel drafts.");
      return;
    }
    setDrafts(data);
    setLoadError(null);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

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

  if (auth.status !== "authed") return null;

  const visible = (drafts ?? []).filter(
    (d) => statusFilter === "all" || d.status === statusFilter,
  );

  return (
    <div className="grid gap-6">
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
              {visible.map((d) => (
                <Tr key={d.id}>
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
                    <Tag>{d.status}</Tag>
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
                      <span style={{ color: "var(--m-text-muted)" }}>—</span>
                    )}
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
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
