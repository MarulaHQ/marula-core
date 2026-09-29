// Report data access for dashboards (TASKLIST R2).
//
// Purpose: typed reads of the shipped R1 API (GET /api/v1/reports/{kind})
// through the generated OpenAPI client, plus CSV export of the SAME report
// via `format=csv`. CSV is a text/csv stream, not a JSON envelope, so it
// bypasses openapi-fetch exactly like attachments do in lib/api.tsx — the
// URL literal stays `satisfies`-anchored to the canonical contract.
//
// Non-purpose: no widget model (dashboards.ts), no rendering.
import type { MarulaPaths } from "@marula-core/sdk";
import { api, apiBaseUrl, loadTokens } from "./api";
import type { ReportKind } from "./dashboards";

const REPORTS_PATH = "/api/v1/reports/{kind}" satisfies keyof MarulaPaths;

/** Widget fetch page size; the API clamps to [1, 500]. */
const WIDGET_PAGE_LIMIT = 100;

export interface ReportPage {
  columns: string[];
  kind: string;
  view: string;
  next_cursor?: string | null;
  rows: Record<string, unknown>[];
  summary: Record<string, unknown>;
}

export type ReportQuery = {
  from?: string | null;
  to?: string | null;
  user_id?: string | null;
  billable?: boolean | null;
  policy_id?: string | null;
  as_of?: string | null;
};

function num(value: unknown): number {
  const n = typeof value === "string" ? Number(value) : value;
  return typeof n === "number" && Number.isFinite(n) ? n : 0;
}

function str(value: unknown): string {
  return typeof value === "string" ? value : "";
}

// --- per-kind row/summary shapes (mirror crates/core-reports columns) -------

export interface RollupRow {
  group_id: string;
  label: string;
  metric: number;
  billable_minutes: number;
  entry_count: number;
}

export interface TimeRollupSummary {
  entries_count: number;
  total_minutes: number;
  billable_minutes: number;
}

export interface ThroughputRow {
  day: string;
  created_count: number;
  resolved_count: number;
}

export interface ThroughputSummary {
  created_total: number;
  resolved_total: number;
}

export interface AgingRow {
  bucket: string;
  bucket_ordinal: number;
  open_count: number;
  oldest_age_days: number;
  avg_age_days: number;
}

export interface AgingSummary {
  open_total: number;
  oldest_age_days: number;
  breached_open: number;
}

export interface SlaAttainmentRow {
  group_id: string;
  label: string;
  metric: number;
  resolutions_met: number;
  attainment_pct: number;
}

export interface SlaAttainmentSummary {
  resolutions_total: number;
  resolutions_met: number;
  attainment_pct: number;
}

export interface SlaBreachRow {
  ticket_id: string;
  ticket_number: string;
  priority: string;
  policy_name: string;
  resolution_due: string;
  resolution_met: boolean;
}

export interface SlaBreachSummary {
  breaches_total: number;
  paused_business_secs_total: number;
}

export interface FirstResponseSummary {
  tickets_total: number;
  answered_count: number;
  unanswered_count: number;
  response_p50_secs: number;
  response_p90_secs: number;
}

export interface FirstResponseRow {
  ticket_id: string;
  ticket_number: string;
  created_at: string;
  first_staff_comment_at: string | null;
}

function pageOf(data: unknown): ReportPage {
  const page = data as ReportPage;
  return {
    ...page,
    rows: Array.isArray(page.rows) ? page.rows.filter((r) => typeof r === "object" && r !== null) : [],
    summary: typeof page.summary === "object" && page.summary !== null ? page.summary : {},
  };
}

/** Fetch one report page (JSON envelope) for a widget. */
export async function fetchReport(kind: ReportKind, query: ReportQuery): Promise<ReportPage> {
  const { data, error } = await api.api.GET(REPORTS_PATH, {
    params: {
      path: { kind },
      query: { ...query, limit: WIDGET_PAGE_LIMIT },
    },
  });
  if (error !== undefined) {
    const detail = (error as { error?: { message?: string } }).error?.message;
    throw new Error(detail ?? `Report ${kind} failed`);
  }
  return pageOf(data);
}

export const rollupRows = (page: ReportPage): RollupRow[] =>
  page.rows.map((r) => ({
    group_id: str(r.group_id),
    label: str(r.label),
    metric: num(r.metric),
    billable_minutes: num(r.billable_minutes),
    entry_count: num(r.entry_count),
  }));

export const timeRollupSummary = (page: ReportPage): TimeRollupSummary => ({
  entries_count: num(page.summary.entries_count),
  total_minutes: num(page.summary.total_minutes),
  billable_minutes: num(page.summary.billable_minutes),
});

export const throughputRows = (page: ReportPage): ThroughputRow[] =>
  page.rows.map((r) => ({
    day: str(r.day),
    created_count: num(r.created_count),
    resolved_count: num(r.resolved_count),
  }));

export const throughputSummary = (page: ReportPage): ThroughputSummary => ({
  created_total: num(page.summary.created_total),
  resolved_total: num(page.summary.resolved_total),
});

export const agingRows = (page: ReportPage): AgingRow[] =>
  page.rows.map((r) => ({
    bucket: str(r.bucket),
    bucket_ordinal: num(r.bucket_ordinal),
    open_count: num(r.open_count),
    oldest_age_days: num(r.oldest_age_days),
    avg_age_days: num(r.avg_age_days),
  }));

export const agingSummary = (page: ReportPage): AgingSummary => ({
  open_total: num(page.summary.open_total),
  oldest_age_days: num(page.summary.oldest_age_days),
  breached_open: num(page.summary.breached_open),
});

export const slaAttainmentRows = (page: ReportPage): SlaAttainmentRow[] =>
  page.rows.map((r) => ({
    group_id: str(r.group_id),
    label: str(r.label),
    metric: num(r.metric),
    resolutions_met: num(r.resolutions_met),
    attainment_pct: num(r.attainment_pct),
  }));

export const slaAttainmentSummary = (page: ReportPage): SlaAttainmentSummary => ({
  resolutions_total: num(page.summary.resolutions_total),
  resolutions_met: num(page.summary.resolutions_met),
  attainment_pct: num(page.summary.attainment_pct),
});

export const slaBreachRows = (page: ReportPage): SlaBreachRow[] =>
  page.rows.map((r) => ({
    ticket_id: str(r.ticket_id),
    ticket_number: str(r.ticket_number),
    priority: str(r.priority),
    policy_name: str(r.policy_name),
    resolution_due: str(r.resolution_due),
    resolution_met: r.resolution_met === true,
  }));

export const slaBreachSummary = (page: ReportPage): SlaBreachSummary => ({
  breaches_total: num(page.summary.breaches_total),
  paused_business_secs_total: num(page.summary.paused_business_secs_total),
});

export const firstResponseSummary = (page: ReportPage): FirstResponseSummary => ({
  tickets_total: num(page.summary.tickets_total),
  answered_count: num(page.summary.answered_count),
  unanswered_count: num(page.summary.unanswered_count),
  response_p50_secs: num(page.summary.response_p50_secs),
  response_p90_secs: num(page.summary.response_p90_secs),
});

/** Per-ticket first-response rows (row keys mirror crates/core-reports
 * catalog.rs FIRST_RESPONSE columns); rows carry the raw UTC timestamps. */
export const firstResponseRows = (page: ReportPage): FirstResponseRow[] =>
  page.rows.map((r) => ({
    ticket_id: str(r.ticket_id),
    ticket_number: str(r.ticket_number),
    created_at: str(r.created_at),
    first_staff_comment_at: typeof r.first_staff_comment_at === "string" ? r.first_staff_comment_at : null,
  }));


// --- CSV export ----------------------------------------------------------------

/** Query string for a CSV export, matching the JSON request it mirrors. */
export function reportCsvUrl(kind: ReportKind, query: ReportQuery): string {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value !== undefined && value !== null && value !== "") {
      params.set(key, String(value));
    }
  }
  params.set("format", "csv");
  const qs = params.toString();
  return `${apiBaseUrl()}${REPORTS_PATH.replace("{kind}", kind)}${qs ? `?${qs}` : ""}`;
}

/** Stream the same report as text/csv and trigger a browser download. */
export async function downloadReportCsv(
  kind: ReportKind,
  query: ReportQuery,
  filename: string,
): Promise<void> {
  const tokens = loadTokens();
  const response = await fetch(reportCsvUrl(kind, query), {
    headers: tokens === null ? {} : { authorization: `Bearer ${tokens.access_token}` },
  });
  if (!response.ok) throw new Error(`CSV export failed (${response.status})`);
  const blob = await response.blob();
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.click();
  URL.revokeObjectURL(url);
}
