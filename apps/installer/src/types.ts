/**
 * The wire shape of `crates/marula-core-cli/src/install/view.rs`.
 *
 * Purpose: one declaration of what the engine sends, shared by both hosts. The
 * native shell receives it from a Tauri command and the loopback fallback from
 * `/api/state`; the payloads are the same struct serialised, so a field renamed
 * on one side is a typecheck failure on this one rather than a blank cell.
 *
 * Non-purpose: no decisions. Nothing here computes a floor, a default or a
 * consequence — those arrive decided, from the descriptor.
 */

export interface HostFacts {
  cores: number;
  ram_mb: number | null;
  free_disk_gb: number | null;
  docker: string | null;
  gpu: string | null;
  lines: string[];
}

export interface Floors {
  cores: number;
  ram_mb: number;
  disk_gb: number;
}

export interface Service {
  name: string;
  role: string;
  ports: number[];
  needs: string[];
}

export interface ProfileSet {
  name: string;
  value: string;
  secret: boolean;
}

export interface Profile {
  name: string;
  title: string;
  description: string;
  floors: Floors;
  services: Service[];
  fits: boolean;
  missing: string[];
  sets: ProfileSet[];
}

export type ParamKind = "string" | "integer" | "boolean" | "url" | "secret";

export interface Param {
  name: string;
  group: string;
  kind: ParamKind;
  required: boolean;
  default: string | null;
  secret: boolean;
  description: string;
  consequence: string;
  default_rationale: string | null;
  requires: string[];
  conflicts: string[];
  /** `"value (profile)"` when a shipped profile pins this lever. */
  profile_value: string | null;
  /** The closed set this lever accepts. Empty means free text. */
  values: string[];
}

export interface ConfigRow {
  name: string;
  /** The descriptor's group, or `infrastructure` for the compose-only values. */
  group: string;
  /** The descriptor's one-line description, or why an infrastructure value exists. */
  description: string;
  value: string;
  secret: boolean;
  present: boolean;
  /** `you` | `profile` | `generated` | `kept` | `default`. */
  source: string;
}

export interface Shortfall {
  profile: string;
  needs: string[];
}

export interface Recommendation {
  profile: string | null;
  why: string[];
  shortfall: Shortfall | null;
  embedding_fits: boolean;
}

export interface Status {
  target: string;
  services: string[];
  error: string | null;
}

export interface State {
  host: HostFacts;
  profiles: Profile[];
  recommendation: Recommendation;
  params: Param[];
  groups: string[];
  env: ConfigRow[];
  status: Status;
  dir: string;
  embedding: string;
  retrieval: string;
  param_count: number;
}

export interface PlanStep {
  kind: "write" | "run";
  headline: string;
  reason: string;
  source: string;
}

export interface Plan {
  profile: string;
  target: string;
  raw: string;
  steps: PlanStep[];
  manual: string[];
  config: ConfigRow[];
  generated: string[];
  /** Floors this host misses, when a developer install planned past them. */
  unmet_floors: string[];
}

export interface Check {
  name: string;
  ok: boolean;
  detail: string;
}

export interface Doctor {
  ok: boolean;
  checks: Check[];
  services: string[];
  error: string | null;
}

export interface ApplyState {
  running: boolean;
  done: string[];
  failed: string | null;
  manual: string[];
}

export interface PlanRequest {
  profile: string;
  overrides: Record<string, string>;
  /** `build` compiles this checkout, `registry` pulls the release's digests. */
  images?: "build" | "registry";
  /** What the long-running services do after an exit. */
  restart?: "unless-stopped" | "always" | "no";
  /** Developer mode may plan a shape this machine cannot carry. */
  allow_unfit?: boolean;
}

export interface Credentials {
  /** A one-time token redeemed from the terminal's URL, browser host only. */
  token?: string;
}
