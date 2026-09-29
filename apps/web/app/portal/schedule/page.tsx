// /portal/schedule — appointment requests (Portal v2): browse published
// free slots for the next two weeks, ask for one, and track pending /
// decided requests. A request is an INTENT: staff approve (which books
// the calendar) or decline. Client copy stays honest about that.
//
// Wave 17 lane 8 UX pass: slots grouped by DAY (help-center/booking-flow
// pattern — date pill rows instead of one flattened strip), a person
// selector, an explicit selected-time confirm card before the title/notes
// form, honest status labels, staggered entrances. Same pinned endpoints,
// same honest intent semantics.
"use client";

import { friendlyMutationText } from "../../../lib/error-copy";
import { Button, EmptyState, Input, Label, Spinner, StatusTag, type StatusTone } from "@marula-core/ui";
import { useEffect, useMemo, useState } from "react";
import {
  portalApi,
  useRequirePortalAuth,
  type PortalAppointmentRequest,
  type PortalStaffSlots,
} from "../../../lib/portal-api";

type Slot = { userId: string; name: string; starts: string; ends: string };

function fmtTime(iso: string): string {
  return new Date(iso).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

function fmtDay(iso: string): string {
  const d = new Date(iso);
  const today = new Date();
  const local = new Date(today.getFullYear(), today.getMonth(), today.getDate());
  const deltaDays = Math.round(
    (new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime() - local.getTime()) / 86_400_000,
  );
  if (deltaDays === 0) return "Today";
  if (deltaDays === 1) return "Tomorrow";
  return d.toLocaleDateString(undefined, { weekday: "long", month: "short", day: "numeric" });
}

const STATUS_LABEL: Record<PortalAppointmentRequest["status"], string> = {
  pending: "Pending",
  approved: "Approved",
  declined: "Declined",
  cancelled: "Withdrawn",
};

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
  const [picked, setPicked] = useState<Slot | null>(null);
  const [title, setTitle] = useState("");
  const [notes, setNotes] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const range = useMemo(() => {
    const from = new Date().toISOString().slice(0, 10);
    const to = new Date(Date.now() + 13 * 24 * 3600 * 1000).toISOString().slice(0, 10);
    return { from, to };
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

  /** Flatten staff slots into day groups (chronological). */
  const days = useMemo(() => {
    if (slots === null) return null;
    const byDay = new Map<string, Slot[]>();
    for (const staff of slots) {
      for (const slot of staff.slots) {
        const key = slot.starts_at.slice(0, 10);
        const list = byDay.get(key) ?? [];
        list.push({
          userId: staff.user_id,
          name: staff.display_name,
          starts: slot.starts_at,
          ends: slot.ends_at,
        });
        byDay.set(key, list);
      }
    }
    const dayEntries = [...byDay.entries()]
      .map(([day, group]) => [day, group.sort((a, b) => (a.starts < b.starts ? -1 : 1))] as const)
      .sort(([a], [b]) => (a < b ? -1 : 1))
      .slice(0, 10);
    return dayEntries.map(([day, list]) => ({
      day,
      label: fmtDay(list[0]?.starts ?? day),
      slots: list,
    }));
  }, [slots]);

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
    <section className="m-rise">
      <div className="mb-5">
        <h1 className="text-xl font-semibold tracking-tight">Schedule a visit</h1>
        <p className="mt-0.5 text-sm" style={{ color: "var(--m-text-muted)" }}>
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
        <p className="mb-3 text-xs" style={{ color: "var(--m-text-secondary)" }} role="status">
          {notice}
        </p>
      )}

      {picked !== null && (
        <div data-testid="picked-slot-summary" className="m-rise mb-4 rounded-[0.75rem] border-2 p-4" style={{ borderColor: "var(--m-accent)", background: "var(--m-surface)" }}>
          <p className="text-sm font-medium">
            {fmtDay(picked.starts)}, {fmtTime(picked.starts)}–{fmtTime(picked.ends)} · with {picked.name}
          </p>
          <div className="mt-3 grid gap-3 sm:grid-cols-2">
            <div className="space-y-1.5">
              <Label htmlFor="appt-title">What is it about?</Label>
              <Input
                id="appt-title"
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                placeholder="Quarterly review, config change…"
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="appt-notes">Details (optional)</Label>
              <Input
                id="appt-notes"
                value={notes}
                onChange={(e) => setNotes(e.target.value)}
              />
            </div>
          </div>
          <div className="mt-3 flex gap-2">
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
      ) : slots.length === 0 || (days !== null && days.length === 0) ? (
        <EmptyState message="No published availability in the next two weeks — check back soon." />
      ) : (
        <div className="space-y-5">
          {(days ?? []).map((d) => (
            <div key={d.day}>
              <h2
                className="mb-2 text-xs font-semibold uppercase tracking-wide"
                style={{ color: "var(--m-text-muted)" }}
              >
                {d.label}
              </h2>
              <ul className="flex flex-wrap gap-2">
                {d.slots.map((slot) => {
                  const active =
                    picked !== null && picked.starts === slot.starts && picked.userId === slot.userId;
                  return (
                    <li key={`${slot.userId}-${slot.starts}`}>
                      <button
                        type="button"
                        onClick={() =>
                          setPicked({
                            userId: slot.userId,
                            name: slot.name,
                            starts: slot.starts,
                            ends: slot.ends,
                          })
                        }
                        aria-pressed={active}
                        aria-label={`${fmtDay(slot.starts)}, ${fmtTime(slot.starts)} with ${slot.name}`}
                        className="rounded-[0.75rem] border px-2.5 py-1.5 text-xs transition-colors duration-100 hover:bg-[var(--m-surface-hover)]"
                        style={{
                          borderColor: active ? "var(--m-accent)" : "var(--m-border)",
                          color: active ? "var(--m-accent)" : "var(--m-text-secondary)",
                          background: "var(--m-surface)",
                        }}
                      >
                        {fmtTime(slot.starts)}
                        <span
                          className="ml-1.5"
                          style={{ color: active ? "var(--m-accent)" : "var(--m-text-muted)" }}
                        >
                          {slot.name.split(" ")[0]}
                        </span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            </div>
          ))}
        </div>
      )}

      <h2 className="mb-2 mt-9 text-sm font-semibold">Your requests</h2>
      {requests === null ? (
        <div className="flex justify-center py-8">
          <Spinner aria-label="Loading requests" />
        </div>
      ) : requests.length === 0 ? (
        <EmptyState message="No appointment requests yet — pick a time above." />
      ) : (
        <ul
          className="overflow-hidden rounded-[0.75rem] border"
          style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
        >
          {requests.map((r, i) => (
            <li
              key={r.id}
              className="m-rise flex items-center justify-between gap-3 border-b px-3 py-3 last:border-b-0"
              style={{ borderColor: "var(--m-border)", animationDelay: `${Math.min(i * 30, 180)}ms` }}
            >
              <div className="min-w-0">
                <div className="text-sm font-medium">{r.title}</div>
                <div className="text-xs tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                  {fmtDay(r.starts_at)}, {fmtTime(r.starts_at)}
                  {r.decision_note !== null && ` · ${r.decision_note}`}
                </div>
              </div>
              <div className="flex shrink-0 items-center gap-2">
                <StatusTag tone={requestTone(r.status)}>{STATUS_LABEL[r.status]}</StatusTag>
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
