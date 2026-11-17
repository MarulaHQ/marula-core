// Declarative feature tours (Wave 26 lane 2, leg B; content re-authored
// Wave 71 lane 3, extended Wave 72 to the record surfaces).
//
// Purpose: per-surface first-visit highlights authored as DATA — a tour
// is a surface + ordered steps targeting elements by selector (data
// attributes / existing testids). No per-page hand-coding: the scheduler
// picks the tour for the current surface (lib/tours/surfaces.ts), the
// overlay renders it. A tour auto-starts only when every step target
// actually exists on screen, so a layout change degrades to "no tour",
// never a broken overlay.
//
// Every step must be visible at the same moment: the readiness check runs
// once against the whole step list, so a tour cannot cross tabs (a target
// behind an unopened tab fails it). Author record tours against the
// surface's default view.
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
  /** Tour surfaces (lib/tours/surfaces.ts) that carry it. */
  surfaces: string[];
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
    surfaces: ["today"],
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
    surfaces: ["tickets"],
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
    surfaces: ["boards"],
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
    surfaces: ["dispatch"],
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
    surfaces: ["projects"],
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
    surfaces: ["dashboards"],
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
    surfaces: ["automations"],
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
    // /stock used to have no route-key rule, so this tour rode the
    // "default" fallback and belonged to every unmapped path. Wave 72
    // gave it a surface of its own (lib/tours/surfaces.ts); the id is
    // unchanged because nothing a reader sees was rewritten.
    id: "stock",
    surfaces: ["stock"],
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
    surfaces: ["clients"],
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
    surfaces: ["billing"],
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
    surfaces: ["kb"],
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
    // Wave 72: the engagement record. Authored against the default Plan
    // view; tab panels are not targets because the readiness check runs
    // once for the whole step list and a hidden tab would fail it.
    id: "project-record",
    surfaces: ["project-record"],
    title: "Run one engagement",
    steps: [
      {
        target: '[data-slot="commitment-bar"]',
        title: "The promise, in one line",
        body: "Started, promised go-live, next milestone and what is at risk. When a date slips, this strip is the first place it shows.",
      },
      {
        target: '[role="tablist"]',
        title: "Four views, no reload",
        body: "Plan holds phases and dates, Timeline is the same work as a chart, Tasks is the list to triage, Time and budget is what the engagement has burned.",
      },
      {
        target: '[data-testid="plan-add-task"]',
        title: "A starts the next task",
        body: "Task, or the A key, starts a row where you are. Enter commits it and Escape cancels; a task with no dates waits under Not dated yet rather than pretending to be scheduled.",
      },
      {
        target: '[data-testid="project-schedule"]',
        title: "Change a date, read the ripple",
        body: "Change a start date or scale a set of rows and the preview lists every task that shifts before anything saves. The undo banner names what was applied, and each change is one audit entry.",
      },
    ],
  },
  {
    id: "approvals",
    surfaces: ["approvals"],
    title: "Clear the queue",
    steps: [
      {
        target: '[data-testid="approvals-stat-unapproved"]',
        title: "Three numbers, one decision",
        body: "Unapproved is what still needs you, Non-billable is time that cannot be billed, and Flagged appears only when the audit has answers to show.",
      },
      {
        target: '[aria-label="Views"]',
        title: "The queue and the paper trail",
        body: "Queue is today's work. Approved, My decisions and Rejected are the record, and a rejected entry keeps the reason the technician has to read.",
      },
      {
        target: '[data-testid="approvals-queue"]',
        title: "Decide without leaving the row",
        body: "j and k move, x selects, a approves, and anything else opens the reject reason. Your own time is read-only here, because nobody signs off their own hours.",
      },
    ],
  },
  {
    id: "calendar",
    surfaces: ["calendar"],
    title: "Three views of the same week",
    steps: [
      {
        target: '[aria-label="Calendar view"]',
        title: "Week, month, agenda",
        body: "One read model, three instruments. Week is the one to work from; agenda answers what is next without the grid.",
      },
      {
        target: '[aria-label="Entry legend"]',
        title: "Know the class before the colour",
        body: "Appointments, on call, synced busy time, time off and ticket deadlines each carry their own mark, so colour is never the only cue.",
      },
      {
        target: '[data-testid="week-grid"]',
        title: "Drag to move; a conflict is refused",
        body: "Appointments snap to fifteen minutes. If the slot is taken the move is rejected and the entry stays where it was, with the reason on screen.",
      },
      {
        target: '[aria-label="Next"]',
        title: "The arrows move a week",
        body: "Previous and Next step a week, or a month in month view. Today comes back to this week from wherever you have wandered.",
      },
    ],
  },
  {
    id: "ticket-record",
    surfaces: ["ticket-record"],
    title: "Work the record",
    steps: [
      {
        target: '[data-testid="wb-ticket-title"]',
        title: "The record is the ticket",
        body: "Number, title, status and priority sit at the top, and the status stepper under them moves the ticket without opening a form.",
      },
      {
        target: '[aria-label="Ticket sections"]',
        title: "Six tabs, one record",
        body: "Activity is the timeline, Details carries the fields, Related links parent and child tickets, and Time Entries, Files and Notes keep their own history.",
      },
      {
        target: '[data-testid="notes-composer"]',
        title: "Every note has an audience",
        body: "The composer sets who reads it: an internal note, an in-house update or a client-visible reply that lands in the portal. The switch is part of the post.",
      },
      {
        target: '[data-testid="wb-close-ticket"]',
        title: "Wrap up stops the clock",
        body: "Wrap up stops the timer, mints the time entry and applies the closing status in one sheet, so the hours cannot go missing between three screens.",
      },
    ],
  },
  {
    id: "purchasing",
    surfaces: ["purchasing"],
    title: "Buy the parts",
    steps: [
      {
        target: '[data-testid="ledger-shortcut-awaiting"]',
        title: "What suppliers still owe",
        body: "Awaiting receive is every open order with something still to arrive. Press it to work only those.",
      },
      {
        target: '[data-testid="ledger-shortcut-overdue"]',
        title: "Catch the late ones",
        body: "Past expected is open orders that missed their date, which is the list to call about rather than the one to read.",
      },
      {
        target: '[data-testid="purchasing-new"]',
        title: "One dialog, then the order page",
        body: "Vendor, currency, project, expected date and the lines go in here. It lands on the order's own page, where receiving and bills live.",
      },
      {
        target: '[data-testid="purchasing-vendors-link"]',
        title: "Vendors sit beside orders",
        body: "The directory every order points at. Open it when the question is who supplies a part rather than what was ordered.",
      },
    ],
  },
  {
    // Wave 26 tour, retargeted in Wave 71 onto the merge queue button.
    // Wave 72 moved it to the workbench it names (the button opened a
    // page that had three steps of its own) and rewrote the content, so
    // the id is bumped again per the re-arm convention.
    id: "inventory-workbench:v3",
    surfaces: ["inventory-workbench"],
    title: "One device, one record",
    steps: [
      {
        target: '[aria-label="Dedupe queues"]',
        title: "Work one class at a time",
        body: "Devices, contacts and clients are separate queues because they merge on different keys. The count on each tab is what the matcher found for that class.",
      },
      {
        target: '[data-testid="jev-run"]',
        title: "Let Jev look first",
        body: "The scan reports what it found and how it chose; a capped report says more candidates remain, so a quiet result is never read as a clean estate.",
      },
      {
        target: '[data-testid="workbench-queue-count"]',
        title: "Pending is work, deferred is a decision",
        body: "Pending waits on you; deferred was parked on purpose. Accept two records into one, keep them apart, or defer until someone who knows answers.",
      },
    ],
  },
];

/** The per-session auto-start cap: at most ONE tour starts itself per
 * browser session, so a person hopping surfaces never gets nagged. */
export const SESSION_TOUR_CAP = 1;

export function tourForSurface(
  surface: string,
  completed: readonly string[],
  isAdmin: boolean,
): Tour | null {
  for (const tour of TOURS) {
    if (!tour.surfaces.includes(surface)) continue;
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
