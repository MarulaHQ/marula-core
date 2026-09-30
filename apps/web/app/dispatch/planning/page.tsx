// /dispatch/planning — dispatch v2 resource planning (Wave 9 lane 9
// close-out).
//
// Purpose: the planner's view beyond the day board: a capacity horizon
// per technician (GET /dispatch/capacity) with the open unassigned-queue
// depth beside it, the org skill catalog with holder counts
// (GET /dispatch/skills), and the read-only dispatch-connector surface
// (technicians + upcoming schedule pushes) so external planner
// integrations can be inspected without any write affordance here.
//
// Also the home of the Wave 34 claims close-out panels: committed project
// load per technician (claim 1), the editable phase allocations that feed
// it, and the curated per-technician work-next queue (claim 2). They live
// here rather than on separate pages because the question they answer —
// "who is free, and what should they do first" — is one question, and a
// planner switching tabs to answer it would be solving it twice.
//
// Non-purpose: no drag-and-drop scheduling (the day board owns that); the
// connector view is read-only by design — pushes arrive from the
// connector API, not this screen.
"use client";

import {
  Button,
  Card,
  EmptyState,
  Spinner,
  Table,
  Tag,
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import { CapacityForecastPanel } from "./capacity-forecast-panel";
import { PhaseAllocationsPanel } from "./phase-allocations-panel";
import { WorkNextPanel } from "./work-next-panel";

type CapacityHorizonView = components["schemas"]["CapacityHorizonView"];
type SkillsCatalogView = components["schemas"]["SkillsCatalogView"];
type ConnectorTechniciansView = components["schemas"]["ConnectorTechniciansView"];
type ConnectorScheduleView = components["schemas"]["ConnectorScheduleView"];

export default function DispatchPlanningPage() {
  const auth = useRequireAuth();
  const [capacity, setCapacity] = useState<CapacityHorizonView | null>(null);
  const [skills, setSkills] = useState<SkillsCatalogView | null>(null);
  const [techs, setTechs] = useState<ConnectorTechniciansView | null>(null);
  const [schedule, setSchedule] = useState<ConnectorScheduleView | null>(null);
  const [connectorError, setConnectorError] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [showConnector, setShowConnector] = useState(false);

  const refresh = useCallback(async () => {
    const [cap, sk] = await Promise.all([
      api.api.GET("/api/v1/dispatch/capacity", { params: { query: { days: 7 } } }),
      api.api.GET("/api/v1/dispatch/skills"),
    ]);
    if (cap.error !== undefined || sk.error !== undefined) {
      setLoadError("Failed to load planning data.");
      return;
    }
    setCapacity(cap.data ?? null);
    setSkills(sk.data ?? null);
    setLoadError(null);
  }, []);

  const loadConnector = useCallback(async () => {
    setConnectorError(null);
    const [t, s] = await Promise.all([
      api.api.GET("/api/v1/dispatch-connector/technicians"),
      api.api.GET("/api/v1/dispatch-connector/schedule", {
        params: { query: { days: 7 } },
      }),
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

  return (
    <div className="grid gap-6 p-4 md:p-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-lg font-semibold">Resource planning</h1>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Seven-day capacity horizon, demand depth, and the skill catalog. Day
            windows are UTC, matching the dispatch API.
          </p>
        </div>
        <div className="flex gap-2">
          <Button variant="ghost" size="sm" onClick={() => void refresh()}>
            Refresh
          </Button>
          <Button variant="secondary" size="sm" onClick={() => setShowConnector((v) => !v)}>
            {showConnector ? "Hide connector" : "Connector status"}
          </Button>
        </div>
      </div>

      {/*
        The Wave 34 panels sit ABOVE the horizon, not after it. The horizon
        answers "who is free in the next week"; the forecast answers "and how
        much of that week is already committed to a project", which is the
        question that changes what you do with the answer.
      */}
      <CapacityForecastPanel onError={setLoadError} />
      <PhaseAllocationsPanel onError={setLoadError} />
      <WorkNextPanel onError={setLoadError} />

      {loadError !== null ? (
        <p role="alert" className="text-[13px]">{loadError}</p>
      ) : capacity === null || skills === null ? (
        <Spinner />
      ) : (
        <>
          <Card>
            <h2 className="p-4 pb-0 text-sm font-semibold">
              Capacity horizon — {capacity.from}, {capacity.days} days
              <span className="ml-3 font-normal" style={{ color: "var(--m-text-secondary)" }}>
                {capacity.unassigned_count} unassigned in queue
              </span>
            </h2>
            {capacity.technicians.length === 0 ? (
              <div className="p-4">
                <EmptyState message="No technicians with availability windows yet. Set working hours on the calendar surface to plan against capacity." />
              </div>
            ) : (
              <Table>
                <thead>
                  <tr>
                    <Th>Technician</Th>
                    {capacity.technicians[0]?.days.map((d) => (
                      <Th key={d.date} className="text-center">{d.date.slice(5)}</Th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {capacity.technicians.map((t) => (
                    <Tr key={t.user_id}>
                      <Td>{t.display_name}</Td>
                      {t.days.map((d) => {
                        const over = d.utilization_percent > 100;
                        const tight = d.utilization_percent > 85 && !over;
                        const tone = over ? "danger" : tight ? "warning" : "neutral";
                        return (
                          <Td
                            key={d.date}
                            className="text-center tabular-nums"
                            title={`${d.free_minutes} free of ${d.available_minutes} min`}
                          >
                            <span
                              data-testid={`capacity-cell-${t.user_id}-${d.date}`}
                              className="inline-block rounded-full px-2 py-0.5 text-[11px] font-medium"
                              style={{
                                color: `var(--m-status-${tone}-fg)`,
                                background: `var(--m-status-${tone}-bg)`,
                              }}
                            >
                              {d.utilization_percent}%
                            </span>
                          </Td>
                        );
                      })}
                    </Tr>
                  ))}
                </tbody>
              </Table>
            )}
          </Card>

          <Card>
            <h2 className="p-4 pb-0 text-sm font-semibold">Skill catalog</h2>
            {skills.skills.length === 0 ? (
              <div className="p-4">
                <EmptyState message="No skills recorded. Grant skills to technicians to feed the dispatch matcher." />
              </div>
            ) : (
              <Table>
                <thead>
                  <tr>
                    <Th>Skill</Th>
                    <Th className="text-right">Holders</Th>
                    <Th className="text-right">Active now</Th>
                  </tr>
                </thead>
                <tbody>
                  {skills.skills.map((s) => (
                    <Tr key={s.skill}>
                      <Td>
                        <Tag>{s.skill}</Tag>
                      </Td>
                      <Td className="text-right tabular-nums">{s.holders}</Td>
                      <Td className="text-right tabular-nums">{s.active_holders}</Td>
                    </Tr>
                  ))}
                </tbody>
              </Table>
            )}
          </Card>
        </>
      )}

      {showConnector ? (
        <Card>
          <h2 className="p-4 pb-0 text-sm font-semibold">
            Dispatch connector (read-only)
          </h2>
          {connectorError !== null ? (
            <div className="p-4">
              <EmptyState message={connectorError} />
            </div>
          ) : techs === null || schedule === null ? (
            <div className="p-4">
              <Spinner />
            </div>
          ) : (
            <div className="grid gap-4 p-4">
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                What the external connector API serves: {techs.technicians.length} technicians,{" "}
                {schedule.appointments.length} bookings over the next {schedule.days} days.
                Pushes land from the connector; this surface only reads them back.
              </p>
              <Table>
                <thead>
                  <tr>
                    <Th>Connector technician</Th>
                    <Th>Skills (active)</Th>
                  </tr>
                </thead>
                <tbody>
                  {techs.technicians.map((t) => (
                    <Tr key={t.user_id}>
                      <Td>{t.display_name}</Td>
                      <Td>{t.skills.length > 0 ? t.skills.join(", ") : "—"}</Td>
                    </Tr>
                  ))}
                </tbody>
              </Table>
            </div>
          )}
        </Card>
      ) : null}

      <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
        Travel and mileage drafts live on{" "}
        <Link href="/dispatch/travel" className="underline decoration-[var(--m-border)] underline-offset-2">
          /dispatch/travel
        </Link>
        .
      </p>
    </div>
  );
}
