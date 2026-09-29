// Declarative feature tours (Wave 26 lane 2, leg B).
//
// Purpose: per-surface first-visit highlights authored as DATA — a tour
// is a route-key + ordered steps targeting elements by selector (data
// attributes / existing testids). No per-page hand-coding: the scheduler
// picks the tour for the current route, the overlay renders it. A tour
// auto-starts only when every step target actually exists on screen, so
// a layout change degrades to "no tour", never a broken overlay.

export interface TourStep {
  /** Selector for the highlighted element (must exist on the surface). */
  target: string;
  title: string;
  body: string;
}

export interface Tour {
  /** Stable id persisted in tours.completed (capped JSON log). */
  id: string;
  /** Route keys (same vocabulary as the help manifest) that carry it. */
  routeKeys: string[];
  title: string;
  steps: TourStep[];
  /** Admin-only surfaces gate their tour the same way admin docs gate. */
  adminOnly?: boolean;
}

export const TOURS: readonly Tour[] = [
  {
    id: "workbar",
    routeKeys: ["today"],
    title: "Your first day on Today",
    steps: [
      {
        target: '[data-testid="search-launcher"]',
        title: "Find anything",
        body: "Tickets, clients, devices — the unified search is one keystroke away (⌘K) from every surface.",
      },
      {
        target: '[data-testid="top-bar"]',
        title: "Everything in the top bar",
        body: "Search, notifications, and the ? help button live up here. When a timer runs, the work bar appears below with your quick notes and wrap-up.",
      },
    ],
  },
  {
    id: "inventory-workbench",
    routeKeys: ["inventory"],
    title: "Inventory at a glance",
    steps: [
      {
        target: '[data-testid="inventory-search"]',
        title: "Search the estate",
        body: "Look up any device across synced sources. Records pulled from overlapping connectors merge into one rich entry.",
      },
      {
        target: '[data-testid="open-workbench"]',
        title: "The merge workbench",
        body: "When duplicates show up, the merge workbench is where candidates become single clean records.",
      },
    ],
  },
  {
    id: "help-entry",
    routeKeys: ["tickets", "boards"],
    title: "Help lives in the top bar",
    steps: [
      {
        target: '[data-testid="help-button"]',
        title: "The ? button",
        body: "Guides for the surface you're on, searchable, linked to the full docs site. Open it whenever you're stuck.",
      },
    ],
  },
];

/** The per-session auto-start cap: at most ONE tour starts itself per
 * browser session, so a person hopping surfaces never gets nagged. */
export const SESSION_TOUR_CAP = 1;

export function tourForRouteKey(
  routeKey: string,
  completed: readonly string[],
  isAdmin: boolean,
): Tour | null {
  for (const tour of TOURS) {
    if (!tour.routeKeys.includes(routeKey)) continue;
    if (completed.includes(tour.id)) continue;
    if (tour.adminOnly && !isAdmin) continue;
    return tour;
  }
  return null;
}

/** All step targets must exist before a tour may start. */
export function tourTargetsReady(tour: Tour): boolean {
  if (typeof document === "undefined") return false;
  return tour.steps.every((s) => document.querySelector(s.target) !== null);
}

export function sessionTourCount(): number {
  try {
    return Number(window.sessionStorage.getItem("marula-tour-session") ?? "0");
  } catch {
    return 0;
  }
}

export function bumpSessionTourCount(): void {
  try {
    window.sessionStorage.setItem(
      "marula-tour-session",
      String(sessionTourCount() + 1),
    );
  } catch {
    // storage unavailable — cap degrades to per-navigation
  }
}
