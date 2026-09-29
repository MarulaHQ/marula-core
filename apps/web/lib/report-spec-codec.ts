// Report-spec codec + widget drill-down mapping (Wave 23 lane 1, items 2+3).
//
// One base64url codec ("r" JSON payload) carries a ReportSpec plus optional
// inclusive UTC day bounds through a `/reports?spec=` deep link, and every
// dashboard widget maps to a spec the R3 engine actually serves (columns and
// filters mirror crates/core-reports/src/catalog.rs — user_id/client_id/
// policy_id assignee_id are real columns; day_utc/billable are filterable on
// time_entries; breached is filterable on sla_outcomes).
//
// Non-purpose: no fetching (reports.ts owns transport), no rendering.
"use client";

import type { components } from "@marula-core/sdk";
import type { WidgetDef } from "./dashboards";

type ReportSpec = components["schemas"]["ReportSpec"];
type FilterValue = components["schemas"]["FilterValue"];

const VERSION = 1;

export interface DeepLinkSpec {
  spec: ReportSpec;
  /** Inclusive UTC day bounds (YYYY-MM-DD); absent = server default window. */
  from?: string;
  to?: string;
}

export function encodeSpecToQuery(
  spec: ReportSpec,
  bounds?: { from?: string; to?: string },
): string {
  const payload = { v: VERSION, spec, from: bounds?.from ?? "", to: bounds?.to ?? "" };
  return base64UrlUrlSafe(JSON.stringify(payload));
}

/** Tolerant decode: wrong shapes return null (caller leaves the builder alone). */
export function decodeSpecFromQuery(encoded: string | undefined | null): DeepLinkSpec | null {
  if (typeof encoded !== "string" || encoded === "") return null;
  let json: string | null = null;
  try {
    json = decodeBase64UrlSafe(encoded);
  } catch {
    return null;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(json);
  } catch {
    return null;
  }
  if (parsed === null || typeof parsed !== "object") return null;
  const p = parsed as { v?: unknown; spec?: unknown; from?: unknown; to?: unknown };
  if (p.v !== VERSION || !isReportSpec(p.spec)) return null;
  const spec = p.spec as unknown as ReportSpec;
  return {
    spec,
    from: typeof p.from === "string" && /^\d{4}-\d{2}-\d{2}$/.test(p.from) ? p.from : undefined,
    to: typeof p.to === "string" && /^\d{4}-\d{2}-\d{2}$/.test(p.to) ? p.to : undefined,
  };
}

function isReportSpec(x: unknown): boolean {
  if (x === null || typeof x !== "object") return false;
  const s = x as Partial<ReportSpec>;
  return (
    typeof s.view === "string" &&
    Object.keys(VIEW_SET).includes(s.view as string) &&
    typeof s.viz === "string" &&
    Object.keys(VIZ_SET).includes(s.viz as string) &&
    (s.group_by === undefined || isStringArray(s.group_by)) &&
    isSpecFilters(s.filters) &&
    isSpecMeasures(s.measures)
  );
}

const VIEW_SET: Record<string, true> = {
  tickets: true,
  time_entries: true,
  sla_outcomes: true,
  gap_days: true,
  first_response: true,
};
const VIZ_SET: Record<string, true> = { bar: true, line: true, table: true, stat: true };

function isStringArray(a: unknown): boolean {
  return Array.isArray(a) && a.every((x) => typeof x === "string");
}

function isSpecFilters(f: unknown): boolean {
  if (f === undefined) return true;
  if (!Array.isArray(f)) return false;
  return f.every(
    (x) =>
      x !== null &&
      typeof x === "object" &&
      typeof (x as { column?: unknown }).column === "string" &&
      typeof (x as { op?: unknown }).op === "string",
  );
}

function isSpecMeasures(m: unknown): boolean {
  if (!Array.isArray(m) || m.length === 0) return false;
  return m.every(
    (x) =>
      x !== null &&
      typeof x === "object" &&
      typeof (x as { agg?: unknown }).agg === "string",
  );
}

function base64UrlUrlSafe(input: string): string {
  const bytes = new TextEncoder().encode(input);
  const chunks: string[] = [];
  for (let i = 0; i < bytes.length; i += 0x8000) {
    chunks.push(String.fromCharCode(...bytes.subarray(i, i + 0x8000)));
  }
  return btoa(chunks.join("")).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function decodeBase64UrlSafe(input: string): string {
  const b64 = input.replace(/-/g, "+").replace(/_/g, "/");
  const bin = atob(b64 + "=".repeat((4 - (b64.length % 4)) % 4));
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i += 1) bytes[i] = bin.charCodeAt(i);
  return new TextDecoder().decode(bytes);
}

// --- widget → spec -------------------------------------------------------------

/** The drill-down spec per widget (R3 vocabulary — see module docs). */
export function widgetSpec(def: WidgetDef): DeepLinkSpec {
  switch (def.id) {
    case "time_by_user:self":
    case "time_by_user:org": {
      const spec: ReportSpec = {
        view: "time_entries",
        filters: [{ column: "user_id", op: "is_not_null" }],
        group_by: ["user_id"],
        measures: [{ agg: "sum", column: "minutes" }],
        viz: "bar",
      };
      return { spec };
    }
    case "time_by_client:org":
      return {
        spec: {
          view: "time_entries",
          filters: [{ column: "client_id", op: "is_not_null" }],
          group_by: ["client_id"],
          measures: [{ agg: "sum", column: "minutes" }],
          viz: "bar",
        },
      };
    case "ticket_throughput:self":
    case "ticket_throughput:org":
      return { spec: throughputSpec() };
    case "ticket_aging:self":
    case "ticket_aging:org":
      return { spec: { view: "tickets", measures: [{ agg: "count" }], viz: "table" } };
    case "sla_attainment:self":
    case "sla_attainment:org":
      return {
        spec: {
          view: "sla_outcomes",
          group_by: ["policy_id"],
          measures: [{ agg: "count" }],
          viz: "bar",
        },
      };
    case "sla_breaches:org":
      return {
        spec: {
          view: "sla_outcomes",
          filters: [{ column: "breached", op: "eq", value: { bool: true } }],
          group_by: ["policy_id"],
          measures: [{ agg: "count" }],
          viz: "bar",
        },
      };
    case "first_response:self":
    case "first_response:org":
      return {
        spec: {
          view: "first_response",
          group_by: ["assignee_id"],
          measures: [{ agg: "count" }],
          viz: "bar",
        },
      };
    default:
      return { spec: { view: "tickets", measures: [{ agg: "count" }], viz: "table" } };
  }
}

/** The deep-link href for a widget's drill-down spec. */
export function buildWidgetDeepLink(def: WidgetDef): string {
  const { spec, from, to } = widgetSpec(def);
  return `/reports?spec=${encodeSpecToQuery(spec, { from, to })}`;
}

/** Href for a row-level deep link (already-built DeepLinkSpec). */
export function buildWidgetRowLink(
  link: DeepLinkSpec,
): string {
  return `/reports?spec=${encodeSpecToQuery(link.spec, { from: link.from, to: link.to })}`;
}

/** Throughput's day spine has no single-expansion through the spec engine —
 * the builder shows the tickets timeline via the day bucket filter instead. */
export function throughputSpec(): ReportSpec {
  return { view: "tickets", measures: [{ agg: "count" }], viz: "bar" };
}

// --- row-level deep links ------------------------------------------------------

/** Pin a rollup row's grouping (user/client) as an eq filter. */
export function rollupRowSpec(
  groupId: string,
): DeepLinkSpec {
  return {
    spec: {
      view: "time_entries",
      filters: [{ column: "user_id", op: "eq", value: { uuid: groupId } }],
      measures: [{ agg: "sum", column: "minutes" }],
      viz: "bar",
    },

  };
}

/** Throughput rows pin the UTC day as a time_entries day_utc-eq filter. */
export function throughputRowSpec(day: string): DeepLinkSpec {
  return {
    spec: {
      view: "time_entries",
      filters: [{ column: "day_utc", op: "eq", value: { date: day.slice(0, 10) } }],
      group_by: ["user_id"],
      measures: [{ agg: "sum", column: "minutes" }],
      viz: "bar",
    },
  };
}

/** Attainment rows pin the policy. */
export function attainmentRowSpec(groupId: string): DeepLinkSpec {
  return {
    spec: {
      view: "sla_outcomes",
      filters: [{ column: "policy_id", op: "eq", value: { uuid: groupId } }],
      measures: [{ agg: "count" }],
      viz: "table",
    },
  };
}
