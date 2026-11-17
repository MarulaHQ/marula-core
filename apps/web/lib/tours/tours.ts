// Declarative feature tours (Wave 26 lane 2, leg B; content re-authored
// Wave 71 lane 3 against the qa-w71-tours research).
//
// Purpose: per-surface first-visit highlights authored as DATA — a tour
// is a route-key + ordered steps targeting elements by selector (data
// attributes / existing testids). No per-page hand-coding: the scheduler
// picks the tour for the current route, the overlay renders it. A tour
// auto-starts only when every step target actually exists on screen, so
// a layout change degrades to "no tour", never a broken overlay.
//
// Tour ids are durable: tours.completed (and its localStorage fallback)
// stores them per account, so an id never changes meaning. The ids below
// are version one — they carry no suffix. A deliberate CONTENT rewrite
// re-arms the tour by appending the next version to the id ("tickets"
// becomes "tickets:v2", then "tickets:v3"), so accounts that saw v1 are
// shown the new content once. Never rename an id in place, and never
// bump the version for a selector fix that does not change what the
// tour says.

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
    // Wave 71 rewrote this tour's content (the Today queue steps are new),
    // so its id is bumped per the re-arm convention above: accounts that
    // completed the Wave 26 tour see the rewrite once.
    id: "workbar:v2",
    routeKeys: ["today"],
    title: "Today, read top to bottom",
    steps: [
      {
        target: '[data-testid="surface-header-actions"] button',
        title: "Log the forgotten minutes",
        body: "Ended a call and never pressed play? Quick log (L) puts the minutes on the right ticket after the fact.",
      },
      {
        target: 'a[href="#overdue"]',
        title: "Read the strip",
        body: "Five cards sum up the day — what's yours, what changed, what's breaching. Press one to jump to its tickets.",
      },
      {
        target: '#m-main-content a[href="/tickets"]',
        title: "Work the queue",
        body: "Your queue groups assigned work with Overdue on top; All tickets opens the whole queue when it's time to plan instead of react.",
      },
    ],
  },
  {
    id: "tickets",
    routeKeys: ["tickets"],
    title: "The queue is the day",
    steps: [
      {
        target: '[aria-label="Ticket views"]',
        title: "Pick your slice",
        body: "My queue is your side of the house. Unassigned is new work waiting for an owner — take it or hand it on.",
      },
      {
        target: ".m-filter-builder-root button",
        title: "Make it yours",
        body: "Filter the queue to your tickets — the clauses save with the view and survive the reload.",
      },
      {
        target: '[aria-label="Attention shortcuts"]',
        title: "Work what's burning",
        body: "At risk and Breached are the SLA piles, and the counts are live. Press Breached and start there.",
      },
    ],
  },
  {
    id: "boards",
    routeKeys: ["boards"],
    title: "The board is the workflow",
    steps: [
      {
        target: '[data-testid^="kanban-column-"]',
        title: "Drag the status",
        body: "Drag a card into the column the work just became — the ticket's status follows the column, no form in between.",
      },
      {
        target: '[data-testid="board-filter-unassigned"]',
        title: "Find the ownerless",
        body: "Unassigned filters to cards with nobody on them. Worth a glance whenever a board goes quiet.",
      },
      {
        target: '[data-testid="board-search"]',
        title: "Find one card",
        body: "Search by number or title — kinder than scanning when the board outgrows the screen.",
      },
      {
        target: '[data-testid="new-record"]',
        title: "Start one here",
        body: "New ticket opens straight onto this board with SLA clocks already running.",
      },
    ],
  },
  {
    id: "dispatch",
    routeKeys: ["dispatch"],
    title: "Book the slots",
    steps: [
      {
        target: '[aria-label="Unassigned queue"]',
        title: "The pile is the plan",
        body: "Everything waiting for a slot stacks here, and the day empties it from the top.",
      },
      {
        target: '[aria-label="Priority"]',
        title: "Trim the pile",
        body: "P1, P2, P3+ — filter to the tier you actually have hands for today.",
      },
      {
        target: '[data-testid="dispatch-footer"]',
        title: "Keys work too",
        body: "Arrows move the cursor, Enter picks up and drops, S splits, Esc backs out of anything. The footer keeps the cheat sheet.",
      },
      {
        target: ".dp-lane",
        title: "Book the slot",
        body: "Drag an unscheduled job onto a tech's row to book the slot; drag an edge to stretch the work.",
      },
    ],
  },
  {
    id: "projects",
    routeKeys: ["projects"],
    title: "Shape the engagement",
    steps: [
      {
        target: '[aria-label="Project views"]',
        title: "Watch At risk",
        body: "All, Active and At risk slice the book. At risk is the one that pays for the glance.",
      },
      {
        target: 'input[aria-label="Search projects"]',
        title: "Jump to a project",
        body: "Search by name or client when the book runs long.",
      },
      {
        target: '[aria-label="Filter by client"]',
        title: "One client's book",
        body: "Filter by client to see every engagement, budget and due date in one relationship.",
      },
      {
        target: '[data-testid="new-project"]',
        title: "Start one",
        body: "New project opens the shell — phases and tasks give it shape on the project's page.",
      },
    ],
  },
  {
    id: "dashboards",
    routeKeys: ["dashboards"],
    title: "Build the wall",
    steps: [
      {
        target: '[aria-label="Saved arrangements"]',
        title: "Start from a preset",
        body: "Saved arrangements swap the whole wall at once — apply one, then bend it.",
      },
      {
        target: '[aria-controls="widget-picker"]',
        title: "Add the cards",
        body: "Add widget opens the catalogue — widgets arrive with their size, saved reports arrive as the card they store.",
      },
      {
        target: '[data-testid="dashboard-describe-entry"]',
        title: "Or describe it",
        body: "Say what a card should answer and the draft gets checked against the same registry the builder saves through — nothing stores until you accept it.",
      },
    ],
  },
  {
    id: "automations",
    routeKeys: ["automations"],
    title: "Automate the repeats",
    steps: [
      {
        target: '[data-testid="automation-new-rule"]',
        title: "Start from a starter",
        body: "New rule opens the builder with starter shapes — take the closest WHEN · IF · THEN and bend it.",
      },
      {
        target: '[data-testid="surface-header-meta"] button',
        title: "Count the live ones",
        body: "Enabled counts the rules in service; press the pill to see only those. Paused rules keep their wiring.",
      },
      {
        target: '[aria-label="Authoring view"]',
        title: "Go graphical",
        body: "Canvas redraws the same rules as a graph — worth the hop once logic branches.",
      },
      {
        target: '#m-main-content a[href="/automations/runs"]',
        title: "Watch one run",
        body: "After a rule fires, Runs shows every step it took and exactly where it stopped.",
      },
    ],
  },
  {
    // /stock has no route-key rule yet (ROUTE_KEY_RULES maps it to the
    // "default" fallback), so this tour rides "default" and the
    // target-existence check keeps it to pages that actually render the
    // stock ledger. If /stock gains its own route key, move it there.
    id: "stock",
    routeKeys: ["default"],
    title: "Count what you own",
    steps: [
      {
        target: '[data-testid="stock-new-item"]',
        title: "Stock a part",
        body: "New item adds to the ledger — flip the tool switch if it's a kit that leaves the building.",
      },
      {
        target: '[data-testid="stock-receive"]',
        title: "Book arrivals",
        body: "Receive records stock into a location, and the movement ledger remembers who moved what.",
      },
      {
        target: '[data-testid="stock-dispatch"]',
        title: "Parts to jobs",
        body: "Dispatch to ticket consumes parts against the job that used them, so the trail stays intact.",
      },
      {
        target: '[data-testid="ledger-shortcut-reorder"]',
        title: "Beat the empty bin",
        body: "At reorder point counts items at their minimum — press it before the next job finds out the hard way.",
      },
    ],
  },
  {
    id: "clients",
    routeKeys: ["clients"],
    title: "One row per client",
    steps: [
      {
        target: '[aria-label="Client views"]',
        title: "Slice the book",
        body: "Open work, Assets and Billing history are the same clients, asked three different questions.",
      },
      {
        target: 'input[aria-label="Search clients"]',
        title: "Find them fast",
        body: "Search by name or contact; the record you land on holds everything else.",
      },
      {
        target: '[data-testid="new-record"]',
        title: "Add a client",
        body: "New client makes the record — sites, contacts and contracts attach on its page, and imports land the same way.",
      },
    ],
  },
  {
    id: "billing",
    routeKeys: ["billing"],
    title: "Bill the work",
    steps: [
      {
        target: '[data-testid="billing-stat-overdue"]',
        title: "Chase the late ones",
        body: "Overdue counts invoices past their terms — press the pill to work only those.",
      },
      {
        target: ".m-filter-builder-root button",
        title: "Read a period",
        body: "Filter the ledger by number, client, state or due date before you go hunting.",
      },
      {
        target: '[data-testid="surface-header-actions"] button',
        title: "Run the invoices",
        body: "New invoice run previews every billable contract for the window, and points you at the verification workspace before anything issues.",
      },
    ],
  },
  {
    id: "kb",
    routeKeys: ["kb"],
    title: "Write it once",
    steps: [
      {
        target: 'input[aria-label="Search knowledge"]',
        title: "Check first",
        body: "Search reaches every client's documents, devices and secrets — the / key focuses the box.",
      },
      {
        target: '[aria-label="Search scope"]',
        title: "Narrow the hunt",
        body: "Scope the search to clients, documents, configurations or secrets when the whole estate is noise.",
      },
      {
        target: '#m-main-content a[href="/kb/articles"]',
        title: "Then write it down",
        body: "When the fix is worth keeping, All articles is where it gets written — future-you is the reader.",
      },
    ],
  },
  {
    // Wave 26 tour, retargeted and rewritten in Wave 71: the page lost its
    // search field, so the tour now leads with the merge queue it has. The
    // content changed, so the id is bumped per the re-arm convention.
    id: "inventory-workbench:v2",
    routeKeys: ["inventory"],
    title: "One device, one record",
    steps: [
      {
        target: '[data-testid="open-workbench"]',
        title: "Mind the merge queue",
        body: "Overlapping connectors nominate duplicates; the workbench is where they become single clean records.",
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
