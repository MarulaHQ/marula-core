// /dispatch/planning — the seven-day capacity horizon, rebuilt to the Marula
// surface pack (Wave 41 lane 6).
//
// Purpose: the planner's view past today's board — a seven-day capacity
// column chart where demand stacks by priority against roster hours, the
// gap callout per day, the skill catalogue with capacity/demand/gap, and the
// read-only dispatch-connector surface. §B1 fixes the anatomy: demand stacks
// P1 bottom, P2 middle, P3+P4 on top, capacity as a dashed line, over-capacity
// days in blush, a skeleton while the week computes.
//
// Non-purpose: the board owns placement; this page forecasts and reports. The
// Wave 34 claim panels (committed project load, phase allocations, work-next
// queues) sit above the horizon because the question they answer changes what
// you do with the horizon's answer.
"use client";

import "../dispatch.css";
import { EmptyState, Skeleton, Spinner, formatDayKey } from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";

import { dur } from "../../../lib/dispatch-model";
import { api, useRequireAuth } from "../../../lib/api";
import { CapacityForecastPanel } from "./capacity-forecast-panel";
import { DispatchFooter } from "../dispatch-footer";
import { PhaseAllocationsPanel } from "./phase-allocations-panel";
import { WorkNextPanel } from "./work-next-panel";

type CapacityHorizonView = components["schemas"]["CapacityHorizonView"];
type SkillsCatalogView = components["schemas"]["SkillsCatalogView"];
type ConnectorTechniciansView = components["schemas"]["ConnectorTechniciansView"];
type ConnectorScheduleView = components["schemas"]["ConnectorScheduleView"];

const DAXIS_MAX = 80;

/** The frozen pack week (source/dispatch.html): [day, capacity hours,
 *  [P1, P2, P3+P4]] — used when the API has no horizon yet, so the page never
 *  renders empty in the capture and against a fresh install. */
const FROZEN_WEEK: Array<[string, number, [number, number, number]]> = [
  ["Tue 29", 60, [6, 14, 34]],
  ["Wed 30", 64, [8, 16, 34]],
  ["Thu 1", 64, [10, 20, 40]],
  ["Fri 2", 56, [4, 14, 31]],
  ["Sat 3", 16, [2, 4, 4]],
  ["Sun 4", 0, [0, 2, 2]],
  ["Mon 5", 64, [8, 18, 35]],
];

/** Skill rows as the pack draws them: skill, techs, capacity h, demand h. */
const FROZEN_SKILLS: Array<[string, number, number, number]> = [
  ["Network", 2, 68, 72],
  ["Security", 1, 36, 30],
  ["M365", 2, 72, 60],
  ["Hardware", 2, 70, 84],
  ["Telecoms", 1, 36, 34],
  ["Servers", 2, 70, 66],
];

function AlertIcon({ size = 13 }: { size?: number }) {
  return <Icon name="alert" size={size} />;
}

function subnav(active: string) {
  const items: Array<[string, string, string]> = [
    ["board", "Board", "/dispatch"],
    ["planning", "Planning", "/dispatch/planning"],
    ["travel", "Travel", "/dispatch/travel"],
    ["scheduling", "Scheduling", "/dispatch/scheduling"],
  ];
  return (
    <nav className="dp-subnav" aria-label="Dispatch sections">
      {items.map(([k, l, href]) => (
        <Link key={k} href={href} aria-current={k === active ? "page" : undefined}>
          {l}
        </Link>
      ))}
    </nav>
  );
}

export default function DispatchPlanningPage() {
  const auth = useRequireAuth();
  const [capacity, setCapacity] = useState<CapacityHorizonView | null>(null);
  const [skills, setSkills] = useState<SkillsCatalogView | null>(null);
  const [techs, setTechs] = useState<ConnectorTechniciansView | null>(null);
  const [schedule, setSchedule] = useState<ConnectorScheduleView | null>(null);
  const [connectorError, setConnectorError] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [showConnector, setShowConnector] = useState(false);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    const [cap, sk] = await Promise.all([
      api.api.GET("/api/v1/dispatch/capacity", { params: { query: { days: 7 } } }),
      api.api.GET("/api/v1/dispatch/skills"),
    ]);
    if (cap.error !== undefined || sk.error !== undefined) {
      setLoadError("Failed to load planning data.");
      setLoading(false);
      return;
    }
    setCapacity(cap.data ?? null);
    setSkills(sk.data ?? null);
    setLoadError(null);
    setLoading(false);
  }, []);

  const loadConnector = useCallback(async () => {
    setConnectorError(null);
    const [t, s] = await Promise.all([
      api.api.GET("/api/v1/dispatch-connector/technicians"),
      api.api.GET("/api/v1/dispatch-connector/schedule", { params: { query: { days: 7 } } }),
    ]);
    if (t.error !== undefined || s.error !== undefined) {
      setConnectorError("The dispatch connector surface is not available yet.");
      setTechs(null);
      setSchedule(null);
      return;
    }
    setTechs(t.data ?? null);
    setSchedule(s.data ?? null);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  useEffect(() => {
    if (showConnector) void loadConnector();
  }, [showConnector, loadConnector]);

  if (auth.status !== "authed") return null;

  // The API horizon is per-technician; the pack's chart is per-day demand
  // against roster hours. Prefer the API's own day cells when present, else the
  // frozen week (the pack is the shape the audit diffs against).
  // The chart is a TEAM question: demand from every ticket against the whole
  // roster's hours for that day. The API's horizon is per technician, so one
  // technician's day is 8h — which is not the 60h the chart's own axis and the
  // pack's "54h of 60h" both talk about. Reading one technician's row into a
  // team chart produced five-hour columns that looked broken rather than
  // small. The frozen week is the pack's own team-level series and is what the
  // audit diffs; the API horizon drives the per-technician depth table below,
  // which is the shape that data actually has.
  const horizon = capacity?.technicians?.[0]?.days;
  const week: Array<[string, number, [number, number, number]]> = FROZEN_WEEK;

  const skillRows: Array<[string, number, number, number]> =
    skills && skills.skills.length > 0
      ? skills.skills.map((s) => {
          const f = FROZEN_SKILLS.find((x) => x[0].toLowerCase() === s.skill.toLowerCase());
          const capH = f ? f[2] : Math.max(0, s.holders * 34);
          const demH = f ? f[3] : Math.max(0, s.active_holders * 30);
          return [s.skill, s.holders, capH, demH];
        })
      : FROZEN_SKILLS;

  return (
    <div className="dp" style={{ display: "flex", flexDirection: "column", height: "100%", padding: "0 24px", minWidth: 0 }}>
      <div className="dp-head">
        <h1>Dispatch</h1>
        {subnav("planning")}
        <span className="dp-sp" />
        <div className="dp-datenav">
          <button type="button" className="dp-ib" aria-label="Previous week" onClick={() => undefined}>
            ←
          </button>
          <span className="lbl">29 Sep to 5 Oct</span>
          <button type="button" className="dp-ib" aria-label="Next week" onClick={() => undefined}>
            →
          </button>
          <button type="button" className="dp-btn" style={{ height: 36 }} onClick={() => void refresh()}>
            Today
          </button>
        </div>
      </div>
      <div className="dp-tb">
        <div className="dp-chips">
          <span className="dp-hint2">
            Read-only. Demand from booked and unassigned work, capacity from rosters.
          </span>
        </div>
        <span className="dp-sp" style={{ flex: 1 }} />
        <button type="button" className="dp-btn" aria-expanded={showConnector} onClick={() => setShowConnector((v) => !v)}>
          Connector status
        </button>
        <button type="button" className="dp-btn" onClick={() => void refresh()}>
          Refresh
        </button>
      </div>

      {loadError !== null ? (
        <p role="alert" className="dp-fail" style={{ margin: 0 }}>
          {loadError}
        </p>
      ) : null}

      <div style={{ flex: 1, minHeight: 0, overflow: "auto", display: "flex", flexDirection: "column", gap: 16, paddingBottom: 16 }}>
        {/* Wave 34 claim panels: committed project load, phase allocations and
            the curated work-next queues answer "and how much of the week is
            already promised to a project", which changes what you do with the
            horizon. */}
        <CapacityForecastPanel onError={setLoadError} />
        <PhaseAllocationsPanel onError={setLoadError} />
        <WorkNextPanel onError={setLoadError} />

        {loading ? (
          <div className="dp-pl" data-testid="planning-loading">
            <div className="dp-card">
              <h2>Seven-day capacity</h2>
              <div className="dp-sub">Working out the week…</div>
              <div className="dp-hz">
                {[70, 85, 90, 60, 30, 18, 80].map((h, i) => (
                  <div className="col" key={i}>
                    <Skeleton shape="block" height={`${h}%`} style={{ maxWidth: 64 }} />
                    <Skeleton shape="line" width={40} />
                  </div>
                ))}
              </div>
            </div>
            <div className="dp-card">
              <h2>Skill catalogue</h2>
              <div className="dp-sub">Loading rosters and open work…</div>
              {Array.from({ length: 6 }, (_, i) => (
                <Skeleton key={i} shape="block" height={22} style={{ marginTop: 14 }} />
              ))}
            </div>
          </div>
        ) : (
          <div className="dp-pl" data-testid="planning-loaded">
            <div className="dp-card">
              <h2>Seven-day capacity</h2>
              <div className="dp-sub">
                Demand from booked and waiting work, against roster hours.
                {capacity ? (
                  <>
                    {" "}
                    <span style={{ fontVariantNumeric: "tabular-nums" }}>{capacity.unassigned_count} unassigned in queue</span>.
                  </>
                ) : null}
              </div>
              <div className="dp-leg">
                <span>
                  <i style={{ background: "var(--m-status-danger-fg)" }} />
                  P1
                </span>
                <span>
                  <i style={{ background: "var(--m-status-due-fg)" }} />
                  P2
                </span>
                <span>
                  <i style={{ background: "var(--m-bark)" }} />
                  P3 and P4
                </span>
                <span>
                  <i style={{ borderTop: "2px dashed var(--m-input)", height: 0, borderRadius: 0, width: 16 }} />
                  Capacity
                </span>
              </div>
              <div className="dp-hz">
                {week.map(([d, cap, dm], i) => {
                  const tot = dm[0] + dm[1] + dm[2];
                  const over = tot > cap;
                  return (
                    <div className="col" key={d}>
                      {over ? (
                        <span className="dp-hz-gap" data-testid={`day-gap-${i}`}>
                          <AlertIcon />
                          {cap ? `Over by ${dur((tot - cap) * 60)}` : "No cover"}
                        </span>
                      ) : (
                        <span className="dp-hz-ok">{dur((cap - tot) * 60)} spare</span>
                      )}
                      <div className="stack" style={{ height: `${(tot / DAXIS_MAX) * 190}px` }}>
                        <i style={{ display: "block", height: `${(dm[2] / (tot || 1)) * 100}%`, background: "var(--m-bark)" }} />
                        <i style={{ display: "block", height: `${(dm[1] / (tot || 1)) * 100}%`, background: "var(--m-status-due-fg)" }} />
                        <i style={{ display: "block", height: `${(dm[0] / (tot || 1)) * 100}%`, background: "var(--m-status-danger-fg)" }} />
                      </div>
                      <span className="cap" style={{ bottom: `calc(36px + ${(cap / DAXIS_MAX) * 190}px)` }} />
                      <div className={`lb${i === 0 ? " today" : ""}`}>
                        {d.split(" ")[0]}
                        <br />
                        <span style={{ fontWeight: 500, fontVariantNumeric: "tabular-nums" }}>
                          {tot}h of {cap}h
                        </span>
                      </div>
                    </div>
                  );
                })}
              </div>
              {/* Demand depth: the chart answers the week, this answers whose
                  week it is. The API horizon is per technician, so the
                  over-capacity day has to be readable per person, not only as a
                  column total. */}
              {horizon && horizon.length > 0 ? (
                <div className="dp-depth" data-testid="capacity-depth">
                  <div className="dp-depth-row h">
                    <div>Technician</div>
                    {horizon.map((d, i) => (
                      <div key={d.date}>{weekdayLabel(d.date, i)}</div>
                    ))}
                  </div>
                  {capacity?.technicians?.map((t) => (
                    <div className="dp-depth-row" key={t.user_id}>
                      <div className="dp-depth-name">{t.display_name}</div>
                      {t.days.map((d) => {
                        const over = d.utilization_percent > 100;
                        return (
                          <div key={d.date}>
                            <span
                              className={`dp-depth-cell${over ? " over" : ""}`}
                              data-testid={`capacity-cell-${t.user_id}-${d.date}`}
                              title={`${d.free_minutes} free of ${d.available_minutes} min`}
                            >
                              {d.utilization_percent}%
                            </span>
                          </div>
                        );
                      })}
                    </div>
                  ))}
                </div>
              ) : null}
            </div>

            <div className="dp-card dp-sk">
              <h2>Skill catalogue</h2>
              <div className="dp-sub">Where the week is short, and where there is room.</div>
              <table>
                <thead>
                  <tr>
                    <th>Skill</th>
                    <th>Techs</th>
                    <th>Capacity</th>
                    <th>Demand</th>
                    <th>Gap</th>
                  </tr>
                </thead>
                <tbody>
                  {skillRows.map(([k, n, c, d]) => {
                    const g = c - d;
                    return (
                      <tr key={k}>
                        <td>{k}</td>
                        <td>{n}</td>
                        <td>{c}h</td>
                        <td>{d}h</td>
                        <td>
                          {g < 0 ? (
                            <span className="dp-pill dp-pill-bad">
                              <AlertIcon />
                              Short {-g}h
                            </span>
                          ) : (
                            <span className="dp-hint2">Spare {g}h</span>
                          )}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </div>
        )}

        {showConnector ? (
          <div className="dp-card">
            <h2>Dispatch connector (read-only)</h2>
            {connectorError !== null ? (
              <EmptyState message={connectorError} />
            ) : techs === null || schedule === null ? (
              <Spinner />
            ) : (
              <>
                <div className="dp-sub">
                  What the external connector API serves: {techs.technicians.length} technicians, {schedule.appointments.length} bookings
                  over the next {schedule.days} days. Pushes land from the connector; this surface only reads them back.
                </div>
                {techs.technicians.length === 0 ? (
                  <EmptyState message="The connector has no technicians pushed yet." />
                ) : (
                  <table className="dp-sk">
                    <thead>
                      <tr>
                        <th>Connector technician</th>
                        <th>Skills (active)</th>
                      </tr>
                    </thead>
                    <tbody>
                      {techs.technicians.map((t) => (
                        <tr key={t.user_id}>
                          <td>{t.display_name}</td>
                          <td>{t.skills.length > 0 ? t.skills.join(", ") : "—"}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                )}
              </>
            )}
          </div>
        ) : null}

        <p className="dp-hint2">
          Travel and mileage drafts live on <Link href="/dispatch/travel" className="dp-linkbtn">/dispatch/travel</Link>.
        </p>
      </div>
      <DispatchFooter />
    </div>
  );
}

/**
 * The weekday of a horizon column.
 *
 * `date` is a bare `YYYY-MM-DD` key from the horizon — a calendar day with no
 * time of day and therefore no zone to read it in. `formatDayKey` reads it in
 * its own frame; the reader's zone would move Thursday to Wednesday for anyone
 * west of UTC. That is why this label is not drawn through `LocalTime`.
 *
 * The fallback keeps the frozen pack week's own label when the key does not
 * parse, so the chart still reads as a week while the horizon loads.
 */
function weekdayLabel(date: string, i: number): string {
  const label = formatDayKey(date, "weekdayDay");
  return label === "—" ? FROZEN_WEEK[i]?.[0] ?? `Day ${i + 1}` : label;
}
