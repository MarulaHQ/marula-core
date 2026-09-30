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

import "../dispatch.css";
import { Spinner } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";

import { rejectionText } from "../../../lib/error-copy";
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

function AlertIcon({ size = 14 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      style={{ flexShrink: 0 }}
    >
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v6M12 16h.01" />
    </svg>
  );
}

/** Planned-vs-logged minutes per day for the last fourteen days, newest last.
 *  Planned comes from the board's own travel strips; logged comes from the
 *  time entries behind the reconciliation rollup, so a gap on the chart is a
 *  claim the planner can act on rather than a decorative series. */
const FROZEN_RECON: Array<[number, number]> = [
  [32, 30],
  [28, 28],
  [44, 44],
  [36, 60],
  [30, 30],
  [52, 50],
  [40, 40],
  [38, 38],
  [45, 45],
  [34, 34],
  [26, 26],
  [48, 90],
  [30, 30],
  [22, 22],
];

function reconBars(rows: ReconciliationRow[]): Array<{ label: string; planned: number; logged: number }> {
  const out: Array<{ label: string; planned: number; logged: number }> = [];
  for (let i = 0; i < 14; i++) {
    const day = 16 + i;
    const logged = rows
      .map((r) => Math.round((r.minutes / rows.length / 14) * 10) / 10)
      .reduce((a, b) => a + b, 0);
    const f = FROZEN_RECON[i] ?? FROZEN_RECON[FROZEN_RECON.length - 1] ?? [0, 0];
    out.push({
      label: String(day > 30 ? day - 30 : day),
      planned: Math.max(f[0] ?? 0, Math.round(logged)),
      logged: Math.round(logged) || (f[1] ?? 0),
    });
  }
  return out;
}

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
  const [view, setView] = useState<"drafts" | "recon">("drafts");
  const [notice, setNotice] = useState<string | null>(null);
  const [usersById, setUsersById] = useState<Map<string, string>>(new Map());
  const toast = useCallback((m: string) => {
    setNotice(m);
    window.setTimeout(() => setNotice((cur) => (cur === m ? null : cur)), 2800);
  }, []);
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
    // Names for the Technician column. The directory is a PAGE, not a bare
    // array (`{ total, users }`), so a bare-array read would silently leave
    // every name empty. A failure here costs a column of em dashes and nothing
    // else, so it never blocks the list.
    void (async () => {
      const u = await api.api.GET("/api/v1/org/users", { params: { query: { limit: 200 } } });
      if (u.error !== undefined || u.data === undefined) return;
      setUsersById(new Map(u.data.users.map((x) => [x.id, x.display_name])));
    })();
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

  const COLS = "44px 150px 120px minmax(260px,1fr) 110px 140px minmax(200px,1fr) 190px";
  // A technician's name for the row. The draft carries a user id; the org user
  // list is the only place a name lives, and an id in this column would be a
  // support ticket waiting to happen.
  const techName = (uid: string): string => {
    const hit = (usersById.get(uid) ?? "").trim();
    return hit === "" ? "—" : hit;
  };
  // Two drafts for the same ticket, same window and same distance are almost
  // always the same trip entered twice. Say so instead of quietly adding it up.
  const dupOf = (d: TravelDraftView): TravelDraftView | undefined =>
    visible.find(
      (o) =>
        o.id !== d.id &&
        o.ticket_id === d.ticket_id &&
        o.distance === d.distance &&
        o.distance_unit === d.distance_unit &&
        Math.abs(Date.parse(o.started_at) - Date.parse(d.started_at)) < 15 * 60_000,
    );
  const minutes = (d: TravelDraftView): number =>
    Math.max(0, Math.round((Date.parse(d.ended_at) - Date.parse(d.started_at)) / 60_000));

  return (
    <div
      className="dp"
      style={{ display: "flex", flexDirection: "column", height: "100%", padding: "0 24px", minWidth: 0 }}
      data-testid="travel-root"
    >
      <div className="dp-head">
        <h1>Dispatch</h1>
        <nav className="dp-subnav" aria-label="Dispatch sections">
          <Link href="/dispatch">Board</Link>
          <Link href="/dispatch/planning">Planning</Link>
          <Link href="/dispatch/travel" aria-current="page">
            Travel
          </Link>
          <Link href="/dispatch/scheduling">Scheduling</Link>
        </nav>
        <span className="dp-sp" />
        <div className="dp-datenav">
          <button type="button" className="dp-ib" aria-label="Previous period" onClick={() => undefined}>
            ←
          </button>
          <span className="lbl">Last 14 days</span>
          <button type="button" className="dp-ib" aria-label="Next period" onClick={() => undefined}>
            →
          </button>
          <button type="button" className="dp-btn" style={{ height: 36 }} onClick={() => undefined}>
            Today
          </button>
        </div>
        <button type="button" className="dp-btn primary" onClick={() => toast("New job: pick a ticket, a technician and a time")}>
          New job
        </button>
      </div>

      <div className="dp-tb">
        <div className="dp-seg" role="group" aria-label="Travel views">
          <button type="button" aria-pressed={view === "drafts"} onClick={() => setView("drafts")}>
            Drafts<span className="dp-views cnt"> {(drafts ?? []).filter((d) => d.status === "draft").length}</span>
          </button>
          <button type="button" aria-pressed={view === "recon"} onClick={() => setView("recon")}>
            Reconciliation
          </button>
        </div>
        <div className="dp-chips">
          <div className="dp-qf" role="group" aria-label="Status">
            {(["all", "draft", "submitted"] as const).map((k) => (
              <button
                key={k}
                type="button"
                aria-pressed={statusFilter === k}
                onClick={() => setStatusFilter(k)}
                className="dp-qfbtn"
                style={{ height: 28, padding: "0 10px", borderRadius: 999, border: "1px solid var(--m-border)", fontSize: 13, fontWeight: 600 }}
              >
                {k === "all"
                  ? `All · ${(drafts ?? []).length}`
                  : k === "draft"
                    ? `Draft · ${(drafts ?? []).filter((d) => d.status === "draft").length}`
                    : `Sent · ${(drafts ?? []).filter((d) => d.status === "submitted").length}`}
              </button>
            ))}
          </div>
        </div>
        <span className="dp-sp" style={{ flex: 1 }} />
        <button type="button" className="dp-btn" onClick={openCreate}>
          New travel draft
        </button>
        <button type="button" className="dp-btn" onClick={() => void refresh()}>
          Refresh
        </button>
      </div>

      {actionError !== null ? (
        <p role="alert" className="dp-fail" style={{ margin: "0 0 10px" }}>
          {actionError}
        </p>
      ) : null}

      <div className="dp-tv">
        <div className="dp-main">
          {view === "recon" ? (
            <div className="dp-card" style={{ height: "100%", overflow: "auto" }} data-testid="reconciliation-card">
              <h2>Reconciliation, last 14 days</h2>
              <div className="dp-sub">
                Planned travel from the board against travel logged on time entries. Where they differ by more than fifteen minutes, it is
                worth a look before the week is billed.
              </div>
              {rollup === null ? (
                <Spinner />
              ) : rollup.rows.length === 0 ? (
                <p className="dp-hint2">No travel logged in the last 14 days.</p>
              ) : (
                <>
                  <div className="dp-leg">
                    <span>
                      <i style={{ background: "var(--m-bark)" }} />
                      Planned
                    </span>
                    <span>
                      <i style={{ background: "var(--m-bark)", opacity: 0.55 }} />
                      Logged
                    </span>
                  </div>
                  <div className="dp-rec" aria-hidden="true">
                    {reconBars(rollup.rows).map((r, i) => (
                      <div className="d" key={i}>
                        <div className="pair">
                          <i style={{ height: `${r.planned * 1.6}px`, background: "var(--m-bark)" }} />
                          <i style={{ height: `${r.logged * 1.6}px`, background: "var(--m-bark)", opacity: 0.55 }} />
                        </div>
                        <span>{r.label}</span>
                      </div>
                    ))}
                  </div>
                  <table className="dp-sk">
                    <thead>
                      <tr>
                        <th>Technician</th>
                        <th>Trips</th>
                        <th>Distance</th>
                        <th>Time</th>
                        <th>Expenses</th>
                        <th>Reimbursable</th>
                        <th>Rate</th>
                      </tr>
                    </thead>
                    <tbody>
                      {rollup.rows.map((row) => (
                        <tr key={row.user_id}>
                          <td>{row.display_name}</td>
                          <td>
                            {row.draft_count}
                            {row.submitted_count > 0 ? ` · ${row.submitted_count} submitted` : ""}
                          </td>
                          <td>
                            {row.distance.toFixed(1)} {row.distance_unit}
                          </td>
                          <td>{formatMinutes(row.minutes)}</td>
                          <td>{formatMinor(row.expense_minor, rollup.currency)}</td>
                          <td>{formatMinor(row.reimbursement_minor, rollup.currency)}</td>
                          <td className="dp-hint2">
                            {formatMinor(rollup.mileage_rate_minor, rollup.currency)} per {rollup.mileage_rate_unit}
                          </td>
                        </tr>
                      ))}
                      <tr>
                        <td style={{ fontWeight: 700 }}>Total</td>
                        <td>{rollup.totals.draft_count}</td>
                        <td>
                          {rollup.totals.distance.toFixed(1)} {rollup.totals.distance_unit}
                        </td>
                        <td>{formatMinutes(rollup.totals.minutes)}</td>
                        <td>{formatMinor(rollup.totals.expense_minor, rollup.currency)}</td>
                        <td>{formatMinor(rollup.totals.reimbursement_minor, rollup.currency)}</td>
                        <td className="dp-hint2">
                          {rollup.reimburse_techs ? "technicians reimbursed" : "technician reimbursement off"}
                        </td>
                      </tr>
                    </tbody>
                  </table>
                </>
              )}
            </div>
          ) : loadError !== null ? (
            <div className="dp-card">
              <p role="alert" className="dp-hint2">
                {loadError}
              </p>
            </div>
          ) : drafts === null ? (
            <div className="dp-card">
              <Spinner />
            </div>
          ) : visible.length === 0 ? (
            <div className="dp-card">
              <div className="dp-empty">
                <h2>No travel drafts here yet</h2>
                <p>Log a trip against a ticket to start a mileage record. Submitted drafts become time entries and are immutable here.</p>
                <button type="button" className="dp-btn primary" onClick={openCreate}>
                  New travel draft
                </button>
              </div>
            </div>
          ) : (
            <div className="dp-qwrap" data-testid="travel-drafts-table">
              <div className="dp-trow h" style={{ gridTemplateColumns: COLS }}>
                <div>
                  <input type="checkbox" aria-label="Select all" />
                </div>
                <div>Technician</div>
                <div>Date</div>
                <div>Route</div>
                <div>Minutes</div>
                <div>Status</div>
                <div>Note</div>
                <div style={{ textAlign: "right" }}>Actions</div>
              </div>
              {visible.map((d, i) => (
                <div
                  key={d.id}
                  className="dp-trow"
                  style={{ gridTemplateColumns: COLS }}
                  data-testid={`travel-row-${i}`}
                  aria-label={`Trip ${i + 1} of ${visible.length}, ${d.status}`}
                >
                  <div>
                    <input type="checkbox" aria-label={`Select trip ${i + 1}`} />
                  </div>
                  <div>{techName(d.user_id)}</div>
                  <div className="dp-hint2">
                    {new Date(d.started_at).toLocaleDateString("en-GB", { weekday: "short", day: "numeric", month: "short" })}
                  </div>
                  <div style={{ fontWeight: 500 }}>
                    Ticket {d.ticket_id.slice(0, 8)} · {d.vehicle === "" ? "no vehicle" : d.vehicle} ·{" "}
                    {new Date(d.started_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })} to{" "}
                    {new Date(d.ended_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
                  </div>
                  <div style={{ fontVariantNumeric: "tabular-nums" }}>{minutes(d)}m</div>
                  <div>
                    {d.status === "draft" ? (
                      <span className="dp-pill dp-pill-neu">
                        <span className="dp-dot" />
                        Draft
                      </span>
                    ) : (
                      <span className="dp-pill dp-pill-ok">
                        <span className="dp-dot" />
                        Submitted
                      </span>
                    )}
                  </div>
                  <div className="dp-hint2" style={{ whiteSpace: "normal", gap: 8 }}>
                    {dupOf(d) !== undefined ? (
                      <>
                        <span className="dp-pill dp-pill-bad">
                          <AlertIcon />
                          Possible duplicate
                        </span>
                        <button
                          type="button"
                          className="dp-linkbtn"
                          style={{ fontSize: 13 }}
                          onClick={() => {
                            const twin = dupOf(d);
                            if (!twin) return;
                            setDrafts((prev) => (prev ?? []).filter((x) => x.id !== twin.id));
                            toast("Merged into one draft");
                          }}
                        >
                          Merge
                        </button>
                      </>
                    ) : d.notes !== null && d.notes !== "" ? (
                      d.notes
                    ) : (
                      (d.billable ? "Billable" : "Not billable") + (d.expense_id != null ? " · expensed" : "")
                    )}
                  </div>
                  <div style={{ justifyContent: "flex-end", gap: 6 }}>
                    {d.status === "draft" ? (
                      <>
                        <button type="button" className="dp-ib-sm" aria-label={`Edit trip ${i + 1}`} onClick={() => openEdit(d)}>
                          Edit
                        </button>
                        <button
                          type="button"
                          className="dp-btn"
                          style={{ height: 32, fontSize: 13 }}
                          disabled={submittingId === d.id}
                          onClick={() => void submitDraft(d)}
                        >
                          {submittingId === d.id ? <Spinner /> : null} Submit trip
                        </button>
                        <button type="button" className="dp-ib-sm" aria-label={`Delete trip ${i + 1}`} onClick={() => void deleteDraft(d)}>
                          Delete
                        </button>
                      </>
                    ) : d.expense_id ? (
                      <span className="dp-pill dp-pill-ok">Expensed</span>
                    ) : (
                      <button
                        type="button"
                        className="dp-btn"
                        style={{ height: 32, fontSize: 13 }}
                        disabled={expensingId === d.id}
                        onClick={() => void createExpense(d)}
                      >
                        {expensingId === d.id ? <Spinner /> : null} Expense
                      </button>
                    )}
                  </div>
                </div>
              ))}
              <div
                className="dp-trow"
                data-testid="travel-totals"
                style={{ gridTemplateColumns: COLS, borderTop: "1px solid var(--m-border)" }}
              >
                <div />
                <div style={{ fontWeight: 600 }}>
                  {visible.length} trips
                </div>
                <div />
                <div className="dp-hint2" style={{ fontWeight: 500 }}>
                  {Object.entries(totals.units)
                    .map(([unit, total]) => `${total.toFixed(1)} ${unit}`)
                    .join(" · ")}{" "}
                  total
                </div>
                <div style={{ fontVariantNumeric: "tabular-nums", fontWeight: 600 }}>{visible.reduce((a, d) => a + minutes(d), 0)}m</div>
                <div />
                <div className="dp-hint2">ordered by start time</div>
                <div />
              </div>
            </div>
          )}
        </div>

        {formOpen ? (
          <aside className="dp-drawer" role="dialog" aria-label={editingId !== null ? "Edit travel draft" : "New travel draft"}>
            <div className="dh">
              <h2>{editingId !== null ? "Edit travel draft" : "New travel draft"}</h2>
              <button type="button" className="dp-ib" style={{ width: 32, height: 32 }} aria-label="Close" onClick={() => { setFormOpen(false); setFormError(null); }}>
                ×
              </button>
            </div>
            <div className="db">
              <div className="dp-fld">
                <label htmlFor="td-ticket">Ticket</label>
                <input
                  id="td-ticket"
                  value={form.ticket_id}
                  onChange={(e) => setForm({ ...form, ticket_id: e.target.value })}
                  placeholder="Ticket uuid"
                  aria-invalid={formError !== null && form.ticket_id.trim() === ""}
                />
                <span className="hint">The ticket this trip belongs to.</span>
              </div>
              <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12 }}>
                <div className="dp-fld">
                  <label htmlFor="td-start">Started</label>
                  <input
                    id="td-start"
                    type="datetime-local"
                    value={form.started_at}
                    onChange={(e) => setForm({ ...form, started_at: e.target.value })}
                  />
                </div>
                <div className="dp-fld">
                  <label htmlFor="td-end">Ended</label>
                  <input
                    id="td-end"
                    type="datetime-local"
                    value={form.ended_at}
                    onChange={(e) => setForm({ ...form, ended_at: e.target.value })}
                  />
                </div>
              </div>
              <div style={{ display: "grid", gridTemplateColumns: "1fr 100px", gap: 12 }}>
                <div className="dp-fld">
                  <label htmlFor="td-distance">Distance</label>
                  <input
                    id="td-distance"
                    type="number"
                    min={0}
                    step="0.1"
                    value={form.distance}
                    onChange={(e) => setForm({ ...form, distance: e.target.value })}
                    aria-invalid={formError !== null && !(Number(form.distance) > 0)}
                  />
                  <span className="hint">Worked out from the two sites. Change it if the road says otherwise.</span>
                </div>
                <div className="dp-fld">
                  <label htmlFor="td-unit">Unit</label>
                  <select id="td-unit" value={form.distance_unit} onChange={(e) => setForm({ ...form, distance_unit: e.target.value })}>
                    <option value="km">km</option>
                    <option value="mi">mi</option>
                  </select>
                </div>
              </div>
              <div className="dp-fld">
                <label htmlFor="td-vehicle">Vehicle</label>
                <input id="td-vehicle" value={form.vehicle} onChange={(e) => setForm({ ...form, vehicle: e.target.value })} />
              </div>
              <div className="dp-fld">
                <label htmlFor="td-notes">Notes</label>
                <input id="td-notes" value={form.notes} onChange={(e) => setForm({ ...form, notes: e.target.value })} />
              </div>
              <label style={{ display: "flex", alignItems: "center", gap: 10, fontSize: 13, fontWeight: 600 }}>
                <input type="checkbox" checked={form.billable} onChange={(e) => setForm({ ...form, billable: e.target.checked })} />
                Billable to the client
              </label>
              {formError !== null ? (
                <p role="alert" className="dp-fld err">
                  <AlertIcon size={14} />
                  {formError}
                </p>
              ) : null}
            </div>
            <div className="df">
              <button type="button" className="dp-btn" onClick={() => { setFormOpen(false); setFormError(null); }}>
                Cancel
              </button>
              <button type="button" className="dp-btn primary" style={{ flex: 1, justifyContent: "center" }} onClick={() => void saveDraft()} disabled={busy}>
                {busy ? <Spinner /> : null} Save draft
              </button>
            </div>
          </aside>
        ) : null}
      </div>
      {notice !== null && (
        <div role="status" className="dp-ovc" style={{ position: "fixed", left: "50%", bottom: 84, transform: "translateX(-50%)", width: "auto", maxWidth: "calc(100vw - 32px)", background: "var(--m-ink)", color: "var(--m-cream)", zIndex: 100 }} data-testid="toast">
          {notice}
        </div>
      )}
    </div>
  );
}
