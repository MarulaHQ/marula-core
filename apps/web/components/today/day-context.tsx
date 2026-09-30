// Today's on-call and schedule, for /today.
//
// Purpose: the two facts a technician needs before they decide what to pick up
// — who is holding the pager, and what is already booked today. Both come from
// ONE call (`GET /api/v1/dispatch/day`), because `TechnicianView` carries
// `appointments` and `on_call` side by side, and asking twice would let the two
// cards disagree about the same day.
//
// Degradation is deliberate: the dispatch endpoints are dispatch-scoped, so a
// technician without that scope gets two cards that say so, not a Today page
// that failed. Nothing here blocks the queue above it — the panel loads after
// mount and reports its own state.
//
// Non-purpose: no writes. Claiming a shift or moving an appointment happens in
// dispatch; this is the read that tells you whether you need to go there.
"use client";

import { Badge, Card, Spinner } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useEffect, useMemo, useState } from "react";
import { api } from "../../lib/api";

type Day = components["schemas"]["DispatchDayView"];
type Tech = components["schemas"]["TechnicianView"];

type PanelState = "loading" | "ready" | "unavailable";

export function TodayDayContext(): React.JSX.Element {
  const [day, setDay] = useState<Day | null>(null);
  const [state, setState] = useState<PanelState>("loading");

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const res = await api.api.GET("/api/v1/dispatch/day", {
          params: { query: { date: new Date().toISOString().slice(0, 10) } },
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
  }, []);

  if (state === "loading") {
    return (
      <Card>
        <div className="flex items-center gap-2 p-3">
          <Spinner label="Loading today’s rota" />
        </div>
      </Card>
    );
  }

  if (state === "unavailable" || day === null) {
    return (
      <Card>
        <div className="grid gap-1 p-3">
          <h2 className="m-day-context-title">On call &amp; schedule</h2>
          <p className="m-day-context-note">
            The rota is not available to your account. Dispatch holds the rota and
            its permissions.
          </p>
        </div>
      </Card>
    );
  }

  const techs: Tech[] = Array.isArray(day.technicians) ? day.technicians : [];
  return (
    <div className="grid gap-3 md:grid-cols-2">
      <OnCallCard techs={techs} />
      <ScheduleCard techs={techs} />
    </div>
  );
}

function OnCallCard({ techs }: { techs: Tech[] }): React.JSX.Element {
  const now = Date.now();
  // A block is "now" when the clock is inside its window. Half-open on purpose:
  // a shift ending at 09:00 and the next starting at 09:00 must not both claim
  // the instant, or the pager shows two people at handover.
  const live = useMemo(
    () =>
      techs
        .flatMap((tech) =>
          (tech.on_call ?? []).map((block) => ({ tech, block })),
        )
        .filter(({ block }) => Date.parse(block.starts_at) <= now && now < Date.parse(block.ends_at))
        .sort((a, b) => a.block.category.localeCompare(b.block.category)),
    [techs, now],
  );

  return (
    <Card>
      <div className="grid gap-1.5 p-3">
        <h2 className="m-day-context-title">
          On call now
          {live.length > 0 ? <span className="m-day-context-count">{live.length}</span> : null}
        </h2>
        {live.length === 0 ? (
          <p className="m-day-context-note">Nobody is holding a rotation right now.</p>
        ) : (
          <ul className="m-day-list">
            {live.map(({ tech, block }) => (
              <li className="m-day-row" key={block.id}>
                <span className="m-day-name">{tech.display_name}</span>
                <Badge tone="due">{block.category}</Badge>
                <span className="m-day-meta">{windowLabel(block.starts_at, block.ends_at)}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </Card>
  );
}

function ScheduleCard({ techs }: { techs: Tech[] }): React.JSX.Element {
  const appointments = useMemo(
    () =>
      techs
        .flatMap((tech) => (tech.appointments ?? []).map((appt) => ({ tech, appt })))
        .sort((a, b) => a.appt.starts_at.localeCompare(b.appt.starts_at)),
    [techs],
  );

  return (
    <Card>
      <div className="grid gap-1.5 p-3">
        <h2 className="m-day-context-title">
          Today&rsquo;s schedule
          {appointments.length > 0 ? (
            <span className="m-day-context-count">{appointments.length}</span>
          ) : null}
        </h2>
        {appointments.length === 0 ? (
          <p className="m-day-context-note">Nothing is booked for today.</p>
        ) : (
          <ul className="m-day-list">
            {appointments.map(({ tech, appt }) => (
              <li className="m-day-row" key={appt.id}>
                <span className="m-day-when font-mono tabular-nums">
                  {new Date(appt.starts_at).toLocaleTimeString([], {
                    hour: "2-digit",
                    minute: "2-digit",
                    hour12: false,
                  })}
                </span>
                <span className="m-day-name" title={appt.title}>
                  {appt.title}
                </span>
                {/* Only a status that is not the default `scheduled` earns a
                    badge: a column of "scheduled" chips says nothing, and the
                    exceptions are the rows worth noticing. */}
                {appt.status !== "scheduled" ? <Badge tone="neutral">{appt.status}</Badge> : null}
                <span className="m-day-meta">{tech.display_name}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </Card>
  );
}

/** "09:00–17:00", or a day-prefixed form when a block crosses midnight. */
function windowLabel(startsAt: string, endsAt: string): string {
  const opts: Intl.DateTimeFormatOptions = { hour: "2-digit", minute: "2-digit", hour12: false };
  const start = new Date(startsAt);
  const end = new Date(endsAt);
  const sameDay = start.toDateString() === end.toDateString();
  const from = start.toLocaleTimeString([], opts);
  const to = end.toLocaleTimeString([], opts);
  return sameDay ? `${from}–${to}` : `${from}–${end.toLocaleDateString()} ${to}`;
}
