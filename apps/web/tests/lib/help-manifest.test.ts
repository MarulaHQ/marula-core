// Route-key derivation + manifest helpers (Wave 26 lane 2).
import { describe, expect, it } from "vitest";
import {
  DEFAULT_ROUTE_KEY,
  routeKeyForPath,
} from "../../lib/help/route-keys";
import {
  docsForRouteKey,
  docsUrl,
  filterByAudience,
  helpManifest,
  isAdminViewer,
  rankByRecency,
  searchAllDocs,
} from "../../lib/help/manifest";

describe("routeKeyForPath", () => {
  it("maps surface families to manifest keys", () => {
    expect(routeKeyForPath("/tickets")).toBe("tickets");
    expect(routeKeyForPath("/tickets/30000000-0000-4000-8000-000000000001")).toBe("tickets");
    expect(routeKeyForPath("/today")).toBe("today");
    expect(routeKeyForPath("/invoices")).toBe("billing");
    expect(routeKeyForPath("/settings/fx")).toBe("settings-fx");
    expect(routeKeyForPath("/settings/api-keys")).toBe("settings-api-keys");
    expect(routeKeyForPath("/settings/anything-else")).toBe("settings-general");
    expect(routeKeyForPath("/settings/inventory")).toBe("inventory");
  });

  it("falls back to the default key for unmapped surfaces", () => {
    expect(routeKeyForPath("/dev")).toBe(DEFAULT_ROUTE_KEY);
  });
});

describe("help manifest", () => {
  const permissions = (ps: string[]) => ps;

  it("carries a default entry so the panel is never empty", () => {
    expect(docsForRouteKey(DEFAULT_ROUTE_KEY).length).toBeGreaterThan(0);
  });

  it("keeps every doc path well-formed and deep-linkable", () => {
    for (const docs of Object.values(helpManifest().routes)) {
      for (const d of docs.docs ?? []) {
        expect(d.path).toMatch(/^(tutorials|how-to|explanation|reference)\//);
        expect(docsUrl(d)).toMatch(/^https:\/\/marula.app\/docs\/.+\.(html|md)$/);
      }
    }
  });

  it("ranks candidates newest-first, deterministically", () => {
    const ranked = rankByRecency(docsForRouteKey("ai"));
    expect(ranked.length).toBeGreaterThan(1);
    for (let i = 1; i < ranked.length; i++) {
      expect(ranked[i - 1]!.updated >= ranked[i]!.updated).toBe(true);
    }
  });

  it("never surfaces admin-only docs to members (leg C role gate)", () => {
    expect(isAdminViewer(permissions(["special:manage_boards"]))).toBe(true);
    expect(isAdminViewer(permissions(["admin:view"]))).toBe(true);
    expect(isAdminViewer(permissions(["tickets:create"]))).toBe(false);
    const member = filterByAudience(docsForRouteKey("boards"), permissions(["tickets:create"]));
    expect(member.some((d) => d.audience === "admin")).toBe(false);
    const admin = filterByAudience(docsForRouteKey("boards"), permissions(["special:manage_boards"]));
    expect(admin.some((d) => d.audience === "admin")).toBe(true);
  });

  it("search spans all routes and dedupes by path", async () => {
    const hits = searchAllDocs("ticket", permissions([]));
    expect(hits.length).toBeGreaterThan(0);
    const paths = hits.map((h) => h.path);
    expect(new Set(paths).size).toBe(paths.length);
    expect(searchAllDocs("x", permissions([]))).toEqual([]);
  });
});
