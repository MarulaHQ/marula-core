"use client";

// Dated Gantt for a project's phases, with each bar carrying its budget
// burn (Wave 34 claims close-out, claim 6).
//
// The composition follows the research in design-research.md: a fixed name
// rail that scrolls WITH the bar track (Wrike / ClickUp / Canva all do this),
// bars snapped to a visible grid so two phases' overlap is readable at a
// glance, and the overspend label ON the bar the overspend happened on
// rather than in a separate chart.
//
// The honest state matters more here than anywhere else on the page: a
// phase with no dates is NOT drawn at an invented position. It appears in
// the undated list below the chart, labelled as not being on the timeline,
// because putting a bar at "today" would assert a schedule nobody chose.

import {
  Card,
  CardHeader,
  EmptyState,
  Spinner,
  Tag,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useEffect, useState } from "react";
import { api } from "../lib/api";

type Gantt = components["schemas"]["ProjectGanttView"];
type Phase = components["schemas"]["PhaseGanttView"];

const ALERT_TONE = {
  none: undefined,
  warn: "warning",
  over: "danger",
} as const;

const MS_PER_DAY = 86_400_000;

export function ProjectGantt({ projectId }: { projectId: string }) {
  const [gantt, setGantt] = useState<Gantt | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    (async () => {
      try {
        const res = await api.api.GET("/api/v1/projects/{id}/gantt", {
          params: { path: { id: projectId } },
        });
        if (live) {
          if (res.data) setGantt(res.data as Gantt);
          else setError("could not load the timeline");
        }
      } catch (e) {
        if (live) setError(e instanceof Error ? e.message : "could not load");
      } finally {
        if (live) setLoading(false);
      }
    })();
    return () => {
      live = false;
    };
  }, [projectId]);

  if (loading) return <Spinner label="Loading the timeline" />;
  if (error) {
    return (
      <Card>
        <CardHeader>
          <h2 className="p-4 pb-0 text-sm font-semibold">Timeline</h2>
        </CardHeader>
        <p className="p-4 text-[13px]">{error}</p>
      </Card>
    );
  }
  if (!gantt) return null;

  const dated = gantt.dated;
  const undated = gantt.undated;

  if (dated.length === 0 && undated.length === 0) {
    return (
      <Card>
        <CardHeader>
          <h2 className="p-4 pb-0 text-sm font-semibold">Timeline</h2>
        </CardHeader>
        <EmptyState message="This project has no phases yet. Add phases to the project, then give them dates to draw a timeline." />
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <h2 className="p-4 pb-0 text-sm font-semibold">
          Timeline
          <span className="ml-3 font-normal" style={{ color: "var(--m-text-secondary)" }}>
            Planned phase windows and budget burn
          </span>
        </h2>
      </CardHeader>
      {dated.length === 0 ? (
        <p style={{ color: "var(--m-text-secondary)", fontSize: "12px" }}>
          No phase has dates yet, so there is no timeline to draw. The phases
          are listed below in order.
        </p>
      ) : (
        <GanttChart
          dated={dated}
          windowFrom={gantt.window_from ?? null}
          windowTo={gantt.window_to ?? null}
        />
      )}

      {undated.length > 0 ? (
        <div style={{ marginTop: "12px" }}>
          <p
            style={{
              color: "var(--m-text-secondary)",
              fontSize: "12px",
              marginBottom: "4px",
            }}
          >
            Not on the timeline — {undated.length} phase
            {undated.length === 1 ? "" : "s"} with no dates. Give both a start
            and an end to place {undated.length === 1 ? "it" : "them"} on the
            chart.
          </p>
          <ul
            style={{
              listStyle: "none",
              padding: 0,
              margin: 0,
              fontSize: "13px",
            }}
          >
            {undated.map((p) => (
              <li
                key={p.id}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "8px",
                  padding: "4px 0",
                }}
              >
                <PhaseStatusTag phase={p} />
                {p.name}
                <BurnLabel phase={p} />
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </Card>
  );
}

function GanttChart({
  dated,
  windowFrom,
  windowTo,
}: {
  dated: Phase[];
  windowFrom: string | null;
  windowTo: string | null;
}) {
  // Scale to the PLAN, not to today: a chart anchored on "now" leaves half
  // the bars off-screen for a project that starts next month.
  const start = Date.parse(`${windowFrom ?? dated[0]!.start_date!}T00:00:00Z`);
  const end = Date.parse(`${windowTo ?? dated[dated.length - 1]!.end_date!}T00:00:00Z`);
  const span = Math.max(end - start, MS_PER_DAY);

  // Week ticks, capped so a long project does not produce a hundred labels.
  const weeks: number[] = [];
  for (let t = 0; t <= span; t += 7 * MS_PER_DAY) {
    if (weeks.length < 16) weeks.push(t);
  }

  return (
    <div style={{ overflowX: "auto" }}>
      <div style={{ minWidth: `${420 + weeks.length * 56}px` }}>
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "200px 1fr",
            gap: "12px",
            alignItems: "end",
            marginBottom: "4px",
          }}
        >
          <span style={{ fontSize: "11px", color: "var(--m-text-secondary)" }}>
            Phase
          </span>
          <div
            style={{
              position: "relative",
              height: "16px",
              borderBottom: "1px solid var(--m-border)",
            }}
          >
            {weeks.map((t) => (
              <span
                key={t}
                style={{
                  position: "absolute",
                  left: `${(t / span) * 100}%`,
                  fontSize: "11px",
                  color: "var(--m-text-secondary)",
                  transform: "translateX(2px)",
                  whiteSpace: "nowrap",
                }}
              >
                {new Date(start + t).toISOString().slice(5, 10)}
              </span>
            ))}
          </div>
        </div>

        {dated.map((p) => {
          const s = Date.parse(`${p.start_date!}T00:00:00Z`);
          const e = Date.parse(`${p.end_date!}T00:00:00Z`);
          const left = ((s - start) / span) * 100;
          const width = Math.max(((e - s) / span) * 100, 1.2);
          const alert = p.burn_alert ?? "none";
          return (
            <div
              key={p.id}
              style={{
                display: "grid",
                gridTemplateColumns: "200px 1fr",
                gap: "12px",
                alignItems: "center",
                padding: "4px 0",
              }}
            >
              <span
                style={{
                  display: "grid",
                  gap: "1px",
                  fontSize: "13px",
                  minWidth: 0,
                }}
              >
                <span
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "6px",
                    minWidth: 0,
                  }}
                >
                  <PhaseStatusTag phase={p} />
                  <span
                    style={{
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}
                  >
                    {p.name}
                  </span>
                </span>
                <span style={{ paddingLeft: "2px" }}>
                  <BarLabel phase={p} />
                </span>
              </span>

              <div
                style={{
                  position: "relative",
                  height: "20px",
                  background:
                    "repeating-linear-gradient(to right, var(--m-border) 0 1px, transparent 1px 7.14%)",
                }}
              >
                <span
                  title={`${p.name}: ${p.start_date} → ${p.end_date}${
                    p.planned_days ? ` (${p.planned_days}d)` : ""
                  }`}
                  style={{
                    position: "absolute",
                    left: `${left}%`,
                    width: `${width}%`,
                    top: "2px",
                    bottom: "2px",
                    borderRadius: "3px",
                    // `--m-chart-1`, not `--m-accent`.
                    //
                    // `--m-accent` is a SURFACE colour that flips mode:
                    // #f7e3a8 on light, #6b4308 on dark. A bar filled with it
                    // is clearly visible in light mode and effectively
                    // invisible in dark — the vision pass caught exactly that,
                    // twice, because the first fix (a soft fill plus an
                    // accent ring) was the same brown against the same dark
                    // background.
                    //
                    // `--m-chart-1` is the token set built FOR data marks and
                    // it is mode-aware: #a8641a on light, #e8b83a on dark. A
                    // bar drawn in it is legible in both, with no per-mode
                    // branch and nothing to keep in sync.
                    background:
                      alert === "over"
                        ? "var(--m-status-danger-fg)"
                        : alert === "warn"
                          ? "var(--m-status-warning-fg)"
                          : "var(--m-chart-1)",
                    opacity: p.status === "completed" ? 0.6 : 1,
                  }}
                />

              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function PhaseStatusTag({ phase }: { phase: Phase }) {
  // Only the kit's four tones. An "in progress" phase gets `warning` rather
  // than an invented "info" step — the ramp has no informational slot, and
  // adding one here would be an off-palette state.
  const tone =
    phase.status === "completed"
      ? "success"
      : phase.status === "in_progress" || phase.status === "active"
        ? "warning"
        : "neutral";
  return <Tag tone={tone}>{phase.status.replace("_", " ")}</Tag>;
}

/**
 * What the rail says under a phase's name: the burn state when there is
 * one, otherwise the planned duration, otherwise nothing.
 *
 * Burn outranks duration because it is the thing a planner is looking for.
 * "Unbudgeted" is spelled out rather than rendered blank, because an empty
 * cell next to two labelled ones reads as missing data.
 */
function BarLabel({ phase }: { phase: Phase }) {
  if (phase.burn_alert === "warn" || phase.burn_alert === "over") {
    return (
      <span
        style={{
          color:
            phase.burn_alert === "over"
              ? "var(--m-status-danger-fg)"
              : "var(--m-status-warning-fg)",
          fontWeight: 500,
        }}
      >
        {Math.round(
          phase.budget_minor
            ? (phase.consumed_minor / phase.budget_minor) * 100
            : 0,
        )}
        % of budget
      </span>
    );
  }
  if (phase.budget_minor === null || phase.budget_minor === undefined) {
    return <span style={{ color: "var(--m-text-secondary)" }}>no budget</span>;
  }
  return (
    <span style={{ color: "var(--m-text-secondary)" }}>
      {phase.planned_days ?? 0}d planned
    </span>
  );
}

/**
 * Budget burn against the phase's own budget.
 *
 * `None` for an unbudgeted phase, and it renders as "no budget" rather than
 * green. "Unbudgeted" and "under budget" are different facts, and a phase
 * with no budget shown as on-track is a false reassurance.
 */
function BurnLabel({ phase }: { phase: Phase }) {
  if (phase.budget_minor === null || phase.budget_minor === undefined) {
    return (
      <span style={{ color: "var(--m-text-secondary)", fontSize: "11px" }}>
        no budget
      </span>
    );
  }
  const pct = phase.budget_minor
    ? Math.round((phase.consumed_minor / phase.budget_minor) * 100)
    : 0;
  return (
    <span style={{ fontVariantNumeric: "tabular-nums" }}>
      {pct}% of {formatMinor(phase.budget_minor, phase.currency)}
    </span>
  );
}

function formatMinor(minor: number, currency: string | null | undefined): string {
  const value = minor / 100;
  return `${currency ?? ""} ${value.toLocaleString(undefined, {
    minimumFractionDigits: 0,
    maximumFractionDigits: 2,
  })}`.trim();
}
