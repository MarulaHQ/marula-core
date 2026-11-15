// /projects/portfolio — the quarter view (S7.1–S7.8).
//
// Every project on one axis, grouped with summaries, at-risk only with the
// reason in words, and a not-scheduled tray that never puts undated work on the
// chart. A shared link reproduces the exact view because the server stores the
// snapshot it sent.

"use client";

import {
  Badge,
  Button,
  EmptyState,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  SurfaceHeader,
} from "@marula-core/ui";
import { Segmented, SegmentedItem } from "@marula-core/ui/kit";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useSearchParams } from "next/navigation";
import { useRequireAuth } from "../../lib/api";
import {
  fetchPortfolio,
  fetchSharedPortfolio,
  sharePortfolio,
  type PortfolioProject,
  type PortfolioView,
} from "./delivery-api";

function dayMs(date: string): number {
  return new Date(`${date}T00:00:00Z`).getTime();
}

function position(date: string, from: string, to: string): number {
  const span = dayMs(to) - dayMs(from);
  if (span <= 0) return 0;
  return Math.min(100, Math.max(0, ((dayMs(date) - dayMs(from)) / span) * 100));
}

function ProjectRow({ project, view }: { project: PortfolioProject; view: PortfolioView }) {
  const left = project.start_date === null ? 0 : position(project.start_date, view.from, view.to);
  const right = project.due_date === null ? 100 : position(project.due_date, view.from, view.to);
  return (
    <li className="flex items-stretch gap-3 border-b border-border py-2 last:border-b-0">
      <div className="w-64 shrink-0">
        <div className="flex items-center gap-2">
          <span className="truncate text-[13px] font-medium">{project.name}</span>
          {project.at_risk ? <Badge tone="due">At risk</Badge> : null}
        </div>
        <div className="text-[12px] text-muted-foreground">
          {project.client_name ?? "No client"} · {project.status.replaceAll("_", " ")}
        </div>
        {project.risk_reasons.length > 0 ? (
          <ul className="mt-0.5 text-[12px] text-muted-foreground">
            {project.risk_reasons.map((reason) => (
              <li key={reason}>{reason}</li>
            ))}
          </ul>
        ) : null}
      </div>

      <div
        className="relative min-h-[34px] flex-1"
        role="img"
        aria-label={`${project.name}: ${project.start_date ?? "not scheduled"} to ${
          project.due_date ?? "not dated"
        }`}
      >
        {project.start_date !== null || project.due_date !== null ? (
          <div
            className="absolute top-1/2 h-[10px] -translate-y-1/2 rounded-pill border border-border"
            style={{
              left: `${left}%`,
              width: `${Math.max(right - left, 1)}%`,
              background:
                project.status === "completed"
                  ? "var(--m-mc-green-soft, var(--m-accent))"
                  : project.status === "on_hold"
                    ? "transparent"
                    : "color-mix(in srgb, var(--m-accent) 55%, transparent)",
              borderStyle: project.status === "on_hold" ? "dashed" : "solid",
            }}
          />
        ) : null}
        {project.phases.map((phase) => {
          const phaseLeft = position(phase.start_date, view.from, view.to);
          const phaseRight = position(phase.end_date, view.from, view.to);
          return (
            <div
              key={phase.id}
              className="absolute top-1/2 h-[6px] -translate-y-1/2 rounded-pill"
              style={{
                left: `${phaseLeft}%`,
                width: `${Math.max(phaseRight - phaseLeft, 0.6)}%`,
                marginTop: `${(project.phases.indexOf(phase) % 3) * 7 - 7}px`,
                background:
                  phase.status === "completed"
                    ? "var(--m-text)"
                    : "color-mix(in srgb, var(--m-text) 45%, transparent)",
              }}
              title={`${phase.name}: ${phase.start_date} → ${phase.end_date}`}
            />
          );
        })}
      </div>
    </li>
  );
}

export function PortfolioQuarter() {
  const auth = useRequireAuth();
  const search = useSearchParams();
  const shareToken = search.get("share");

  const [view, setView] = useState<PortfolioView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [groupBy, setGroupBy] = useState("client");
  const [zoom, setZoom] = useState<"quarter" | "weeks">("quarter");
  const [atRiskOnly, setAtRiskOnly] = useState(false);
  const [shareNote, setShareNote] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    setError(null);
    try {
      if (shareToken !== null) {
        setView(await fetchSharedPortfolio(shareToken));
      } else {
        setView(await fetchPortfolio({ group_by: groupBy, zoom, at_risk_only: atRiskOnly }));
      }
    } catch (e) {
      setError((e as Error).message);
    }
  }, [shareToken, groupBy, zoom, atRiskOnly]);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void load();
  }, [auth.status, load]);

  const totals = useMemo(() => {
    const groups = view?.groups ?? [];
    const projects = groups.reduce((sum, group) => sum + group.project_count, 0);
    const risk = groups.reduce((sum, group) => sum + group.at_risk_count, 0);
    return { projects, risk };
  }, [view]);

  async function onShare() {
    setBusy(true);
    setError(null);
    try {
      const share = await sharePortfolio({ group_by: groupBy, zoom });
      const url = `${window.location.origin}${share.path}`;
      try {
        await navigator.clipboard.writeText(url);
        setShareNote("Link copied. It reproduces this exact view for anyone allowed to see it.");
      } catch {
        setShareNote(url);
      }
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  if (auth.status === "loading" || (view === null && error === null)) {
    return (
      <div className="flex items-center gap-2 p-8 text-[13px] text-muted-foreground">
        <Spinner /> Reading the portfolio…
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-5 p-6">
      <SurfaceHeader
        title="Portfolio"
        description="Every project on one axis, for the coordinator. Weeks across, projects down, and a reason wherever one needs attention."
        meta={
          <div className="flex items-center gap-3 text-[13px] text-muted-foreground">
            <span className="font-mono tabular-nums">{totals.projects} projects</span>
            <span className="font-mono tabular-nums">{totals.risk} at risk</span>
            {shareToken !== null ? <Badge tone="neutral">Shared view</Badge> : null}
          </div>
        }
        actions={
          shareToken === null ? (
            <Button variant="secondary" disabled={busy} onClick={() => void onShare()}>
              Share this view
            </Button>
          ) : null
        }
      />

      {error !== null ? (
        <div role="alert" className="flex items-center gap-3 rounded-ctl border border-border p-3 text-[13px]">
          <span className="flex-1">The portfolio could not be read: {error}</span>
          <Button variant="secondary" onClick={() => void load()}>
            Try again
          </Button>
        </div>
      ) : null}
      {shareNote !== null ? (
        <p className="rounded-ctl bg-muted px-3 py-2 text-[13px] text-muted-foreground">{shareNote}</p>
      ) : null}

      {shareToken === null ? (
        <div className="flex flex-wrap items-center gap-3">
          <Segmented
            type="single"
            value={zoom}
            onValueChange={(value: string) => {
              if (value === "quarter" || value === "weeks") setZoom(value);
            }}
            aria-label="Zoom"
          >
            <SegmentedItem value="quarter">Quarter</SegmentedItem>
            <SegmentedItem value="weeks">Four weeks</SegmentedItem>
          </Segmented>

          <Select value={groupBy} onValueChange={setGroupBy}>
            <SelectTrigger className="w-44" aria-label="Group by">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="client">Group by client</SelectItem>
              <SelectItem value="status">Group by status</SelectItem>
              <SelectItem value="none">One list</SelectItem>
            </SelectContent>
          </Select>

          <label className="flex items-center gap-2 text-[13px] text-muted-foreground">
            <input
              type="checkbox"
              checked={atRiskOnly}
              onChange={(event) => setAtRiskOnly(event.target.checked)}
            />
            At risk only
          </label>
        </div>
      ) : null}

      {view !== null ? (
        <>
          <div className="overflow-x-auto rounded-card border border-border">
            <div className="min-w-[820px]">
              <div className="sticky top-0 z-10 flex gap-3 border-b border-border bg-card px-3 py-2 text-[12px] text-muted-foreground">
                <div className="w-64 shrink-0">Project</div>
                <div className="flex flex-1">
                  {view.weeks.map((week) => (
                    <div key={week.start} className="flex-1 border-l border-border pl-1 first:border-l-0">
                      {week.label}
                    </div>
                  ))}
                </div>
              </div>

              {(view.groups ?? []).length === 0 ? (
                <div className="p-6">
                  <EmptyState
                    message={
                      atRiskOnly
                        ? "Nothing is at risk in this range. Clear the at-risk filter to see the rest of the portfolio."
                        : "No projects in this range. Widen the range, or start from the not-scheduled tray below."
                    }
                  />
                </div>
              ) : (
                view.groups.map((group) => (
                  <section key={group.key} aria-label={group.label}>
                    <div className="flex items-center justify-between gap-2 bg-muted/50 px-3 py-1.5 text-[12px]">
                      <span className="font-semibold">{group.label}</span>
                      <span className="text-muted-foreground">
                        {group.project_count} project{group.project_count === 1 ? "" : "s"}
                        {group.at_risk_count > 0 ? ` · ${group.at_risk_count} at risk` : ""}
                      </span>
                    </div>
                    <ul className="px-3">
                      {group.projects.map((project) => (
                        <ProjectRow key={project.id} project={project} view={view} />
                      ))}
                    </ul>
                  </section>
                ))
              )}
            </div>
          </div>

          {view.unscheduled.length > 0 ? (
            <section aria-label="Not scheduled" className="rounded-card border border-border p-3">
              <h2 className="text-[13px] font-semibold">
                Not scheduled ({view.unscheduled.length})
              </h2>
              <p className="mb-2 text-[12px] text-muted-foreground">
                Projects with no dates wait here. They are never drawn on the chart as
                if someone had planned them.
              </p>
              <ul className="flex flex-wrap gap-2">
                {view.unscheduled.map((project) => (
                  <li key={project.id} className="rounded-ctl border border-border px-2 py-1 text-[12px]">
                    {project.name}
                    <span className="ml-2 text-muted-foreground">{project.status}</span>
                  </li>
                ))}
              </ul>
            </section>
          ) : null}
        </>
      ) : null}
    </div>
  );
}
