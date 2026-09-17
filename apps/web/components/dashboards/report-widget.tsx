// ReportWidget — one dashboard row: fetches its report, renders the right
// visualization, exposes CSV export (same report, format=csv) and removal.
//
// Purpose: single card per widget id from the fixed catalog; personal-lens
// widgets pin user_id to the caller so they stay personal even for
// managers, org-lens widgets omit it (managers only — the picker and
// preset filters enforce that; the server re-enforces it regardless).
// Wave 17 lane 1: mockup-grade card chrome — quiet uppercase title row,
// hover/focus-revealed actions, typed skeleton loading, error state with
// retry, and the shared empty sentence per widget. Staggered entrance via
// .m-rise (transform/opacity, ≤120ms, reduced-motion honored).
// Non-purpose: no persistence — the page owns widget rows.
"use client";

import { Button, Card, EmptyState, StatusTag, Tag, Table, Td, Th, Tr } from "@meridian/ui";
import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import type { WidgetDef } from "../../lib/dashboards";
import { formatMinutes, formatSeconds } from "../../lib/dashboards";
import {
  agingRows,
  agingSummary,
  downloadReportCsv,
  fetchReport,
  firstResponseSummary,
  rollupRows,
  slaAttainmentRows,
  slaAttainmentSummary,
  slaBreachRows,
  slaBreachSummary,
  throughputRows,
  throughputSummary,
  timeRollupSummary,
} from "../../lib/reports";
import type { ReportPage } from "../../lib/reports";
import {
  AgingBarChart,
  AttainmentBarChart,
  RollupBarChart,
  ThroughputLineChart,
} from "./widget-charts";

interface Props {
  def: WidgetDef;
  /** Caller's user id — pins personal-lens queries. */
  meUserId: string;
  /** Staggered-entrance delay in ms (caps at 240 on the page). */
  entranceDelay?: number;
  onRemove: () => void;
}

interface LoadState {
  status: "loading" | "ready" | "error";
  page?: ReportPage;
  error?: string;
}

function priorityTone(priority: string): "danger" | "warning" | "neutral" {
  if (priority.toLowerCase() === "p1") return "danger";
  if (priority.toLowerCase() === "p2") return "warning";
  return "neutral";
}

/** Attainment tagged with the semantic status scale. */
function attainmentTone(pct: number): "success" | "warning" | "danger" {
  if (pct >= 90) return "success";
  if (pct >= 70) return "warning";
  return "danger";
}

export function ReportWidget({ def, meUserId, entranceDelay = 0, onRemove }: Props) {
  const [state, setState] = useState<LoadState>({ status: "loading" });
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setState({ status: "loading" });
    void (async () => {
      try {
        const page = await fetchReport(def.kind, {
          user_id: def.lens === "self" ? meUserId : null,
        });
        if (!cancelled) setState({ status: "ready", page });
      } catch (e) {
        if (!cancelled) {
          setState({ status: "error", error: e instanceof Error ? e.message : String(e) });
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [def.kind, def.lens, meUserId, attempt]);

  const exportCsv = () => {
    void downloadReportCsv(
      def.kind,
      { user_id: def.lens === "self" ? meUserId : null },
      `${def.kind}.csv`,
    ).catch(() => {});
  };

  let body: ReactNode;
  if (state.status === "loading") {
    body = <WidgetSkeleton label={`Loading ${def.title}`} />;
  } else if (state.status === "error") {
    body = (
      <div className="grid gap-2" role="alert">
        <p className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {state.error ?? "The report failed to load."}
        </p>
        <div>
          <Button variant="secondary" size="sm" onClick={() => setAttempt((a) => a + 1)}>
            Retry
          </Button>
        </div>
      </div>
    );
  } else {
    body = <WidgetBody def={def} page={state.page} />;
  }

  return (
    <Card
      className="m-rise group grid gap-3 p-4 transition-shadow duration-100 hover:shadow-md"
      data-widget-id={def.id}
      style={{ animationDelay: `${entranceDelay}ms` }}
    >
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <h3 className="truncate text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
            {def.title}
          </h3>
          <Tag tone={def.lens === "org" ? "neutral" : "outline"}>
            {def.lens === "org" ? "Org" : "Mine"}
          </Tag>
        </div>
        <div className="flex items-center gap-1 opacity-0 transition-opacity duration-100 group-focus-within:opacity-100 group-hover:opacity-100">
          <Button variant="ghost" size="sm" onClick={exportCsv} aria-label={`Export ${def.title} as CSV`}>
            CSV
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={onRemove}
            aria-label={`Remove ${def.title}`}
          >
            Remove
          </Button>
        </div>
      </div>
      {body}
    </Card>
  );
}

/** Typed loading state: quiet skeleton bars, no spinner churn. */
function WidgetSkeleton({ label }: { label: string }) {
  return (
    <div className="grid gap-2" aria-busy="true" aria-label={label}>
      <div className="m-skel h-3 w-1/3" />
      <div className="m-skel h-3 w-2/3" />
      <div className="m-skel h-3 w-1/2" />
      <div className="m-skel h-3 w-1/4" />
    </div>
  );
}

function WidgetBody({ def, page }: { def: WidgetDef; page: ReportPage | undefined }) {
  if (page === undefined) return null;
  if (page.rows.length === 0) {
    return <EmptyState message={def.empty} />;
  }
  switch (def.id) {
    case "time_by_user:self":
    case "time_by_user:org":
    case "time_by_client:org": {
      const rows = rollupRows(page);
      const s = timeRollupSummary(page);
      return (
        <>
          <RollupBarChart
            rows={rows}
            ariaLabel={`${def.title} bar chart, hours per group`}
            width={def.id.startsWith("time_by_user") ? 1360 : 720}
          />
          <Footer>
            {formatMinutes(s.total_minutes)} total · {formatMinutes(s.billable_minutes)} billable
          </Footer>
        </>
      );
    }
    case "ticket_throughput:self":
    case "ticket_throughput:org": {
      const rows = throughputRows(page);
      const s = throughputSummary(page);
      return (
        <>
          <ThroughputLineChart rows={rows} ariaLabel={`${def.title} line chart, created vs resolved per day`} />
          <Footer>
            {s.created_total} created · {s.resolved_total} resolved
          </Footer>
        </>
      );
    }
    case "ticket_aging:self":
    case "ticket_aging:org": {
      const rows = agingRows(page);
      const s = agingSummary(page);
      return (
        <>
          <AgingBarChart rows={rows} ariaLabel={`${def.title} bar chart, open tickets per age bucket`} />
          <Footer>
            {s.open_total} open · {s.breached_open} breached
          </Footer>
        </>
      );
    }
    case "sla_attainment:self":
    case "sla_attainment:org": {
      const rows = slaAttainmentRows(page);
      const s = slaAttainmentSummary(page);
      return (
        <>
          <AttainmentBarChart rows={rows} ariaLabel={`${def.title} bar chart, attainment percent per policy`} />
          <Footer>
            <span style={{ color: `var(--m-status-${attainmentTone(s.attainment_pct)}-fg)` }}>
              {s.attainment_pct}%
            </span>{" "}
            attainment over {s.resolutions_total} resolutions
          </Footer>
        </>
      );
    }
    case "sla_breaches:org": {
      const rows = slaBreachRows(page);
      const s = slaBreachSummary(page);
      const shown = rows.slice(0, 8);
      return (
        <>
          <Table>
            <thead>
              <tr>
                <Th>Ticket</Th>
                <Th>Priority</Th>
                <Th>Policy</Th>
                <Th>Resolution due</Th>
              </tr>
            </thead>
            <tbody>
              {shown.map((r) => (
                <Tr key={r.ticket_id}>
                  <Td>{r.ticket_number}</Td>
                  <Td>
                    <StatusTag tone={priorityTone(r.priority)}>{r.priority.toUpperCase()}</StatusTag>
                  </Td>
                  <Td>{r.policy_name}</Td>
                  <Td>{r.resolution_due.slice(0, 10)}</Td>
                </Tr>
              ))}
            </tbody>
          </Table>
          <Footer>
            {s.breaches_total} breaches · {formatSeconds(s.paused_business_secs_total)} paused SLA time
          </Footer>
        </>
      );
    }
    case "first_response:self":
    case "first_response:org": {
      const s = firstResponseSummary(page);
      return (
        <>
          <div className="grid grid-cols-3 gap-3 max-w-xl">
            <Stat label="Median response" value={formatSeconds(s.response_p50_secs)} />
            <Stat label="90th percentile" value={formatSeconds(s.response_p90_secs)} />
            <Stat label="Unanswered" value={String(s.unanswered_count)} />
          </div>
          <Footer>{s.answered_count} of {s.tickets_total} answered</Footer>
        </>
      );
    }
    default:
      return null;
  }
}

function Footer({ children }: { children: ReactNode }) {
  return (
    <p
      className="border-t pt-2 text-[12px]"
      style={{ color: "var(--m-text-secondary)", borderColor: "var(--m-border)" }}
    >
      {children}
    </p>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid gap-0.5">
      <span
        className="text-[10px] font-medium uppercase tracking-wide"
        style={{ color: "var(--m-text-muted)" }}
      >
        {label}
      </span>
      <span className="font-mono text-lg font-semibold tabular-nums" style={{ color: "var(--m-text)" }}>
        {value}
      </span>
    </div>
  );
}
