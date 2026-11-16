// Rail model — the left icon rail's sections and their pages.
//
// Purpose: the app-shell navigation tree (Wave 15 lane 6, restructured in the
// nav-accordion lane). The old top header's grouped nav is absorbed here: every
// staff surface stays reachable through its section's disclosure, at its
// existing URL — no route moves, no dead links (enforced by
// tests/app/rail-model.test.ts, which resolves every href against the actual
// route tree under apps/web/app).
//
// Shape: Today is a destination of its own, first, with no children — it used
// to be the second link inside Tickets, four clicks from the rail. Settings is
// also a leaf: /settings carries its own grouped section rail
// (lib/settings/ia.ts), so twenty settings links do not belong in the main nav.
// Every other section lists its pages inline, with optional `group` headings
// so a long list reads as two or three short ones.
//
// Non-purpose: no permissions filtering (surfaces 404/RBAC-guard themselves);
// no badges beyond the Tickets open-count (owned by the rail component).

export interface RailLink {
  label: string;
  href: string;
  /** Sub-heading inside the section's disclosure. Consecutive links that share
   *  a group render under one heading; the heading is presentational only. */
  group?: string;
}

export interface RailSection {
  id: string;
  label: string;
  /** The section's primary page — where the rail row navigates. */
  href: string;
  /** Pathname prefixes (besides href) that count as "in this section". */
  match: string[];
  links: RailLink[];
}

export const RAIL_SECTIONS: RailSection[] = [
  {
    // Promoted out of Tickets (nav-accordion lane): the day's queue is the
    // surface techs open first, and it was buried inside another section's
    // flyout. No children — /today is a page, not a folder.
    id: "today",
    label: "Today",
    href: "/today",
    match: ["/today"],
    links: [],
  },
  {
    id: "tickets",
    label: "Tickets",
    href: "/tickets",
    match: ["/tickets", "/boards", "/calendar", "/dispatch"],
    links: [
      { label: "All tickets", href: "/tickets" },
      { label: "Boards", href: "/boards" },
      { label: "Calendar", href: "/calendar" },
      { label: "Dispatch", href: "/dispatch" },
      // Planning and Travel are dispatch views, and the dispatch header draws
      // them as its own sub-nav on every dispatch route.
    ],
  },
  {
    id: "clients",
    label: "Clients",
    href: "/clients",
    match: ["/clients", "/projects", "/contracts", "/billing", "/approvals", "/sales"],
    links: [
      { label: "Clients", href: "/clients", group: "Accounts" },
      { label: "Projects", href: "/projects", group: "Accounts" },
      { label: "Contracts", href: "/contracts", group: "Accounts" },
      { label: "Billing", href: "/billing", group: "Money" },
      { label: "Time approvals", href: "/approvals", group: "Money" },
      // The money strip carries Pipeline, Quotes, Catalog, Contracts,
      // Approvals, Invoices and Credit notes (lib/nav/surface-subnavs.ts); the
      // rail keeps one row that lands on the cluster.
      { label: "Sales", href: "/sales/pipeline", group: "Growth" },
    ],
  },
  {
    id: "devices",
    label: "Devices",
    href: "/assets",
    match: ["/assets", "/inventory"],
    links: [
      { label: "Devices", href: "/assets" },
      // This surface is the configuration-item ledger + merge queue, not
      // physical stock. The route keeps its path; /stock owns physical goods.
      { label: "Config inventory", href: "/inventory" },
    ],
  },
  {
    id: "stock",
    label: "Stock",
    href: "/stock",
    match: ["/stock"],
    links: [
      { label: "Stock", href: "/stock" },
      { label: "Locations", href: "/stock/locations" },
      // Loans is the tools board's second tab.
      { label: "Tools", href: "/stock/tools" },
    ],
  },
  {
    id: "purchasing",
    label: "Purchasing",
    href: "/purchasing",
    match: ["/purchasing"],
    links: [
      { label: "Purchase orders", href: "/purchasing" },
      { label: "Vendors", href: "/purchasing/vendors" },
    ],
  },
  {
    id: "automation",
    label: "Automations",
    href: "/automations",
    match: ["/automations", "/alerts"],
    links: [
      { label: "Automations", href: "/automations" },
      { label: "Alerts", href: "/alerts" },
      // Webhook deliveries are the second tab on Runs, and are administered
      // on the integrations page.
      { label: "Runs", href: "/automations/runs" },
    ],
  },
  {
    id: "knowledge",
    label: "Knowledge",
    href: "/kb",
    match: ["/kb", "/documents", "/passwords", "/imports", "/kb/secrets"],
    links: [
      { label: "Knowledge base", href: "/kb" },
      { label: "Documents", href: "/documents" },
      // Secret categories are the vault's second tab.
      { label: "Vault", href: "/passwords" },
      { label: "Imports", href: "/imports" },
    ],
  },
  {
    id: "reports",
    label: "Reports",
    href: "/reports",
    match: ["/reports", "/dashboards", "/scorecards", "/provenance", "/audit"],
    links: [
      { label: "Reports", href: "/reports" },
      { label: "Dashboards", href: "/dashboards" },
      { label: "Scorecards", href: "/scorecards" },
      // Provenance is the audit trail's second tab.
      { label: "Audit", href: "/audit" },
    ],
  },
  {
    // No children: /settings renders its own grouped section rail
    // (apps/web/lib/settings/ia.ts), which lists all 28 settings pages. The
    // main rail used to duplicate twenty of them in one flyout.
    id: "settings",
    label: "Settings",
    href: "/settings",
    match: ["/settings"],
    links: [],
  },
];

/** The rail section that owns a pathname, or null (unknown surfaces). */
export function activeRailSection(pathname: string): RailSection | null {
  for (const section of RAIL_SECTIONS) {
    if (pathname === section.href) return section;
    if (section.match.some((prefix) => pathname === prefix || pathname.startsWith(`${prefix}/`))) {
      return section;
    }
  }
  return null;
}

/** Is a specific rail link "the page you are on"? (exact or child-of). */
export function isRailLinkActive(pathname: string, href: string): boolean {
  return pathname === href || pathname.startsWith(`${href}/`);
}

/**
 * The single page in a section that is "the page you are on".
 *
 * Longest match wins, because the shortest does too much: on
 * /billing/credit-notes both "/billing" and "/billing/credit-notes" match, and
 * marking both `aria-current="page"` tells a screen reader the person is in two
 * places. The nested route is the one page that is actually open.
 */
export function activeRailHref(pathname: string, section: RailSection): string | null {
  let best: string | null = null;
  for (const link of section.links) {
    if (!isRailLinkActive(pathname, link.href)) continue;
    if (best === null || link.href.length > best.length) best = link.href;
  }
  return best;
}
