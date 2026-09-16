// Rail model integrity: every rail href (sections + flyout links) must
// resolve to an actual route under apps/web/app — no dead links (Wave 15
// lane 6 binding: "ALL existing routes keep their paths; no dead links").
// Also: the Tickets section's primary target must be the real standalone
// list route (resolves the /tickets 404 QA finding), and the old MainNav
// groups must be fully represented (no surface lost in the move).
import { readFileSync, existsSync, readdirSync, statSync } from "fs";
import { join } from "path";
import { describe, expect, it } from "vitest";
import { RAIL_SECTIONS } from "../../components/shell/rail-model";

// The old top-header nav tree (components/main-nav.tsx, absorbed by the
// rail in Wave 15 lane 6) — kept here verbatim as the reachability record:
// every surface that was reachable before must stay reachable after.
const MAIN_NAV_SURFACES: string[] = [
  "/today",
  "/boards",
  "/search",
  "/calendar",
  "/dispatch",
  "/dispatch/planning",
  "/dispatch/travel",
  "/clients",
  "/projects",
  "/contracts",
  "/billing",
  "/approvals",
  "/billing/credit-notes",
  "/sales/pipeline",
  "/sales/quotes",
  "/sales/catalog",
  "/inventory",
  "/documents",
  "/kb",
  "/passwords",
  "/imports",
  "/dashboards",
  "/provenance",
  "/alerts",
  "/reports",
  "/scorecards",
  "/automations",
  "/audit",
  "/settings",
  "/settings/boards",
  "/settings/general",
  "/settings/team",
  "/settings/workflow",
  "/settings/tags",
  "/settings/pm-schedules",
  "/settings/sla",
  "/settings/fx",
  "/settings/notifications",
  "/settings/preferences",
  "/settings/portal",
  "/settings/mail",
  "/settings/security",
  "/settings/api-keys",
  "/settings/integrations",
  "/settings/ai",
  "/settings/agent-platform",
];

const APP_DIR = join(__dirname, "../../app");

/** Collect every route path served by a page.tsx under apps/web/app. */
function collectRoutes(dir: string, prefix = ""): Set<string> {
  const routes = new Set<string>();
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (!statSync(full).isDirectory()) continue;
    const segment = entry.replace(/\(.*\)/g, "").replace(/\[.*\]/g, ":param");
    const next = `${prefix}/${segment}`.replace(/\/+/g, "/");
    if (existsSync(join(full, "page.tsx"))) routes.add(next);
    for (const r of collectRoutes(full, next)) routes.add(r);
  }
  return routes;
}

function routeExists(routes: Set<string>, href: string): boolean {
  if (routes.has(href)) return true;
  // Dynamic parents: /tickets/abc resolves under /tickets/:param etc.
  const segments = href.split("/").filter(Boolean);
  return [...routes].some((route) => {
    const parts = route.split("/").filter(Boolean);
    if (parts.length !== segments.length) return false;
    return parts.every((p, i) => p === segments[i] || p === ":param");
  });
}

describe("rail model route coverage", () => {
  const routes = collectRoutes(APP_DIR);

  it("resolves every rail section href to a real route", () => {
    for (const section of RAIL_SECTIONS) {
      expect(routeExists(routes, section.href), `${section.href} missing`).toBe(true);
    }
  });

  it("resolves every rail flyout link to a real route", () => {
    for (const section of RAIL_SECTIONS) {
      for (const link of section.links) {
        expect(routeExists(routes, link.href), `${link.href} missing`).toBe(true);
      }
    }
  });

  it("the Tickets rail target is the real standalone list route", () => {
    expect(routes.has("/tickets")).toBe(true);
    const tickets = RAIL_SECTIONS.find((s) => s.id === "tickets");
    expect(tickets?.href).toBe("/tickets");
  });

  it("carries over every MainNav surface (nothing lost in the move)", () => {
    // /search moved deliberately: the slim top bar owns unified search via
    // the ⌘K launcher (Wave 15 lane 6 scope) — the rail keeps content nav.
    const MOVED_TO_TOP_BAR = new Set(["/search"]);
    const railHrefs = new Set<string>();
    for (const section of RAIL_SECTIONS) {
      railHrefs.add(section.href);
      for (const link of section.links) railHrefs.add(link.href);
    }
    for (const href of MAIN_NAV_SURFACES) {
      if (MOVED_TO_TOP_BAR.has(href)) continue;
      expect(
        railHrefs.has(href),
        `MainNav surface ${href} is unreachable from the rail`,
      ).toBe(true);
    }
  });

  it("flyout links are unique within a section", () => {
    for (const section of RAIL_SECTIONS) {
      const hrefs = section.links.map((l) => l.href);
      expect(new Set(hrefs).size).toBe(hrefs.length);
    }
  });
});
