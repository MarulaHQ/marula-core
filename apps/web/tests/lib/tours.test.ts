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
    expect(tour?.id).toBe("workbar");
  });

  it("never repeats a completed tour (first-visit-per-surface)", () => {
    expect(tourForRouteKey("today", ["workbar"], false)).toBeNull();
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
    const tour = TOURS.find((t) => t.id === "workbar");
    if (tour) expect(tourTargetsReady(tour)).toBe(false);
  });

  it("returns true once all targets mount", () => {
    document.body.innerHTML =
      '<div data-testid="search-launcher"></div><header data-testid="top-bar"></header>';
    const tour = TOURS.find((t) => t.id === "workbar");
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
