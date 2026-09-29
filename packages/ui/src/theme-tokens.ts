/**
 * Theme-token schema v1 — the frontend mirror of
 * crates/core-domain/src/theme_pack.rs (docs/theme.md §3, Wave 24).
 *
 * Purpose: generalize the Marula token set into a VERSIONED preset schema and
 * ship the built-in range. `MARULA` is default-v1 — its values are EXACTLY the
 * `--m-*` anchors tokens.css already declares, so selecting any other preset is
 * the only visual delta this schema can produce.
 *
 * Non-purpose: persistence (typed settings on the core-api), distribution
 * (theme packs over ADR 0009), and custom pack application (the packs'
 * coerced manifests arrive from the backend already schema-valid; apply is a
 * `data-preset` attribute swap for built-ins and an inline style-tag for packs).
 *
 * WHAT A PRESET MAY AND MAY NOT MOVE (wave41 D8, brand spec §4.5). A preset
 * restyles surfaces. It may not redefine what a colour MEANS. The primary and
 * its ink-to-gold mode flip, `destructive`, the status pairs and the assistant
 * inversion are the brand's, and an interface built against them has to keep
 * working when the brand is restyled. So the vocabulary below is split in two:
 * SHIFTABLE_AXIS (neutrals, chrome, the accent TINT) is writable, and
 * FIXED_ROLES is not. The fixed roles are deliberately absent from
 * ROLE_TO_TOKEN, which makes the constraint structural rather than a matter of
 * checker configuration — `assertPresetIsLegal` is the belt to that braces.
 *
 * Terracotta, talavera and kraft stay illustration-only per spec §4.2. They
 * become marketplace packs, bound by the same rule.
 */

export const THEME_TOKEN_SCHEMA_VERSION = 1;
export const DEFAULT_PRESET = "marula";

/** The closed semantic role vocabulary (mirrors TOKEN_ROLES in Rust). */
export const TOKEN_ROLES = [
  "background",
  "surface",
  "overlay",
  "border",
  "text",
  "text_secondary",
  "text_muted",
  "accent",
  "on_accent",
  "destructive",
  "status_success_fg",
  "status_success_bg",
  "status_warning_fg",
  "status_warning_bg",
  "status_danger_fg",
  "status_danger_bg",
  "status_neutral_fg",
  "status_neutral_bg",
  "chart_1",
  "chart_2",
  "chart_3",
  "chart_4",
  "chart_5",
] as const;

export type TokenRole = (typeof TOKEN_ROLES)[number];
export type TokenValues = Partial<Record<TokenRole, string>> & Record<string, string | undefined>;

/**
 * Roles a preset may never write. The tokens they would have reached are not
 * in ROLE_TO_TOKEN at all, so a preset naming one is dropped rather than
 * repainted; this list exists so the boundary is legible and testable instead
 * of implied by an omission.
 */
export const FIXED_ROLES = [
  "primary",
  "on_primary",
  "assistant",
  "assistant_foreground",
  "assistant_button",
  "assistant_button_foreground",
  "assistant_line",
  "timeline_client",
  "timeline_note",
  "timeline_system",
  "timeline_time",
  "timeline_agent",
  "gold_text",
  "input",
  "edition_core",
  "edition_enterprise",
  "edition_cloud",
] as const;

export type FixedRole = (typeof FIXED_ROLES)[number];

export interface ThemePreset {
  /** Stable id (settings + data-preset attribute value). */
  readonly id: string;
  readonly label: string;
  readonly description: string;
  readonly light: TokenValues;
  readonly dark: TokenValues;
}

/**
 * `marula` — the Neutral light column of the rendered pack, verbatim from
 * tokens.css. This is the pinned default-v1 (docs/theme.md §2). The accent
 * carried here is the gold-soft TINT, not gold: gold is 1.7:1 on this canvas
 * and fails, and the flipping primary lives on tokens.css where the mode flip
 * happens.
 */
export const MARULA: ThemePreset = {
  id: "marula",
  label: "Marula",
  description: "Neutral canvas, ink text, gold-soft interactive tint. The default.",
  light: {
    background: "#f8f7f4",
    surface: "#ffffff",
    overlay: "#ffffff",
    border: "#e2e0da",
    text: "#2b2a33",
    text_secondary: "#5f584f",
    text_muted: "#6f685d",
    accent: "#f7e3a8",
    on_accent: "#6b4308",
    destructive: "#b42318",
    status_success_fg: "#3b5222",
    status_success_bg: "#e4edcb",
    status_warning_fg: "#6b4308",
    status_warning_bg: "#fbebbf",
    status_danger_fg: "#b42318",
    status_danger_bg: "#fde8e5",
    status_neutral_fg: "#4a443d",
    status_neutral_bg: "#efece6",
    chart_1: "#a8641a",
    chart_2: "#5e7f3a",
    chart_3: "#d9822b",
    chart_4: "#7c746a",
    chart_5: "#a9c25d",
  },
  dark: {
    background: "#2b2a33",
    surface: "#34333d",
    overlay: "#34333d",
    border: "#43424d",
    text: "#f6efd9",
    text_secondary: "#beb5a2",
    text_muted: "#b0a894",
    accent: "#3a3944",
    on_accent: "#e8b83a",
    destructive: "#fb8a80",
    status_success_fg: "#d6e5a8",
    status_success_bg: "#2e3a25",
    status_warning_fg: "#f2cd6b",
    status_warning_bg: "#4a3a1a",
    status_danger_fg: "#fb8a80",
    status_danger_bg: "#3b1a18",
    status_neutral_fg: "#e2cfa9",
    status_neutral_bg: "#3a342a",
    chart_1: "#e8b83a",
    chart_2: "#a9c25d",
    chart_3: "#f0b27a",
    chart_4: "#beb5a2",
    chart_5: "#cdb38b",
  },
};

/**
 * `marula-warm` — the cream column of the same pack, and the mobile default
 * (wave41 D2). It moves the neutral axis and nothing else: the canvas warms,
 * the chrome warms with it, and the border cools a touch. The status pairs are
 * carried here only so the manifest round-trips whole; ROLE_TO_TOKEN will not
 * write them, which is exactly the constraint that keeps both light themes
 * legal under spec §4.5.
 */
export const MARULA_WARM: ThemePreset = {
  id: "marula-warm",
  label: "Marula warm",
  description: "Cream canvas with the same ink and gold. The mobile default.",
  light: {
    background: "#fbf7ec",
    surface: "#ffffff",
    overlay: "#ffffff",
    border: "#e6dcc2",
    text: "#2b2a33",
    text_secondary: "#5f584f",
    text_muted: "#6b6357",
    accent: "#f7e3a8",
    on_accent: "#6b4308",
    destructive: "#b42318",
    status_success_fg: "#3b5222",
    status_success_bg: "#e4edcb",
    status_warning_fg: "#6b4308",
    status_warning_bg: "#fbebbf",
    status_danger_fg: "#b42318",
    status_danger_bg: "#fde8e5",
    // Brand-fixed, so identical to `marula`. The rendered pack varies its
    // offline tint between the two light columns; that variation is the canvas
    // showing through, not a second status pair, so the warm column takes the
    // brand value and the warm canvas supplies the warmth.
    status_neutral_fg: "#4a443d",
    status_neutral_bg: "#efece6",
    chart_1: "#a8641a",
    chart_2: "#5e7f3a",
    chart_3: "#d9822b",
    chart_4: "#7c746a",
    chart_5: "#a9c25d",
  },
  dark: MARULA.dark,
};

export const HIGH_CONTRAST: ThemePreset = {
  id: "high_contrast",
  label: "High contrast",
  description: "Accessibility preset — plain paper, pure ink, boosted status pairs.",
  light: {
    background: "#ffffff",
    surface: "#f2f2f2",
    overlay: "#ffffff",
    border: "#585f66",
    text: "#000000",
    text_secondary: "#24292e",
    text_muted: "#3a4147",
    accent: "#f7e3a8",
    on_accent: "#5c3606",
    destructive: "#8b1a15",
    status_success_fg: "#134f22",
    status_success_bg: "#d8f0dc",
    status_warning_fg: "#5c3606",
    status_warning_bg: "#fbe8b8",
    status_danger_fg: "#8b1a15",
    status_danger_bg: "#fde0dd",
    status_neutral_fg: "#33393e",
    status_neutral_bg: "#dfe2e4",
    chart_1: "#26416b",
    chart_2: "#8f2f18",
    chart_3: "#0f5a3a",
    chart_4: "#5b4a09",
    chart_5: "#2c2333",
  },
  dark: {
    background: "#000000",
    surface: "#101214",
    overlay: "#0a0c0d",
    border: "#8b9499",
    text: "#ffffff",
    text_secondary: "#e4e8ea",
    text_muted: "#cdd6da",
    accent: "#3a3944",
    on_accent: "#e8b83a",
    destructive: "#ff9b92",
    status_success_fg: "#58f585",
    status_success_bg: "#0b2b13",
    status_warning_fg: "#ffd04e",
    status_warning_bg: "#3d2400",
    status_danger_fg: "#ff9b9b",
    status_danger_bg: "#450a0a",
    status_neutral_fg: "#dde3e6",
    status_neutral_bg: "#272b2e",
    chart_1: "#96b9e8",
    chart_2: "#ffbf8f",
    chart_3: "#9be6c8",
    chart_4: "#d9c26a",
    chart_5: "#c7a8e0",
  },
};

/** Shipped built-ins, display order. */
export const BUILTIN_PRESETS: readonly ThemePreset[] = [MARULA, MARULA_WARM, HIGH_CONTRAST];

export const BUILTIN_PRESET_IDS = BUILTIN_PRESETS.map((p) => p.id);

/**
 * The one axis a preset is allowed to move, stated positively so the checker
 * and this module read the same way. Neutrals, chrome and the accent tint.
 */
export const SHIFTABLE_AXIS: readonly TokenRole[] = [
  "background",
  "surface",
  "overlay",
  "border",
  "text",
  "text_secondary",
  "text_muted",
  "accent",
  "on_accent",
  "chart_1",
  "chart_2",
  "chart_3",
  "chart_4",
  "chart_5",
];

/** Roles in TOKEN_ROLES that a preset may not write (wave41 D8). */
export const UNSHIFTABLE_ROLES: readonly TokenRole[] = TOKEN_ROLES.filter(
  (role) => !(SHIFTABLE_AXIS as readonly string[]).includes(role),
);

export function isBuiltinPreset(id: string): boolean {
  return BUILTIN_PRESET_IDS.includes(id);
}

export function presetById(id: string): ThemePreset | undefined {
  return BUILTIN_PRESETS.find((p) => p.id === id);
}

/**
 * The CSS custom-property block for one preset mode: roles → token names.
 * Only the SHIFTABLE axis is writable. `destructive` and the status pairs stay
 * in TOKEN_ROLES so a full pack round-trips through the same validator, but
 * ROLE_TO_TOKEN omits them, so a preset naming one is dropped rather than
 * repainted.
 */
const ROLE_TO_TOKEN: Record<string, string> = {
  background: "--m-canvas",
  surface: "--m-surface",
  overlay: "--m-overlay-surface",
  border: "--m-border",
  text: "--m-text",
  text_secondary: "--m-text-secondary",
  text_muted: "--m-text-muted",
  accent: "--m-accent",
  on_accent: "--m-on-accent",
  chart_1: "--m-chart-1",
  chart_2: "--m-chart-2",
  chart_3: "--m-chart-3",
  chart_4: "--m-chart-4",
  chart_5: "--m-chart-5",
};

const HEX_RE = /^#[0-9a-f]{6}$/;

export function isValidTokenValue(role: string, value: string): boolean {
  return TOKEN_ROLES.includes(role as TokenRole) && HEX_RE.test(value);
}

/** Why a preset is not legal, or `null` when it is. */
export function presetViolation(presetId: string, values: TokenValues): string | null {
  for (const [role, value] of Object.entries(values)) {
    if (typeof value !== "string") continue;
    if ((FIXED_ROLES as readonly string[]).includes(role)) {
      return `${presetId}: "${role}" is a brand-fixed role and may not be set by a theme preset`;
    }
    if ((UNSHIFTABLE_ROLES as readonly string[]).includes(role)) {
      return `${presetId}: "${role}" is outside the shiftable axis (neutrals, chrome, accent tint)`;
    }
    if (!HEX_RE.test(value)) {
      return `${presetId}: "${role}" is not a #rrggbb hex (got "${value}")`;
    }
  }
  return null;
}

/**
 * The subset of a preset's values that it actually DECLARES as an override:
 * the shiftable axis. A built-in carries the whole resolved vocabulary so a
 * full pack round-trips through the same validator; this is the part that
 * reaches the DOM, and the part `presetViolation` is asked to judge.
 */
export function shiftableValues(values: TokenValues): TokenValues {
  const out: TokenValues = {};
  for (const [role, value] of Object.entries(values)) {
    if ((SHIFTABLE_AXIS as readonly string[]).includes(role) && typeof value === "string") {
      out[role] = value;
    }
  }
  return out;
}

/** Throws when a preset would repaint a brand semantic. */
export function assertPresetIsLegal(presetId: string, values: TokenValues): void {
  const violation = presetViolation(presetId, values);
  if (violation) throw new Error(violation);
}

export interface PackTheme {
  slug: string;
  label?: string;
  light?: TokenValues;
  dark?: TokenValues;
}

/** Renders a preset as CSS rules for BOTH modes at SAFE_LIGHT priority:
 * scoped by the `data-preset` attribute so the unset root stays exactly
 * the tokens.css baseline. Used for built-ins (SSR-serialized) and for
 * installed packs (client-injected style tag). */
export function presetCss(
  presetId: string,
  light: TokenValues,
  dark: TokenValues,
): string {
  const rules: string[] = [];
  const role = (roleName: string, value: string): string =>
    ROLE_TO_TOKEN[roleName] ? `${ROLE_TO_TOKEN[roleName]}: ${value};` : "";
  const lines = (values: TokenValues): string[] =>
    Object.entries(values)
      .filter(([roleName, v]) => typeof v === "string" && isValidTokenValue(roleName, v as string))
      .map(([roleName, v]) => role(roleName, v as string))
      .filter(Boolean);
  rules.push(`:root[data-preset="${presetId}"] {\n${lines(light).join("\n")}\n}`);
  rules.push(
    `:root[data-preset="${presetId}"][data-theme="dark"] {\n${lines(dark).join("\n")}\n}`,
  );
  return rules.join("\n\n");
}

export function builtinPresetCss(): string {
  return BUILTIN_PRESETS.filter((p) => p.id !== DEFAULT_PRESET)
    .map((p) => presetCss(p.id, p.light, p.dark))
    .join("\n\n");
}
