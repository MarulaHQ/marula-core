// Report builder v1 logic (the private build tracker R6; ADR 0001).
//
// Purpose: the typed client-side half of the thin in-app builder. Draft
// spec state, catalog-driven option lists, and the aggregate/operand
// compatibility rules MIRROR crates/core-reports (spec.rs validate) — the
// server re-validates everything, so this layer exists only to make the
// invalid states unrepresentable in the form before a round-trip.
//
// Non-purpose: no SQL text, no execution — runs ride
// POST /api/v1/reports/run-spec (inline preview) and
// GET /api/v1/report-definitions/{id}/run (saved reports), compiled
// server-side by the Rust compiler exclusively.
import type { components } from "@marula-core/sdk";

type ViewId = components["schemas"]["ViewId"];
type VizHint = components["schemas"]["VizHint"];
type Aggregate = components["schemas"]["Aggregate"];
type FilterOp = components["schemas"]["FilterOp"];
type FilterValue = components["schemas"]["FilterValue"];
type ReportSpec = components["schemas"]["ReportSpec"];
type CatalogView = components["schemas"]["CatalogView"];

export const VIEW_LABELS: Record<ViewId, string> = {
  tickets: "Tickets",
  time_entries: "Time entries",
  sla_outcomes: "SLA outcomes",
  gap_days: "Coverage gaps",
  first_response: "First response",
};

export const AGG_LABELS: Record<Aggregate, string> = {
  count: "Count",
  sum: "Sum",
  avg: "Average",
  min: "Earliest",
  max: "Latest",
};

export const OP_LABELS: Record<FilterOp, string> = {
  eq: "is",
  not_eq: "is not",
  lt: "before",
  lt_eq: "at or before",
  gt: "after",
  gt_eq: "at or after",
  is_null: "is empty",
  is_not_null: "is set",
};

/** Caps mirror core-reports spec.rs. */
export const MAX_FILTERS = 8;
export const MAX_GROUP_BY = 3;
export const MAX_MEASURES = 4;

/** Aggregate → value classes it accepts (mirror of Aggregate::accepts). */
export function aggAccepts(agg: Aggregate, valueType: string): boolean {
  switch (agg) {
    case "count":
      return true;
    case "sum":
    case "avg":
      return valueType === "int";
    // Min/max: order statistics over numerics and ordered temporal types;
    // text excluded on purpose (mirrors the compiler).
    case "min":
    case "max":
      return valueType === "int" || valueType === "timestamp" || valueType === "date";
  }
}

/** Group-by candidates: every projected column except tenancy columns. */
export function groupableColumns(view: CatalogView): CatalogView["columns"] {
  return view.columns.filter((c) => c.name !== "org_id");
}

export interface MeasureDraft {
  agg: Aggregate;
  column: string | null;
}

export interface FilterDraft {
  column: string;
  op: FilterOp;
  /** Raw operand text; parsed into the typed FilterValue at build time. */
  rawValue: string;
}

export interface BuilderDraft {
  view: ViewId;
  filters: FilterDraft[];
  groupBy: string[];
  measures: MeasureDraft[];
  viz: VizHint;
  /** Inclusive UTC day bounds (YYYY-MM-DD); empty = server default window. */
  from: string;
  to: string;
}

export function emptyDraft(view: ViewId = "tickets"): BuilderDraft {
  return {
    view,
    filters: [],
    groupBy: [],
    measures: [{ agg: "count", column: null }],
    viz: "bar",
    from: "",
    to: "",
  };
}

/** The typed operand a draft filter's value parses into, per column class. */
export function operandType(valueType: string): "uuid" | "text" | "timestamp" | "date" | "int" | "bool" {
  switch (valueType) {
    case "uuid":
      return "uuid";
    case "timestamp":
      return "timestamp";
    case "date":
      return "date";
    case "int":
      return "int";
    case "bool":
      return "bool";
    default:
      return "text";
  }
}

/** Parses the raw operand text into the wire FilterValue variant; `null`
 * when the text is not (yet) a valid operand — the filter row shows the
 * field-level problem instead of sending a rejected spec. */
export function parseOperand(valueType: string, raw: string): FilterValue | null {
  const text = raw.trim();
  switch (operandType(valueType)) {
    case "uuid":
      return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(text)
        ? { uuid: text }
        : null;
    case "date":
      return /^\d{4}-\d{2}-\d{2}$/.test(text) ? { date: text } : null;
    case "timestamp":
      if (text === "") return null;
      // Accept a bare datetime-local value; require a parseable instant.
      {
        const iso = /\d{2}:\d{2}/.test(text) && !/[zZ+]|[zZ]$/.test(text) ? `${text}:00Z` : text;
        const t = new Date(iso);
        return Number.isNaN(t.getTime()) ? null : { timestamp: t.toISOString() };
      }
    case "int": {
      if (!/^-?\d+$/.test(text)) return null;
      const n = Number(text);
      return Number.isSafeInteger(n) ? { int: n } : null;
    }
    case "bool":
      return text === "true" ? { bool: true } : text === "false" ? { bool: false } : null;
    default:
      return text === "" ? null : { text };
  }
}

/** Local mirror of ReportSpec::validate — returns the FIRST problem (or
 * null when the draft is sendable). Copy phrasing names the fix. */
export function draftProblem(draft: BuilderDraft, view: CatalogView | undefined): string | null {
  if (view === undefined) return null;
  const byName = new Map(view.columns.map((c) => [c.name, c.value_type]));
  if (draft.measures.length === 0) return "Add at least one measure.";
  if (draft.measures.length > MAX_MEASURES) return `At most ${MAX_MEASURES} measures.`;
  for (const m of draft.measures) {
    if (m.agg === "count") continue;
    if (m.column === null || m.column === "") return `${AGG_LABELS[m.agg]} needs a column.`;
    const vt = byName.get(m.column);
    if (vt === undefined) return `Column ${m.column} is not on this view.`;
    if (!aggAccepts(m.agg, vt)) return `${AGG_LABELS[m.agg]} does not apply to ${m.column}.`;
  }
  if (draft.groupBy.length > MAX_GROUP_BY) return `At most ${MAX_GROUP_BY} groupings.`;
  for (const g of draft.groupBy) {
    if (g === "org_id") return "org_id cannot be a grouping.";
    if (!byName.has(g)) return `Column ${g} is not on this view.`;
  }
  if (draft.filters.length > MAX_FILTERS) return `At most ${MAX_FILTERS} filters.`;
  const filterable = new Map(view.filterable.map((c) => [c.name, c.value_type]));
  for (const f of draft.filters) {
    const vt = filterable.get(f.column);
    if (vt === undefined) return `${f.column} is not filterable on this view.`;
    const takesValue = f.op !== "is_null" && f.op !== "is_not_null";
    if (!takesValue) continue;
    if (parseOperand(vt, f.rawValue) === null) {
      return `Filter ${f.column}: enter a valid ${operandType(vt)} value.`;
    }
  }
  return null;
}

/** Builds the wire spec from a validated draft. Measure output names ride
 * the server's scheme (agg_slug + column, bare count) only for labels. */
export function draftToSpec(draft: BuilderDraft, viewCatalog?: CatalogView): ReportSpec {
  const vt = (name: string): string =>
    viewCatalog?.columns.find((c) => c.name === name)?.value_type ?? "text";
  return {
    view: draft.view,
    filters: draft.filters
      .filter((f) => f.op === "is_null" || f.op === "is_not_null" || f.rawValue.trim() !== "")
      .map((f) => {
        const filterVt = vt(f.column);
        return {
          column: f.column,
          op: f.op,
          value:
            f.op === "is_null" || f.op === "is_not_null" || f.rawValue.trim() === ""
              ? undefined
              : parseOperand(filterVt, f.rawValue) ?? undefined,
        };
      }),
    group_by: [...draft.groupBy],
    measures: draft.measures.map((m) => ({
      agg: m.agg,
      column: m.agg === "count" && m.column === null ? undefined : m.column ?? undefined,
    })),
    viz: draft.viz,
  };
}

export function measureLabel(m: MeasureDraft): string {
  if (m.agg === "count" && (m.column === null || m.column === "")) return "count";
  return `${m.agg}_${m.column ?? ""}`;
}

/** Chart/table data shaping for the preview. Rows are objects keyed by
 * output column name (group-by dims + measure output names). */
export interface PreviewSeries {
  labels: string[];
  /** One numeric series per measure. */
  series: { name: string; values: number[] }[];
}

export function previewSeries(page: {
  rows: Record<string, unknown>[];
  columns: string[];
}, measures: MeasureDraft[]): PreviewSeries {
  const dims = page.columns.filter((c) => !measures.some((m) => measureLabel(m) === c));
  const labelDim = dims[0] ?? null;
  const labels = page.rows.map((r, i) => {
    const v = labelDim === null ? "" : r[labelDim];
    if (typeof v === "string" && v !== "") return v;
    if (labelDim === null) return "total";
    return `row ${i + 1}`;
  });
  const series = measures.map((m) => ({
    name: measureLabel(m),
    values: page.rows.map((r) => {
      const n = r[measureLabel(m)];
      const num = typeof n === "string" ? Number(n) : n;
      return typeof num === "number" && Number.isFinite(num) ? num : 0;
    }),
  }));
  return { labels, series };
}

export function labelize(name: string): string {
  return name
    .replace(/_/g, " ")
    .replace(/\b\w/g, (c) => c.toUpperCase());
}
