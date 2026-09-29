// Wave 23 lane 1, items 2+3: the /reports?spec= codec — round trip, tolerant
// decode, and the widget→spec mapping uses only catalog-valid vocabulary.
import { describe, expect, it } from "vitest";
import type { components } from "@marula-core/sdk";
import {
  attainmentRowSpec,
  decodeSpecFromQuery,
  encodeSpecToQuery,
  rollupRowSpec,
  throughputRowSpec,
  widgetSpec,
} from "../../lib/report-spec-codec";
import { WIDGET_CATALOG } from "../../lib/dashboards";

type ReportSpec = components["schemas"]["ReportSpec"];

const SPEC: ReportSpec = {
  view: "time_entries",
  filters: [{ column: "user_id", op: "eq", value: { uuid: "00000000-0000-4000-8000-00000000000a" } }],
  group_by: ["client_id"],
  measures: [{ agg: "sum", column: "minutes" }],
  viz: "bar",
};

describe("report-spec codec", () => {
  it("round-trips spec + bounds through base64url", () => {
    const q = encodeSpecToQuery(SPEC, { from: "2026-09-01", to: "2026-09-19" });
    expect(q).not.toMatch(/[+/=]/);
    const back = decodeSpecFromQuery(q);
    expect(back?.spec).toEqual(SPEC);
    expect(back?.from).toBe("2026-09-01");
    expect(back?.to).toBe("2026-09-19");
  });

  it("decodes garbage and wrong payloads to null", () => {
    expect(decodeSpecFromQuery(undefined)).toBeNull();
    expect(decodeSpecFromQuery("!!!!not-base64")).toBeNull();
    expect(decodeSpecFromQuery(encodeSpecToQuery({ } as unknown as ReportSpec))).toBeNull();
    expect(decodeSpecFromQuery(encodeSpecToQuery({ ...SPEC, view: "nope" } as unknown as ReportSpec))).toBeNull();
    expect(decodeSpecFromQuery(encodeSpecToQuery({ ...SPEC, measures: [] } as unknown as ReportSpec))).toBeNull();
    expect(decodeSpecFromQuery(encodeSpecToQuery(SPEC, { from: "nope" }))?.from).toBeUndefined();
  });

  it("maps every widget to catalog-valid drill-down specs", () => {
    for (const def of WIDGET_CATALOG) {
      const dl = widgetSpec(def);
      expect(VIEWS.has(dl.spec.view)).toBe(true);
      expect(dl.spec.measures.length).toBeGreaterThan(0);
      for (const f of dl.spec.filters ?? []) {
        expect(FILTERABLE[dl.spec.view]).toContain(f.column);
      }
      for (const g of dl.spec.group_by ?? []) {
        expect(g).not.toBe("org_id");
      }
    }
  });

  it("row-level deep links pin the row's grouping/dimension", () => {
    expect(rollupRowSpec("00000000-0000-4000-8000-0000000000aa").spec.filters?.[0]).toEqual({
      column: "user_id",
      op: "eq",
      value: { uuid: "00000000-0000-4000-8000-0000000000aa" },
    });
    expect(throughputRowSpec("2026-09-18T20:00:00Z").spec.filters?.[0]).toEqual({
      column: "day_utc",
      op: "eq",
      value: { date: "2026-09-18" },
    });
    expect(attainmentRowSpec("00000000-0000-4000-8000-00000000p001").spec.view).toBe("sla_outcomes");
  });
});

const VIEWS = new Set(["tickets", "time_entries", "sla_outcomes", "gap_days", "first_response"]);
const FILTERABLE: Record<string, string[]> = {
  tickets: ["board_id", "client_id", "site_id", "priority", "category"],
  time_entries: ["ticket_id", "user_id", "client_id", "billable", "work_type", "day_utc"],
  sla_outcomes: ["policy_id", "assignee_id", "priority", "breached", "response_met", "resolution_met"],
  gap_days: ["user_id", "has_gap"],
  first_response: ["client_id", "assignee_id"],
};
