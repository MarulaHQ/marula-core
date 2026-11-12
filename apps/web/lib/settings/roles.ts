// The builtin-role catalog — plain-language permission truth for the two
// invite surfaces (the /settings/team dialog and the setup wizard's invite
// step), so both render the SAME words about what a role can do.
//
// Source of truth: core_domain::policy BuiltinRole::permissions()
// (crates/core-domain/src/policy.rs L353-L573). Every summary below was
// derived from that function, not from marketing instinct:
//   owner / admin  — full matrix on all 14 modules + ten admin specials.
//   dispatcher     — full matrix on tickets/scheduling/clients; view+edit on
//                    time/projects/commercial/kb/reporting; view on the two
//                    vaults; special:manage_boards.
//   technician     — full matrix on tickets/kb; view+create+edit on the
//                    vaults; view clients/time/scheduling + create/edit time
//                    + edit scheduling + view reporting; no specials.
//   senior_tech    — the technician set verbatim + special:approve_time.
//   bookkeeper     — full matrix on invoicing; view clients/commercial/time/
//                    reporting; export reporting; view_financials +
//                    manage_billing.
//   auditor        — the view action on all 14 modules + view_audit_log.
//   read_only      — the view action on all 14 modules, nothing else.
//
// The admin/owner permissions() match arm is deliberately the same code path:
// the wire-level difference is ownership protection (the org must always keep
// an active owner), which is what the summaries say.
//
// Non-purpose: no permission LOGIC here. Authorization is the backend's job;
// this file only carries the words the two surfaces show.

export const BUILTIN_ROLES = [
  "owner",
  "admin",
  "dispatcher",
  "senior_tech",
  "technician",
  "bookkeeper",
  "auditor",
  "read_only",
] as const;

export type BuiltinRole = (typeof BUILTIN_ROLES)[number];

/** The role a fresh invite starts on when nobody has said otherwise. */
export const INVITE_DEFAULT_ROLE: BuiltinRole = "technician";

export interface RoleCatalogEntry {
  slug: BuiltinRole;
  /** The human label the two surfaces render everywhere a role is named. */
  label: string;
  /** One sentence: what this person does in the workspace. */
  does: string;
  /** Grouped plain-language permission summary, in reading order. */
  summary: string[];
}

export const ROLE_CATALOG: Record<BuiltinRole, RoleCatalogEntry> = {
  owner: {
    slug: "owner",
    label: "Owner",
    does: "Owns the workspace: every module, every admin right, and billing.",
    summary: [
      "Every module, every action: view, create, edit, delete, export.",
      "Manages users, roles, boards, plugins, API keys and the client portal.",
      "Sees financials, approves time entries, and can read the audit trail.",
      "The org always keeps at least one active owner — that is this seat.",
    ],
  },
  admin: {
    slug: "admin",
    label: "Admin",
    does: "Runs the workspace day to day with the same permissions as the owner.",
    summary: [
      "Every module, every action: view, create, edit, delete, export.",
      "Manages users, roles, boards, plugins, API keys and the client portal.",
      "Sees financials, approves time entries, and can read the audit trail.",
      "The one difference from an owner: the last active owner is protected, so the org can never be locked out.",
    ],
  },
  dispatcher: {
    slug: "dispatcher",
    label: "Dispatcher",
    does: "Runs scheduling and dispatch, keeping the work assigned.",
    summary: [
      "Full control of tickets, scheduling and clients, from creating to deleting and exporting.",
      "Views and edits time entries, projects, price lists, the knowledge base and reports.",
      "Views the password and secrets vaults.",
      "Creates and edits boards.",
      "No invoicing, no financial rates, and no user or role management.",
    ],
  },
  senior_tech: {
    slug: "senior_tech",
    label: "Senior tech",
    does: "Everything a technician does, plus approving other people's time.",
    summary: [
      "Everything a technician holds: tickets, knowledge base, time entries, the vaults.",
      "Approves time entries.",
      "No financial visibility — cost rates stay hidden.",
    ],
  },
  technician: {
    slug: "technician",
    label: "Technician",
    does: "Does the billable work: works tickets and logs time.",
    summary: [
      "Full control of tickets and the knowledge base — create, edit, delete and export.",
      "Creates and edits time entries.",
      "Uses the password and secrets vaults — view, create and edit.",
      "Views clients, edits scheduled work, and views reports.",
      "No approvals, no financial visibility, no user management.",
    ],
  },
  bookkeeper: {
    slug: "bookkeeper",
    label: "Bookkeeper",
    does: "Owns the money side: invoicing and billing, with no ticket work.",
    summary: [
      "Full control of invoicing: runs, voids, payments, credit notes.",
      "Views clients, prices, time entries and reports, and exports reports.",
      "Holds financial visibility and manages billing.",
      "No ticket work at all.",
    ],
  },
  auditor: {
    slug: "auditor",
    label: "Auditor",
    does: "Reviews everything, writes nothing.",
    summary: [
      "Views every module.",
      "Reads the audit trail.",
      "No writes anywhere.",
    ],
  },
  read_only: {
    slug: "read_only",
    label: "Read only",
    does: "Looks around without touching anything.",
    summary: [
      "Views every module.",
      "No writes anywhere, and no audit-trail access.",
    ],
  },
};

/** Step-1 intent choices for the guided invite flow, each mapped to the
 * builtin role it recommends. Admin is reached through the step-2 override
 * dropdown rather than its own chip, because "admin" is a day-to-day answer
 * to "leads the workspace", not a separate job someone is hired for. */
export interface InviteIntent {
  id: string;
  label: string;
  description: string;
  role: BuiltinRole;
}

export const INVITE_INTENTS: InviteIntent[] = [
  {
    id: "leads",
    label: "Leads the workspace",
    description: "Sets up the org, manages people, and answers for it.",
    role: "owner",
  },
  {
    id: "dispatch",
    label: "Runs scheduling and dispatch",
    description: "Assigns and schedules the work.",
    role: "dispatcher",
  },
  {
    id: "tech",
    label: "Does billable technical work",
    description: "Works tickets and logs time.",
    role: "technician",
  },
  {
    id: "senior",
    label: "Senior technical lead",
    description: "Like a technician, plus approving time entries.",
    role: "senior_tech",
  },
  {
    id: "books",
    label: "Manages the books",
    description: "Invoices and billing, no ticket work.",
    role: "bookkeeper",
  },
  {
    id: "audit",
    label: "Reviews records read-only",
    description: "Sees everything including the audit trail; writes nothing.",
    role: "auditor",
  },
  {
    id: "look",
    label: "Just needs to look around",
    description: "View-only access to every module.",
    role: "read_only",
  },
];

/** The catalog row for a role slug, safe for an unknown string (a custom
 * role read back from the directory never reaches this flow, but a stale
 * localStorage or a future rename should not crash a render). */
export function roleCatalog(slug: string): RoleCatalogEntry | undefined {
  return (ROLE_CATALOG as Record<string, RoleCatalogEntry | undefined>)[slug];
}
