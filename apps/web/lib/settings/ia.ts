// The settings IA — eight groups, the pack's page rows, and the real routes.
//
// Purpose: ONE list that both the section rail and the /settings index read,
// so the two can never disagree. The pack (brand/marula-mockups/
// marula-surface-pack, web/settings, states 01) draws eight groups in an
// accordion rail and eight cards on the index; this file is those two views
// of the same table.
//
// Two honest differences from the pack, both recorded in the lane landing
// record rather than hidden:
//   1. The pack drew 26 pages. The app owns more routes than the pack drew,
//      and a route that is not on the index is a route nobody finds. The index
//      draws the pack's rows (so the drawn counts hold) and the rail lists
//      every real page in the same eight groups.
//   2. Where the app has merged two drawn rows onto one route, both rows are
//      kept and both point at that route. A card that listed one page twice
//      would be a lie about the count.
//
// Non-purpose: no permission logic, no data fetching, no layout. A page that
// a user may not open still lists; it guards itself.

export interface SettingsPage {
  /** The label the pack draws. */
  label: string;
  /** The one-line description the pack draws. */
  description: string;
  /** The real route. */
  href: string;
  /** Several drawn rows landing on one route: the extra label shown in place
   * of the description, so the card is honest about the merge. */
  alias?: string;
  /** True when this row exists only because the pack drew it and the app has
   * no route of its own for it — the row is a pointer at the nearest real
   * surface rather than a page. */
  derived?: boolean;
}

export interface SettingsGroup {
  id: string;
  label: string;
  /** The pages the index card draws, in pack order. */
  pages: SettingsPage[];
}

/** The eight groups, with the pack's page rows and descriptions verbatim. */
export const SETTINGS_GROUPS: SettingsGroup[] = [
  {
    id: "workspace",
    label: "Workspace",
    pages: [
      { label: "General", description: "Name, time zone, currency", href: "/settings/general" },
      { label: "Branding", description: "Logo, colours, email footer", href: "/settings/appearance" },
      { label: "Notifications", description: "What reaches whom, and how", href: "/settings/notifications" },
    ],
  },
  {
    id: "organisation",
    label: "Organisation",
    pages: [
      { label: "People", description: "Staff, invitations, teams", href: "/settings/team" },
      {
        label: "Roles and permissions",
        description: "Who can see and change what",
        href: "/settings/team",
        alias: "Same page as People",
      },
      {
        label: "Business hours",
        description: "Working days and holidays",
        href: "/settings/sla",
      },
      {
        label: "Sites and regions",
        description: "Offices, zones, travel",
        href: "/settings/general",
        derived: true,
      },
    ],
  },
  {
    id: "workflow",
    label: "Workflow",
    pages: [
      { label: "Boards", description: "Boards and their columns", href: "/settings/boards" },
      {
        label: "Statuses",
        description: "What a ticket can be, and what that does",
        href: "/settings/workflow",
      },
      {
        label: "Priorities and SLAs",
        description: "Response and resolution targets",
        href: "/settings/sla",
        alias: "Same page as Business hours",
      },
      { label: "Work types", description: "What time is spent on", href: "/settings/time-categories" },
      { label: "Canned replies", description: "Answers you type twice", href: "/settings/reply-snippets" },
    ],
  },
  {
    id: "data",
    label: "Data",
    pages: [
      { label: "Custom fields", description: "Extra facts on records", href: "/settings/taxonomy" },
      { label: "Imports", description: "Bring records in", href: "/imports" },
      { label: "Exports", description: "Take records out", href: "/settings/export" },
      { label: "Retention", description: "How long things are kept", href: "/settings/self-host", derived: true },
    ],
  },
  {
    id: "communication",
    label: "Communication",
    pages: [
      { label: "Email", description: "Mailboxes and routing", href: "/settings/email" },
      { label: "Portal", description: "What clients see", href: "/settings/portal" },
      { label: "Chat and phone", description: "Channels and numbers", href: "/settings/integrations", derived: true },
    ],
  },
  {
    id: "security",
    label: "Security",
    pages: [
      { label: "Sign-in and MFA", description: "How people get in", href: "/settings/security" },
      {
        label: "Devices",
        description: "Which phones may sign in",
        href: "/settings/security",
        alias: "Same page as Sign-in and MFA",
      },
      { label: "Audit log", description: "Who did what, and when", href: "/audit" },
    ],
  },
  {
    id: "ai",
    label: "AI",
    pages: [
      { label: "Overview", description: "Turn AI on, and grant surfaces", href: "/settings/ai" },
      {
        label: "Shadow runs",
        description: "What it would have said",
        href: "/settings/ai/shadow",
      },
    ],
  },
  {
    id: "platform",
    label: "Platform",
    pages: [
      {
        label: "API and webhooks",
        description: "Keys, events and endpoints",
        href: "/settings/integrations",
      },
      { label: "Plugins", description: "Installed and available", href: "/settings/plugins" },
    ],
  },
];

/** Every real /settings route that exists in the app, by group id.
 *
 * The rail reads this; the index reads `SETTINGS_GROUPS`. They cover the same
 * eight groups, and the rail is the one that is allowed to be longer, because
 * a merged product has more pages than the pack drew.
 */
export const SETTINGS_RAIL: Record<string, SettingsPage[]> = {
  workspace: [
    { label: "General", description: "Name, time zone, currency", href: "/settings/general" },
    { label: "Preferences", description: "Your own view of the product", href: "/settings/preferences" },
    { label: "Appearance", description: "Theme, mode and density", href: "/settings/appearance" },
    { label: "Notifications", description: "What reaches whom, and how", href: "/settings/notifications" },
    { label: "Onboarding", description: "What new staff are shown", href: "/settings/onboarding" },
    { label: "Shortcuts", description: "The keyboard profile", href: "/settings/shortcuts" },
  ],
  organisation: [
    { label: "People", description: "Staff, invitations, teams", href: "/settings/team" },
    { label: "Business hours", description: "Working days and holidays", href: "/settings/sla" },
  ],
  workflow: [
    { label: "Boards", description: "Boards and their columns", href: "/settings/boards" },
    { label: "Statuses", description: "What a ticket can be, and what that does", href: "/settings/workflow" },
    { label: "Priorities and SLAs", description: "Response and resolution targets", href: "/settings/sla" },
    { label: "Work types", description: "What time is spent on", href: "/settings/time-categories" },
    { label: "Canned replies", description: "Answers you type twice", href: "/settings/reply-snippets" },
    { label: "Taxonomy", description: "Types and categories", href: "/settings/taxonomy" },
    { label: "Tags", description: "Labels on records", href: "/settings/tags" },
    { label: "PM schedules", description: "Recurring maintenance", href: "/settings/pm-schedules" },
  ],
  data: [
    { label: "Exports", description: "Take records out", href: "/settings/export" },
    { label: "FX rates", description: "The rate book", href: "/settings/fx" },
    { label: "Inventory merge", description: "Identity confidence", href: "/settings/inventory" },
  ],
  communication: [
    { label: "Email", description: "Mailboxes and routing", href: "/settings/email" },
    { label: "Email templates", description: "What outbound mail says", href: "/settings/email/templates" },
    { label: "Portal", description: "What clients see", href: "/settings/portal" },
    { label: "Surveys", description: "CSAT collection", href: "/settings/surveys" },
  ],
  security: [
    { label: "Sign-in and MFA", description: "How people get in", href: "/settings/security" },
    { label: "API keys", description: "Machine credentials", href: "/settings/api-keys" },
    {
      label: "Quarantined files",
      description: "Held uploads and their release trail",
      href: "/settings/quarantine",
    },
  ],
  ai: [
    { label: "Overview", description: "Turn AI on, and grant surfaces", href: "/settings/ai" },
    { label: "Shadow runs", description: "What it would have said", href: "/settings/ai/shadow" },
    { label: "Agent platform", description: "MCP and agent skills", href: "/settings/agent-platform" },
  ],
  platform: [
    { label: "API and webhooks", description: "Keys, events and endpoints", href: "/settings/integrations" },
    { label: "Plugins", description: "Installed and available", href: "/settings/plugins" },
    { label: "Label templates", description: "What a scan means", href: "/settings/labels" },
    { label: "Toolboxes", description: "The day pack", href: "/settings/toolboxes" },
    { label: "Self-host services", description: "The ops stack", href: "/settings/self-host" },
  ],
};

/** The group a pathname belongs to, or null for /settings itself. */
export function groupForPathname(
  pathname: string,
): { group: SettingsGroup; page: SettingsPage } | null {
  for (const [groupId, pages] of Object.entries(SETTINGS_RAIL)) {
    const group = SETTINGS_GROUPS.find((g) => g.id === groupId);
    if (!group) continue;
    for (const page of pages) {
      if (pathname === page.href || pathname.startsWith(`${page.href}/`)) {
        return { group, page };
      }
    }
  }
  return null;
}
