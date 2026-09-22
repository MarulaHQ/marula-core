/**
 * Theme-token schema v1 — the frontend mirror of
 * crates/core-domain/src/theme_pack.rs (docs/theme.md §3, Wave 24).
 *
 * Purpose: generalize the Tangerine token set into a VERSIONED preset
 * schema and ship the built-in range. `TANGERINE` is default-v1 — its
 * values are EXACTLY the `--m-*` anchors tokens.css already declares, so
 * selecting any other preset is the only visual delta this schema can
 * produce.
 *
 * Non-purpose: persistence (typed settings on the core-api), distribution
 * (theme packs over ADR 0009), and custom pack application (the packs'
 * coerced manifests arrive from the backend already schema-valid; apply
 * is a `data-preset` attribute swap for built-ins and an inline style-tag
 * for packs).
 */

export const THEME_TOKEN_SCHEMA_VERSION = 1;
export const DEFAULT_PRESET = "tangerine";

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

export interface ThemePreset {
  /** Stable id (settings + data-preset attribute value). */
  readonly id: string;
  readonly label: string;
  readonly description: string;
  readonly light: TokenValues;
  readonly dark: TokenValues;
}

/** Tangerine, verbatim from tokens.css — the pinned default-v1 (docs/theme.md §2). */
export const TANGERINE: ThemePreset = {
  id: "tangerine",
  label: "Tangerine",
  description: "The warm default — cream canvas, soft ink, tangerine interactive.",
  light: {
    background: "#e8ebed",
    surface: "#ffffff",
    overlay: "#ffffff",
    border: "#dcdfe2",
    text: "#333333",
    text_secondary: "#4b5563",
    text_muted: "#505967",
    accent: "#e05d38",
    on_accent: "#ffffff",
    destructive: "#ef4444",
    status_success_fg: "#15803d",
    status_success_bg: "#dcfce7",
    status_warning_fg: "#b45309",
    status_warning_bg: "#fef3c7",
    status_danger_fg: "#b91c1c",
    status_danger_bg: "#fee2e2",
    status_neutral_fg: "#4b5563",
    status_neutral_bg: "#e5e7eb",
    chart_1: "#86a7c8",
    chart_2: "#eea591",
    chart_3: "#5a7ca6",
    chart_4: "#466494",
    chart_5: "#334c82",
  },
  dark: {
    background: "#1c2433",
    surface: "#2a3040",
    overlay: "#262b38",
    border: "#3d4354",
    text: "#e5e5e5",
    text_secondary: "#a3a3a3",
    text_muted: "#a3a3a3",
    accent: "#e05d38",
    on_accent: "#ffffff",
    destructive: "#ef4444",
    status_success_fg: "#4ade80",
    status_success_bg: "#14532d",
    status_warning_fg: "#fbbf24",
    status_warning_bg: "#78350f",
    status_danger_fg: "#f87171",
    status_danger_bg: "#7f1d1d",
    status_neutral_fg: "#a3a3a3",
    status_neutral_bg: "#2a303e",
    chart_1: "#86a7c8",
    chart_2: "#e6a08f",
    chart_3: "#5a7ca6",
    chart_4: "#466494",
    chart_5: "#334c82",
  },
};

/** Built-in range (documents/apes the accent-only rule; every value set
 * passes the same AA/3:1 floors the backend asserts at the boundary). */
export const SLATE: ThemePreset = {
  id: "slate",
  label: "Slate",
  description: "Neutral tool-grade grey, cool in both modes.",
  light: {
    background: "#eef0f2",
    surface: "#ffffff",
    overlay: "#ffffff",
    border: "#d3d8dd",
    text: "#111827",
    text_secondary: "#374151",
    text_muted: "#4b5563",
    accent: "#3d5467",
    on_accent: "#ffffff",
    destructive: "#ef4444",
    status_success_fg: "#15803d",
    status_success_bg: "#dcfce7",
    status_warning_fg: "#b45309",
    status_warning_bg: "#fef3c7",
    status_danger_fg: "#b91c1c",
    status_danger_bg: "#fee2e2",
    status_neutral_fg: "#4b5563",
    status_neutral_bg: "#e5e7eb",
    chart_1: "#6b7f92",
    chart_2: "#8ba3b0",
    chart_3: "#4c6273",
    chart_4: "#31414f",
    chart_5: "#1f2b35",
  },
  dark: {
    background: "#141a21",
    surface: "#1e2731",
    overlay: "#182029",
    border: "#333d47",
    text: "#e6e9eb",
    text_secondary: "#a9b3bc",
    text_muted: "#8c98a2",
    accent: "#7fa8c6",
    on_accent: "#10161c",
    destructive: "#ef4444",
    status_success_fg: "#4ade80",
    status_success_bg: "#14532d",
    status_warning_fg: "#fbbf24",
    status_warning_bg: "#78350f",
    status_danger_fg: "#f87171",
    status_danger_bg: "#7f1d1d",
    status_neutral_fg: "#a3a3a3",
    status_neutral_bg: "#2a303e",
    chart_1: "#7f95a6",
    chart_2: "#9db1c0",
    chart_3: "#5f7585",
    chart_4: "#3f5160",
    chart_5: "#2a3844",
  },
};

export const HIGH_CONTRAST: ThemePreset = {
  id: "high_contrast",
  label: "High contrast",
  description: "Accessibility preset — deep ink on plain paper, boosted status pairs.",
  light: {
    background: "#ffffff",
    surface: "#f2f2f2",
    overlay: "#ffffff",
    border: "#585f66",
    text: "#000000",
    text_secondary: "#24292e",
    text_muted: "#3a4147",
    accent: "#7a2e10",
    on_accent: "#ffffff",
    destructive: "#8b1a15",
    status_success_fg: "#134f22",
    status_success_bg: "#d8f0dc",
    status_warning_fg: "#7a4a08",
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
    accent: "#ffb193",
    on_accent: "#000000",
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

export const COPPER: ThemePreset = {
  id: "copper",
  label: "Copper",
  description: "Brand-warm second option — cream canvas, copper interactive, ochre charts.",
  light: {
    background: "#f6efe6",
    surface: "#fffaf3",
    overlay: "#fffaf3",
    border: "#ded0bd",
    text: "#302418",
    text_secondary: "#544232",
    text_muted: "#5f4d3a",
    accent: "#ab4e20",
    on_accent: "#ffffff",
    destructive: "#ba2525",
    status_success_fg: "#1d6634",
    status_success_bg: "#dcf2e0",
    status_warning_fg: "#8a5c0e",
    status_warning_bg: "#fbeabf",
    status_danger_fg: "#991b1b",
    status_danger_bg: "#fde3e3",
    status_neutral_fg: "#514435",
    status_neutral_bg: "#e8e0d4",
    chart_1: "#96694a",
    chart_2: "#c99a6e",
    chart_3: "#6f4a2a",
    chart_4: "#4c3222",
    chart_5: "#322020",
  },
  dark: {
    background: "#221912",
    surface: "#2e241c",
    overlay: "#281f18",
    border: "#4a3c2d",
    text: "#f2eae1",
    text_secondary: "#c9b9a6",
    text_muted: "#a4927e",
    accent: "#e2915f",
    on_accent: "#211610",
    destructive: "#f2866c",
    status_success_fg: "#7bdb9d",
    status_success_bg: "#123a1d",
    status_warning_fg: "#f4cd7c",
    status_warning_bg: "#3d2c09",
    status_danger_fg: "#f79b93",
    status_danger_bg: "#430b0b",
    status_neutral_fg: "#d4c6b1",
    status_neutral_bg: "#332a20",
    chart_1: "#b58a63",
    chart_2: "#d0ab84",
    chart_3: "#85613f",
    chart_4: "#5d4229",
    chart_5: "#3e2a21",
  },
};

/** Shipped built-ins, display order. */
export const BUILTIN_PRESETS: readonly ThemePreset[] = [TANGERINE, SLATE, COPPER, HIGH_CONTRAST];

export const BUILTIN_PRESET_IDS = BUILTIN_PRESETS.map((p) => p.id);

export function isBuiltinPreset(id: string): boolean {
  return BUILTIN_PRESET_IDS.includes(id);
}

export function presetById(id: string): ThemePreset | undefined {
  return BUILTIN_PRESETS.find((p) => p.id === id);
}

/**
 * The CSS custom-property block for one preset mode: roles → token names.
 * Only the CLOSED vocabulary is written (off-schema keys cannot exist —
 * the backend validates themed manifests against the same vocabulary).
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
  destructive: "--m-destructive",
  status_success_fg: "--m-status-success-fg",
  status_success_bg: "--m-status-success-bg",
  status_warning_fg: "--m-status-warning-fg",
  status_warning_bg: "--m-status-warning-bg",
  status_danger_fg: "--m-status-danger-fg",
  status_danger_bg: "--m-status-danger-bg",
  status_neutral_fg: "--m-status-neutral-fg",
  status_neutral_bg: "--m-status-neutral-bg",
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
      .filter(([roleName, v]) => typeof v === "string" && isValidTokenValue(roleName, v))
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
