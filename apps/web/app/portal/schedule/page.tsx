// /portal/schedule — confirmed visits, and a way to ask for another.
//
// Purpose: the pack's state 09. "Coming up" holds the visits that are actually
// booked, each with a green Confirmed pill, because a person said yes. Under
// it, "Ask for a visit": a day, a time, and a reason.
//
// IT IS A REQUEST, AND THE SCREEN SAYS SO. The pack's own line: "This is a
// request. A person confirms it, so you're never left guessing." A time
// somebody is already busy is struck through and disabled with the reason
// written underneath, so the state is a word and a strike, not a grey. When
// the request goes through, what comes back is "Requested. We'll confirm within
// the hour" and what she sees next — never a booking the screen made on its
// own.
//
// WHY REMOTE KIND: the pinned create operation takes a `kind`, and every slot
// the portal exposes is a remote session slot. "On site" is a booking the
// service desk schedules after a request, not a thing this screen may promise.
"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { friendlyMutationText } from "../../../lib/error-copy";
import { Spinner } from "@marula-core/ui";
import {
  portalApi,
  useRequirePortalAuth,
  type PortalAppointmentRequest,
  type PortalStaffSlots,
} from "../../../lib/portal-api";
import { clockTime, longDate, timeRange } from "../../../lib/portal-copy";
import {
  ConfirmedPill,
  Field,
  PortalCard,
  PortalIcon,
  RequestedPill,
} from "../../../components/portal/portal-ui";

type Slot = { userId: string; name: string; starts: string; ends: string };

function dayLabel(iso: string): string {
  const d = new Date(iso);
  const today = new Date();
  const local = new Date(today.getFullYear(), today.getMonth(), today.getDate());
  const delta = Math.round(
    (new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime() - local.getTime()) / 86_400_000,
  );
  if (delta === 0) return "Today";
  if (delta === 1) return "Tomorrow";
  return d.toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" });
}

/** The day-chip label: "Wed 30 Sep", or "Today"/"Tomorrow" when that is
 *  nearer than the date. */
function dayChip(iso: string): string {
  const label = dayLabel(iso);
  if (label === "Today" || label === "Tomorrow") return label;
  return new Date(iso).toLocaleDateString(undefined, {
    weekday: "short",
    day: "numeric",
    month: "short",
  });
}

const REASONS = [
  "Something is broken",
  "A new starter or a leaver",
  "Moving or setting up equipment",
  "Something else",
] as const;

export default function PortalSchedulePage() {
  useRequirePortalAuth();
  const range = useMemo(() => {
    const from = new Date().toISOString().slice(0, 10);
    const to = new Date(Date.now() + 13 * 24 * 3600 * 1000).toISOString().slice(0, 10);
    return { from, to };
  }, []);

  const [slots, setSlots] = useState<PortalStaffSlots[] | null>(null);
  const [requests, setRequests] = useState<PortalAppointmentRequest[] | null>(null);
  const [picked, setPicked] = useState<Slot | null>(null);
  // The day is chosen first and separately from the time, so the time row only
  // ever offers the hours that are actually open on the day in hand.
  const [pickedDay, setPickedDay] = useState<string | null>(null);
  const [reason, setReason] = useState<string>(REASONS[0]);
  const [title, setTitle] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [requested, setRequested] = useState<string | null>(null);

  const load = useCallback(() => {
    let cancelled = false;
    setError(null);
    portalApi
      .scheduleSlots(range.from, range.to)
      .then((list) => {
        if (!cancelled) setSlots(list);
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(friendlyMutationText(err, "We couldn't load availability."));
      });
    portalApi
      .listAppointmentRequests()
      .then((list) => {
        if (!cancelled) setRequests(list);
      })
      .catch(() => {
        if (!cancelled) setRequests([]);
      });
    return () => {
      cancelled = true;
    };
  }, [range]);

  // Load on mount, and again after a request is sent, so the list the client
  // reads back is the server's answer rather than a local guess.
  useEffect(() => {
    return load();
  }, [load]);

  /** Flatten the staff slots into day groups, chronological. */
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
    return [...byDay.entries()]
      .map(([day, group]) => [day, group.sort((a, b) => (a.starts < b.starts ? -1 : 1))] as const)
      .sort(([a], [b]) => (a < b ? -1 : 1))
      .slice(0, 6)
      .map(([day, list]) => ({
        day,
        label: dayLabel(list[0]?.starts ?? day),
        // The chip label the rail of days shows: "Wed 30 Sep", or
        // "Today"/"Tomorrow" when that is nearer than the date.
        chip: dayChip(list[0]?.starts ?? day),
        slots: list,
      }));
  }, [slots]);

  const confirmed = useMemo(
    () => (requests === null ? [] : requests.filter((r) => r.status === "approved")),
    [requests],
  );
  const waiting = useMemo(
    () => (requests === null ? [] : requests.filter((r) => r.status === "pending")),
    [requests],
  );

  async function submit() {
    if (picked === null || busy) return;
    setBusy(true);
    setError(null);
    try {
      await portalApi.createAppointmentRequest({
        user_id: picked.userId,
        // Derived from the reason, with the optional description appended when
        // she typed one. The API requires a title; the pack does not ask her to
        // write one.
        title: title.trim() === "" ? reason : `${reason} — ${title.trim()}`,
        starts_at: picked.starts,
        ends_at: picked.ends,
        kind: "remote",
      });
      setRequested(`${picked.name} · ${longDate(picked.starts)} at ${clockTime(picked.starts)}`);
      setPicked(null);
      setTitle("");
    } catch (err) {
      setError(friendlyMutationText(err, "We couldn't send that request. Nothing was booked."));
    } finally {
      setBusy(false);
    }
  }

  if (slots === null || days === null || requests === null) {
    return (
      <div style={{ display: "flex", justifyContent: "center", padding: "64px 0" }}>
        <Spinner aria-label="Loading your schedule" />
      </div>
    );
  }

  return (
    <>
      <div>
        <h1 className="portal-title">Schedule</h1>
        <p className="portal-lead">
          Visits your service desk has booked with you, and a way to ask for another.
        </p>
      </div>

      {requested !== null && (
        <div className="portal-next" role="status" data-testid="portal-schedule-requested">
          <b className="portal-next__lead">Requested. We&apos;ll confirm within the hour.</b>
          <span className="portal-hint" style={{ fontSize: 14 }}>
            {requested}. You&apos;ll get an email, and it will appear here once it is confirmed.
          </span>
        </div>
      )}

      <PortalCard title="Coming up" testId="portal-schedule-confirmed">
        {confirmed.length === 0 ? (
          <div className="portal-empty" style={{ padding: "32px 24px" }}>
            <p className="portal-hint">
              Nothing booked. Ask for a time below and a person will confirm it.
            </p>
          </div>
        ) : (
          confirmed.map((r) => (
            <div className="portal-row" key={r.id} style={{ gridTemplateColumns: "70px minmax(0, 1fr) auto" }}>
              <span className="portal-row__title">
                <b style={{ fontSize: 20, lineHeight: 1 }}>
                  {new Date(r.starts_at).getDate()}
                </b>
                <span>
                  {new Date(r.starts_at).toLocaleDateString(undefined, { month: "short" })}
                </span>
              </span>
              <span className="portal-row__title">
                <b>{r.title}</b>
                <span>
                  {timeRange(r.starts_at, r.ends_at)}
                  {r.notes !== null && r.notes !== "" ? ` · ${r.notes}` : ""}
                </span>
              </span>
              <ConfirmedPill />
            </div>
          ))
        )}
      </PortalCard>

      {waiting.length > 0 && (
        <PortalCard title="Requested, waiting for a person" testId="portal-schedule-pending">
          {waiting.map((r) => (
            <div className="portal-row" key={r.id} style={{ gridTemplateColumns: "minmax(0, 1fr) auto" }}>
              <span className="portal-row__title">
                <b>{r.title}</b>
                <span>
                  {dayLabel(r.starts_at)}, {timeRange(r.starts_at, r.ends_at)}
                </span>
              </span>
              <RequestedPill />
            </div>
          ))}
        </PortalCard>
      )}

      <PortalCard title="Ask for a visit" testId="portal-schedule-ask">
        <div
          className="portal-card__body"
          style={{ display: "flex", flexDirection: "column", gap: 16 }}
        >
          <Field label="What is it about?" htmlFor="portal-visit-reason">
            <select
              id="portal-visit-reason"
              className="portal-select"
              value={reason}
              onChange={(e) => setReason(e.target.value)}
            >
              {REASONS.map((r) => (
                <option key={r}>{r}</option>
              ))}
            </select>
          </Field>

          <div className="portal-field" style={{ gap: 8 }}>
            <span className="portal-field__label">Which day?</span>
            {days.length === 0 ? (
              <p className="portal-hint" style={{ margin: 0 }}>
                No times are open in the next two weeks. Contact your service desk and they will
                find you a slot.
              </p>
            ) : (
              /* The pack picks the DAY from chips first, then the TIME for that
                 day (portal/sand/states/09-schedule.png). Times grouped under
                 day headings read as a longer form and make it harder to see
                 which days are open at all. */
              <div className="portal-choices" role="radiogroup" aria-label="Which day?">
                {days.map((d) => (
                  <button
                    key={d.day}
                    type="button"
                    role="radio"
                    aria-checked={pickedDay === d.day}
                    className="portal-choice"
                    onClick={() => setPickedDay(d.day)}
                    data-testid={`portal-day-${d.day}`}
                  >
                    {d.chip}
                  </button>
                ))}
              </div>
            )}
          </div>

          {pickedDay !== null && (
            <div className="portal-field" style={{ gap: 8 }}>
              <span className="portal-field__label">What time?</span>
              <div className="portal-choices" role="radiogroup" aria-label="What time?">
                {(() => {
                  const daySlots = days.find((d) => d.day === pickedDay)?.slots ?? [];
                  // Two people can hold the SAME clock hour (the seed's
                  // Diego and Tessa both open 09:00Z). A bare time would
                  // draw two identical buttons, and the client would be
                  // booking "whoever" — so when a time is shared the button
                  // says who it is with, first name only.
                  const timeCount = new Map<string, number>();
                  for (const s of daySlots) {
                    const label = clockTime(s.starts);
                    timeCount.set(label, (timeCount.get(label) ?? 0) + 1);
                  }
                  return daySlots.map((s) => {
                    const label = clockTime(s.starts);
                    const shared = (timeCount.get(label) ?? 0) > 1;
                    const who = shared ? ` · ${s.name.split(" ")[0]}` : "";
                    return (
                      <button
                        key={`${s.starts}-${s.userId}`}
                        type="button"
                        role="radio"
                        aria-checked={picked?.starts === s.starts && picked.userId === s.userId}
                        aria-label={`${label}${shared ? ` with ${s.name.split(" ")[0]}` : ""}`}
                        className="portal-choice"
                        onClick={() => setPicked(s)}
                        data-testid={`portal-slot-${s.starts}-${s.userId}`}
                      >
                        {label}
                        {who}
                      </button>
                    );
                  });
                })()}
              </div>
            </div>
          )}

          {/* The pack shows no free-text box here: the reason select and the
              day/time carry the request, and the primary is never blocked by
              typing. `createAppointmentRequest` still requires a `title`, so
              the title is DERIVED from the reason, and this description stays
              optional for the case where a sentence is genuinely needed. */}
          <Field
            label="Anything else we should know?"
            htmlFor="portal-visit-title"
            hint="Optional. Who it affects, or what they need."
          >
            <input
              id="portal-visit-title"
              className="portal-input"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="The new starter needs a laptop before Monday"
            />
          </Field>

          {error !== null && (
            <p className="portal-field__error" role="alert">
              <PortalIcon name="alert" size={14} />
              <span>{error}</span>
            </p>
          )}

          {/* The reassurance sits BESIDE the primary, on one line, the way the
              pack sets it — not as a note block underneath. */}
          <div style={{ display: "flex", flexWrap: "wrap", gap: 14, alignItems: "center" }}>
            <button
              type="button"
              className="portal-btn"
              disabled={busy || picked === null}
              onClick={() => void submit()}
              data-testid="portal-schedule-request"
            >
              {busy ? "Sending…" : "Request this time"}
            </button>
            <span className="portal-hint">
              This is a request. A person confirms it, so you&apos;re never left guessing.
            </span>
          </div>
        </div>
      </PortalCard>
    </>
  );
}
