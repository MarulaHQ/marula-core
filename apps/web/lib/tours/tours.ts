// Declarative feature tours (Wave 26 lane 2, leg B; content rewritten in
// the voice pass against the brand voice guide, 2026-11-17).
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
// stores them per account, so an id never changes meaning. A deliberate
// CONTENT rewrite re-arms the tour by appending the next version to the
// id ("tickets" becomes "tickets:v2", then "tickets:v3"), so accounts that
// saw v1 are shown the new content once. Never rename an id in place, and
// never bump the version for a selector fix that does not change what the
// tour says. The voice rewrite bumped every tour.
//
// Copy rules (docs: the brand voice guide): no em dashes, no questions or
// exclamation marks, titles of five words or fewer, bodies under thirty
// words, sentence case, one idea per step, and the body never restates
// the title.

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
    id: "workbar:v3",
    surfaces: ["today"],
    title: "Your day on Today",
    steps: [
      {
        target: '[data-testid="surface-header-actions"] button',
        title: "Log time after the fact",
        body: "Quick log (L) adds time to a ticket after the work is done. Use it when the timer wasn't running.",
      },
      {
        target: 'a[href="#overdue"]',
        title: "Read the strip",
        body: "Five cards count what's yours, what changed and what's breaching. Press one to open those tickets.",
      },
      {
        target: '#m-main-content a[href="/tickets"]',
        title: "Plan from All tickets",
        body: "Your queue lists assigned work, Overdue first. All tickets opens the full queue for planning.",
      },
    ],
  },
  {
    id: "tickets:v2",
    surfaces: ["tickets"],
    title: "Work the queue",
    steps: [
      {
        target: '[aria-label="Ticket views"]',
        title: "Pick a view",
        body: "My queue shows tickets assigned to you. Unassigned shows tickets with no owner. Take one or assign it.",
      },
      {
        target: ".m-filter-builder-root button",
        title: "Filter, then keep it",
        body: "Add filter clauses to narrow the queue. They save with the view and stay after a reload.",
      },
      {
        target: '[aria-label="Attention shortcuts"]',
        title: "Start with Breached",
        body: "Changed since you looked, At risk and Breached are live counts. Breached tickets are already over their SLA.",
      },
    ],
  },
  {
    id: "boards:v2",
    surfaces: ["boards"],
    title: "Work the board",
    steps: [
      {
        target: '[data-testid^="kanban-column-"]',
        title: "Drag to change status",
        body: "Drop a card in a column and the ticket's status changes to match. No form opens.",
      },
      {
        target: '[data-testid="board-filter-unassigned"]',
        title: "Find ownerless cards",
        body: "Unassigned shows only cards with no owner. Use it to find work nobody has picked up.",
      },
      {
        target: '[data-testid="new-record"]',
        title: "Start a ticket here",
        body: "New ticket opens on this board, and its SLA clock starts immediately.",
      },
    ],
  },
  {
    id: "dispatch:v2",
    surfaces: ["dispatch"],
    title: "Book the day",
    steps: [
      {
        target: '[aria-label="Unassigned queue"]',
        title: "Jobs waiting for a slot",
        body: "Every unscheduled job is listed here. A job leaves the list once you book it.",
      },
      {
        target: '[aria-label="Priority"]',
        title: "Filter by priority",
        body: "Show only P1, P2 or P3+ jobs, so you book what your techs can cover today.",
      },
      {
        target: ".dp-lane",
        title: "Drag jobs onto techs",
        body: "Drop a job on a tech's row to book that slot. Drag its edge to change the duration.",
      },
      {
        target: '[data-testid="dispatch-footer"]',
        title: "Book from the keyboard",
        body: "Arrows move, Enter picks up and drops, S splits, Esc cancels. The footer keeps this list on screen.",
      },
    ],
  },
  {
    id: "projects:v2",
    surfaces: ["projects"],
    title: "Find your projects",
    steps: [
      {
        target: '[aria-label="Project views"]',
        title: "Check At risk first",
        body: "All, Active and At risk are three cuts of the same list. At risk shows projects likely to slip.",
      },
      {
        target: '[aria-label="Filter by client"]',
        title: "See one client's work",
        body: "Filter by client to see their projects, budgets and due dates together.",
      },
      {
        target: '[data-testid="new-project"]',
        title: "Start a project",
        body: "New project creates the project record. Add phases and tasks on its own page.",
      },
    ],
  },
  {
    id: "dashboards:v2",
    surfaces: ["dashboards"],
    title: "Build a dashboard",
    steps: [
      {
        target: '[aria-label="Saved arrangements"]',
        title: "Start from a preset",
        body: "Applying a saved arrangement replaces your current layout. Pick the closest one, then edit it.",
      },
      {
        target: '[aria-controls="widget-picker"]',
        title: "Add cards",
        body: "Add widget opens the catalogue. Pick a widget or a saved report to add it as a card.",
      },
      {
        target: '[data-testid="dashboard-describe-entry"]',
        title: "Or describe the card",
        body: "Describe what a card should answer and you get a draft. Nothing is saved until you accept it.",
      },
    ],
  },
  {
    id: "automations:v2",
    surfaces: ["automations"],
    title: "Automate repeat work",
    steps: [
      {
        target: '[data-testid="automation-new-rule"]',
        title: "Build a rule",
        body: "Every rule reads WHEN a trigger fires, IF conditions match, THEN actions run. New rule opens the builder with starters to edit.",
      },
      {
        target: '[data-testid="surface-header-meta"] button',
        title: "Check what's live",
        body: "Enabled counts rules in service. Press it to list only those. Paused rules keep their setup.",
      },
      {
        target: '[aria-label="Authoring view"]',
        title: "See it as a graph",
        body: "Canvas shows the same rules as a graph. Use it when a rule branches and the list is hard to read.",
      },
      {
        target: '#m-main-content a[href="/automations/runs"]',
        title: "Check what ran",
        body: "Runs lists each step a rule took and where it stopped. Open it when a rule didn't do what you expected.",
      },
    ],
  },
  {
    id: "stock:v2",
    surfaces: ["stock"],
    title: "Track parts",
    steps: [
      {
        target: '[data-testid="stock-new-item"]',
        title: "Add parts and tools",
        body: "New item adds a part to the ledger. Turn on the tool switch for kit that leaves the building and returns.",
      },
      {
        target: '[data-testid="stock-receive"]',
        title: "Book arrivals in",
        body: "Receive puts delivered stock into a location. The movement ledger records who moved what.",
      },
      {
        target: '[data-testid="stock-dispatch"]',
        title: "Charge parts to a ticket",
        body: "Dispatch to ticket takes parts out of stock against the ticket that used them, and keeps a record.",
      },
      {
        target: '[data-testid="ledger-shortcut-reorder"]',
        title: "Reorder before you run out",
        body: "At reorder point lists items at their minimum level. Press it to see what to order before a job needs it.",
      },
    ],
  },
  {
    id: "clients:v2",
    surfaces: ["clients"],
    title: "Find a client",
    steps: [
      {
        target: '[aria-label="Client views"]',
        title: "Switch views",
        body: "The same clients, three views: open work, assets and billing history. Switch to the one you need.",
      },
      {
        target: 'input[aria-label="Search clients"]',
        title: "Search by name or contact",
        body: "Type a client or contact name. The record you open holds everything else.",
      },
      {
        target: '[data-testid="new-record"]',
        title: "Add a client",
        body: "New client creates the record. Add sites, contacts and contracts on its own page.",
      },
    ],
  },
  {
    id: "billing:v2",
    surfaces: ["billing"],
    title: "Bill the work",
    steps: [
      {
        target: '[data-testid="billing-stat-overdue"]',
        title: "Chase overdue invoices",
        body: "Overdue counts invoices past their payment terms. Press it to see only those.",
      },
      {
        target: ".m-filter-builder-root button",
        title: "Find an invoice or period",
        body: "Filter by invoice number, client, state or due date to find one invoice or a whole period.",
      },
      {
        target: '[data-testid="surface-header-actions"] button',
        title: "Preview before invoicing",
        body: "New invoice run previews every billable contract for the period before anything issues, and links to the verification workspace.",
      },
    ],
  },
  {
    id: "kb:v2",
    surfaces: ["kb"],
    title: "Find and write answers",
    steps: [
      {
        target: 'input[aria-label="Search knowledge"]',
        title: "Search first",
        body: "Search covers every client's documents, devices and secrets. Press / to jump to the box.",
      },
      {
        target: '[aria-label="Search scope"]',
        title: "Narrow the search",
        body: "Limit results to clients, documents, configurations or secrets.",
      },
      {
        target: '#m-main-content a[href="/kb/articles"]',
        title: "Write up the fix",
        body: "Write up fixes worth keeping under All articles, so the next tech can find them.",
      },
    ],
  },
  {
    id: "project-record:v2",
    surfaces: ["project-record"],
    title: "Run a project",
    steps: [
      {
        target: '[data-slot="commitment-bar"]',
        title: "The promise at a glance",
        body: "Shows the start date, promised go-live, next milestone and anything at risk. A slipped date appears here first.",
      },
      {
        target: '[role="tablist"]',
        title: "Four views of one project",
        body: "Plan holds phases and dates, Timeline charts them, Tasks is the list to triage, Time and budget shows what's been spent.",
      },
      {
        target: '[data-testid="plan-add-task"]',
        title: "Add a task with A",
        body: "Press A, or Task, to add a row where you are. Enter saves it, Escape cancels. Tasks without dates wait under Not dated yet.",
      },
      {
        target: '[data-testid="project-schedule"]',
        title: "Preview date changes",
        body: "Change a start date or scale rows and the preview lists every task that moves before anything saves. Each change is one audit entry.",
      },
    ],
  },
  {
    id: "approvals:v2",
    surfaces: ["approvals"],
    title: "Clear the queue",
    steps: [
      {
        target: '[data-testid="approvals-stat-unapproved"]',
        title: "Read the three counts",
        body: "Unapproved is time waiting on you. Non-billable can't be invoiced. Flagged shows only when the audit has found something.",
      },
      {
        target: '[aria-label="Views"]',
        title: "Queue and history",
        body: "Queue is today's work. Approved, My decisions and Rejected are the record. A rejected entry keeps its reason for the tech to read.",
      },
      {
        target: '[data-testid="approvals-queue"]',
        title: "Decide from the keyboard",
        body: "J and K move, X selects, A approves, R rejects with a reason. You can't approve your own time, so it's read-only.",
      },
    ],
  },
  {
    // `connect-calendar` is on a button that renders in every sync state:
    // the panel always mounts and the button toggles its own label, so the
    // step does not depend on the account being unconnected.
    id: "calendar:v2",
    surfaces: ["calendar"],
    title: "Read the calendar",
    steps: [
      {
        target: '[aria-label="Calendar view"]',
        title: "Week, month or agenda",
        body: "Same entries, three layouts. Week is the working view. Agenda lists what's next without the grid.",
      },
      {
        target: '[aria-label="Entry legend"]',
        title: "Know the entry types",
        body: "Appointments, on call, synced busy time, time off and ticket deadlines each get a distinct pattern, so colour isn't the only clue.",
      },
      {
        target: '[data-testid="week-grid"]',
        title: "Drag to reschedule",
        body: "Appointments snap to 15 minutes. If the new slot is taken, the move is refused and the entry stays put, with the reason shown.",
      },
      {
        target: '[data-testid="connect-calendar"]',
        title: "Connect your calendar",
        body: "Connect calendar sets up two-way sync with an external calendar. Its busy time is included when bookings are checked for conflicts.",
      },
    ],
  },
  {
    id: "ticket-record:v2",
    surfaces: ["ticket-record"],
    title: "Work a ticket",
    steps: [
      {
        target: '[data-testid="wb-ticket-title"]',
        title: "Number, title, status",
        body: "Priority and status sit up here too. The status stepper below moves the ticket on without opening a form.",
      },
      {
        target: '[aria-label="Ticket sections"]',
        title: "Six tabs, one ticket",
        body: "Activity is the timeline, Details holds the fields, Related links parent and child tickets. Time Entries, Files and Notes each keep their own history.",
      },
      {
        target: '[data-testid="notes-composer"]',
        title: "Choose who reads the note",
        body: "The label above the box sets who reads the note: internal note, in-house update, or a client-visible reply that appears in the portal. Check it before you post.",
      },
      {
        target: '[data-testid="wb-close-ticket"]',
        title: "Wrap up and log time",
        body: "Wrap up stops the timer, creates the time entry and applies the closing status in one sheet, so the hours can't go missing.",
      },
    ],
  },
  {
    id: "purchasing:v2",
    surfaces: ["purchasing"],
    title: "Order parts",
    steps: [
      {
        target: '[data-testid="ledger-shortcut-awaiting"]',
        title: "See what's still owed",
        body: "Awaiting receive is every open order with parts still to arrive. Press it to list only those.",
      },
      {
        target: '[data-testid="ledger-shortcut-overdue"]',
        title: "Chase late orders",
        body: "Past expected lists open orders that missed their expected date. Use it to follow up with suppliers.",
      },
      {
        target: '[data-testid="purchasing-new"]',
        title: "Create an order",
        body: "Enter vendor, currency, project, expected date and lines. You land on the order's own page, where receiving and bills live.",
      },
      {
        target: '[data-testid="purchasing-vendors-link"]',
        title: "Check the vendor list",
        body: "Vendors is the directory every order points to. Open it to find who supplies a part.",
      },
    ],
  },
  {
    id: "inventory-workbench:v4",
    surfaces: ["inventory-workbench"],
    title: "Merge duplicates",
    steps: [
      {
        target: '[aria-label="Dedupe queues"]',
        title: "One queue per record type",
        body: "Devices, contacts and clients merge on different keys, so each has its own queue. Each tab's count is what the matcher found.",
      },
      {
        target: '[data-testid="jev-run"]',
        title: "Run the scan first",
        body: "Jev scans for candidates and says how it chose them. If the report is capped, more remain. Nothing merges until you decide.",
      },
      {
        target: '[data-testid="workbench-queue-count"]',
        title: "Pending or deferred",
        body: "Pending is waiting on you. Deferred was set aside deliberately. For each pair, merge, keep apart, or defer until someone can confirm.",
      },
    ],
  },
];

/** The per-session auto-start cap: at most ONE tour starts itself per
 * browser session, so a person hopping surfaces never gets nagged. */
export const SESSION_TOUR_CAP = 1;

/** Whether a tour may run for this viewer: not already completed, and the
 * admin gate satisfied. Selection only; readiness is a separate check. */
export function tourAllowed(
  tour: Tour,
  completed: readonly string[],
  isAdmin: boolean,
): boolean {
  if (completed.includes(tour.id)) return false;
  if (tour.adminOnly && !isAdmin) return false;
  return true;
}

export function tourForSurface(
  surface: string,
  completed: readonly string[],
  isAdmin: boolean,
): Tour | null {
  for (const tour of TOURS) {
    if (!tour.surfaces.includes(surface)) continue;
    if (!tourAllowed(tour, completed, isAdmin)) continue;
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
