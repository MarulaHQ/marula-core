// /portal/schedule — appointment requests (Portal v2): browse published
// free slots for the next two weeks, ask for one, and track pending /
// decided requests. A request is an INTENT: staff approve (which books
// the calendar) or decline. Client copy stays honest about that.
"use client";

import { friendlyMutationText } from "../../../lib/error-copy";
import { Button, EmptyState, Spinner, StatusTag, type StatusTone } from "@meridian/ui";
import { useEffect, useMemo, useState } from "react";
import {
  portalApi,
  usePortalAuth,
  useRequirePortalAuth,
  type PortalAppointmentRequest,
  type PortalStaffSlots,
} from "../../../lib/portal-api";

function isoDate(d: Date): string {
  return d.toISOString().slice(0, 10);
}

function fmtTime(iso: string): string {
  return new Date(iso).toLocaleString(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function requestTone(status: PortalAppointmentRequest["status"]): StatusTone {
  switch (status) {
    case "approved":
      return "success";
    case "declined":
      return "danger";
    case "pending":
      return "warning";
    default:
      return "neutral";
  }
}

export default function PortalSchedulePage() {
  useRequirePortalAuth();
  const [slots, setSlots] = useState<PortalStaffSlots[] | null>(null);
  const [requests, setRequests] = useState<PortalAppointmentRequest[] | null>(null);
  const [picked, setPicked] = useState<{ userId: string; name: string; starts: string; ends: string } | null>(
    null,
  );
  const [title, setTitle] = useState("");
  const [notes, setNotes] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const range = useMemo(() => {
    const from = new Date();
    const to = new Date(Date.now() + 13 * 24 * 3600 * 1000);
    return { from: isoDate(from), to: isoDate(to) };
  }, []);

  function load() {
    let cancelled = false;
    setError(null);
    portalApi
      .scheduleSlots(range.from, range.to)
      .then((s) => {
        if (!cancelled) setSlots(s);
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(friendlyMutationText(err, "Failed to load availability"));
      });
    portalApi
      .listAppointmentRequests()
      .then((r) => {
        if (!cancelled) setRequests(r);
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(friendlyMutationText(err, "Failed to load your requests"));
      });
    return () => {
      cancelled = true;
    };
  }

  useEffect(() => load(), [range]);

  async function submit() {
    if (picked === null) return;
    if (title.trim() === "") {
      setNotice("Give the request a short title.");
      return;
    }
    setSubmitting(true);
    setNotice(null);
    try {
      await portalApi.createAppointmentRequest({
        user_id: picked.userId,
        title: title.trim(),
        starts_at: picked.starts,
        ends_at: picked.ends,
        kind: "remote",
        notes: notes.trim() === "" ? undefined : notes.trim(),
      });
      setPicked(null);
      setTitle("");
      setNotes("");
      setNotice("Request sent — your provider will confirm or suggest another time.");
      load();
    } catch (err: unknown) {
      setNotice(friendlyMutationText(err, "Sending the request failed; try again."));
      load();
    } finally {
      setSubmitting(false);
    }
  }

  async function cancel(id: string) {
    try {
      await portalApi.cancelAppointmentRequest(id);
      load();
    } catch (err: unknown) {
      setNotice(friendlyMutationText(err, "Cancelling failed; the request may already be decided."));
    }
  }

  return (
    <section>
      <div className="mb-4">
        <h1 className="text-base font-semibold">Schedule a visit</h1>
        <p className="mt-0.5 text-xs" style={{ color: "var(--m-text-muted)" }}>
          Free times below come from your provider&apos;s published
          availability. You ask; they confirm.
        </p>
      </div>

      {error !== null && (
        <p className="mb-3 text-sm" style={{ color: "var(--m-status-danger)" }}>
          {error}
        </p>
      )}
      {notice !== null && (
        <p className="mb-3 text-xs" style={{ color: "var(--m-text-secondary)" }}>
          {notice}
        </p>
      )}

      {picked !== null && (
        <div
          className="mb-4 rounded-md border p-3"
          style={{ borderColor: "var(--m-accent)" }}
        >
          <p className="text-sm font-medium">
            {fmtTime(picked.starts)} · with {picked.name}
          </p>
          <div className="mt-2 flex flex-wrap items-end gap-2">
            <label className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              What is it about?
              <input
                type="text"
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                className="mt-1 w-64 rounded-md border bg-transparent px-2 py-1 text-sm outline-none transition-colors duration-100 focus:border-[var(--m-accent)]"
                style={{ borderColor: "var(--m-border)" }}
                placeholder="Quarterly review, config change…"
              />
            </label>
            <label className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              Details (optional)
              <input
                type="text"
                value={notes}
                onChange={(e) => setNotes(e.target.value)}
                className="mt-1 w-64 rounded-md border bg-transparent px-2 py-1 text-sm outline-none transition-colors duration-100 focus:border-[var(--m-accent)]"
                style={{ borderColor: "var(--m-border)" }}
              />
            </label>
            <Button size="sm" onClick={submit} disabled={submitting}>
              {submitting ? "Sending…" : "Send request"}
            </Button>
            <Button variant="ghost" size="sm" onClick={() => setPicked(null)}>
              Cancel
            </Button>
          </div>
        </div>
      )}

      {slots === null ? (
        <div className="flex justify-center py-16">
          <Spinner aria-label="Loading availability" />
        </div>
      ) : slots.length === 0 ? (
        <EmptyState message="No published availability in the next two weeks — check back soon." />
      ) : (
        <div className="space-y-4">
          {slots.map((staff) => (
            <div key={staff.user_id}>
              <h2 className="mb-1 text-xs font-medium uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                With {staff.display_name}
              </h2>
              <ul className="flex flex-wrap gap-2">
                {staff.slots.slice(0, 12).map((slot) => {
                  const active =
                    picked !== null && picked.starts === slot.starts_at && picked.userId === staff.user_id;
                  return (
                    <li key={slot.starts_at}>
                      <button
                        type="button"
                        onClick={() =>
                          setPicked({
                            userId: staff.user_id,
                            name: staff.display_name,
                            starts: slot.starts_at,
                            ends: slot.ends_at,
                          })
                        }
                        aria-pressed={active}
                        className="rounded-md border px-2.5 py-1.5 text-xs transition-colors duration-100 hover:bg-[var(--m-surface-hover)]"
                        style={{
                          borderColor: active ? "var(--m-accent)" : "var(--m-border)",
                          color: active ? "var(--m-accent)" : "var(--m-text-secondary)",
                        }}
                      >
                        {fmtTime(slot.starts_at)}
                      </button>
                    </li>
                  );
                })}
              </ul>
            </div>
          ))}
        </div>
      )}

      <h2 className="mb-1 mt-8 text-sm font-semibold">Your requests</h2>
      {requests === null ? (
        <div className="flex justify-center py-8">
          <Spinner aria-label="Loading requests" />
        </div>
      ) : requests.length === 0 ? (
        <EmptyState message="No appointment requests yet — pick a time above." />
      ) : (
        <ul className="divide-y rounded-md border" style={{ borderColor: "var(--m-border)" }}>
          {requests.map((r) => (
            <li key={r.id} className="flex items-center justify-between gap-3 px-3 py-2.5">
              <div className="min-w-0">
                <div className="text-sm font-medium">{r.title}</div>
                <div className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                  {fmtTime(r.starts_at)}
                  {r.decision_note !== null && ` · ${r.decision_note}`}
                </div>
              </div>
              <div className="flex shrink-0 items-center gap-2">
                <StatusTag tone={requestTone(r.status)}>{r.status}</StatusTag>
                {r.status === "pending" && (
                  <Button variant="ghost" size="sm" onClick={() => cancel(r.id)}>
                    Withdraw
                  </Button>
                )}
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
