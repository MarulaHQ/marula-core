// Dashboards domain model (the private build tracker R2) — the pure half: widget catalog,
// default presets, per-user widget-row persistence through the settings
// registry, and the RBAC seam.
//
// Persistence: widget rows ride the EXISTING settings-registry machinery
// from wave 2.5 — one user-scope registry key holding a JSON string payload
// (the registry's Text kind; there is no array kind). The Rust declaration
// `dashboards.widgets` lands with the core-side lane; the wire pattern here
// is byte-identical to the 2.5 settings pages (GET /api/v1/settings/user →
// find key; PUT /api/v1/settings/user {key, value}).
//
// Non-purpose: no fetching (reports.ts owns transport), no rendering.

/** The report kinds the R1 API serves (docs: GET /api/v1/reports/{kind}). */
export const REPORT_KINDS = [
  "time_by_user",
  "time_by_client",
  "time_by_ticket",
  "sla_attainment",
  "sla_breaches",
  "ticket_throughput",
  "ticket_aging",
  "first_response",
] as const;
export type ReportKind = (typeof REPORT_KINDS)[number];

/** Data lens of a widget. `self` pins the personal scope; `org` is org-wide. */
export type WidgetLens = "self" | "org";

export type WidgetViz = "bar" | "line" | "table" | "stat";

export interface WidgetDef {
  /** Stable id, `${report kind}:${lens}` — the stored form. */
  readonly id: string;
  readonly kind: ReportKind;
  readonly lens: WidgetLens;
  readonly title: string;
  readonly viz: WidgetViz;
  /** Empty-state sentence shown when the report page has no rows. */
  readonly empty: string;
}

function def(
  kind: ReportKind,
  lens: WidgetLens,
  title: string,
  viz: WidgetViz,
  empty: string,
): WidgetDef {
  return { id: `${kind}:${lens}`, kind, lens, title, viz, empty };
}

/** The fixed widget catalog. Widgets are enumerated, never user-authored —
 * R3 report specs are a later wave; R2 rows only reference these ids. */
export const WIDGET_CATALOG: readonly WidgetDef[] = [
  // Personal lens — meaningful for everyone.
  def("time_by_user", "self", "My time", "bar", "No time logged in the last 30 days."),
  def("ticket_throughput", "self", "My throughput", "line", "No tickets created or resolved in the last 30 days."),
  def("ticket_aging", "self", "My open tickets", "bar", "No open tickets assigned to you."),
  def("sla_attainment", "self", "My SLA attainment", "bar", "No resolved tickets with an SLA policy in the last 30 days."),
  def("first_response", "self", "My first response", "stat", "No tickets assigned to you in the last 30 days."),
  // Org lens — visible to managers only (server forces non-managers onto
  // their own rows, so an org widget would mislabel their data).
  def("time_by_user", "org", "Time by person", "bar", "No time logged in the last 30 days."),
  def("time_by_client", "org", "Time by client", "bar", "No time logged in the last 30 days."),
  def("ticket_throughput", "org", "Ticket throughput", "line", "No tickets created or resolved in the last 30 days."),
  def("ticket_aging", "org", "Open ticket aging", "bar", "No open tickets."),
  def("sla_attainment", "org", "SLA attainment", "bar", "No resolved tickets with an SLA policy in the last 30 days."),
  def("sla_breaches", "org", "SLA breaches", "table", "No SLA breaches in the last 30 days."),
  def("first_response", "org", "First response", "stat", "No tickets created in the last 30 days."),
];

const CATALOG_BY_ID: ReadonlyMap<string, WidgetDef> = new Map(
  WIDGET_CATALOG.map((w) => [w.id, w]),
);

export function widgetDef(id: string): WidgetDef | undefined {
  return CATALOG_BY_ID.get(id);
}

/** A named default arrangement. Defaults-first: these ARE the dashboards
 * until the user edits their rows. */
export interface Preset {
  readonly id: string;
  readonly title: string;
  readonly widgets: readonly string[];
}

export const PRESETS: readonly Preset[] = [
  {
    id: "my_day",
    title: "My day",
    widgets: [
      "time_by_user:self",
      "ticket_throughput:self",
      "ticket_aging:self",
      "first_response:self",
    ],
  },
  {
    id: "team_load",
    title: "Team load",
    widgets: [
      "time_by_user:org",
      "time_by_client:org",
      "ticket_throughput:org",
      "ticket_aging:org",
    ],
  },
  {
    id: "sla_health",
    title: "SLA health",
    widgets: ["sla_attainment:org", "sla_breaches:org", "first_response:org"],
  },
];

// --- RBAC seam ----------------------------------------------------------------

/**
 * Manager test matching the server's report scoping (reports.rs:
 * `is_owner_or_admin()` → org-wide, everyone else self). Among the built-in
 * roles only Owner and Admin hold `special:manage_users`, so that permission
 * is the exact /api/v1/me proxy for the server decision. The server stays
 * the enforcement point either way — non-managers are force-scoped to self
 * on every report request.
 */
export function isManagerPermissions(permissions: readonly string[]): boolean {
  return permissions.includes("special:manage_users");
}

/** Org-lens widgets need the manager seam; personal widgets never do. */
export function widgetVisible(def: WidgetDef, manager: boolean): boolean {
  return def.lens === "self" || manager;
}

/** Presets the current user can actually see (every widget must be visible). */
export function visiblePresets(manager: boolean): Preset[] {
  return PRESETS.filter((p) =>
    p.widgets.every((id) => {
      const d = widgetDef(id);
      return d !== undefined && widgetVisible(d, manager);
    }),
  );
}

/**
 * Defaults-first: the rows a user starts with. `My day` for everyone (its
 * widgets are personal-lens, so RBAC-safe by construction).
 */
export function defaultRows(): string[] {
  return [...(PRESETS[0]?.widgets ?? [])];
}

// --- persistence payload ------------------------------------------------------

/** Registry key holding the per-user widget rows (user scope, Text kind). */
export const WIDGET_ROWS_KEY = "dashboards.widgets";

const PAYLOAD_VERSION = 1;

export interface WidgetRowsPayload {
  version: number;
  widgets: string[];
}

/** Encode rows for the registry value (JSON string — the Text kind). */
export function encodeWidgetRows(rows: readonly string[]): string {
  const payload: WidgetRowsPayload = {
    version: PAYLOAD_VERSION,
    widgets: [...rows],
  };
  return JSON.stringify(payload);
}

/**
 * Decode a stored registry value. Tolerant by design: non-string values,
 * unparseable payloads, wrong versions and unknown/stale widget ids are
 * dropped, and the result dedupes while preserving order. Returns null when
 * nothing usable was stored (callers fall back to defaults).
 */
export function decodeWidgetRows(stored: unknown): string[] | null {
  if (typeof stored !== "string") return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(stored);
  } catch {
    return null;
  }
  if (typeof parsed !== "object" || parsed === null) return null;
  const candidate = parsed as Partial<WidgetRowsPayload>;
  if (candidate.version !== PAYLOAD_VERSION || !Array.isArray(candidate.widgets)) {
    return null;
  }
  const rows: string[] = [];
  for (const id of candidate.widgets) {
    if (typeof id === "string" && CATALOG_BY_ID.has(id) && !rows.includes(id)) {
      rows.push(id);
    }
  }
  return rows;
}

/** Add a widget id to rows (no duplicates). Returns null when already present. */
export function addWidget(rows: readonly string[], id: string): string[] | null {
  if (rows.includes(id) || !CATALOG_BY_ID.has(id)) return null;
  return [...rows, id];
}

/** Remove a widget id from rows. */
export function removeWidget(rows: readonly string[], id: string): string[] {
  return rows.filter((r) => r !== id);
}

// --- display helpers ----------------------------------------------------------

/** Minutes to a compact hours string ("6h 30m" / "45m"). */
export function formatMinutes(minutes: number): string {
  if (!Number.isFinite(minutes) || minutes <= 0) return "0m";
  const h = Math.floor(minutes / 60);
  const m = Math.round(minutes % 60);
  if (h === 0) return `${m}m`;
  if (m === 0) return `${h}h`;
  return `${h}h ${m}m`;
}

/** Seconds to a compact duration ("42s" / "4m 20s" / "1h 12m"). */
export function formatSeconds(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "0s";
  const s = Math.round(seconds);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  const rem = s % 60;
  if (m < 60) return rem === 0 ? `${m}m` : `${m}m ${rem}s`;
  const h = Math.floor(m / 60);
  const mrem = m % 60;
  return mrem === 0 ? `${h}h` : `${h}h ${mrem}m`;
}

/** Fixed presentation labels for ticket-aging buckets (server order). */
export const AGING_BUCKET_LABELS: Record<string, string> = {
  under_1d: "< 1d",
  d1_3: "1-3d",
  d3_7: "3-7d",
  d7_30: "7-30d",
  over_30d: "> 30d",
};
