// The surface pack's /today anatomy (brand/marula-mockups/marula-surface-pack/
// web/today/): the greeting header, the five attention cards, the "Your day"
// schedule with typed 56px rows, and yesterday's hatched gap bar with On call
// and Travel beside it. The grouped queue stays on the page below — the pack
// draws the day, the queue keeps the work reachable (anchors from the cards).
// Non-purpose: no dispatch-board mechanics; the quick-log dialog lives on the
// page and owns its own never-auto-submit rules.
"use client";

import type { components } from "@marula-core/sdk";
import { Button, Card, Spinner } from "@marula-core/ui";
import { Icon, type IconName } from "@marula-core/ui/kit";
import { useEffect, useMemo, useState } from "react";
import { api } from "../../lib/api";
import { fmtMins } from "../../lib/format";

type DispatchDay = components["schemas"]["DispatchDayView"];
type Tech = components["schemas"]["TechnicianView"];
type Appointment = DispatchDay["technicians"][number]["appointments"][number];
type TravelDraft = components["schemas"]["TravelDraftView"];
type TimeEntry = components["schemas"]["TimeEntry"];

export type DayRow = {
  key: string;
  kind: "done" | "now" | "free" | "travel" | "upcoming";
  start: Date | null;
  minutes: number | null;
  title: string;
  subtitle: string | null;
  ticketNumber: string | null;
  href: string | null;
};

/** Compose the pack's Your-day rows from the dispatch day: bookings in
 * order, free gaps of half an hour or more, and the running booking marked
 * "now" when the clock sits inside its window. */
export function dayRowsFrom(
  appointments: Appointment[],
  now: number,
): DayRow[] {
  const rows: DayRow[] = [];
  const sorted = [...appointments].sort((a, b) => a.starts_at.localeCompare(b.starts_at));
  const dayStart = new Date(now);
  dayStart.setHours(8, 0, 0, 0);
  let cursor = dayStart.getTime();
  for (const appt of sorted) {
    const start = Date.parse(appt.starts_at);
    const end = Date.parse(appt.ends_at);
    if (Number.isNaN(start) || Number.isNaN(end)) continue;
    if (start - cursor >= 30 * 60_000) {
      rows.push({
        key: `free-${start}`,
        kind: "free",
        start: new Date(cursor),
        minutes: Math.round((start - cursor) / 60_000),
        title: "Free",
        subtitle: null,
        ticketNumber: null,
        href: null,
      });
    }
    const running = start <= now && now < end;
    const done = appt.status === "done";
    const kind: DayRow["kind"] = done ? "done" : running ? "now" : "upcoming";
    rows.push({
      key: appt.id,
      kind,
      start: new Date(start),
      minutes: Math.round((end - start) / 60_000),
      title: appt.client_name ?? appt.title,
      subtitle:
        appt.client_name !== null && appt.client_name !== undefined
          ? appt.title
          : (appt.ticket_number ?? null),
      ticketNumber: appt.ticket_number ?? null,
      href: appt.ticket_id !== null && appt.ticket_id !== undefined ? `/tickets/${appt.ticket_id}` : null,
    });
    cursor = Math.max(cursor, end);
  }
  // Travel rows: appointments that carry a leg become hatched travel rows
  // ahead of their booking, labelled from the dispatch leg's own data.
  for (const appt of sorted) {
    if (appt.travel_minutes > 0) {
      const start = Date.parse(appt.starts_at);
      if (Number.isNaN(start)) continue;
      rows.push({
        key: `travel-${appt.id}`,
        kind: "travel",
        start: new Date(start - appt.travel_minutes * 60_000),
        minutes: appt.travel_minutes,
        title: `Travel to ${appt.client_name ?? "site"}`,
        subtitle:
          appt.travel_distance_meters !== null && appt.travel_distance_meters !== undefined && appt.travel_distance_meters > 0
            ? `${Math.round(appt.travel_distance_meters / 1000)} km · ${appt.travel_minutes} min`
            : `${appt.travel_minutes} min`,
        ticketNumber: null,
        href: null,
      });
    }
  }
  return rows.sort((a, b) => (a.start?.getTime() ?? 0) - (b.start?.getTime() ?? 0));
}

/** One 56px row of the Your-day card, drawn to the pack: time column with a
 * duration line, a quiet glyph, title over subtitle, and the right pill. */
export function DayRowView({ row, now }: { row: DayRow; now: number }) {
  const time =
    row.start !== null
      ? row.start.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false })
      : "";
  const duration = row.minutes !== null ? row.minutes >= 60 ? `${Math.floor(row.minutes / 60)}h` : `${row.minutes}m` : null;
  const glyph: Record<DayRow["kind"], IconName> = {
    done: "check",
    now: "play",
    free: "pause",
    travel: "car",
    upcoming: "clock",
  };
  const pill = (() => {
    switch (row.kind) {
      case "done":
        return (
          <span
            className="rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide"
            style={{ background: "var(--m-status-success-bg)", color: "var(--m-status-success-fg)" }}
          >
            Done
          </span>
        );
      case "now":
        return (
          <span
            className="rounded-full px-2 py-0.5 text-[10px] font-semibold tabular-nums"
            style={{ background: "var(--m-accent)", color: "var(--m-accent-fg, var(--m-text))" }}
          >
            Running
          </span>
        );
      case "free":
        return (
          <span className="text-[10px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
            Until {row.start !== null ? endLabel(row.start, row.minutes ?? 0) : ""}
          </span>
        );
      case "travel":
        return (
          <span
            className="rounded-full border border-dashed px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide"
            style={{ borderColor: "var(--m-status-warning-fg)", color: "var(--m-status-warning-fg)" }}
          >
            Draft
          </span>
        );
      default:
        return null;
    }
  })();
  const isNow = row.kind === "now";
  const body = (
    <>
      <span className="grid w-14 shrink-0 gap-0.5">
        <span className="tabular-nums text-xs font-medium" style={{ color: "var(--m-text)" }}>
          {time}
        </span>
        {duration !== null && (
          <span className="tabular-nums text-[10px]" style={{ color: "var(--m-text-muted)" }}>
            {duration}
          </span>
        )}
      </span>
      <span
        className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full"
        style={{
          background: isNow ? "var(--m-accent)" : "var(--m-surface-2)",
          color: isNow ? "var(--m-text)" : "var(--m-text-secondary)",
        }}
        aria-hidden="true"
      >
        <Icon name={glyph[row.kind]} size={13} />
      </span>
      <span className="grid min-w-0 flex-1 gap-0">
        <span className="flex min-w-0 items-center gap-2">
          <span className="truncate text-[13px] font-medium" style={{ color: "var(--m-text)" }}>
            {row.ticketNumber !== null && <span className="mr-1 text-xs tabular-nums" style={{ color: "var(--m-text-muted)" }}>{row.ticketNumber}</span>}
            {row.title}
          </span>
        </span>
        {row.subtitle !== null && row.subtitle !== row.title && (
          <span className="truncate text-xs" style={{ color: "var(--m-text-muted)" }}>
            {row.subtitle}
          </span>
        )}
      </span>
      <span className="ml-auto flex shrink-0 items-center">{pill}</span>
    </>
  );
  return (
    <div
      className="row-hover flex min-h-14 items-center gap-3 px-2"
      style={
        isNow
          ? { background: "var(--m-surface-2)", boxShadow: `inset 2px 0 0 var(--m-accent)` }
          : row.kind === "travel"
            ? { background: "var(--m-bg-subtle)" }
            : undefined
      }
    >
      {row.href !== null ? (
        <a href={row.href} className="flex min-w-0 flex-1 items-center gap-3 outline-offset-2">
          {body}
        </a>
      ) : (
        body
      )}
    </div>
  );
}

function endLabel(start: Date, minutes: number): string {
  return new Date(start.getTime() + minutes * 60_000).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

/** The pack's two-column day body. Left: Your day. Right: Yesterday's
 * hatched gap bar, On call, Travel. Everything degrades to quiet notes
 * when the dispatch rota is not available to the account. */
export function YourDayPanel({ now }: { now: number }) {
  const [day, setDay] = useState<DispatchDay | null>(null);
  const [state, setState] = useState<"loading" | "ready" | "unavailable">("loading");

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const res = await api.api.GET("/api/v1/dispatch/day", {
          params: { query: { date: new Date(now).toISOString().slice(0, 10) } },
        });
        if (cancelled) return;
        if (res.data === undefined) {
          setState("unavailable");
          return;
        }
        setDay(res.data);
        setState("ready");
      } catch {
        if (!cancelled) setState("unavailable");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [now]);

  if (state === "loading") {
    return (
      <Card className="m-rise flex items-center gap-2 p-3">
        <Spinner label="Loading today’s rota" />
      </Card>
    );
  }
  if (state === "unavailable" || day === null) {
    return (
      <Card className="m-rise grid gap-1 p-3">
        <h2 className="text-[10px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
          Your day
        </h2>
        <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
          The rota is not available to your account. Dispatch holds the rota and its permissions.
        </p>
      </Card>
    );
  }

  const techs: Tech[] = Array.isArray(day.technicians) ? day.technicians : [];
  const me = techs.length === 1 ? techs[0] : techs.find((t) => t.user_id !== undefined);
  const appointments = me !== undefined ? (me.appointments ?? []).filter((a) => a.status !== "cancelled") : [];
  const rows = dayRowsFrom(appointments, now);

  const body =
    rows.length === 0 ? (
      <QuietDayEmpty />
    ) : (
      <ol role="list" className="flex flex-col">
        {rows.map((row) => (
          <li key={row.key}>
            <DayRowView row={row} now={now} />
          </li>
        ))}
      </ol>
    );

  return (
    <div className="grid gap-4 lg:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
      <Card className="m-rise grid gap-1 p-3">
        <div className="flex items-baseline justify-between gap-2">
          <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
            Your day
          </h2>
          <span className="text-[11px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
            {new Date(now).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" })} · now{" "}
            {new Date(now).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false })}
          </span>
        </div>
        {body}
      </Card>
      <div className="grid content-start gap-4">
        <YesterdayCard now={now} />
        <OnCallCard techs={techs} now={now} />
        <TravelCard now={now} />
      </div>
    </div>
  );
}

/** The pack's quiet-day empty state: nothing booked, nothing due. */
function QuietDayEmpty() {
  return (
    <div className="flex flex-col items-center gap-2 px-4 py-8 text-center">
      <span
        className="flex h-14 w-14 items-center justify-center rounded-full"
        style={{ border: "1.5px dashed var(--m-border)", color: "var(--m-status-warning-fg)" }}
        aria-hidden="true"
      >
        <Icon name="pause" size={22} />
      </span>
      <p className="text-sm font-medium" style={{ color: "var(--m-text)" }}>
        Nothing on today
      </p>
      <p className="max-w-xs text-xs" style={{ color: "var(--m-text-muted)" }}>
        No visits, no calls, no tickets due. The queue below still shows everything assigned to you.
      </p>
    </div>
  );
}

/** Yesterday at a glance: logged vs expected, a hatched gap bar, and the
 * log-time button. Entry windows come from the time-entries list; the bar
 * hatches whatever the clock expected but nobody logged. */
export function YesterdayCard({ now }: { now: number }) {
  const yesterday = new Date(now - 86_400_000);
  const dayIso = yesterday.toISOString().slice(0, 10);
  const [day, setDay] = useState<components["schemas"]["GapDayView"] | null>(null);
  const [entries, setEntries] = useState<TimeEntry[] | null>(null);
  const onLog = typeof window !== "undefined" ? () => {
    window.dispatchEvent(new CustomEvent("marula:open-quick-log"));
  } : undefined;

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const since = `${dayIso}T00:00:00Z`;
        const [gaps, ents] = await Promise.all([
          api.api.GET("/api/v1/time-gaps/me"),
          api.api.GET("/api/v1/time-entries", {
            params: { query: { since, limit: 100 } },
          }),
        ]);
        if (cancelled) return;
        const gapDay = (gaps.data?.days ?? []).find((d) => d.day === dayIso) ?? null;
        setDay(gapDay);
        setEntries(
          (ents.data ?? []).filter((e) => e.started_at.slice(0, 10) === dayIso),
        );
      } catch {
        if (!cancelled) {
          setDay(null);
          setEntries(null);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [dayIso]);

  const complete = day !== null && !day.has_gap;
  const expected = day?.expected_minutes ?? 480;
  const logged = day?.logged_minutes ?? 0;

  return (
    <Card className="m-rise grid gap-2 p-3" aria-label="Missing time">
      <div className="flex items-baseline justify-between gap-2">
        <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
          Yesterday
        </h2>
        {day !== null &&
          (complete ? (
            <span
              className="rounded-full px-2 py-0.5 text-[10px] font-semibold"
              style={{ background: "var(--m-status-success-bg)", color: "var(--m-status-success-fg)" }}
            >
              Complete
            </span>
          ) : (
            <span
              className="rounded-full px-2 py-0.5 text-[10px] font-semibold tabular-nums"
              style={{ background: "var(--m-status-warning-bg)", color: "var(--m-status-warning-fg)" }}
            >
              {fmtMins(day.unaccounted_minutes)} not logged
            </span>
          ))}
      </div>
      {day === null ? (
        <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
          No time expected for {yesterday.toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" })}.
        </p>
      ) : (
        <>
          <p className="text-xs tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
            {fmtMins(logged)} logged of {fmtMins(expected)}
            {!complete && (
              <span style={{ color: "var(--m-status-warning-fg)" }}> — {fmtMins(day.unaccounted_minutes)} unaccounted</span>
            )}
          </p>
          <GapBar dayIso={dayIso} expected={expected} logged={logged} entries={entries ?? []} />
          <Button size="sm" onClick={onLog}>
            Log time
          </Button>
        </>
      )}
    </Card>
  );
}

/** The pack's hatched gap bar: logged windows as solid stone, the day's gap
 * hatched in the warning tint, lunch and before/after as quiet ground, with
 * a sparse time axis underneath. */
function GapBar({
  dayIso,
  expected,
  logged,
  entries,
}: {
  dayIso: string;
  expected: number;
  logged: number;
  entries: TimeEntry[];
}) {
  const startHour = 8;
  const endHour = 17;
  const span = (endHour - startHour) * 60;
  const pct = (mins: number) => `${Math.min(100, Math.max(0, (mins / span) * 100))}%`;
  const segments: { left: string; width: string; tone: "entry" }[] = [];
  for (const e of entries) {
    const s = new Date(e.started_at);
    const en = new Date(e.ended_at);
    if (Number.isNaN(s.getTime()) || Number.isNaN(en.getTime())) continue;
    const from = (s.getHours() - startHour) * 60 + s.getMinutes();
    const to = (en.getHours() - startHour) * 60 + en.getMinutes();
    if (to <= 0 || from >= span) continue;
    segments.push({ left: pct(Math.max(0, from)), width: pct(Math.min(span, to) - Math.max(0, from)), tone: "entry" });
  }
  const axis = [8, 10, 12, 14, 17];
  return (
    <div className="grid gap-1">
      <div
        className="relative h-3 overflow-hidden rounded-[var(--m-radius-container)]"
        style={{
          background: `repeating-linear-gradient(135deg, var(--m-status-warning-bg) 0 4px, transparent 4px 8px)`,
          border: "1px solid var(--m-border)",
        }}
        role="img"
        aria-label={`${fmtMins(logged)} logged of ${fmtMins(expected)} expected`}
      >
        {segments.map((s, i) => (
          <span
            key={i}
            className="absolute inset-y-0"
            style={{ left: s.left, width: s.width, background: "var(--m-meter-fill)" }}
          />
        ))}
      </div>
      <div className="relative h-3" aria-hidden="true">
        {axis.map((h) => (
          <span
            key={h}
            className="absolute text-[9px] tabular-nums"
            style={{ left: `${((h - startHour) / (endHour - startHour)) * 100}%`, transform: "translateX(-50%)", color: "var(--m-text-muted)" }}
          >
            {String(h).padStart(2, "0")}:00
          </span>
        ))}
      </div>
      <span className="sr-only">{dayIso}</span>
    </div>
  );
}

/** Who holds the pager: tonight's rotation with the active holder marked. */
function OnCallCard({ techs, now }: { techs: Tech[]; now: number }) {
  const blocks = useMemo(
    () =>
      techs
        .flatMap((tech) => (tech.on_call ?? []).map((block) => ({ tech, block })))
        .sort((a, b) => a.block.starts_at.localeCompare(b.block.starts_at)),
    [techs],
  );
  const live = blocks.filter(
    ({ block }) => Date.parse(block.starts_at) <= now && now < Date.parse(block.ends_at),
  );
  const tonight = blocks.find(({ block }) => new Date(block.starts_at).toDateString() === new Date(now).toDateString());
  return (
    <Card className="m-rise grid gap-1.5 p-3">
      <div className="flex items-baseline justify-between gap-2">
        <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
          On call
        </h2>
        {tonight !== undefined && (
          <span
            className="rounded-full px-2 py-0.5 text-[10px] font-semibold"
            style={{ background: "var(--m-accent)", color: "var(--m-text)" }}
          >
            Tonight
          </span>
        )}
      </div>
      {blocks.length === 0 ? (
        <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
          Nobody is holding a rotation right now.
        </p>
      ) : (
        <ul role="list" className="flex flex-col">
          {blocks.slice(0, 4).map(({ tech, block }) => {
            const isActive = live.some((l) => l.block.id === block.id);
            const initials = tech.display_name
              .split(/\s+/)
              .slice(0, 2)
              .map((w) => w[0]?.toUpperCase() ?? "")
              .join("");
            return (
              <li key={block.id} className="flex min-h-9 items-center gap-2">
                <span
                  className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-[10px] font-semibold"
                  style={{ background: "var(--m-surface-2)", color: "var(--m-text-secondary)" }}
                  aria-hidden="true"
                >
                  {initials}
                </span>
                <span className="min-w-0 flex-1 truncate text-xs" style={{ color: "var(--m-text)" }}>
                  {tech.display_name}
                </span>
                <span className="text-[11px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                  {windowLabel(block.starts_at, block.ends_at)}
                </span>
                {isActive && (
                  <span
                    className="rounded-full px-2 py-0.5 text-[10px] font-semibold"
                    style={{ background: "var(--m-accent)", color: "var(--m-text)" }}
                  >
                    On call
                  </span>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </Card>
  );
}

/** The day's travel drafts with a review link; quiet when there are none. */
function TravelCard({ now }: { now: number }) {
  const [drafts, setDrafts] = useState<TravelDraft[] | null>(null);
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const res = await api.api.GET("/api/v1/dispatch/travel-drafts");
        if (!cancelled) setDrafts(Array.isArray(res.data) ? res.data : []);
      } catch {
        if (!cancelled) setDrafts([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [now]);
  return (
    <Card className="m-rise grid gap-1.5 p-3">
      <div className="flex items-baseline justify-between gap-2">
        <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
          Travel
        </h2>
        {drafts !== null && drafts.length > 0 && (
          <span
            className="rounded-full px-2 py-0.5 text-[10px] font-semibold"
            style={{ background: "var(--m-accent)", color: "var(--m-text)" }}
          >
            {drafts.length} draft{drafts.length === 1 ? "" : "s"}
          </span>
        )}
      </div>
      {drafts === null ? (
        <Spinner label="Loading trips" />
      ) : drafts.length === 0 ? (
        <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
          No trips today.
        </p>
      ) : (
        <ul role="list" className="flex flex-col">
          {drafts.slice(0, 3).map((draft) => (
            <li key={draft.id} className="flex min-h-9 items-center gap-2">
              <span
                className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full"
                style={{ background: "var(--m-surface-2)", color: "var(--m-text-secondary)" }}
                aria-hidden="true"
              >
                <Icon name="car" size={13} />
              </span>
              <span className="grid min-w-0 flex-1">
                <span className="truncate text-xs font-medium" style={{ color: "var(--m-text)" }}>
                  {draftLabel(draft)}
                </span>
                <span className="truncate text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                  Ready when you arrive.
                </span>
              </span>
              <a
                href="/dispatch/travel"
                className="rounded-full px-2 py-1 text-[11px] font-medium hover:underline"
                style={{ border: "1px solid var(--m-border)", color: "var(--m-text)" }}
              >
                Review
              </a>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}

function draftLabel(draft: TravelDraft): string {
  const start = new Date(draft.started_at);
  const end = new Date(draft.ended_at);
  const when = Number.isNaN(start.getTime())
    ? null
    : `${start.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false })}–${Number.isNaN(end.getTime()) ? "" : end.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false })}`;
  const distance =
    draft.distance > 0
      ? `${Math.round(draft.distance)} ${draft.distance_unit === "mi" ? "mi" : "km"}`
      : null;
  const parts = [when, distance].filter((p): p is string => p !== null);
  return parts.length > 0 ? parts.join(" · ") : "Trip";
}

function windowLabel(startsAt: string, endsAt: string): string {
  const opts: Intl.DateTimeFormatOptions = { hour: "2-digit", minute: "2-digit", hour12: false };
  const start = new Date(startsAt);
  const end = new Date(endsAt);
  if (Number.isNaN(start.getTime()) || Number.isNaN(end.getTime())) return "";
  const sameDay = start.toDateString() === end.toDateString();
  const from = start.toLocaleTimeString([], opts);
  const to = end.toLocaleTimeString([], opts);
  return sameDay ? `${from}–${to}` : `${from}–${end.toLocaleDateString(undefined, { day: "numeric", month: "short" })} ${to}`;
}
