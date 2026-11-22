// Surface sub-navs — the pages a rail section no longer lists, and the page
// that now owns them.
//
// Purpose: one record of every cross-route tab strip, so the rail can stay
// short without stranding a surface. The rail stopped listing Planning and
// Travel (Dispatch draws its own sub-nav), Credit notes and Quotes (the money
// cluster's sub-nav), Loans (Tools), Webhook activity (Runs), Secret categories
// (Vault) and Provenance (Audit). Each of those routes appears in exactly one
// tab set below, and tests/app/rail-model.test.ts counts this file in the
// reachability record — a demoted page with no tab set fails the test.
//
// The strip is the surface pack's tab row (`.tabs2` on the quotes page):
// 14px labels, 10px 2px padding, a 2px underline on the current tab, 22px
// gaps. Not pills: pills are the in-page ViewSwitch, tabs are page-to-page.
//
// `dispatch` is the one set not rendered by <SurfaceSubnav>: the dispatch
// pages draw the same row themselves (`dp-subnav`, on their header row). It is
// recorded here so the reachability test reads truthfully.
//
// Non-purpose: no permissions, no counts, no state. This is a table of links.

export interface SurfaceSubnavTab {
  label: string;
  href: string;
}

export interface SurfaceSubnavSet {
  id: string;
  /** The nav's accessible name. */
  label: string;
  tabs: SurfaceSubnavTab[];
}

export const SURFACE_SUBNAVS: SurfaceSubnavSet[] = [
  {
    id: "dispatch",
    label: "Dispatch sections",
    tabs: [
      { label: "Board", href: "/dispatch" },
      { label: "Planning", href: "/dispatch/planning" },
      { label: "Travel", href: "/dispatch/travel" },
      { label: "Scheduling", href: "/dispatch/scheduling" },
    ],
  },
  {
    // The money cluster, drawn on the pack's quotes page. Pipeline and Catalog
    // join the pack's five because they are the sales half of the same cluster
    // and the rail now points at them from one Sales row.
    id: "money",
    label: "Money",
    tabs: [
      { label: "Pipeline", href: "/sales/pipeline" },
      { label: "Quotes", href: "/sales/quotes" },
      { label: "Catalog", href: "/sales/catalog" },
      { label: "Contracts", href: "/contracts" },
      { label: "Time approvals", href: "/approvals" },
      { label: "Invoices", href: "/billing" },
      { label: "Credit notes", href: "/billing/credit-notes" },
    ],
  },
  {
    id: "stock-tools",
    label: "Tool views",
    tabs: [
      { label: "Tools", href: "/stock/tools" },
      { label: "Client loans", href: "/stock/loans" },
      { label: "Vendor rentals", href: "/stock/vendor-rentals" },
      { label: "E-waste", href: "/stock/ewaste" },
    ],
  },
  {
    id: "automation-activity",
    label: "Automation activity",
    tabs: [
      { label: "Runs", href: "/automations/runs" },
      // Deliveries are administered on the integrations page; the tab says so
      // rather than pretending the ledger lives under /automations.
      { label: "Webhook activity", href: "/settings/integrations" },
    ],
  },
  {
    id: "vault",
    label: "Vault views",
    tabs: [
      { label: "Vault", href: "/passwords" },
      { label: "Secret categories", href: "/kb/secrets" },
    ],
  },
  {
    id: "audit",
    label: "Evidence views",
    tabs: [
      { label: "Audit", href: "/audit" },
      { label: "Provenance", href: "/provenance" },
    ],
  },
];
