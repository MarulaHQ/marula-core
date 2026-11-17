// Declarative tours (Wave 26 lane 2, leg B): route selection, completion
// suppression, session cap, and the settings-seam log parsers.
import { describe, expect, it, beforeEach } from "vitest";
import {
  parseCompletedTours,
  parseDocFeedback,
} from "../../lib/help/help-settings";
import {
  SESSION_TOUR_CAP,
  TOURS,
  tourForRouteKey,
  tourTargetsReady,
} from "../../lib/tours/tours";

describe("tourForRouteKey", () => {
  it("selects the tour authored for the surface", () => {
    const tour = tourForRouteKey("today", [], false);
    expect(tour?.id).toBe("workbar:v2");
  });

  it("never repeats a completed tour (first-visit-per-surface)", () => {
    expect(tourForRouteKey("today", ["workbar:v2"], false)).toBeNull();
  });

  it("gates admin-only tours from members", () => {
    const adminTour = TOURS.find((t) => t.adminOnly);
    if (adminTour) {
      const key = adminTour.routeKeys[0]!;
      expect(tourForRouteKey(key, [], false)).toBeNull();
      expect(tourForRouteKey(key, [], true)).not.toBeNull();
    }
  });

  it("authors every tour against the manifest route vocabulary", () => {
    const manifest = helpManifestRoutes();
    for (const tour of TOURS) {
      for (const key of tour.routeKeys) {
        expect(manifest).toContain(key);
      }
    }
  });

  it("keeps every tour id unique (tours.completed stores them)", () => {
    const ids = TOURS.map((t) => t.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("keeps first-wave tours inside the 3–6 step budget (research §4.3)", () => {
    const firstWave = new Set([
      "workbar:v2",
      "tickets",
      "boards",
      "dispatch",
      "projects",
      "dashboards",
      "automations",
      "stock",
      "clients",
      "billing",
      "kb",
    ]);
    for (const tour of TOURS) {
      if (!firstWave.has(tour.id)) continue;
      expect(
        tour.steps.length,
        `${tour.id} has ${tour.steps.length} steps`,
      ).toBeGreaterThanOrEqual(3);
      expect(
        tour.steps.length,
        `${tour.id} has ${tour.steps.length} steps`,
      ).toBeLessThanOrEqual(6);
    }
  });

  it("never targets the nav rail (the nav gets zero steps)", () => {
    for (const tour of TOURS) {
      for (const step of tour.steps) {
        expect(step.target).not.toContain("rail-");
        expect(step.target).not.toContain("nav-rail");
      }
    }
  });

  function helpManifestRoutes(): string[] {
    // Imported lazily to keep this test focused on the tours data.
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const m = require("../../lib/help/help-manifest.json") as {
      routes: Record<string, unknown>;
    };
    return Object.keys(m.routes);
  }
});

describe("tourTargetsReady", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
  });

  it("requires every step target to exist before starting", () => {
    document.body.innerHTML = '<div data-testid="search-launcher"></div>';
    const tour = TOURS.find((t) => t.id === "workbar:v2");
    if (tour) expect(tourTargetsReady(tour)).toBe(false);
  });

  it("returns true once all targets mount", () => {
    // The workbar tour's own step targets (Wave 71 content): the Today
    // header actions row, the first attention card's anchor, and the
    // queue card's All tickets link.
    document.body.innerHTML =
      '<div id="m-main-content">' +
      '<div data-testid="surface-header-actions"><button></button></div>' +
      '<a href="#overdue"></a>' +
      '<a href="/tickets"></a>' +
      "</div>";
    const tour = TOURS.find((t) => t.id === "workbar:v2");
    if (tour) expect(tourTargetsReady(tour)).toBe(true);
  });
});

describe("settings-seam logs", () => {
  it("parses the capped tours.completed JSON array", () => {
    expect(parseCompletedTours("")).toEqual([]);
    expect(parseCompletedTours(null)).toEqual([]);
    expect(parseCompletedTours(false)).toEqual([]);
    expect(parseCompletedTours('["a","b"]')).toEqual(["a", "b"]);
    expect(parseCompletedTours("not-json")).toEqual([]);
    expect(parseCompletedTours('{"x":1}')).toEqual([]);
  });

  it("parses was-this-helpful entries and rejects junk", () => {
    expect(parseDocFeedback("")).toEqual([]);
    const raw = JSON.stringify([
      { doc: "how-to/get-help", up: true, at: "2026-09-23T00:00:00Z" },
    ]);
    const parsed = parseDocFeedback(raw);
    expect(parsed.length).toBe(1);
    expect(parsed[0]!.up).toBe(true);
    expect(parseDocFeedback("[1,2]")).toEqual([]);
    expect(parseDocFeedback("nope")).toEqual([]);
  });

  it("caps the session at one auto-started tour", () => {
    expect(SESSION_TOUR_CAP).toBe(1);
  });
});
