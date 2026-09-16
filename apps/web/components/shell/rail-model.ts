// Rail model — the left icon rail's sections and their flyout links.
//
// Purpose: the app-shell navigation tree (Wave 15 lane 6). The old top
// header's grouped nav (Workspace/Dispatch/Money/Ops/Settings) is absorbed
// here: every staff surface stays reachable through its section's flyout
// panel, at its existing URL — no route moves, no dead links (enforced by
// tests/app/rail-model.test.tsx, which resolves every href against the
// actual route tree under apps/web/app).
//
// Non-purpose: no permissions filtering (surfaces 404/RBAC-guard
// themselves, as they did under MainNav); no badges beyond the Tickets
// open-count (owned by the rail component).

export interface RailLink {
  label: string;
  href: string;
}

export interface RailSection {
  id: string;
  label: string;
  /** The section's primary page — where the rail icon navigates. */
  href: string;
  /** Pathname prefixes (besides href) that count as "in this section". */
  match: string[];
  links: RailLink[];
}

export const RAIL_SECTIONS: RailSection[] = [
  {
    id: "tickets",
    label: "Tickets",
    href: "/tickets",
    match: ["/tickets", "/today", "/boards", "/calendar", "/dispatch"],
    links: [
      { label: "Tickets", href: "/tickets" },
      { label: "Today", href: "/today" },
      { label: "Boards", href: "/boards" },
      { label: "Calendar", href: "/calendar" },
      { label: "Dispatch board", href: "/dispatch" },
      { label: "Planning", href: "/dispatch/planning" },
      { label: "Travel", href: "/dispatch/travel" },
    ],
  },
  {
    id: "clients",
    label: "Clients",
    href: "/clients",
    match: ["/clients", "/projects", "/contracts", "/billing", "/approvals", "/sales"],
    links: [
      { label: "Clients", href: "/clients" },
      { label: "Projects", href: "/projects" },
      { label: "Contracts", href: "/contracts" },
      { label: "Billing", href: "/billing" },
      { label: "Credit notes", href: "/billing/credit-notes" },
      { label: "Time approvals", href: "/approvals" },
      { label: "Sales pipeline", href: "/sales/pipeline" },
      { label: "Quotes", href: "/sales/quotes" },
      { label: "Catalog", href: "/sales/catalog" },
    ],
  },
  {
    id: "devices",
    label: "Devices",
    href: "/assets",
    match: ["/assets", "/inventory"],
    links: [
      { label: "Devices", href: "/assets" },
      { label: "Inventory", href: "/inventory" },
    ],
  },
  {
    id: "automation",
    label: "Automation",
    href: "/automations",
    match: ["/automations", "/alerts"],
    links: [
      { label: "Automations", href: "/automations" },
      { label: "Alerts", href: "/alerts" },
    ],
  },
  {
    id: "knowledge",
    label: "Knowledge",
    href: "/kb",
    match: ["/kb", "/documents", "/passwords", "/imports"],
    links: [
      { label: "Knowledge base", href: "/kb" },
      { label: "Documents", href: "/documents" },
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
      { label: "Provenance", href: "/provenance" },
      { label: "Audit", href: "/audit" },
    ],
  },
  {
    id: "settings",
    label: "Settings",
    href: "/settings",
    match: ["/settings"],
    links: [
      { label: "Overview", href: "/settings" },
      { label: "Boards", href: "/settings/boards" },
      { label: "General", href: "/settings/general" },
      { label: "Team", href: "/settings/team" },
      { label: "Workflow", href: "/settings/workflow" },
      { label: "Tags", href: "/settings/tags" },
      { label: "PM schedules", href: "/settings/pm-schedules" },
      { label: "SLA", href: "/settings/sla" },
      { label: "Taxonomy", href: "/settings/taxonomy" },
      { label: "FX rates", href: "/settings/fx" },
      { label: "Notifications", href: "/settings/notifications" },
      { label: "Preferences", href: "/settings/preferences" },
      { label: "Portal", href: "/settings/portal" },
      { label: "Mail", href: "/settings/mail" },
      { label: "Security", href: "/settings/security" },
      { label: "API keys", href: "/settings/api-keys" },
      { label: "Integrations", href: "/settings/integrations" },
      { label: "AI", href: "/settings/ai" },
      { label: "Agent platform", href: "/settings/agent-platform" },
    ],
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
