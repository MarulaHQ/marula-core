// Declarative tours (Wave 26 lane 2, leg B; surfaces and the record wave
// in Wave 72): surface selection, completion suppression, session cap,
// and the settings-seam log parsers.
import { describe, expect, it, beforeEach } from "vitest";
import {
  parseCompletedTours,
  parseDocFeedback,
} from "../../lib/help/help-settings";
import manifest from "../../lib/help/help-manifest.json";
import { routeKeyForPath } from "../../lib/help/route-keys";
import { tourSurfaceForPath } from "../../lib/tours/surfaces";
import {
  SESSION_TOUR_CAP,
  TOURS,
  tourAllowed,
  tourForSurface,
  tourTargetsReady,
  type Tour,
} from "../../lib/tours/tours";

/** One representative path per surface a tour is authored against. The
 *  tests below use it to prove every tour is reachable by a real path,
 *  and that the path still has documentation under its own key. */
const SURFACE_PATHS: Record<string, string> = {
  today: "/today",
  tickets: "/tickets",
  "ticket-record": "/tickets/30000000-0000-4000-8000-000000000001",
  boards: "/boards",
  dispatch: "/dispatch",
  projects: "/projects",
  "project-record": "/projects/40000000-0000-4000-8000-000000000001",
  dashboards: "/dashboards",
  automations: "/automations",
  stock: "/stock",
  clients: "/clients",
  billing: "/billing",
  kb: "/kb",
  calendar: "/calendar",
  approvals: "/approvals",
  purchasing: "/purchasing",
  "inventory-workbench": "/inventory/workbench",
};

describe("tourSurfaceForPath", () => {
  it("resolves a record path to its own surface, not the list's key", () => {
    expect(tourSurfaceForPath("/projects/40000000-0000-4000-8000-000000000001")).toBe(
      "project-record",
    );
    expect(tourSurfaceForPath("/projects/40000000-0000-4000-8000-000000000001/timeline")).toBe(
      "project-record",
    );
    expect(
      tourSurfaceForPath("/tickets/30000000-0000-4000-8000-000000000001"),
    ).toBe("ticket-record");
  });

  it("keeps the list's key for its static sub-pages", () => {
    // These are lists in their own right, and the list tour's targets are
    // not on them — readiness declines the tour rather than mis-keying it.
    expect(tourSurfaceForPath("/projects")).toBe("projects");
    expect(tourSurfaceForPath("/projects/portfolio")).toBe("projects");
    expect(tourSurfaceForPath("/projects/my-work")).toBe("projects");
    expect(tourSurfaceForPath("/projects/templates/abc")).toBe("projects");
    expect(tourSurfaceForPath("/tickets/classic")).toBe("tickets");
    expect(tourSurfaceForPath("/tickets/classic/abc")).toBe("tickets");
  });

  it("gives stock and purchasing their own surfaces instead of the fallback", () => {
    // Before Wave 72 /stock rode "default"; any unmapped path shared it.
    expect(tourSurfaceForPath("/stock")).toBe("stock");
    expect(tourSurfaceForPath("/stock/loans")).toBe("stock");
    expect(tourSurfaceForPath("/purchasing")).toBe("purchasing");
    expect(tourSurfaceForPath("/purchasing/50000000-0000-4000-8000-000000000001")).toBe(
      "purchasing",
    );
    // The workbench is a different surface from its queue.
    expect(tourSurfaceForPath("/inventory")).toBe("inventory");
    expect(tourSurfaceForPath("/inventory/workbench")).toBe("inventory-workbench");
  });

  it("falls back to the docs route key for every other surface", () => {
    expect(tourSurfaceForPath("/settings/fx")).toBe(routeKeyForPath("/settings/fx"));
    expect(tourSurfaceForPath("/settings/inventory")).toBe(routeKeyForPath("/settings/inventory"));
    expect(tourSurfaceForPath("/nothing-here")).toBe("default");
  });

  it("keeps the docs key on the parent surface for a record page", () => {
    // The help panel must still answer with the list's documentation.
    expect(routeKeyForPath("/projects/abc")).toBe("projects");
    expect(routeKeyForPath("/tickets/abc")).toBe("tickets");
    expect(routeKeyForPath("/stock")).toBe("default");
  });
});

describe("tourForSurface", () => {
  it("selects the tour authored for the surface", () => {
    expect(tourForSurface("today", [], false)?.id).toBe("workbar:v3");
    expect(tourForSurface("project-record", [], false)?.id).toBe("project-record:v2");
  });

  it("never repeats a completed tour (first-visit-per-surface)", () => {
    expect(tourForSurface("today", ["workbar:v3"], false)).toBeNull();
  });

  it("selects through the surface list with the gate applied (controlled fixture)", () => {
    // The predicate test below proves the rule; this proves SELECTION runs
    // it, which a bypassed check in tourForSurface would not.
    const adminTour: Tour = {
      id: "fixture:admin",
      surfaces: ["fixture"],
      title: "Admin fixture",
      adminOnly: true,
      steps: [{ target: "#fixture", title: "One", body: "Two." }],
    };
    expect(tourForSurface("fixture", [], false, [adminTour])).toBeNull();
    expect(tourForSurface("fixture", [], true, [adminTour])).toBe(adminTour);
    // A different surface never selects it, even for an admin.
    expect(tourForSurface("elsewhere", [], true, [adminTour])).toBeNull();
  });

  it("gates admin-only tours from members (controlled fixture)", () => {
    // The registry has no admin-only tour, so a conditional over it would
    // never exercise the gate. The predicate takes the tour, so a fixture
    // proves both halves: a member is refused and an admin is allowed.
    const adminTour: Tour = {
      id: "fixture:admin",
      surfaces: ["fixture"],
      title: "Admin fixture",
      adminOnly: true,
      steps: [{ target: "#fixture", title: "One", body: "Two." }],
    };
    expect(tourAllowed(adminTour, [], false)).toBe(false);
    expect(tourAllowed(adminTour, [], true)).toBe(true);
    // A completed tour is refused even for an admin.
    expect(tourAllowed(adminTour, ["fixture:admin"], true)).toBe(false);
    // The same tour without the flag is fine for a member.
    expect(tourAllowed({ ...adminTour, adminOnly: false }, [], false)).toBe(true);
  });

  it("gives every tour a surface a real path resolves to, with docs under it", () => {
    const documented = Object.keys(manifest.routes);
    for (const tour of TOURS) {
      for (const surface of tour.surfaces) {
        const path = SURFACE_PATHS[surface];
        expect(path, `${tour.id} names surface "${surface}" with no path in the test`).toBeDefined();
        expect(tourSurfaceForPath(path!), `${path} does not resolve to ${surface}`).toBe(surface);
        // Help docs keep using the docs key, so it must still resolve to a
        // documented surface even when the tour surface is finer.
        expect(documented).toContain(routeKeyForPath(path!));
      }
    }
  });

  it("never puts two tours on one surface (the second would be dead data)", () => {
    const seen = new Map<string, string>();
    for (const tour of TOURS) {
      for (const surface of tour.surfaces) {
        const other = seen.get(surface);
        expect(other, `${surface} is claimed by ${other} and ${tour.id}`).toBeUndefined();
        seen.set(surface, tour.id);
      }
    }
  });

  it("keeps every tour id unique (tours.completed stores them)", () => {
    const ids = TOURS.map((t) => t.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("keeps every tour inside the 3–6 step budget (research §4.3)", () => {
    for (const tour of TOURS) {
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

  it("keeps every step inside the brand voice limits (no dashes, short lines)", () => {
    // The voice guide's rules, mechanised: commas and colons, never em
    // dashes; no questions or exclamations; titles of five words or fewer;
    // bodies under thirty words. The guide is the authority, so a rewrite
    // that breaks it should fail here rather than at review.
    const banned = [
      "revolutionary",
      "supercharge",
      "seamless",
      "leverage",
      "synergy",
      "simply",
      "users",
      "robust",
      "effortless",
      "intuitive",
      "powerful",
    ];
    for (const tour of TOURS) {
      expect(tour.title.split(/\s+/).length, `${tour.id} title is too long`).toBeLessThanOrEqual(5);
      for (const step of tour.steps) {
        const copy = `${step.title} ${step.body}`;
        expect(copy, `${tour.id}: ${step.title}`).not.toMatch(/[—–]/);
        expect(copy, `${tour.id}: ${step.title}`).not.toMatch(/[?!]/);
        expect(
          step.body.split(/\s+/).filter(Boolean).length,
          `${tour.id}: ${step.title} body is too long`,
        ).toBeLessThan(30);
        for (const word of banned) {
          expect(copy.toLowerCase(), `${tour.id}: ${step.title} uses "${word}"`).not.toMatch(
            new RegExp(`\\b${word}\\b`),
          );
        }
      }
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

});

describe("tourTargetsReady", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
  });

  it("requires every step target to exist before starting", () => {
    document.body.innerHTML = '<div data-testid="search-launcher"></div>';
    const tour = TOURS.find((t) => t.id === "workbar:v3");
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
    const tour = TOURS.find((t) => t.id === "workbar:v3");
    if (tour) expect(tourTargetsReady(tour)).toBe(true);
  });

  it("declines a record tour while a target is still missing", () => {
    // The record tour mounts its commitment strip with the first read, so
    // a partial DOM must not start it.
    const tour = TOURS.find((t) => t.id === "project-record:v2");
    if (!tour) throw new Error("project-record tour missing");
    document.body.innerHTML = '<div data-slot="commitment-bar"></div>';
    expect(tourTargetsReady(tour)).toBe(false);
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
