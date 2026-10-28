/**
 * The installer's decisions about presentation, in one testable place.
 *
 * Purpose: everything the screens compute is here as pure functions — which
 * floors a host misses, which lever a value came from, which warnings a choice
 * raises, how a parameter search ranks, and how values group under a human
 * heading. The components render these answers and hold no rules of their own,
 * so a rule can be tested without a browser.
 *
 * Non-purpose: no engine decisions. A profile's floors, a lever's consequence
 * and a default's rationale all arrive from the descriptor; this module only
 * compares, groups and labels them.
 */

import type { ConfigRow, HostFacts, Param, Profile, State } from "../types";

/** Which install the operator asked for on the first screen. */
export type PathMode = "recommended" | "developer";

export interface StepMeta {
  id: "path" | "machine" | "shape" | "settings" | "review";
  label: string;
}

export const STEPS: readonly StepMeta[] = [
  { id: "path", label: "How" },
  { id: "machine", label: "This machine" },
  { id: "shape", label: "Shape" },
  { id: "settings", label: "Settings" },
  { id: "review", label: "Review" },
] as const;

export interface FloorRow {
  label: string;
  need: string;
  have: string;
  ok: boolean;
}

/** The three floors, each compared with the number the host actually reports. */
export function floorRows(profile: Profile, host: HostFacts): FloorRow[] {
  return [
    {
      label: "cores",
      need: `${profile.floors.cores}`,
      have: `${host.cores}`,
      ok: host.cores >= profile.floors.cores,
    },
    {
      label: "memory",
      need: formatMb(profile.floors.ram_mb),
      have: host.ram_mb === null ? "not published" : formatMb(host.ram_mb),
      ok: host.ram_mb !== null && host.ram_mb >= profile.floors.ram_mb,
    },
    {
      label: "disk",
      need: `${profile.floors.disk_gb} GB`,
      have: host.free_disk_gb === null ? "not readable" : `${host.free_disk_gb} GB`,
      ok: host.free_disk_gb !== null && host.free_disk_gb >= profile.floors.disk_gb,
    },
  ];
}

export function formatMb(mb: number): string {
  if (mb >= 1024) {
    const gb = mb / 1024;
    return `${Number.isInteger(gb) ? gb : gb.toFixed(1)} GB`;
  }
  return `${mb} MB`;
}

/** The profile a fresh install starts on: the engine's pick, else the best fit. */
export function startingProfile(state: State): string {
  const recommended = state.profiles.find((profile) => profile.name === state.recommendation.profile);
  if (recommended) return recommended.name;
  const fitting = state.profiles.find((profile) => profile.fits);
  return fitting?.name ?? state.profiles[0]?.name ?? "";
}

export type Source = "you" | "profile" | "generated" | "kept" | "default" | "unset";

export interface Effective {
  value: string | null;
  source: Source;
}

/**
 * What a lever will be when the plan runs: the operator's override, then the
 * profile's preset, then the catalogue default. `null` means the value is absent
 * and the process would refuse to boot without it.
 */
export function effectiveValue(
  name: string,
  state: State,
  profileName: string,
  overrides: Record<string, string>,
): Effective {
  const override = overrides[name];
  if (override !== undefined) {
    return { value: override, source: "you" };
  }
  const profile = state.profiles.find((candidate) => candidate.name === profileName);
  const preset = profile?.sets.find((set) => set.name === name);
  if (preset) return { value: preset.value, source: "profile" };
  const existing = state.env.find((row) => row.name === name && row.present);
  if (existing && existing.source !== "generated") {
    return { value: existing.value, source: "kept" };
  }
  const param = state.params.find((candidate) => candidate.name === name);
  if (param?.default) return { value: param.default, source: "default" };
  return { value: null, source: "unset" };
}

export interface Warning {
  kind: "needs" | "conflicts" | "required";
  text: string;
}

/**
 * What choosing this lever implies. A lever is "in use" when it has a value from
 * any source; its requirements are checked against the same rule, and a conflict
 * fires only when the other lever is in use too.
 */
export function warningsFor(
  param: Param,
  state: State,
  profileName: string,
  overrides: Record<string, string>,
): Warning[] {
  const warnings: Warning[] = [];
  const mine = effectiveValue(param.name, state, profileName, overrides);
  if (mine.value === null && param.required) {
    warnings.push({ kind: "required", text: `${param.name} must be set before the stack will boot.` });
  }
  if (mine.value === null) return warnings;

  for (const need of param.requires) {
    const other = effectiveValue(need, state, profileName, overrides);
    if (other.value === null) {
      warnings.push({ kind: "needs", text: `Also set ${need}, or this lever does nothing.` });
    }
  }
  for (const conflict of param.conflicts) {
    const other = effectiveValue(conflict, state, profileName, overrides);
    if (other.value !== null) {
      warnings.push({
        kind: "conflicts",
        text: `${conflict} is set too, and the process reads only one of them. Clear it first.`,
      });
    }
  }
  return warnings;
}

/** A lever the recommended path asks about, with the words a person would use. */
export interface CuratedField {
  group: string;
  name: string;
  label: string;
  /** Shown under the control as the reason this default is the default. */
  hint?: string;
}

/**
 * The seven decisions the recommended path puts on screen.
 *
 * This is a view of the catalogue, not a second catalogue: a name the
 * descriptor no longer publishes is skipped at render time, and the developer
 * path always edits the whole catalogue.
 */
export const CURATED: readonly CuratedField[] = [
  {
    group: "Address",
    name: "WEB_BASE_URL",
    label: "The address your team will open",
    hint: "Marula prints this in invites and links to it from email.",
  },
  {
    group: "Embeddings",
    name: "MARULA_EMBEDDING_PROVIDER",
    label: "Where search embeddings run",
    hint: "Local keeps ticket text on this machine. OpenAI sends it to a provider.",
  },
  {
    group: "Embeddings",
    name: "MARULA_EMBEDDING_TIER",
    label: "Size of the local model",
    hint: "Bigger models recall more and need more memory. The default fits this machine.",
  },
  {
    group: "Files and objects",
    name: "OBJECT_STORE",
    label: "Where attachments are kept",
    hint: "Local disk is the default. S3 is for a bucket you already back up.",
  },
  {
    group: "Files and objects",
    name: "OBJECT_STORE_LOCAL_DIR",
    label: "Local attachment directory",
    hint: "Used when attachments are kept on this machine.",
  },
  {
    group: "Email",
    name: "EMAIL_TRANSPORT",
    label: "How Marula sends mail",
    hint: "Nothing sends until this and the from-address are set.",
  },
  {
    group: "Email",
    name: "EMAIL_FROM_ADDRESS",
    label: "The address mail comes from",
    hint: "Also what recipients see in the from line.",
  },
] as const;

/** The curated fields that still exist in this descriptor. */
export function curatedFields(state: State): CuratedField[] {
  const published = new Set(state.params.map((param) => param.name));
  return CURATED.filter((field) => published.has(field.name));
}

const GROUP_LABELS: Record<string, string> = {
  ai: "Embeddings and retrieval",
  auth: "Sign-in",
  cache: "Cache",
  cadence: "Background cadence",
  database: "Database",
  email: "Email",
  infrastructure: "Infrastructure",
  lifecycle: "Retention",
  limits: "Rate limits",
  objects: "Files and objects",
  observability: "Observability",
  retries: "Retries",
  routing: "Routing",
  secrets: "Keys and secrets",
  topology: "Address and topology",
};

const GROUP_ORDER = [
  "topology",
  "ai",
  "database",
  "objects",
  "email",
  "lifecycle",
  "infrastructure",
  "auth",
  "secrets",
  "limits",
  "routing",
  "cache",
  "cadence",
  "retries",
  "observability",
];

export function groupLabel(group: string): string {
  return GROUP_LABELS[group] ?? humanLabel(group);
}

export interface ValueGroup {
  group: string;
  title: string;
  rows: ConfigRow[];
}

/** Values under the descriptor's own groups, in a stable order. */
export function groupValues(rows: ConfigRow[]): ValueGroup[] {
  const byGroup = new Map<string, ConfigRow[]>();
  for (const row of rows) {
    // A payload from an older engine carries no group; it goes under `other`
    // rather than taking the window down.
    const group = row.group || "other";
    const list = byGroup.get(group) ?? [];
    list.push(row);
    byGroup.set(group, list);
  }
  const known = GROUP_ORDER.filter((group) => byGroup.has(group));
  const unknown = [...byGroup.keys()].filter((group) => !GROUP_ORDER.includes(group)).sort();
  return [...known, ...unknown].map((group) => ({
    group,
    title: groupLabel(group),
    rows: (byGroup.get(group) ?? []).sort((a, b) => a.name.localeCompare(b.name)),
  }));
}

const LABELS: Record<string, string> = {
  BIND_ADDR: "API listen address",
  DATABASE_URL: "Database URL",
  EMAIL_FROM_ADDRESS: "From address",
  EMAIL_TRANSPORT: "Email transport",
  GARAGE_RPC_SECRET: "Object store RPC secret",
  GLITCHTIP_SECRET_KEY: "Error tracker key",
  MARULA_APP_PASSWORD: "Application database password",
  MARULA_DB_POOL_MAX: "Database pool size",
  MARULA_EMBEDDING_API_KEY: "Embedding API key",
  MARULA_EMBEDDING_PROVIDER: "Embedding provider",
  MARULA_EMBEDDING_TIER: "Local model size",
  MARULA_HOST_MEMORY_MB: "Declared machine memory",
  MARULA_INTERNAL_TOKEN: "Worker seam token",
  MARULA_RETENTION_PROFILE: "Retention profile",
  MARULA_RETRIEVAL_MODE: "Retrieval mode",
  MARULA_SECRET_KEY: "Integration encryption key",
  OBJECT_STORE: "File storage",
  OBJECT_STORE_LOCAL_DIR: "Local file directory",
  POSTGRES_PASSWORD: "Database owner password",
  PUBLIC_BASE_URL: "Public address",
  S3_BUCKET: "Bucket",
  S3_ENDPOINT: "S3 endpoint",
  SMTP_HOST: "SMTP host",
  TRUSTED_PROXIES: "Trusted proxies",
  WEB_BASE_URL: "Address the team opens",
};

/**
 * A lever name in words. The descriptor names are precise and unfriendly; a
 * value table that shows only `MARULA_DB_POOL_MAX` makes the reader translate
 * every row, so the label is human and the technical name moves to the tooltip.
 */
const ACRONYMS = new Set([
  "api",
  "aws",
  "cdn",
  "cf",
  "cidr",
  "cors",
  "csv",
  "db",
  "gpu",
  "http",
  "hmac",
  "http",
  "https",
  "id",
  "ip",
  "jwt",
  "oauth",
  "pdf",
  "rsa",
  "s3",
  "ses",
  "smtp",
  "sso",
  "tls",
  "ttl",
  "uri",
  "url",
  "uuid",
]);

export function humanLabel(name: string): string {
  if (!name) return "";
  const known = LABELS[name];
  if (known) return known;
  const words = name
    .replace(/^(MARULA|EMAIL|S3|AWS_SES|MICROSOFT_GRAPH|GOOGLE_CALENDAR)_/, "")
    .toLowerCase()
    .split("_")
    .filter(Boolean);
  return words
    .map((word) => (ACRONYMS.has(word) ? word.toUpperCase() : word))
    .map((word, index) => (index === 0 ? word.charAt(0).toUpperCase() + word.slice(1) : word))
    .join(" ");
}

/**
 * Ranked search over the whole catalogue. The ranking is deliberately crude and
 * deterministic: a name hit always beats prose, because an operator hunting a
 * lever types its name.
 */
export function searchParams(params: Param[], query: string): Param[] {
  const trimmed = query.trim().toLowerCase();
  if (trimmed === "") return params;
  const scored: { param: Param; score: number }[] = [];
  for (const param of params) {
    const name = param.name.toLowerCase();
    const group = param.group.toLowerCase();
    const prose = `${param.description} ${param.consequence} ${param.default_rationale ?? ""}`.toLowerCase();
    let score = 0;
    if (name === trimmed) score = 100;
    else if (name.startsWith(trimmed)) score = 80;
    else if (name.includes(trimmed)) score = 60;
    else if (group.startsWith(trimmed) || group.includes(trimmed)) score = 40;
    else if (prose.includes(trimmed)) score = 20;
    if (score > 0) scored.push({ param, score });
  }
  scored.sort((a, b) => b.score - a.score || a.param.name.localeCompare(b.param.name));
  return scored.map((entry) => entry.param);
}

/** Parameters under the descriptor's groups, in catalogue order. */
export function paramsByGroup(params: Param[]): { group: string; title: string; params: Param[] }[] {
  const byGroup = new Map<string, Param[]>();
  for (const param of params) {
    const group = param.group || "other";
    const list = byGroup.get(group) ?? [];
    list.push(param);
    byGroup.set(group, list);
  }
  return [...byGroup.entries()].map(([group, list]) => ({
    group,
    title: groupLabel(group),
    params: list,
  }));
}

export interface SourceCounts {
  you: number;
  kept: number;
  generated: number;
  other: number;
}

export function sourceCounts(rows: { source: string }[]): SourceCounts {
  const counts: SourceCounts = { you: 0, kept: 0, generated: 0, other: 0 };
  for (const row of rows) {
    if (row.source === "you") counts.you += 1;
    else if (row.source === "kept") counts.kept += 1;
    else if (row.source === "generated") counts.generated += 1;
    else counts.other += 1;
  }
  return counts;
}

/** One line under a shape's title. */
export function servicesLine(profile: Profile): string {
  const ports = Array.from(new Set(profile.services.flatMap((service) => service.ports))).sort(
    (a, b) => a - b,
  );
  const count = profile.services.length;
  return `${count} ${count === 1 ? "process" : "processes"} · ports ${ports.join(", ")}`;
}

/**
 * A shape in one sentence. The descriptor's own description is written for the
 * docs, where it has room; on a card it is three lines of topology before the
 * reader has chosen. These are the same facts, shortened, and the full sentence
 * stays one disclosure away.
 */
const SHAPE_LINES: Record<string, string> = {
  split: "Two processes, each restarting on its own, behind a transaction pooler.",
  "all-in-one": "One process on one box: the background loops live inside the API.",
};

export function shapeLine(profile: Profile): string {
  return SHAPE_LINES[profile.name] ?? (profile.description.split(". ")[0] ?? profile.description) + ".";
}

/** Whether a step can be left. */
export function canLeave(index: number, state: State, profileName: string): boolean {
  if (index === 0) return true;
  if (index === 2) {
    const profile = state.profiles.find((candidate) => candidate.name === profileName);
    return Boolean(profile?.fits);
  }
  return true;
}
