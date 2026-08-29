// Unit tests: dashboards domain model — catalog integrity, preset/RBAC
// rules, settings-registry payload codec, add/remove, display formatting.
import { describe, expect, it } from "vitest";
import {
  AGING_BUCKET_LABELS,
  PRESETS,
  REPORT_KINDS,
  WIDGET_CATALOG,
  WIDGET_ROWS_KEY,
  addWidget,
  decodeWidgetRows,
  defaultRows,
  encodeWidgetRows,
  formatMinutes,
  formatSeconds,
  isManagerPermissions,
  removeWidget,
  visiblePresets,
  widgetDef,
  widgetVisible,
} from "../../lib/dashboards";

const WIZARD_VIZ = new Set(["bar", "line", "table", "stat"]);

describe("widget catalog", () => {
  it("maps every widget to a shipped report kind with a valid shape", () => {
    for (const w of WIDGET_CATALOG) {
      expect(REPORT_KINDS).toContain(w.kind);
      expect(w.id).toBe(`${w.kind}:${w.lens}`);
      expect(WIZARD_VIZ.has(w.viz)).toBe(true);
      expect(w.title.length).toBeGreaterThan(0);
      expect(w.empty.length).toBeGreaterThan(0);
    }
  });

  it("resolves ids through widgetDef and nothing else", () => {
    expect(widgetDef("time_by_user:self")?.title).toBe("My time");
    expect(widgetDef("time_by_user:org")?.title).toBe("Time by person");
    expect(widgetDef("made_up:self")).toBeUndefined();
  });
});

describe("default presets", () => {
  it("reference known widget ids", () => {
    for (const preset of PRESETS) {
      for (const id of preset.widgets) {
        expect(widgetDef(id), `${preset.id} references ${id}`).toBeDefined();
      }
    }
  });

  it("default rows are the My day preset (defaults-first)", () => {
    expect(defaultRows()).toEqual(PRESETS[0]?.widgets);
    expect(PRESETS.map((p) => p.title)).toEqual(["My day", "Team load", "SLA health"]);
  });
});

describe("RBAC seam", () => {
  it("matches the server's owner-or-admin seam via manage_users", () => {
    expect(isManagerPermissions(["special:manage_users"])).toBe(true);
    expect(isManagerPermissions(["special:manage_users", "tickets:view"])).toBe(true);
    expect(isManagerPermissions(["special:manage_boards"])).toBe(false);
    expect(isManagerPermissions([])).toBe(false);
  });

  it("org widgets are manager-only; personal widgets are universal", () => {
    const self = widgetDef("time_by_user:self");
    const org = widgetDef("time_by_user:org");
    expect(self && widgetVisible(self, false)).toBe(true);
    expect(org && widgetVisible(org, false)).toBe(false);
    expect(org && widgetVisible(org, true)).toBe(true);
  });

  it("non-managers only see the My day preset", () => {
    expect(visiblePresets(true).map((p) => p.id)).toEqual(["my_day", "team_load", "sla_health"]);
    expect(visiblePresets(false).map((p) => p.id)).toEqual(["my_day"]);
  });
});

describe("widget rows persistence payload", () => {
  it("round-trips through the registry Text value", () => {
    const rows = ["time_by_user:self", "sla_breaches:org"];
    const decoded = decodeWidgetRows(encodeWidgetRows(rows));
    expect(decoded).toEqual(rows);
  });

  it("uses the settings-registry key with the 2.5 conventions", () => {
    expect(WIDGET_ROWS_KEY).toBe("dashboards.widgets");
    const payload = JSON.parse(encodeWidgetRows(["time_by_user:self"])) as {
      version: number;
      widgets: string[];
    };
    expect(payload.version).toBe(1);
    expect(payload.widgets).toEqual(["time_by_user:self"]);
  });

  it("drops unknown, duplicate and stale ids while preserving order", () => {
    const stored = JSON.stringify({
      version: 1,
      widgets: ["time_by_user:self", "nope:goneself", "time_by_user:self", "ticket_aging:org"],
    });
    expect(decodeWidgetRows(stored)).toEqual(["time_by_user:self", "ticket_aging:org"]);
  });

  it("returns null for anything not a stored v1 payload", () => {
    expect(decodeWidgetRows(null)).toBeNull();
    expect(decodeWidgetRows(undefined)).toBeNull();
    expect(decodeWidgetRows(42)).toBeNull();
    expect(decodeWidgetRows("not json")).toBeNull();
    expect(decodeWidgetRows(JSON.stringify({ version: 2, widgets: [] }))).toBeNull();
    expect(decodeWidgetRows(JSON.stringify({ version: 1, widgets: "nope" }))).toBeNull();
  });

  it("addWidget appends once and rejects unknown or duplicate ids", () => {
    expect(addWidget(["time_by_user:self"], "sla_breaches:org")).toEqual([
      "time_by_user:self",
      "sla_breaches:org",
    ]);
    expect(addWidget(["time_by_user:self"], "time_by_user:self")).toBeNull();
    expect(addWidget([], "made_up:self")).toBeNull();
  });

  it("removeWidget keeps the remaining order", () => {
    expect(removeWidget(["a", "b", "c"], "b")).toEqual(["a", "c"]);
  });
});

describe("display formatting", () => {
  it("formats minutes as compact hours", () => {
    expect(formatMinutes(45)).toBe("45m");
    expect(formatMinutes(60)).toBe("1h");
    expect(formatMinutes(210)).toBe("3h 30m");
    expect(formatMinutes(0)).toBe("0m");
  });

  it("formats seconds as compact durations", () => {
    expect(formatSeconds(42)).toBe("42s");
    expect(formatSeconds(260)).toBe("4m 20s");
    expect(formatSeconds(3600)).toBe("1h");
    expect(formatSeconds(4320)).toBe("1h 12m");
    expect(formatSeconds(0)).toBe("0s");
  });

  it("covers every aging bucket with a presentation label", () => {
    for (const bucket of ["under_1d", "d1_3", "d3_7", "d7_30", "over_30d"]) {
      expect(AGING_BUCKET_LABELS[bucket]).toBeDefined();
    }
  });
});
