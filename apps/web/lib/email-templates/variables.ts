// The per-kind placeholder vocabulary + human labels for the template editor.
//
// Source of truth: `core_domain::templates::variables_for_kind` (read-only
// reference) — the same closed lists the server validates writes against.
// The editor prefers the vocabulary the API returns with the templates list
// (`GET /api/v1/templates` → `vocabulary: kind → keys`), and falls back to
// these static lists when that call is unavailable, so the dropdown and the
// server cannot disagree for long: a stale static list only delays an
// insertion, the server's write-time check still rejects anything
// out-of-vocabulary with a typed 400.

/** The template kinds, in the order the editor and index show them. */
export const TEMPLATE_KIND_ORDER = [
  "email",
  "ticket",
  "note",
  "quote_invoice",
  "project",
  "workflow",
  "kb",
] as const;

/** Human label for a template kind. */
export function templateKindLabel(kind: string): string {
  switch (kind) {
    case "email":
      return "Email";
    case "ticket":
      return "Ticket";
    case "note":
      return "Note";
    case "quote_invoice":
      return "Quote and invoice";
    case "project":
      return "Project";
    case "workflow":
      return "Workflow";
    case "kb":
      return "Knowledge base";
    default:
      // A kind a newer backend seeded: show the raw kind rather than hide it.
      return kind.replace(/_/g, " ").replace(/^\w/, (c) => c.toUpperCase());
  }
}

/** Ticket-flavored kinds share one vocabulary (ticket + client + contact). */
const EMAIL_VARIABLES = [
  "org.name",
  "user.name",
  "client.name",
  "contact.name",
  "contact.email",
  "contact.phone",
  "ticket.number",
  "ticket.summary",
  "ticket.priority",
  "ticket.status",
  "assignee.name",
] as const;

const QUOTE_VARIABLES = [
  "org.name",
  "user.name",
  "client.name",
  "contact.name",
  "contact.email",
  "contact.phone",
  "quote.number",
  "quote.status",
  "quote.total",
  "quote.currency",
] as const;

const PROJECT_VARIABLES = [
  "org.name",
  "user.name",
  "client.name",
  "contact.name",
  "contact.email",
  "contact.phone",
  "project.name",
  "project.status",
] as const;

const KB_VARIABLES = ["org.name", "user.name"] as const;

/** The static fallback vocabulary, mirroring `variables_for_kind`. */
const STATIC_VOCABULARY: Record<string, readonly string[]> = {
  email: EMAIL_VARIABLES,
  ticket: EMAIL_VARIABLES,
  note: EMAIL_VARIABLES,
  workflow: EMAIL_VARIABLES,
  quote_invoice: QUOTE_VARIABLES,
  project: PROJECT_VARIABLES,
  kb: KB_VARIABLES,
};

/**
 * The placeholder list for one kind: the API's vocabulary when the list
 * call supplied it, the static mirror otherwise. Unknown kinds (a newer
 * backend's kind this build has never seen) get an empty list — insertion
 * stays off for them rather than guessing at keys.
 */
export function vocabularyForKind(kind: string, fromApi?: { kind: string; variables: string[] }[]): string[] {
  const entry = fromApi?.find((v) => v.kind === kind);
  if (entry) return entry.variables;
  return [...(STATIC_VOCABULARY[kind] ?? [])];
}

/** Human label for one placeholder key: `client.name` → "Client name". */
export function variableLabel(key: string): string {
  return key
    .split(".")
    .map((segment) =>
      (segment === "org" ? "organisation" : segment.replace(/_/g, " ")).replace(
        /^\w/,
        (c) => c.toUpperCase(),
      ),
    )
    .join(" ");
}
