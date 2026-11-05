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

// --- card vocabulary (Wave 46 lane 1 contract) --------------------------------
//
// The payload of `GET /api/report-catalog` is the ONLY source of the span and
// height values this surface draws. `crates/core-reports/src/card.rs` owns the
// names; this file reads them back. Offering a value the payload does not list
// would mean a second vocabulary, which is the drift lane 1 exists to remove.

/** One entry of the catalogue's `shapes`, kept as the server named it. */
export interface CardShapeInfo {
  /** `chart` | `stat` | `table` as the server chose to spell them. */
  readonly shape: string;
  /** Viz hints that produce this shape — how the surface picks it. */
  readonly viz: readonly string[];
  readonly default_span: string;
  readonly default_height: string;
}

/** The card vocabulary, narrowed to the members this surface reads. */
export interface CardVocabulary {
  /** Every span the grid accepts, in the server's order. */
  readonly spans: readonly string[];
  /** Every height the grid accepts, in the server's order. */
  readonly heights: readonly string[];
  readonly shapes: readonly CardShapeInfo[];
  /** The card rules in the server's own words, shown beside the picker. */
  readonly rules: readonly string[];
}

/** Lane 1's `CardSize`: the one size a card stores (`CardSpec.size`). */
export interface CardSize {
  readonly span: string;
  readonly height: string;
}

/** One card on the grid: a widget id plus its size. */
export interface WidgetPlacement {
  readonly id: string;
  readonly size: CardSize;
}

/**
 * A saved report placed on a dashboard.
 *
 * The catalogue's widgets are fixed report kinds; a saved report is a stored
 * definition, and the dashboard stores its id under this prefix. The id is the
 * whole reference — the definition itself (its spec and its card) is read from
 * `GET /api/v1/report-definitions`, so the card a dashboard card draws with is
 * the card the definition stores, never a copy of it.
 */
export const DEFINITION_WIDGET_PREFIX = "definition:";

export function definitionWidgetId(definitionId: string): string {
  return `${DEFINITION_WIDGET_PREFIX}${definitionId}`;
}

/** The definition id behind a placement id, or null when it is a catalogue id. */
export function definitionIdOfWidgetId(id: string): string | null {
  if (!id.startsWith(DEFINITION_WIDGET_PREFIX)) return null;
  const suffix = id.slice(DEFINITION_WIDGET_PREFIX.length);
  return suffix === "" ? null : suffix;
}

export function isDefinitionWidgetId(id: string): boolean {
  return definitionIdOfWidgetId(id) !== null;
}

/** A stored size as a person reads it: `half` is "Half", `extra_wide` is "Extra wide". */
export function humanSlug(slug: string): string {
  const words = slug.replace(/_/g, " ").trim();
  if (words === "") return slug;
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** A non-empty, deduped list of non-empty strings, or null. */
function stringList(value: unknown): string[] | null {
  if (!Array.isArray(value) || value.length === 0) return null;
  const out: string[] = [];
  for (const entry of value) {
    if (typeof entry !== "string" || entry === "") return null;
    if (!out.includes(entry)) out.push(entry);
  }
  return out;
}

/**
 * Narrow a `GET /api/report-catalog` body to the vocabulary the grid can use.
 *
 * Returns null when the payload cannot drive a size control: no spans, no
 * heights, or no shape whose defaults appear in those lists. A shape whose
 * defaults are missing is dropped rather than offered, because choosing it
 * would store a size the grid does not accept.
 */
export function cardVocabulary(payload: unknown): CardVocabulary | null {
  if (typeof payload !== "object" || payload === null) return null;
  const raw = payload as {
    spans?: unknown;
    heights?: unknown;
    shapes?: unknown;
    card_rules?: unknown;
  };
  const spans = stringList(raw.spans);
  const heights = stringList(raw.heights);
  if (spans === null || heights === null) return null;
  if (!Array.isArray(raw.shapes)) return null;
  const shapes: CardShapeInfo[] = [];
  for (const entry of raw.shapes) {
    if (typeof entry !== "object" || entry === null) continue;
    const shape = entry as {
      shape?: unknown;
      viz?: unknown;
      default_span?: unknown;
      default_height?: unknown;
    };
    if (typeof shape.shape !== "string" || shape.shape === "") continue;
    if (!Array.isArray(shape.viz)) continue;
    const viz = shape.viz.filter((v): v is string => typeof v === "string" && v !== "");
    if (viz.length === 0) continue;
    if (typeof shape.default_span !== "string" || !spans.includes(shape.default_span)) continue;
    if (typeof shape.default_height !== "string" || !heights.includes(shape.default_height)) continue;
    shapes.push({
      shape: shape.shape,
      viz,
      default_span: shape.default_span,
      default_height: shape.default_height,
    });
  }
  if (shapes.length === 0) return null;
  const rules = Array.isArray(raw.card_rules)
    ? raw.card_rules.filter((r): r is string => typeof r === "string" && r !== "")
    : [];
  return { spans, heights, shapes, rules };
}

/** The shape a widget's viz resolves to, read from the payload's own `viz`. */
export function shapeForViz(
  vocabulary: CardVocabulary,
  viz: string,
): CardShapeInfo | undefined {
  return vocabulary.shapes.find((shape) => shape.viz.includes(viz));
}

/** The size a widget arrives with when the user has not chosen one. */
export function defaultSizeFor(
  def: WidgetDef,
  vocabulary: CardVocabulary,
): CardSize | null {
  const shape = shapeForViz(vocabulary, def.viz);
  if (shape === undefined) return null;
  return { span: shape.default_span, height: shape.default_height };
}

/** True when a value is a span and a height the payload lists. */
export function isCardSize(
  vocabulary: CardVocabulary,
  value: unknown,
): value is CardSize {
  if (typeof value !== "object" || value === null) return false;
  const size = value as { span?: unknown; height?: unknown };
  return (
    typeof size.span === "string" &&
    typeof size.height === "string" &&
    vocabulary.spans.includes(size.span) &&
    vocabulary.heights.includes(size.height)
  );
}

/**
 * The card a saved report stores (migration 0180), narrowed to what this
 * surface reads. Every member is optional because the wire type's size and
 * refresh carry serde defaults server-side; a client that fills them in itself
 * would be inventing a card.
 */
export interface StoredCard {
  readonly window_days: number;
  readonly refresh?: {
    readonly every_minutes?: number;
    readonly reach_days?: number;
  } | null;
  readonly size?: {
    readonly span?: string;
    readonly height?: string;
  } | null;
}

/** The part of a saved definition the dashboard needs to place and label it. */
export interface DefinitionLike {
  readonly id?: string;
  readonly name?: string;
  readonly card?: StoredCard | null;
  readonly spec: { readonly viz: string };
}

/**
 * The size a saved report arrives with on the grid.
 *
 * A stored card's size is READ and checked against the payload. If the
 * definition stores a size the payload no longer lists, this returns null and
 * the page refuses the placement with a reason — falling back to the shape
 * default would silently change a size the author chose. A definition with no
 * card has no stored size, so the shape default is the honest starting point.
 */
export function definitionSize(
  def: DefinitionLike,
  vocabulary: CardVocabulary,
): CardSize | null {
  const stored = def.card?.size;
  if (stored !== undefined && stored !== null) {
    return isCardSize(vocabulary, stored)
      ? { span: stored.span, height: stored.height }
      : null;
  }
  const shape = shapeForViz(vocabulary, def.spec.viz);
  if (shape === undefined) return null;
  return { span: shape.default_span, height: shape.default_height };
}

/**
 * What a stored card will run with, in the words the row and the card print.
 *
 * Both surfaces read this one function: the saved-report row and the dashboard
 * card show the same three facts, in the same order, because they come from the
 * same stored `CardSpec`.
 */
export interface CardReadout {
  /** "Last 30 days" */
  readonly window: string;
  /** "every 1 h" */
  readonly refresh: string;
  /** "reads 7 days per refresh", or null when one refresh covers the window. */
  readonly reach: string | null;
  /** "Half wide, Regular tall" */
  readonly size: string;
  /** One line for a dense cell: window · refresh · size. */
  readonly line: string;
}

/** Minutes as a compact interval: 60 → "1 h", 90 → "1 h 30 min", 30 → "30 min". */
export function intervalLabel(minutes: number): string {
  if (!Number.isFinite(minutes) || minutes <= 0) return "not set";
  const whole = Math.round(minutes);
  if (whole < 60) return `${whole} min`;
  const hours = Math.floor(whole / 60);
  const rest = whole % 60;
  if (rest === 0) return `${hours} h`;
  return `${hours} h ${rest} min`;
}

export function cardReadout(card: StoredCard | null | undefined): CardReadout | null {
  if (card === null || card === undefined) return null;
  const window = `Last ${card.window_days} day${card.window_days === 1 ? "" : "s"}`;
  const every = card.refresh?.every_minutes;
  const refresh = every === undefined ? "refresh not set" : `every ${intervalLabel(every)}`;
  const reachDays = card.refresh?.reach_days ?? 0;
  const reach =
    reachDays > 0 && reachDays !== card.window_days
      ? `reads ${reachDays} day${reachDays === 1 ? "" : "s"} per refresh`
      : null;
  const span = card.size?.span;
  const height = card.size?.height;
  const size =
    span === undefined && height === undefined
      ? "size not set"
      : `${span === undefined ? "size" : `${humanSlug(span)} wide`}, ${
          height === undefined ? "height" : `${humanSlug(height)} tall`
        }`;
  return { window, refresh, reach, size, line: `${window} · ${refresh} · ${size}` };
}

/**
 * Turn widget ids into placements at their shape defaults. Ids whose shape the
 * payload does not describe are dropped: a card cannot be stored without a
 * size the grid accepts, and inventing one is the second vocabulary again.
 */
export function defaultPlacements(
  vocabulary: CardVocabulary,
  ids: readonly string[] = defaultRows(),
): WidgetPlacement[] {
  const out: WidgetPlacement[] = [];
  for (const id of ids) {
    const def = CATALOG_BY_ID.get(id);
    if (def === undefined) continue;
    const size = defaultSizeFor(def, vocabulary);
    if (size === null) continue;
    out.push({ id, size });
  }
  return out;
}

// --- persistence payload ------------------------------------------------------

/** Registry key holding the per-user widget rows (user scope, Text kind). */
export const WIDGET_ROWS_KEY = "dashboards.widgets";

const PAYLOAD_VERSION = 2;
/** The shipped payload: widget ids only, no size. Migrated on read. */
const LEGACY_VERSION = 1;

export interface WidgetsPayload {
  version: number;
  widgets: WidgetPlacement[];
}

/** Encode placements for the registry value (JSON string — the Text kind). */
export function encodeWidgets(placements: readonly WidgetPlacement[]): string {
  const payload: WidgetsPayload = {
    version: PAYLOAD_VERSION,
    widgets: placements.map((p) => ({
      id: p.id,
      size: { span: p.size.span, height: p.size.height },
    })),
  };
  return JSON.stringify(payload);
}

/**
 * Why a stored arrangement could not be read.
 *
 * `size` is the one the card vocabulary owns: a span or height the payload no
 * longer lists. The row's rule covers that case, and the page falls back to the
 * default arrangement. The others mean the document itself is not one this
 * client can read, so the page must not treat it as "no arrangement" and write
 * defaults over it.
 */
export type ArrangementProblem = "version" | "format" | "id" | "size";

export type ArrangementRead =
  | { readonly status: "read"; readonly placements: WidgetPlacement[] }
  /** No arrangement has ever been written: the registry's default document. */
  | { readonly status: "unset" }
  | { readonly status: "unreadable"; readonly problem: ArrangementProblem };

/** True when a placement id names a catalogue widget or a saved report. */
export function knownWidgetId(id: string): boolean {
  return CATALOG_BY_ID.has(id) || isDefinitionWidgetId(id);
}

/**
 * Read a stored registry value against the vocabulary, and say why when it
 * cannot be read.
 *
 * Version 2 is strict. Version 1 (the ids-only payload the shipped page wrote)
 * is migrated by giving each id its shape default, so an existing arrangement
 * survives the upgrade.
 */
export function readArrangement(
  stored: unknown,
  vocabulary: CardVocabulary,
): ArrangementRead {
  if (typeof stored !== "string") {
    return { status: "unreadable", problem: "format" };
  }
  // The registry serves every declared user setting with its default, so an
  // undeclared value is not the only "nothing stored yet": `{}` is the
  // default document and means exactly that. It is not a corrupt arrangement.
  if (stored.trim() === "") {
    return { status: "unset" };
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(stored);
  } catch {
    return { status: "unreadable", problem: "format" };
  }
  if (typeof parsed !== "object" || parsed === null) {
    return { status: "unreadable", problem: "format" };
  }
  const candidate = parsed as { version?: unknown; widgets?: unknown };
  // A document with no version and no widgets is the default, not a payload a
  // newer client wrote: a version we do not know is the case that must not be
  // overwritten.
  if (candidate.version === undefined && candidate.widgets === undefined) {
    return { status: "unset" };
  }
  if (!Array.isArray(candidate.widgets)) {
    return { status: "unreadable", problem: "format" };
  }

  if (candidate.version === LEGACY_VERSION) {
    const migrated: WidgetPlacement[] = [];
    for (const id of candidate.widgets as unknown[]) {
      if (typeof id !== "string" || !CATALOG_BY_ID.has(id)) continue;
      if (migrated.some((p) => p.id === id)) continue;
      const def = CATALOG_BY_ID.get(id);
      if (def === undefined) continue;
      const size = defaultSizeFor(def, vocabulary);
      if (size === null) continue;
      migrated.push({ id, size });
    }
    return { status: "read", placements: migrated };
  }

  if (candidate.version !== PAYLOAD_VERSION) {
    return { status: "unreadable", problem: "version" };
  }
  const placements: WidgetPlacement[] = [];
  for (const entry of candidate.widgets as unknown[]) {
    if (typeof entry !== "object" || entry === null) {
      return { status: "unreadable", problem: "format" };
    }
    const widget = entry as { id?: unknown; size?: unknown };
    if (typeof widget.id !== "string" || !knownWidgetId(widget.id)) {
      return { status: "unreadable", problem: "id" };
    }
    if (placements.some((p) => p.id === widget.id)) {
      return { status: "unreadable", problem: "id" };
    }
    if (!isCardSize(vocabulary, widget.size)) {
      return { status: "unreadable", problem: "size" };
    }
    placements.push({
      id: widget.id,
      size: { span: widget.size.span, height: widget.size.height },
    });
  }
  return { status: "read", placements };
}

/**
 * Decode a stored registry value to placements, or null when it cannot be read.
 *
 * Kept as the narrow form for callers that only need the placements. The page
 * uses [`readArrangement`] instead, because a version, format or identity
 * problem must not be mistaken for "no arrangement" — writing defaults over it
 * would destroy the arrangement the user had.
 */
export function decodeWidgets(
  stored: unknown,
  vocabulary: CardVocabulary,
): WidgetPlacement[] | null {
  const read = readArrangement(stored, vocabulary);
  return read.status === "read" ? read.placements : null;
}

/**
 * Add any placement at an explicit size, or null when it is already present or
 * the size is not one the payload lists.
 *
 * This is the seam a saved report uses: the dashboard reads the definition's
 * stored size (or its shape default), checks it against the same vocabulary,
 * and stores the id under `definition:`.
 */
export function addPlacement(
  placements: readonly WidgetPlacement[],
  id: string,
  size: CardSize,
  vocabulary: CardVocabulary,
): WidgetPlacement[] | null {
  if (placements.some((p) => p.id === id)) return null;
  if (!isCardSize(vocabulary, size)) return null;
  return [...placements, { id, size: { span: size.span, height: size.height } }];
}

/**
 * Add a widget at an explicit size, or at its shape default.
 *
 * The explicit size is the seam a generated card uses: it arrives with the span
 * and height it wants, and both are checked against the payload before it is
 * stored. A size the payload does not list is refused. Returns null when the
 * card is already present, unknown, or has no usable size.
 */
export function addWidget(
  placements: readonly WidgetPlacement[],
  id: string,
  vocabulary: CardVocabulary,
  size?: CardSize,
): WidgetPlacement[] | null {
  const def = CATALOG_BY_ID.get(id);
  if (def === undefined) return null;
  const resolved = size ?? defaultSizeFor(def, vocabulary);
  if (resolved === null) return null;
  return addPlacement(placements, id, resolved, vocabulary);
}

/** Remove a widget id from the grid. */
export function removeWidget(
  placements: readonly WidgetPlacement[],
  id: string,
): WidgetPlacement[] {
  return placements.filter((p) => p.id !== id);
}

/** Move a card one slot earlier (-1) or later (1); no-op at the edges. */
export function moveWidget(
  placements: readonly WidgetPlacement[],
  id: string,
  direction: -1 | 1,
): WidgetPlacement[] {
  const from = placements.findIndex((p) => p.id === id);
  const to = from + direction;
  if (from === -1 || to < 0 || to >= placements.length) return [...placements];
  const next = [...placements];
  const moved = next[from] as WidgetPlacement;
  next[from] = next[to] as WidgetPlacement;
  next[to] = moved;
  return next;
}

/** Resize one card; null when the id is absent or the size is not offered. */
export function resizeWidget(
  placements: readonly WidgetPlacement[],
  id: string,
  size: CardSize,
  vocabulary: CardVocabulary,
): WidgetPlacement[] | null {
  if (!placements.some((p) => p.id === id)) return null;
  if (!isCardSize(vocabulary, size)) return null;
  const next = placements.map((p) =>
    p.id === id ? { id: p.id, size: { span: size.span, height: size.height } } : p,
  );
  return next;
}

/**
 * The arrangement a preset applies. A card already on the grid keeps the size
 * the user chose; a card the preset adds arrives at its shape default. Null
 * when a preset member has no default the payload describes.
 */
export function placementsForPreset(
  ids: readonly string[],
  current: readonly WidgetPlacement[],
  vocabulary: CardVocabulary,
): WidgetPlacement[] | null {
  const out: WidgetPlacement[] = [];
  for (const id of ids) {
    const existing = current.find((p) => p.id === id);
    if (existing !== undefined) {
      out.push(existing);
      continue;
    }
    const def = CATALOG_BY_ID.get(id);
    if (def === undefined) return null;
    const size = defaultSizeFor(def, vocabulary);
    if (size === null) return null;
    out.push({ id, size });
  }
  return out;
}

/** True when the grid holds exactly these ids, in this order. */
export function placementsMatch(
  placements: readonly WidgetPlacement[],
  ids: readonly string[],
): boolean {
  return (
    placements.length === ids.length && placements.every((p, i) => p.id === ids[i])
  );
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
