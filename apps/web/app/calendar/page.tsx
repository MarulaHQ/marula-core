// /calendar — personal/team scheduling (Wave 17 lane 4 UX pass).
//
// Purpose: three presentations over the IF4 calendar read model —
// week time-grid, month grid, agenda list — with every entry class
// differentiated (appointment, booking-with-ticket, on call, synced
// busy time, time off, ticket deadline) and overlaps flagged in place,
// consistent with the sync panel's overlap counters. Appointments drag
// to reschedule (15-minute snap) or reschedule via dialog — both ride
// POST /appointments/{id}/move, whose 409 keeps the atomic conflict
// rejection. Ticket deadlines come from the org-wide deadline read
// model (GET /calendar/deadlines, server-windowed; assignee-scoped in
// personal mode) — documented scope, never invented data.
// Non-purpose: no availability editing, no dispatch-board moves (that
// surface owns its own board), the sync panel owns connection admin.
"use client";

import { friendlyMutationText } from "../../lib/error-copy";
import { Button, Card, Dialog, DialogActions, DialogContent, FieldHint, Input, Label, Spinner, Tag } from "@meridian/ui";
import CalendarSyncPanel from "../../components/calendar-sync-panel";
import { useEffect, useMemo, useState, type CSSProperties } from "react";
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
import {
  assignLanes,
  detectOverlaps,
  entryFromExclusion,
  entryFromItem,
  entryFromTicketDeadline,
  entriesOnDay,
  isoDay,
  mergeEntries,
  mondayOf,
  monthWeeks,
  sameLocalDay,
  snapMinutes,
  type CalendarEntry,
  type EntryKind,
} from "../../lib/calendar-view";

const HOUR_START = 7;
const HOUR_END = 20;
const ROW_PX = 44;

type CalItem = CalendarView["items"][number];
type CalendarExclusion = CalendarView["exclusions"][number];
type ViewMode = "week" | "month" | "agenda";

const NULL_SPEC = {
  hydrate: async () => null,
  relevant: () => false,
  revalidate: async () => null,
  entityIds: () => [],
};

/** One month-grid week slot; null weeks stay inert without moving hooks. */
function useMonthSlot(
  scope: string,
  mode: "personal" | "team",
  weekStart: Date | null,
): CalendarView | null {
  const spec = useMemo(
    () =>
      weekStart === null
        ? NULL_SPEC
        : makeCalendarSpec(api.api, {
            mode,
            date: isoDay(weekStart),
            from: weekStart.toISOString(),
            to: new Date(weekStart.getTime() + 7 * 86_400_000).toISOString(),
          }),
    [mode, weekStart],
  );
  const key =
    weekStart === null ? `calendar:idle:${scope}` : calendarModelKey(scope, isoDay(weekStart));
  const { data } = useInstantCollection(key, spec);
  return data ?? null;
}

const CLASS_LABEL: Record<EntryKind, string> = {
  appointment: "Appointment",
  on_call: "On call",
  synced_busy: "Synced busy",
  time_off: "Time off",
  ticket_deadline: "Deadline",
};

function localInputValue(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}

function labelForRange(view: ViewMode, anchor: Date): string {
  if (view === "month") {
    return anchor.toLocaleDateString(undefined, { month: "long", year: "numeric" });
  }
  const start = mondayOf(anchor);
  const end = new Date(start);
  end.setDate(end.getDate() + 6);
  return `Week of ${isoDay(start)}`;
}

export default function CalendarPage() {
  const auth = useRequireAuth();
  const [anchor, setAnchor] = useState<Date>(() => mondayOf(new Date()));
  const [viewMode, setViewMode] = useState<ViewMode>("week");
  const [mode, setMode] = useState<"personal" | "team">("personal");
  const [error, setError] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [details, setDetails] = useState<CalendarEntry | null>(null);

  // IF4/IF5: one cached read model per scope+week. Visited weeks render
  // from cache instantly; appointment events reconcile the open week.
  const weekIso = isoDay(anchor);
  const weekEnd = useMemo(() => {
    const to = new Date(anchor);
    to.setDate(to.getDate() + 7);
    return to;
  }, [anchor]);
  const calSpec = useMemo(
    () =>
      makeCalendarSpec(api.api, {
        mode,
        date: weekIso,
        from: anchor.toISOString(),
        to: weekEnd.toISOString(),
      }),
    [mode, weekIso, anchor, weekEnd],
  );
  const { data: view, loading, refresh } = useInstantCollection(
    calendarModelKey(mode === "team" ? "team" : (auth.me?.user_id ?? "personal"), weekIso),
    calSpec,
  );
  useInstantChannels(auth.me ? [`org:${auth.me.org_id}`] : []);

  // Month view: fixed six week slots sharing the same read-model keys, so
  // every covered week hydrates and reconciles through the same store.
  // Hooks stay in a fixed order: six explicit slot calls, null-padded.
  const monthWeekRows = useMemo(
    () => monthWeeks(anchor.getFullYear(), anchor.getMonth()),
    [anchor],
  );
  const scope = mode === "team" ? "team" : (auth.me?.user_id ?? "personal");
  const slot0 = useMonthSlot(scope, mode, monthWeekRows[0]?.[0] ?? null);
  const slot1 = useMonthSlot(scope, mode, monthWeekRows[1]?.[0] ?? null);
  const slot2 = useMonthSlot(scope, mode, monthWeekRows[2]?.[0] ?? null);
  const slot3 = useMonthSlot(scope, mode, monthWeekRows[3]?.[0] ?? null);
  const slot4 = useMonthSlot(scope, mode, monthWeekRows[4]?.[0] ?? null);
  const slot5 = useMonthSlot(scope, mode, monthWeekRows[5]?.[0] ?? null);
  const monthSlotViews = [slot0, slot1, slot2, slot3, slot4, slot5];

  // Ticket deadlines overlay: the org-wide deadline read model
  // (GET /calendar/deadlines, server-filtered to the visible window,
  // capped at 62 days) — org-wide in team mode, assignee-scoped in
  // personal mode (same endpoint, assignee_id param keeps the personal
  // scope identical to the old bounded ticket fetch).
  const [deadlines, setDeadlines] = useState<CalendarEntry[]>([]);
  useEffect(() => {
    if (auth.status !== "authed") return;
    let cancelled = false;
    const rangeStart = viewMode === "month"
      ? new Date(anchor.getFullYear(), anchor.getMonth(), 1)
      : new Date(anchor);
    const rangeEnd = viewMode === "month"
      ? new Date(anchor.getFullYear(), anchor.getMonth() + 1, 1)
      : weekEnd;
    void api.api
      .GET("/api/v1/calendar/deadlines", {
        params: {
          query: {
            from: rangeStart.toISOString(),
            to: rangeEnd.toISOString(),
            assignee_id: (mode === "personal" && auth.me) ? auth.me.user_id : null,
          },
        },
      })
      .then((res) => {
        if (cancelled || res.error !== undefined || res.data === undefined) return;
        setDeadlines(
          res.data.deadlines
            .map((d) =>
              entryFromTicketDeadline({
                id: d.ticket_id,
                number: d.number,
                title: d.title,
                due_at: d.due_at,
              }),
            )
            .filter((e): e is CalendarEntry => e !== null),
        );
      });
    return () => {
      cancelled = true;
    };
  }, [auth.status, auth.me, mode, viewMode, anchor, weekEnd]);

  const entries = useMemo(() => {
    const sources: (CalendarView | null)[] =
      viewMode === "month" ? [...monthSlotViews] : [view ?? null];
    const items: CalItem[] = [];
    const exclusions: CalendarExclusion[] = [];
    for (const source of sources) {
      if (source === null) continue;
      items.push(...source.items);
      exclusions.push(...source.exclusions);
    }
    return mergeEntries([
      ...items.map(entryFromItem),
      ...exclusions.map(entryFromExclusion),
      ...deadlines,
    ]);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view, monthSlotViews, deadlines]);

  const days = useMemo(
    () =>
      Array.from({ length: 7 }, (_, i) => {
        const d = new Date(anchor);
        d.setDate(d.getDate() + i);
        return d;
      }),
    [anchor],
  );

  const shift = (delta: number) => {
    const next = new Date(anchor);
    if (viewMode === "month") {
      next.setMonth(next.getMonth() + delta, 1);
    } else {
      next.setDate(next.getDate() + delta * 7);
    }
    setAnchor(mondayOf(next));
  };

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-4 p-4 md:p-6">
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
            Calendar
          </h1>
          <FieldHint>
            {labelForRange(viewMode, anchor)} · {mode === "personal" ? "your bookings" : "whole team"} ·
            rendered in your local timezone
          </FieldHint>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button variant={mode === "personal" ? "primary" : "secondary"} onClick={() => setMode("personal")}>
            Personal
          </Button>
          <Button variant={mode === "team" ? "primary" : "secondary"} onClick={() => setMode("team")}>
            Team
          </Button>
          <span className="mx-1 h-5 w-px" style={{ background: "var(--m-border)" }} />
          <div
            className="flex overflow-hidden rounded-[var(--m-radius-control)] border"
            style={{ borderColor: "var(--m-border)" }}
            role="group"
            aria-label="Calendar view"
          >
            {([["week", "Week"], ["month", "Month"], ["agenda", "Agenda"]] as [ViewMode, string][]).map(
              ([v, label]) => (
              <button
                key={v}
                type="button"
                onClick={() => setViewMode(v)}
                aria-pressed={viewMode === v}
                className="px-3 py-1.5 text-[13px] font-medium transition-colors"
                style={
                  viewMode === v
                    ? { background: "var(--m-accent-soft)", color: "var(--m-accent)" }
                    : { color: "var(--m-text-secondary)" }
                }
              >
                {label}
              </button>
              ),
            )}
          </div>
          <span className="mx-1 h-5 w-px" style={{ background: "var(--m-border)" }} />
          <Button variant="ghost" onClick={() => shift(-1)} aria-label="Previous">
            ←
          </Button>
          <Button
            variant="ghost"
            onClick={() => setAnchor(mondayOf(new Date()))}
          >
            Today
          </Button>
          <Button variant="ghost" onClick={() => shift(1)} aria-label="Next">
            →
          </Button>
          {mode === "personal" && (
            <Button onClick={() => setDialogOpen(true)}>New appointment</Button>
          )}
        </div>
      </header>

      <Legend />

      {error && (
        <span role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </span>
      )}

      <CalendarSyncPanel entries={viewMode === "month" ? undefined : entries} />

      {viewMode === "week" ? (
        <WeekGrid
          days={days}
          entries={entries}
          loading={loading}
          meId={auth.me?.user_id ?? null}
          personal={mode === "personal"}
          onOpenDetails={setDetails}
          onDropMove={handleDropMove}
          weekStart={anchor}
        />
      ) : viewMode === "month" ? (
        <MonthGrid
          weeks={monthWeekRows}
          monthIndex={anchor.getMonth()}
          entries={entries}
          onOpenWeek={(weekStart) => {
            setAnchor(mondayOf(weekStart));
            setViewMode("week");
          }}
        />
      ) : (
        <AgendaList days={days} entries={entries} onOpenDetails={setDetails} />
      )}

      {loading && viewMode !== "month" && (
        <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
          Loading…
        </span>
      )}

      <EventDetailsDialog
        entry={details}
        onClose={() => setDetails(null)}
        onError={(m) => setError(m)}
        onMoved={() => {
          void refresh();
        }}
      />

      <NewAppointmentDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        defaultDate={isoDay(anchor)}
        onCreated={() => void refresh()}
        onError={(m) => setError(m)}
      />
    </div>
  );

  async function handleDropMove(entryId: string, day: Date, offsetY: number) {
    const entry = entries.find((e) => e.id === entryId);
    if (entry === undefined || entry.kind !== "appointment") return;
    const minutes = Math.round(offsetY / ROW_PX * 60) + HOUR_START * 60;
    const snapped = snapMinutes(new Date(day.getFullYear(), day.getMonth(), day.getDate(), 0, minutes));
    const durationMin =
      (new Date(entry.endsAt).getTime() - new Date(entry.startsAt).getTime()) / 60_000;
    const newEnd = new Date(snapped.getTime() + durationMin * 60_000);
    await moveAppointment(entry, snapped, newEnd, setError, () => void refresh());
  }
}

async function moveAppointment(
  entry: CalendarEntry,
  startsAt: Date,
  endsAt: Date,
  onError: (m: string | null) => void,
  onMoved: () => void,
): Promise<boolean> {
  onError(null);
  const res = await api.api.POST("/api/v1/appointments/{id}/move", {
    params: { path: { id: entry.id } },
    body: { starts_at: startsAt.toISOString(), ends_at: endsAt.toISOString() },
  });
  if (res.response.status === 409) {
    onError("That slot is already booked — the move was rejected and the appointment is unchanged.");
    return false;
  }
  if (res.data === undefined) {
    onError(friendlyMutationText(new Error("move rejected")));
    return false;
  }
  onMoved();
  return true;
}

function Legend() {
  const chips: { kind: EntryKind; swatch: CSSProperties }[] = [
    { kind: "appointment", swatch: { background: "var(--m-accent-soft)", borderLeft: "3px solid var(--m-accent)" } },
    { kind: "on_call", swatch: { background: "var(--m-status-neutral-bg)", border: "1px dashed var(--m-border-strong)" } },
    { kind: "synced_busy", swatch: { background: "var(--m-bg-subtle)", border: "1px dashed var(--m-accent-muted)" } },
    { kind: "time_off", swatch: { background: "var(--m-bg-subtle)", border: "1px solid var(--m-border)" } },
    { kind: "ticket_deadline", swatch: { background: "var(--m-status-warning-bg)", border: "1px solid var(--m-status-warning-fg)" } },
  ];
  return (
    <div className="flex flex-wrap items-center gap-2" aria-label="Entry legend">
      {chips.map(({ kind, swatch }) => (
        <span
          key={kind}
          className="inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[11px] font-semibold"
          style={{ ...swatch, color: kind === "ticket_deadline" ? "var(--m-status-warning-fg)" : "var(--m-text-secondary)" }}
        >
          {kind === "ticket_deadline" ? "◆" : null} {CLASS_LABEL[kind]}
        </span>
      ))}
    </div>
  );
}

function TodayBadge({ day }: { day: Date }) {
  const today = new Date();
  const isToday = sameLocalDay(today.toISOString(), day);
  if (!isToday) return null;
  return (
    <span
      className="ml-1 inline-block rounded-full px-1.5 text-[10px] font-bold"
      style={{ background: "var(--m-accent)", color: "var(--m-on-accent)" }}
    >
      today
    </span>
  );
}

function WeekGrid({
  days,
  entries,
  loading,
  meId,
  personal,
  onOpenDetails,
  onDropMove,
  weekStart,
}: {
  days: Date[];
  entries: CalendarEntry[];
  loading: boolean;
  meId: string | null;
  personal: boolean;
  onOpenDetails: (entry: CalendarEntry) => void;
  onDropMove: (entryId: string, day: Date, offsetY: number) => void;
  weekStart: Date;
}) {
  const [dragId, setDragId] = useState<string | null>(null);
  const now = useNowMinute();
  const todayIso = isoDay(new Date());
  return (
    <Card className="min-w-0 overflow-x-auto p-0" data-testid="week-grid">
      <div className="min-w-[840px]">
        <div
          className="grid border-b"
          style={{ gridTemplateColumns: `56px repeat(7, 1fr)`, borderColor: "var(--m-border)" }}
        >
          <div />
          {days.map((d) => {
            const dayEntries = entriesOnDay(entries, d);
            const overlapCount = detectOverlaps(dayEntries).size / 2;
            return (
              <div key={d.toISOString()} className="px-2 py-2 text-center">
                <div className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                  {d.toLocaleDateString(undefined, { weekday: "short" })}
                </div>
                <div className="text-[13px] font-medium" style={{ color: "var(--m-text)" }}>
                  {d.getDate()}
                  <TodayBadge day={d} />
                </div>
                {overlapCount > 0 ? (
                  <span
                    className="mt-0.5 inline-block rounded-full px-1.5 text-[10px] font-bold"
                    style={{ background: "var(--m-status-warning-bg)", color: "var(--m-status-warning-fg)" }}
                    data-testid={`day-overlaps-${isoDay(d)}`}
                  >
                    {overlapCount} overlap{overlapCount === 1 ? "" : "s"}
                  </span>
                ) : null}
              </div>
            );
          })}
        </div>
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
          {days.map((d) => {
            const isToday = isoDay(d) === todayIso;
            const minutesFromDayStart =
              (new Date(d).setHours(0, 0, 0, 0) === new Date(now).setHours(0, 0, 0, 0)
                ? now.getHours() * 60 + now.getMinutes() - HOUR_START * 60
                : null);
            return (
              <div
                key={d.toISOString()}
                className="relative"
                style={{
                  height: ROW_PX * (HOUR_END - HOUR_START),
                  background: isToday ? "var(--m-accent-soft)" : undefined,
                }}
                onDragOver={(e) => {
                  if (dragId !== null) e.preventDefault();
                }}
                onDrop={(e) => {
                  if (dragId === null) return;
                  const rect = e.currentTarget.getBoundingClientRect();
                  onDropMove(dragId, d, e.clientY - rect.top);
                  setDragId(null);
                }}
              >
                {Array.from({ length: HOUR_END - HOUR_START }, (_, i) => (
                  <div
                    key={i}
                    className="border-t"
                    style={{ height: ROW_PX, borderColor: "var(--m-border)" }}
                  />
                ))}
                {minutesFromDayStart !== null && minutesFromDayStart >= 0 && minutesFromDayStart < (HOUR_END - HOUR_START) * 60 ? (
                  <div
                    aria-hidden="true"
                    className="pointer-events-none absolute inset-x-0"
                    style={{ top: minutesFromDayStart * (ROW_PX / 60) }}
                  >
                    <div style={{ height: "1px", background: "var(--m-accent)", opacity: 0.7 }} />
                  </div>
                ) : null}
                {assignLanes(entriesOnDay(entries, d)).map(({ entry, lane, laneCount }) => (
                  <WeekBlock
                    key={entry.id}
                    entry={entry}
                    day={d}
                    dayEntries={entriesOnDay(entries, d)}
                    lane={lane}
                    laneCount={laneCount}
                    draggable={personal && entry.kind === "appointment" && meId !== null && (entry.userId === null || entry.userId === meId)}
                    onDragStart={() => setDragId(entry.id)}
                    onDragEnd={() => setDragId(null)}
                    dragging={dragId === entry.id}
                    onOpen={() => onOpenDetails(entry)}
                  />
                ))}
              </div>
            );
          })}
        </div>
      </div>
      {loading && (
        <div className="flex items-center gap-2 p-2 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
          <Spinner /> Loading week…
        </div>
      )}
      <input type="hidden" data-testid="week-start" value={isoDay(weekStart)} />
    </Card>
  );
}

function useNowMinute(): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const t = setInterval(() => setNow(new Date()), 60_000);
    return () => clearInterval(t);
  }, []);
  return now;
}

function classVisual(entry: CalendarEntry, flagged: boolean): CSSProperties {
  const base: CSSProperties = {
    borderRadius: "var(--m-radius-control)",
    color: "var(--m-text)",
  };
  switch (entry.kind) {
    case "on_call":
      return {
        ...base,
        border: "1px dashed var(--m-border-strong)",
        background: "var(--m-status-neutral-bg)",
      };
    case "synced_busy":
      return {
        ...base,
        border: flagged ? "1px solid var(--m-status-warning-fg)" : "1px dashed var(--m-accent-muted)",
        background: "var(--m-bg-subtle)",
      };
    case "time_off":
      return {
        ...base,
        border: "1px solid var(--m-border)",
        background: "var(--m-bg-subtle)",
        opacity: 0.75,
      };
    case "ticket_deadline":
      return {
        ...base,
        border: "1px solid var(--m-status-warning-fg)",
        background: "var(--m-status-warning-bg)",
        color: "var(--m-status-warning-fg)",
      };
    default:
      return {
        ...base,
        border: flagged ? "1px solid var(--m-status-warning-fg)" : "1px solid var(--m-border-strong)",
        borderLeft: "3px solid var(--m-accent)",
        background: "var(--m-surface)",
      };
  }
}

function WeekBlock({
  entry,
  day,
  dayEntries,
  lane,
  laneCount,
  draggable,
  dragging,
  onDragStart,
  onDragEnd,
  onOpen,
}: {
  entry: CalendarEntry;
  day: Date;
  dayEntries: CalendarEntry[];
  lane: number;
  laneCount: number;
  draggable: boolean;
  dragging: boolean;
  onDragStart: () => void;
  onDragEnd: () => void;
  onOpen: () => void;
}) {
  const start = new Date(entry.startsAt);
  const end = new Date(entry.endsAt);
  const dayStart = new Date(day);
  dayStart.setHours(0, 0, 0, 0);
  const offsetMin = (start.getTime() - dayStart.getTime()) / 60000 - HOUR_START * 60;
  const isInstant = entry.kind === "ticket_deadline";
  const durationMin = isInstant
    ? 24
    : Math.max((end.getTime() - start.getTime()) / 60000, 15);
  if (offsetMin >= (HOUR_END - HOUR_START) * 60 || offsetMin + durationMin <= 0) return null;
  const top = Math.max(offsetMin, 0) * (ROW_PX / 60);
  const height = Math.min(durationMin, (HOUR_END - HOUR_START) * 60) * (ROW_PX / 60) - 2;
  const overlaps = detectOverlaps(dayEntries);
  const flagged = overlaps.has(entry.id);
  const timeLabel = isInstant
    ? `due ${start.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`
    : `${start.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}–${end.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
  const laneWidth = 100 / laneCount;
  const classPrefix =
    entry.kind === "synced_busy" && !entry.title.startsWith("Synced")
      ? "Synced · "
      : entry.kind === "time_off" && !entry.title.toLowerCase().startsWith("time off")
        ? "Time off · "
        : "";
  return (
    <button
      type="button"
      role="figure"
      aria-label={`${CLASS_LABEL[entry.kind]}: ${entry.title}, ${timeLabel}${flagged ? ", overlaps another entry" : ""}${draggable ? ", draggable to reschedule" : ""}`}
      draggable={draggable}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onClick={onOpen}
      data-testid={`week-block-${entry.id}`}
      className={`absolute overflow-hidden px-1 py-0.5 text-left transition-transform duration-100 ${draggable ? "active:cursor-grabbing" : ""}`}
      style={{
        top,
        height,
        left: `calc(${lane * laneWidth}% + 2px)`,
        width: `calc(${laneWidth}% - 4px)`,
        ...classVisual(entry, flagged),
        opacity: dragging ? 0.6 : undefined,
        transform: dragging ? "scale(0.99)" : undefined,
      }}
    >
      <div className="truncate text-[11px] font-medium leading-tight">
        {entry.kind === "ticket_deadline" ? "◆ " : ""}
        {classPrefix}
        {entry.kind === "on_call" ? `On call · ${entry.title}` : entry.title}
      </div>
      <div className="truncate text-[10px]" style={{ color: entry.kind === "ticket_deadline" ? "var(--m-status-warning-fg)" : "var(--m-text-secondary)" }}>
        {timeLabel}
        {entry.kind === "appointment" && entry.itemKind === "on_site" ? " · on site" : ""}
        {entry.kind === "appointment" && entry.ticketId !== null ? " · ticket" : ""}
        {flagged ? " · overlaps" : ""}
      </div>
    </button>
  );
}

function MonthGrid({
  weeks,
  monthIndex,
  entries,
  onOpenWeek,
}: {
  weeks: Date[][];
  monthIndex: number;
  entries: CalendarEntry[];
  onOpenWeek: (weekStart: Date) => void;
}) {
  const todayIso = isoDay(new Date());
  return (
    <Card className="min-w-0 overflow-x-auto p-0" data-testid="month-grid">
      <div className="min-w-[640px]">
        <div className="grid border-b" style={{ gridTemplateColumns: "repeat(7, 1fr)", borderColor: "var(--m-border)" }}>
          {["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].map((d) => (
            <div key={d} className="px-2 py-1.5 text-center text-[11px] font-semibold" style={{ color: "var(--m-text-muted)" }}>
              {d}
            </div>
          ))}
        </div>
        {weeks.map((week, wi) => {
          const monday = week[0];
          if (monday === undefined) return null;
          return (
          <div
            key={monday.toISOString()}
            className="m-rise grid border-b last:border-b-0"
            style={{ gridTemplateColumns: "repeat(7, 1fr)", borderColor: "var(--m-border)", animationDelay: `${Math.min(wi * 40, 160)}ms` }}
          >
            {week.map((d) => {
              const inMonth = d.getMonth() === monthIndex;
              const dayEntries = entriesOnDay(entries, d);
              const visible = dayEntries.slice(0, 3);
              const hidden = dayEntries.length - visible.length;
              return (
                <button
                  key={d.toISOString()}
                  type="button"
                  onClick={() => onOpenWeek(d)}
                  className="min-h-[84px] p-1.5 text-left align-top transition-colors"
                  style={{
                    background: inMonth ? undefined : "var(--m-bg-subtle)",
                    borderRight: "1px solid var(--m-border)",
                    opacity: inMonth ? 1 : 0.6,
                  }}
                  aria-label={`${d.toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" })}, ${dayEntries.length} entries — open week`}
                >
                  <div
                    className="mb-1 inline-flex items-center gap-1 text-[11px] font-semibold tabular-nums"
                    style={{
                      color: isoDay(d) === todayIso ? "var(--m-accent)" : "var(--m-text-secondary)",
                    }}
                  >
                    {d.getDate()}
                  </div>
                  <div className="grid gap-0.5">
                    {visible.map((entry) => (
                      <span
                        key={entry.id}
                        className="truncate rounded px-1 text-[10px]"
                        style={{
                          ...classVisual(entry, false),
                          borderLeftWidth: entry.kind === "appointment" ? "3px" : undefined,
                        }}
                      >
                        {entry.kind === "ticket_deadline" ? "◆ " : ""}
                        {entry.title}
                      </span>
                    ))}
                    {hidden > 0 ? (
                      <span className="text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                        +{hidden} more
                      </span>
                    ) : null}
                  </div>
                </button>
              );
            })}
          </div>
          );
        })}
      </div>
    </Card>
  );
}

function AgendaList({
  days,
  entries,
  onOpenDetails,
}: {
  days: Date[];
  entries: CalendarEntry[];
  onOpenDetails: (entry: CalendarEntry) => void;
}) {
  return (
    <Card className="p-0" data-testid="agenda-list">
      {days.map((day, di) => {
        const dayEntries = entriesOnDay(entries, day);
        const overlaps = detectOverlaps(dayEntries);
        return (
          <div
            key={day.toISOString()}
            className="m-rise border-b last:border-b-0"
            style={{ borderColor: "var(--m-border)", animationDelay: `${Math.min(di * 40, 200)}ms` }}
          >
            <div
              className="flex items-center gap-2 px-4 py-2 text-[12px] font-semibold"
              style={{ background: "var(--m-bg-subtle)", color: "var(--m-text-secondary)" }}
            >
              {day.toLocaleDateString(undefined, { weekday: "long", month: "short", day: "numeric" })}
              <TodayBadge day={day} />
              <span className="ml-auto font-normal tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                {dayEntries.length === 0 ? "nothing scheduled" : `${dayEntries.length} entr${dayEntries.length === 1 ? "y" : "ies"}`}
              </span>
            </div>
            {dayEntries.map((entry) => {
              const start = new Date(entry.startsAt);
              const end = new Date(entry.endsAt);
              const flagged = overlaps.has(entry.id);
              return (
                <button
                  key={entry.id}
                  type="button"
                  onClick={() => onOpenDetails(entry)}
                  className="row-hover flex w-full items-center gap-3 px-4 py-2 text-left transition-colors"
                  data-testid={`agenda-entry-${entry.id}`}
                >
                  <span className="w-28 shrink-0 text-[12px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                    {entry.kind === "ticket_deadline"
                      ? `due ${start.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`
                      : `${start.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}–${end.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`}
                  </span>
                  <Tag
                    tone={
                      entry.kind === "ticket_deadline"
                        ? "warning"
                        : entry.kind === "on_call"
                          ? "neutral"
                          : entry.kind === "synced_busy"
                            ? "outline"
                            : "neutral"
                    }
                  >
                    {CLASS_LABEL[entry.kind]}
                  </Tag>
                  <span className="min-w-0 truncate text-[13px]">{entry.title}</span>
                  {flagged ? (
                    <span className="ml-auto shrink-0 text-[11px] font-semibold" style={{ color: "var(--m-status-warning-fg)" }}>
                      overlaps
                    </span>
                  ) : null}
                </button>
              );
            })}
          </div>
        );
      })}
    </Card>
  );
}

function EventDetailsDialog({
  entry,
  onClose,
  onError,
  onMoved,
}: {
  entry: CalendarEntry | null;
  onClose: () => void;
  onError: (m: string | null) => void;
  onMoved: () => void;
}) {
  const [moving, setMoving] = useState(false);
  const [date, setDate] = useState("");
  const [start, setStart] = useState("");
  const [end, setEnd] = useState("");
  const [saving, setSaving] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);

  useEffect(() => {
    if (entry === null || entry.kind !== "appointment") {
      setMoving(false);
      return;
    }
    const s = new Date(entry.startsAt);
    const e = new Date(entry.endsAt);
    setDate(localInputValue(s).slice(0, 10));
    setStart(localInputValue(s).slice(11));
    setEnd(localInputValue(e).slice(11));
    setMoving(false);
    setLocalError(null);
  }, [entry]);

  if (entry === null) return null;
  const s = new Date(entry.startsAt);
  const e = new Date(entry.endsAt);

  const submitMove = async () => {
    if (entry.kind !== "appointment") return;
    const newStart = new Date(`${date}T${start}`);
    const newEnd = new Date(`${date}T${end}`);
    if (!(newStart < newEnd)) {
      setLocalError("End must be after start.");
      return;
    }
    setSaving(true);
    const ok = await moveAppointment(entry, newStart, newEnd, setLocalError, onMoved);
    setSaving(false);
    if (ok) onClose();
  };

  return (
    <Dialog
      open={entry !== null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent title={entry.title} description={CLASS_LABEL[entry.kind]}>
        <div className="grid gap-3 p-4">
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            {s.toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" })} ·{" "}
            {s.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}–
            {e.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
            {entry.kind === "appointment" && entry.itemKind != null ? ` · ${entry.itemKind.replaceAll("_", " ")}` : ""}
          </p>
          {entry.number !== null ? (
            <p className="text-[13px] tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
              Ticket {entry.number}
            </p>
          ) : null}
          {entry.kind === "synced_busy" ? (
            <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              Imported from a connected external calendar — manage the connection in the sync panel
              above. This blocks booking time but cannot be moved here.
            </p>
          ) : null}
          {entry.kind === "ticket_deadline" ? (
            <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              Ticket deadline — resolve or reschedule on the ticket itself.
            </p>
          ) : null}
          {entry.kind === "appointment" ? (
            moving ? (
              <div className="grid gap-2 rounded-[var(--m-radius-control)] border p-3" style={{ borderColor: "var(--m-border)" }}>
                <Label htmlFor="move-date">Reschedule</Label>
                <Input id="move-date" type="date" value={date} onChange={(ev) => setDate(ev.target.value)} />
                <div className="grid grid-cols-2 gap-2">
                  <Input aria-label="Start time" type="time" value={start} onChange={(ev) => setStart(ev.target.value)} />
                  <Input aria-label="End time" type="time" value={end} onChange={(ev) => setEnd(ev.target.value)} />
                </div>
                <FieldHint>Moves are re-validated: a conflicting slot is rejected atomically.</FieldHint>
              </div>
            ) : (
              <DialogActions>
                <Button variant="secondary" onClick={() => setMoving(true)}>
                  Reschedule
                </Button>
              </DialogActions>
            )
          ) : null}
          {localError !== null ? (
            <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {localError}
            </p>
          ) : null}
          <DialogActions>
            {moving ? (
              <>
                <Button variant="ghost" onClick={() => setMoving(false)}>
                  Back
                </Button>
                <Button onClick={() => void submitMove()} disabled={saving}>
                  {saving ? <Spinner /> : null} Move
                </Button>
              </>
            ) : (
              <Button variant="secondary" onClick={onClose}>
                Close
              </Button>
            )}
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
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
      onError(friendlyMutationText(e));
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
