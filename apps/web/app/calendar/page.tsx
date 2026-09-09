// /calendar — personal/team week calendar (TASKLIST 2.12).
//
// Purpose: minimal scheduling surface over the Stage-2 data model: a
// Monday-start week grid (columns = days, rows = hours) rendering
// appointments and on-call blocks as bordered blocks, plus a
// "New appointment" dialog whose submit runs the conflict dry-run first
// so clashes surface before any write. Weeks hydrate once through the
// IF4 calendar read model (returning to a viewed week renders from cache
// with no refetch and no flash) and appointment.* events reconcile the
// open week as deltas (IF5).
//
// Non-purpose: no drag-and-drop rescheduling (dispatch-board scope), no
// availability editing UI, no iCal feeds. Blocks render in the VIEWER'S
// timezone; the API stays UTC everywhere.
"use client";

import { Button, Card, Dialog, DialogActions, DialogContent, FieldHint, Input, Label } from "@meridian/ui";
import { useEffect, useMemo, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";
import {
  calendarModelKey,
  makeCalendarSpec,
  type CalendarView,
} from "../../lib/instant/read-models";
import {
  useInstantChannels,
  useInstantCollection,
} from "../../lib/use-instant";

const HOUR_START = 7;
const HOUR_END = 20;
const ROW_PX = 44;

type CalItem = CalendarView["items"][number];

function mondayOf(d: Date): Date {
  const out = new Date(d);
  const shift = (out.getDay() + 6) % 7; // Mon=0..Sun=6
  out.setDate(out.getDate() - shift);
  out.setHours(0, 0, 0, 0);
  return out;
}

function isoDay(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

export default function CalendarPage() {
  const auth = useRequireAuth();
  const [weekStart, setWeekStart] = useState<Date>(() => mondayOf(new Date()));
  const [mode, setMode] = useState<"personal" | "team">("personal");
  const [error, setError] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);

  // IF4/IF5: one cached read model per scope+week. Visited weeks render
  // from cache instantly; appointment events reconcile the open week.
  const weekIso = isoDay(weekStart);
  const weekEnd = useMemo(() => {
    const to = new Date(weekStart);
    to.setDate(to.getDate() + 7);
    return to;
  }, [weekStart]);
  const calSpec = useMemo(
    () =>
      makeCalendarSpec(api.api, {
        mode,
        date: weekIso,
        from: weekStart.toISOString(),
        to: weekEnd.toISOString(),
      }),
    [mode, weekIso, weekStart, weekEnd],
  );
  const { data: view, loading, refresh } = useInstantCollection(
    calendarModelKey(mode === "team" ? "team" : (auth.me?.user_id ?? "personal"), weekIso),
    calSpec,
  );
  // Appointment events fan out on the org channel (route table: org first).
  useInstantChannels(auth.me ? [`org:${auth.me.org_id}`] : []);

  const days = useMemo(
    () =>
      Array.from({ length: 7 }, (_, i) => {
        const d = new Date(weekStart);
        d.setDate(d.getDate() + i);
        return d;
      }),
    [weekStart],
  );

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-4 p-4 md:p-6">
      <header className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
            Calendar
          </h1>
          <FieldHint>
            Week of {isoDay(weekStart)} · {mode === "personal" ? "your bookings" : "whole team"} ·
            rendered in your local timezone
          </FieldHint>
        </div>
        <div className="flex items-center gap-2">
          <Button variant={mode === "personal" ? "primary" : "secondary"} onClick={() => setMode("personal")}>
            Personal
          </Button>
          <Button variant={mode === "team" ? "primary" : "secondary"} onClick={() => setMode("team")}>
            Team
          </Button>
          <span className="mx-1 h-5 w-px" style={{ background: "var(--m-border)" }} />
          <Button variant="ghost" onClick={() => shiftWeek(-1)} aria-label="Previous week">
            ←
          </Button>
          <Button variant="ghost" onClick={() => setWeekStart(mondayOf(new Date()))}>
            This week
          </Button>
          <Button variant="ghost" onClick={() => shiftWeek(1)} aria-label="Next week">
            →
          </Button>
          {mode === "personal" && (
            <Button onClick={() => setDialogOpen(true)}>New appointment</Button>
          )}
        </div>
      </header>

      {error && (
        <span role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </span>
      )}

      <Card className="min-w-0 overflow-x-auto p-0">
        <div className="min-w-[840px]">
          {/* Day headers */}
          <div
            className="grid border-b"
            style={{ gridTemplateColumns: `56px repeat(7, 1fr)`, borderColor: "var(--m-border)" }}
          >
            <div />
            {days.map((d) => (
              <div key={d.toISOString()} className="px-2 py-2 text-center">
                <div className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                  {d.toLocaleDateString(undefined, { weekday: "short" })}
                </div>
                <div className="text-[13px] font-medium" style={{ color: "var(--m-text)" }}>
                  {d.getDate()}
                </div>
              </div>
            ))}
          </div>
          {/* Hour rows + absolute blocks */}
          <div className="grid" style={{ gridTemplateColumns: `56px repeat(7, 1fr)` }}>
            <div>
              {Array.from({ length: HOUR_END - HOUR_START }, (_, i) => (
                <div
                  key={i}
                  className="pr-2 text-right text-[10px]"
                  style={{
                    height: ROW_PX,
                    color: "var(--m-text-muted)",
                    transform: "translateY(-6px)",
                  }}
                >
                  {String(HOUR_START + i).padStart(2, "0")}:00
                </div>
              ))}
            </div>
            {days.map((d) => (
              <div
                key={d.toISOString()}
                className="relative"
                style={{ height: ROW_PX * (HOUR_END - HOUR_START) }}
              >
                {Array.from({ length: HOUR_END - HOUR_START }, (_, i) => (
                  <div
                    key={i}
                    className="border-t"
                    style={{ height: ROW_PX, borderColor: "var(--m-border)" }}
                  />
                ))}
                {(view?.items ?? [])
                  .filter((item) => sameLocalDay(item.starts_at, d))
                  .map((item) => (
                    <EventBlock key={item.id} item={item} day={d} />
                  ))}
              </div>
            ))}
          </div>
        </div>
      </Card>

      {loading && (
        <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
          Loading…
        </span>
      )}

      <NewAppointmentDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        defaultDate={isoDay(weekStart)}
        onCreated={() => void refresh()}
        onError={(m) => setError(m)}
      />
    </div>
  );

  function shiftWeek(delta: number) {
    const next = new Date(weekStart);
    next.setDate(next.getDate() + delta * 7);
    setWeekStart(next);
  }
}

function sameLocalDay(iso: string, day: Date): boolean {
  const d = new Date(iso);
  return (
    d.getFullYear() === day.getFullYear() &&
    d.getMonth() === day.getMonth() &&
    d.getDate() === day.getDate()
  );
}

function EventBlock({ item, day }: { item: CalItem; day: Date }) {
  const start = new Date(item.starts_at);
  const end = new Date(item.ends_at);
  const dayStart = new Date(day);
  dayStart.setHours(0, 0, 0, 0);
  const offsetMin = (start.getTime() - dayStart.getTime()) / 60000 - HOUR_START * 60;
  const durationMin = Math.max((end.getTime() - start.getTime()) / 60000, 15);
  if (offsetMin >= (HOUR_END - HOUR_START) * 60 || offsetMin + durationMin <= 0) return null;
  const isOnCall = item.item_type === "on_call";
  const top = Math.max(offsetMin, 0) * (ROW_PX / 60);
  const height = Math.min(durationMin, (HOUR_END - HOUR_START) * 60) * (ROW_PX / 60) - 2;
  return (
    <div
      role="figure"
      aria-label={`${isOnCall ? "On call" : "Appointment"}: ${item.title ?? item.category ?? ""}`}
      className="absolute inset-x-1 overflow-hidden px-1 py-0.5"
      style={{
        top,
        height,
        borderRadius: "var(--m-radius-control)",
        border: isOnCall ? "1px dashed var(--m-border-strong)" : "1px solid var(--m-border-strong)",
        borderLeft: isOnCall ? undefined : "3px solid var(--m-accent)",
        background: isOnCall ? "var(--m-status-neutral-bg)" : "var(--m-paper)",
        color: "var(--m-text)",
      }}
    >
      <div className="truncate text-[11px] font-medium leading-tight">
        {isOnCall ? `On call · ${item.category ?? ""}` : item.title}
      </div>
      {!isOnCall && (
        <div className="truncate text-[10px]" style={{ color: "var(--m-text-secondary)" }}>
          {start.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}–
          {end.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
          {item.kind === "on_site" ? " · on site" : ""}
        </div>
      )}
    </div>
  );
}

function NewAppointmentDialog({
  open,
  onOpenChange,
  defaultDate,
  onCreated,
  onError,
}: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
  defaultDate: string;
  onCreated: () => void;
  onError: (message: string | null) => void;
}) {
  const [title, setTitle] = useState("");
  const [date, setDate] = useState(defaultDate);
  const [startTime, setStartTime] = useState("09:00");
  const [endTime, setEndTime] = useState("10:00");
  const [kind, setKind] = useState("on_site");
  const [ticketId, setTicketId] = useState("");
  const [warning, setWarning] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => setDate(defaultDate), [defaultDate]);

  const submit = async () => {
    setSaving(true);
    setWarning(null);
    onError(null);
    try {
      const startsAt = new Date(`${date}T${startTime}:00`);
      const endsAt = new Date(`${date}T${endTime}:00`);
      if (!(startsAt < endsAt)) {
        setWarning("End must be after start.");
        return;
      }
      // Dry-run first: name clashes before anything is written.
      const me = await api.api.GET("/api/v1/me");
      const check = await api.api.POST("/api/v1/scheduling/conflict-check", {
        body: {
          user_id: (me.data as { user_id: string }).user_id,
          starts_at: startsAt.toISOString(),
          ends_at: endsAt.toISOString(),
        },
      });
      if (check.data?.has_conflict) {
        const names = check.data.conflicts.map((c) => c.title).join(", ");
        setWarning(`Conflicts with: ${names}. Pick another slot.`);
        return;
      }
      const created = await api.api.POST("/api/v1/appointments", {
        body: {
          title,
          starts_at: startsAt.toISOString(),
          ends_at: endsAt.toISOString(),
          kind: kind as "on_site" | "remote" | "internal" | "personal",
          ...(ticketId.trim() ? { ticket_id: ticketId.trim() } : {}),
        },
      });
      if (created.error) {
        const detail =
          (created.error as { error?: { message?: string } }).error?.message ?? "rejected";
        setWarning(detail);
        return;
      }
      onCreated();
      onOpenChange(false);
      setTitle("");
      setTicketId("");
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title="New appointment" description="Booked against your own calendar.">
        <div className="grid gap-3">
          <Label>Title</Label>
          <Input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Site visit" />
          <div className="grid grid-cols-3 gap-2">
            <div>
              <Label>Date</Label>
              <Input type="date" value={date} onChange={(e) => setDate(e.target.value)} />
            </div>
            <div>
              <Label>Start</Label>
              <Input type="time" value={startTime} onChange={(e) => setStartTime(e.target.value)} />
            </div>
            <div>
              <Label>End</Label>
              <Input type="time" value={endTime} onChange={(e) => setEndTime(e.target.value)} />
            </div>
          </div>
          <Label>Kind</Label>
          <select
            value={kind}
            onChange={(e) => setKind(e.target.value)}
            className="h-[var(--m-control-height)] rounded-[var(--m-radius-control)] border px-2 text-[13px]"
            style={{ borderColor: "var(--m-border)", background: "var(--m-surface)", color: "var(--m-text)" }}
          >
            <option value="on_site">On site</option>
            <option value="remote">Remote</option>
            <option value="internal">Internal</option>
            <option value="personal">Personal</option>
          </select>
          <Label>Ticket ID (optional)</Label>
          <Input value={ticketId} onChange={(e) => setTicketId(e.target.value)} placeholder="UUID" />
          {warning && (
            <span role="alert" className="text-[12px]" style={{ color: "var(--m-status-warning-fg)" }}>
              {warning}
            </span>
          )}
        </div>
        <DialogActions>
          <Button variant="secondary" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button disabled={!title.trim() || saving} onClick={() => void submit()}>
            {saving ? "Checking…" : "Create"}
          </Button>
        </DialogActions>
      </DialogContent>
    </Dialog>
  );
}
