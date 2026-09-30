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

import { Button, Card, EmptyState, StatusTag, Tag, Table, Td, Th, Tr } from "@marula-core/ui";
import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import type { WidgetDef } from "../../lib/dashboards";
import { WidgetSkeleton } from "./widget-skeleton";
import { AGING_BUCKET_LABELS, formatMinutes, formatSeconds } from "../../lib/dashboards";
import {
  agingRows,
  agingSummary,
  downloadReportCsv,
  fetchReport,
  firstResponseRows,
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
import { buildWidgetDeepLink, buildWidgetRowLink, attainmentRowSpec, rollupRowSpec, throughputRowSpec } from "../../lib/report-spec-codec";
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
  // Chart ↔ data-table alternative (keyboard a11y: recharts tooltips are
  // hover-only; the table presents the same rows to keyboard/AT readers).
  const [showTable, setShowTable] = useState(false);
  const tableAvailable = def.viz === "bar" || def.viz === "line";

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
    body = <WidgetBody def={def} page={state.page} showTable={showTable} />;
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
          <a
            href={buildWidgetDeepLink(def)}
            data-testid={`widget-drill-${def.id}`}
            className="rounded-[var(--m-radius-control)] px-2 py-1 text-[11px] hover:underline"
            style={{ color: "var(--m-accent)" }}
            aria-label={`Open ${def.title} in the report builder`}
          >
            Builder
          </a>
          {tableAvailable && (
            <Button
              variant="ghost"
              size="sm"
              aria-pressed={showTable}
              data-testid={`widget-table-toggle`}
              aria-label={showTable ? `Show ${def.title} as chart` : `Show ${def.title} as data table`}
              onClick={() => setShowTable((v) => !v)}
            >
              {showTable ? "Chart" : "Table"}
            </Button>
          )}
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

function WidgetBody({
  def,
  page,
  showTable,
}: {
  def: WidgetDef;
  page: ReportPage | undefined;
  showTable: boolean;
}) {
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
      if (showTable) {
        return (
          <>
            <RollupDataTable rows={rows} />
            <Footer>
              {formatMinutes(s.total_minutes)} total · {formatMinutes(s.billable_minutes)} billable
            </Footer>
          </>
        );
      }
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
      if (showTable) {
        return (
          <>
            <ThroughputDataTable rows={rows} />
            <Footer>
              {s.created_total} created · {s.resolved_total} resolved
            </Footer>
          </>
        );
      }
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
      if (showTable) {
        return (
          <>
            <AgingDataTable rows={rows} />
            <Footer>
              {s.open_total} open · {s.breached_open} breached
            </Footer>
          </>
        );
      }
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
      if (showTable) {
        return (
          <>
            <AttainmentDataTable rows={rows} />
            <Footer>
              <span style={{ color: `var(--m-status-${attainmentTone(s.attainment_pct)}-fg)` }}>
                {s.attainment_pct}%
              </span>{" "}
              attainment over {s.resolutions_total} resolutions
            </Footer>
          </>
        );
      }
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
      const rows = firstResponseRows(page);
      const unanswered = rows.filter((r) => r.first_staff_comment_at === null);
      // Unanswered drilldown rides its own toggle (the stat card has no
      // header chart toggle; the drilldown table IS the a11y alternative).
      return (
        <UnansweredStatPanel
          summary={s}
          unanswered={unanswered}
        />
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

// --- keyboard data-table alternatives (Wave 17 lane 12) ------------------------
// Recharts tooltips are hover-only; every chart widget exposes the SAME rows
// as a plain Table the keyboard can traverse. Columns mirror the chart axes
// so chart and table read identically (Amplitude/Canva grammar — see
// design/qa-stragglers/DESIGN-RESEARCH.md).

function RollupDataTable({
  rows,
}: {
  rows: ReturnType<typeof rollupRows>;
}) {
  return (
    <Table aria-label="Hours per group as data">
      <thead>
        <tr>
          <Th>Group</Th>
          <Th className="text-right">Hours</Th>
          <Th className="text-right">Billable hours</Th>
          <Th className="text-right">Entries</Th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <Tr key={r.group_id}>
            <Td>
              <a
                href={buildWidgetRowLink(rollupRowSpec(r.group_id))}
                data-testid={`widget-row-link-${r.group_id}`}
                className={r.group_id !== "" ? "hover:underline" : undefined}
                style={{ color: "var(--m-text)" }}
                aria-label={`Open ${r.label !== "" ? r.label : "ungrouped"} time entries in the report builder`}
              >
                {r.label !== "" ? r.label : r.group_id.slice(0, 8)}
              </a>
            </Td>
            <Td className="text-right font-mono tabular-nums">{formatMinutes(r.metric)}</Td>
            <Td className="text-right font-mono tabular-nums">{formatMinutes(r.billable_minutes)}</Td>
            <Td className="text-right font-mono tabular-nums">{r.entry_count}</Td>
          </Tr>
        ))}
      </tbody>
    </Table>
  );
}

function ThroughputDataTable({
  rows,
}: {
  rows: ReturnType<typeof throughputRows>;
}) {
  return (
    <Table aria-label="Created vs resolved per day as data">
      <thead>
        <tr>
          <Th>Day</Th>
          <Th className="text-right">Created</Th>
          <Th className="text-right">Resolved</Th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <Tr key={r.day}>
            <Td>
              <a
                href={buildWidgetRowLink(throughputRowSpec(r.day))}
                data-testid={`widget-row-link-${r.day}`}
                className="hover:underline"
                style={{ color: "var(--m-text)" }}
                aria-label={`Open ${r.day.slice(0, 10)} time entries in the report builder`}
              >
                {r.day}
              </a>
            </Td>
            <Td className="text-right font-mono tabular-nums">{r.created_count}</Td>
            <Td className="text-right font-mono tabular-nums">{r.resolved_count}</Td>
          </Tr>
        ))}
      </tbody>
    </Table>
  );
}

function AgingDataTable({
  rows,
}: {
  rows: ReturnType<typeof agingRows>;
}) {
  return (
    <Table aria-label="Open tickets per age bucket as data">
      <thead>
        <tr>
          <Th>Age bucket</Th>
          <Th className="text-right">Open</Th>
          <Th className="text-right">Avg age</Th>
          <Th className="text-right">Oldest</Th>
        </tr>
      </thead>
      <tbody>
        {[...rows]
          .sort((a, b) => a.bucket_ordinal - b.bucket_ordinal)
          .map((r) => (
            <Tr key={r.bucket}>
              <Td>{AGING_BUCKET_LABELS[r.bucket] ?? r.bucket}</Td>
              <Td className="text-right font-mono tabular-nums">{r.open_count}</Td>
              <Td className="text-right font-mono tabular-nums">{r.avg_age_days}d</Td>
              <Td className="text-right font-mono tabular-nums">{r.oldest_age_days}d</Td>
            </Tr>
          ))}
      </tbody>
    </Table>
  );
}

function AttainmentDataTable({
  rows,
}: {
  rows: ReturnType<typeof slaAttainmentRows>;
}) {
  return (
    <Table aria-label="Attainment percent per policy as data">
      <thead>
        <tr>
          <Th>Policy</Th>
          <Th className="text-right">Attainment</Th>
          <Th className="text-right">Met</Th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <Tr key={r.group_id}>
            <Td>
              <a
                href={buildWidgetRowLink(attainmentRowSpec(r.group_id))
                }
                data-testid={`widget-row-link-${r.group_id}`}
                className={r.group_id !== "" ? "hover:underline" : undefined}
                style={{ color: "var(--m-text)" }}
                aria-label={`Open ${r.label !== "" ? r.label : "ungrouped"} policy outcomes in the report builder`}
              >
                {r.label !== "" ? r.label : r.group_id.slice(0, 8)}
              </a>
            </Td>
            <Td className="text-right font-mono tabular-nums">{r.attainment_pct}%</Td>
            <Td className="text-right font-mono tabular-nums">{r.resolutions_met}</Td>
          </Tr>
        ))}
      </tbody>
    </Table>
  );
}

// --- first-response drilldown ---------------------------------------------------

/**
 * The First response stat row with the Unanswered drilldown: the Unanswered
 * figure is an expanded-state toggle that opens the unanswered-tickets view —
 * the widget's own rows filtered to those still awaiting a first staff
 * comment, each deep-linking to the ticket. No separate endpoint: the report
 * the widget already fetched carries the per-ticket rows.
 */
function UnansweredStatPanel({
  summary,
  unanswered,
}: {
  summary: ReturnType<typeof firstResponseSummary>;
  unanswered: ReturnType<typeof firstResponseRows>;
}) {
  const [expanded, setExpanded] = useState(false);
  return (
    <>
      <div className="grid grid-cols-3 gap-3 max-w-xl">
        <Stat label="Median response" value={formatSeconds(summary.response_p50_secs)} />
        <Stat label="90th percentile" value={formatSeconds(summary.response_p90_secs)} />
        <div>
          <button
            type="button"
            aria-pressed={expanded}
            aria-expanded={expanded}
            data-testid="unanswered-drilldown"
            onClick={() => setExpanded((v) => !v)}
            className="grid w-full gap-0.5 cursor-pointer rounded-[var(--m-radius-container)] border-0 bg-transparent p-0 text-left outline-offset-4"
          >
            <span className="text-[10px] font-medium uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Unanswered
            </span>
            <span className="flex items-baseline gap-1">
              <span
                className="font-mono text-lg font-semibold tabular-nums hover:underline"
                style={{
                  color:
                    summary.unanswered_count > 0
                      ? "var(--m-status-warning-fg)"
                      : "var(--m-text)",
                }}
              >
                {summary.unanswered_count}
              </span>
              <span
                className="text-[10px] transition-transform duration-100"
                style={{ color: "var(--m-text-muted)", transform: expanded ? "rotate(90deg)" : "rotate(0deg)" }}
                aria-hidden="true"
              >
                ▸
              </span>
            </span>
          </button>
        </div>
      </div>
      {expanded && (
        <UnansweredTicketsTable
          rows={unanswered}
          total={summary.unanswered_count}
        />
      )}
      <Footer>
        {summary.answered_count} of {summary.tickets_total} answered
      </Footer>
    </>
  );
}

function UnansweredTicketsTable({
  rows,
  total,
}: {
  rows: ReturnType<typeof firstResponseRows>;
  total: number;
}) {
  const truncated = rows.length < total;
  return (
    <div>
      <Table aria-label="Tickets awaiting first response">
        <thead>
          <tr>
            <Th>Ticket</Th>
            <Th>Created</Th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <Tr key={r.ticket_id}>
              <Td>
                <a
                  href={`/tickets/${r.ticket_id}`}
                  className="font-mono hover:underline"
                  style={{ color: "var(--m-accent)" }}
                >
                  {r.ticket_number}
                </a>
              </Td>
              <Td>{r.created_at === "" ? "—" : r.created_at.slice(0, 10)}</Td>
            </Tr>
          ))}
        </tbody>
      </Table>
      {truncated && (
        <p className="pt-1 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
          Showing {rows.length} of {total} unanswered (page-1 rows of the report).
        </p>
      )}
    </div>
  );
}
