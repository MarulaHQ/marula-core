// Unit tests: dashboards domain model — catalog integrity, preset/RBAC rules,
// the card vocabulary read from `GET /api/report-catalog`, the settings-registry
// payload codec (sizes and the version 1 migration), add/remove/resize/move,
// and display formatting.
import { describe, expect, it } from "vitest";
import {
  AGING_BUCKET_LABELS,
  PRESETS,
  REPORT_KINDS,
  WIDGET_CATALOG,
  WIDGET_ROWS_KEY,
  addWidget,
  cardVocabulary,
  decodeWidgets,
  defaultPlacements,
  defaultRows,
  defaultSizeFor,
  encodeWidgets,
  formatMinutes,
  formatSeconds,
  isCardSize,
  isManagerPermissions,
  moveWidget,
  placementsForPreset,
  placementsMatch,
  removeWidget,
  resizeWidget,
  shapeForViz,
  visiblePresets,
  widgetDef,
  widgetVisible,
} from "../../lib/dashboards";

const WIZARD_VIZ = new Set(["bar", "line", "table", "stat"]);

// The catalogue as `GET /api/report-catalog` serves it (crates/core-reports
// SHAPES), trimmed to the members this surface reads.
const CATALOG = {
  spans: ["quarter", "third", "half", "full"],
  heights: ["compact", "regular", "tall"],
  shapes: [
    { shape: "chart", viz: ["bar", "line", "pie"], default_span: "half", default_height: "regular" },
    { shape: "stat", viz: ["stat"], default_span: "third", default_height: "compact" },
    { shape: "table", viz: ["table"], default_span: "full", default_height: "tall" },
  ],
  card_rules: ["A card must draw something."],
};

const VOCAB = cardVocabulary(CATALOG);

function vocabulary() {
  if (VOCAB === null) throw new Error("fixture catalogue did not narrow");
  return VOCAB;
}

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

describe("card vocabulary from the catalogue payload", () => {
  it("keeps the lists the server sent, in its order", () => {
    const vocab = vocabulary();
    expect(vocab.spans).toEqual(["quarter", "third", "half", "full"]);
    expect(vocab.heights).toEqual(["compact", "regular", "tall"]);
    expect(vocab.rules).toEqual(["A card must draw something."]);
  });

  it("refuses a payload that cannot drive a size control", () => {
    expect(cardVocabulary(null)).toBeNull();
    expect(cardVocabulary("catalog")).toBeNull();
    expect(cardVocabulary({ heights: ["regular"], shapes: CATALOG.shapes })).toBeNull();
    expect(cardVocabulary({ spans: [], heights: ["regular"], shapes: CATALOG.shapes })).toBeNull();
    expect(cardVocabulary({ spans: ["half"], heights: ["regular"] })).toBeNull();
    expect(
      cardVocabulary({ spans: [1], heights: ["regular"], shapes: CATALOG.shapes }),
    ).toBeNull();
  });

  it("drops a shape whose default is not in the lists, and keeps the rest", () => {
    const vocab = cardVocabulary({
      spans: ["half", "full"],
      heights: ["regular", "tall"],
      shapes: CATALOG.shapes,
    });
    expect(vocab?.shapes.map((s) => s.shape)).toEqual(["chart", "table"]);
    expect(
      cardVocabulary({
        spans: ["quarter"],
        heights: ["compact"],
        shapes: [CATALOG.shapes[0]],
      }),
    ).toBeNull();
  });

  it("picks the shape through the payload's own viz member", () => {
    const vocab = vocabulary();
    expect(shapeForViz(vocab, "bar")?.shape).toBe("chart");
    expect(shapeForViz(vocab, "stat")?.shape).toBe("stat");
    expect(shapeForViz(vocab, "table")?.shape).toBe("table");
    expect(shapeForViz(vocab, "pie")?.shape).toBe("chart");
    expect(shapeForViz(vocab, "heatmap")).toBeUndefined();
  });

  it("derives each widget's arrival size from its shape default", () => {
    const vocab = vocabulary();
    const bar = widgetDef("time_by_user:self");
    const stat = widgetDef("first_response:self");
    const table = widgetDef("sla_breaches:org");
    expect(bar && defaultSizeFor(bar, vocab)).toEqual({ span: "half", height: "regular" });
    expect(stat && defaultSizeFor(stat, vocab)).toEqual({ span: "third", height: "compact" });
    expect(table && defaultSizeFor(table, vocab)).toEqual({ span: "full", height: "tall" });
  });

  it("accepts a size only when both halves are listed", () => {
    const vocab = vocabulary();
    expect(isCardSize(vocab, { span: "half", height: "regular" })).toBe(true);
    expect(isCardSize(vocab, { span: "huge", height: "regular" })).toBe(false);
    expect(isCardSize(vocab, { span: "half", height: "towering" })).toBe(false);
    expect(isCardSize(vocab, null)).toBe(false);
  });

  it("default placements carry a size for every widget on the default preset", () => {
    const placements = defaultPlacements(vocabulary());
    expect(placements.map((p) => p.id)).toEqual(defaultRows());
    for (const placement of placements) {
      expect(isCardSize(vocabulary(), placement.size)).toBe(true);
    }
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

describe("widget placement persistence payload", () => {
  it("round-trips sizes through the registry Text value", () => {
    const placements = [
      { id: "time_by_user:self", size: { span: "full", height: "tall" } },
      { id: "sla_breaches:org", size: { span: "quarter", height: "compact" } },
    ];
    expect(decodeWidgets(encodeWidgets(placements), vocabulary())).toEqual(placements);
  });

  it("uses the settings-registry key with a version 2 card size", () => {
    expect(WIDGET_ROWS_KEY).toBe("dashboards.widgets");
    const payload = JSON.parse(
      encodeWidgets([{ id: "time_by_user:self", size: { span: "half", height: "regular" } }]),
    ) as { version: number; widgets: unknown };
    expect(payload.version).toBe(2);
    expect(payload.widgets).toEqual([
      { id: "time_by_user:self", size: { span: "half", height: "regular" } },
    ]);
  });

  it("falls back to nothing when a stored card names a size the payload does not list", () => {
    const unknownSpan = JSON.stringify({
      version: 2,
      widgets: [{ id: "time_by_user:self", size: { span: "huge", height: "regular" } }],
    });
    const unknownHeight = JSON.stringify({
      version: 2,
      widgets: [{ id: "time_by_user:self", size: { span: "half", height: "towering" } }],
    });
    expect(decodeWidgets(unknownSpan, vocabulary())).toBeNull();
    expect(decodeWidgets(unknownHeight, vocabulary())).toBeNull();
  });

  it("falls back to nothing on an unknown id, a duplicate or a malformed entry", () => {
    const vocab = vocabulary();
    const entry = { id: "time_by_user:self", size: { span: "half", height: "regular" } };
    expect(
      decodeWidgets(JSON.stringify({ version: 2, widgets: [{ ...entry, id: "gone:self" }] }), vocab),
    ).toBeNull();
    expect(decodeWidgets(JSON.stringify({ version: 2, widgets: [entry, entry] }), vocab)).toBeNull();
    expect(decodeWidgets(JSON.stringify({ version: 2, widgets: ["time_by_user:self"] }), vocab)).toBeNull();
    expect(decodeWidgets(JSON.stringify({ version: 2, widgets: "nope" }), vocab)).toBeNull();
    expect(decodeWidgets(JSON.stringify({ version: 3, widgets: [] }), vocab)).toBeNull();
  });

  it("returns null for anything that is not a stored payload", () => {
    const vocab = vocabulary();
    expect(decodeWidgets(null, vocab)).toBeNull();
    expect(decodeWidgets(undefined, vocab)).toBeNull();
    expect(decodeWidgets(42, vocab)).toBeNull();
    expect(decodeWidgets("not json", vocab)).toBeNull();
  });

  it("migrates the shipped version 1 ids payload by giving each id its default size", () => {
    const stored = JSON.stringify({
      version: 1,
      widgets: ["time_by_user:self", "nope:goneself", "time_by_user:self", "sla_breaches:org"],
    });
    expect(decodeWidgets(stored, vocabulary())).toEqual([
      { id: "time_by_user:self", size: { span: "half", height: "regular" } },
      { id: "sla_breaches:org", size: { span: "full", height: "tall" } },
    ]);
  });

  it("keeps an empty version 2 arrangement empty instead of falling back", () => {
    expect(decodeWidgets(JSON.stringify({ version: 2, widgets: [] }), vocabulary())).toEqual([]);
  });
});

describe("placement operations", () => {
  const base = () => defaultPlacements(vocabulary());

  it("addWidget appends at the shape default", () => {
    const placements = base();
    const next = addWidget(placements, "sla_breaches:org", vocabulary());
    expect(next?.at(-1)).toEqual({
      id: "sla_breaches:org",
      size: { span: "full", height: "tall" },
    });
  });

  it("addWidget accepts a generated card's own size, which is the lane 6 seam", () => {
    const next = addWidget(base(), "time_by_client:org", vocabulary(), {
      span: "quarter",
      height: "compact",
    });
    expect(next?.at(-1)?.size).toEqual({ span: "quarter", height: "compact" });
  });

  it("addWidget refuses a duplicate, an unknown id and an unlisted size", () => {
    expect(addWidget(base(), "time_by_user:self", vocabulary())).toBeNull();
    expect(addWidget(base(), "made_up:self", vocabulary())).toBeNull();
    expect(
      addWidget(base(), "time_by_client:org", vocabulary(), { span: "huge", height: "regular" }),
    ).toBeNull();
  });

  it("removeWidget keeps the remaining order", () => {
    const placements = base();
    expect(removeWidget(placements, "ticket_throughput:self").map((p) => p.id)).toEqual([
      "time_by_user:self",
      "ticket_aging:self",
      "first_response:self",
    ]);
  });

  it("moveWidget swaps neighbours and stops at the edges", () => {
    const placements = base();
    expect(moveWidget(placements, "ticket_throughput:self", -1).map((p) => p.id)).toEqual([
      "ticket_throughput:self",
      "time_by_user:self",
      "ticket_aging:self",
      "first_response:self",
    ]);
    expect(moveWidget(placements, "time_by_user:self", -1).map((p) => p.id)).toEqual(
      placements.map((p) => p.id),
    );
    expect(moveWidget(placements, "first_response:self", 1).map((p) => p.id)).toEqual(
      placements.map((p) => p.id),
    );
  });

  it("resizeWidget changes one card and refuses an unlisted size", () => {
    const placements = base();
    const resized = resizeWidget(placements, "time_by_user:self", { span: "quarter", height: "tall" }, vocabulary());
    expect(resized?.[0]?.size).toEqual({ span: "quarter", height: "tall" });
    // Every other card keeps the size it had.
    expect(resized?.slice(1)).toEqual(placements.slice(1));
    expect(
      resizeWidget(base(), "time_by_user:self", { span: "huge", height: "tall" }, vocabulary()),
    ).toBeNull();
    expect(
      resizeWidget(base(), "gone:self", { span: "half", height: "regular" }, vocabulary()),
    ).toBeNull();
  });

  it("applying a preset keeps the size of a card already on the grid", () => {
    const current = [
      { id: "time_by_user:org", size: { span: "quarter", height: "compact" } },
    ];
    const next = placementsForPreset(PRESETS[1]!.widgets, current, vocabulary());
    expect(next?.[0]).toEqual(current[0]);
    expect(next?.map((p) => p.id)).toEqual(PRESETS[1]!.widgets);
  });

  it("matches an arrangement to a preset by ids and order", () => {
    expect(placementsMatch(base(), PRESETS[0]!.widgets)).toBe(true);
    expect(placementsMatch(base().slice(0, 2), PRESETS[0]!.widgets)).toBe(false);
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
