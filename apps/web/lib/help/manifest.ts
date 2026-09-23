// Help manifest access (Wave 26 lane 2).
//
// Purpose: reads the route→doc index produced by the ADR 0009 docs
// pipeline. `help-manifest.json` is the byte-identical copy of
// docs/site/help-manifest.json — `check-docs-site.mjs` fails CI when a
// doc page moves without the manifest following, so this file can never
// point at a dead page. The markdown pages stay the single source of
// truth; this index only routes to them.

import manifest from "./help-manifest.json";
import { DEFAULT_ROUTE_KEY, routeKeyForPath } from "./route-keys";

export interface HelpDoc {
  /** Site-relative page path without extension, e.g. `how-to/get-help`. */
  path: string;
  /** Page title, pinned to the page's first heading by the checker. */
  title: string;
  /** Audience tag mirrored from the page: admin | staff | client | both | all. */
  audience: string;
  /** Last substantive edit date (YYYY-MM-DD) used for recency ranking. */
  updated: string;
}

export interface HelpManifest {
  version: number;
  docsBase: string;
  routes: Record<string, { docs: HelpDoc[] }>;
}

const m = manifest as unknown as HelpManifest;

export function helpManifest(): HelpManifest {
  return m;
}

/** All docs mapped to one route key. Empty for unmapped keys is fine —
 * the panel falls back to the `default` entry (the get-help page). */
export function docsForRouteKey(routeKey: string): HelpDoc[] {
  return m.routes[routeKey]?.docs ?? [];
}

/**
 * Intelligent surfacing, role half (Wave 26 leg C): docs tagged `admin`
 * describe settings-owner work, so members never see them. Admin presets
 * hold the full matrix — the `admin:`/`special:manage_*` grant families —
 * so "holds any of those grants" is the honest proxy for "this guide is
 * for you". Everyone else sees staff/client/both/all pages.
 */
export function isAdminViewer(permissions: readonly string[]): boolean {
  return permissions.some(
    (p) => p.startsWith("special:manage") || p.startsWith("admin:"),
  );
}

export function filterByAudience(
  docs: HelpDoc[],
  permissions: readonly string[],
): HelpDoc[] {
  const admin = isAdminViewer(permissions);
  return docs.filter((d) => d.audience !== "admin" || admin);
}

/**
 * Intelligent surfacing, recency half (Wave 26 leg C): when a route key
 * maps several candidates, the most recently edited guide leads. Ties
 * break on title so the order is deterministic.
 */
export function rankByRecency(docs: HelpDoc[]): HelpDoc[] {
  return [...docs].sort(
    (a, b) =>
      b.updated.localeCompare(a.updated) || a.title.localeCompare(b.title),
  );
}

/**
 * Panel ordering: guides authored for THIS surface always lead (each
 * group recency-ranked); the fallback get-help entry trails so the
 * newest meta page never eclipses the surface's own guide.
 */
export function rankSurfaceFirst(
  surfaceDocs: HelpDoc[],
  fallbackDocs: HelpDoc[],
): HelpDoc[] {
  return [...rankByRecency(surfaceDocs), ...rankByRecency(fallbackDocs)];
}


/** Panel-local search across EVERY indexed guide (deduped by path). */
export function searchAllDocs(
  query: string,
  permissions: readonly string[],
): HelpDoc[] {
  const q = query.trim().toLowerCase();
  if (q.length < 2) return [];
  const byPath = new Map<string, HelpDoc>();
  for (const docs of Object.values(m.routes)) {
    for (const d of docs.docs ?? []) {
      if (!byPath.has(d.path)) byPath.set(d.path, d);
    }
  }
  return rankByRecency(
    [...byPath.values()].filter((d) => {
      if (d.audience === "admin" && !isAdminViewer(permissions)) return false;
      return d.title.toLowerCase().includes(q) || d.path.toLowerCase().includes(q);
    }),
  );
}

/** Deep link into the published docs bundle (`/docs/<path>.html`). */
export function docsUrl(doc: HelpDoc): string {
  const base = m.docsBase.replace(/\/$/, "");
  return `${base}/${doc.path}.html`;
}

export { routeKeyForPath, DEFAULT_ROUTE_KEY };
