/**
 * Typed token API mirroring tokens.css.
 * Purpose: give TS consumers (and tests) a typed view of the Marula palette,
 * status colors, and WCAG math without re-parsing CSS.
 * Non-purpose: this file defines no new colors — every hex must also exist in
 * tokens.css; a test enforces the two stay in sync.
 *
 * Tangerine is retired. The retired ramp is not carried here in any form: no
 * #E05D38, no cool-grey #E8EBED canvas, no blue-slate dark mode. The CI
 * hex-scan blacklists them so they cannot return through the front door.
 */

/** Every hex permitted anywhere in Marula UI (docs/theme.md §3).
 * Preset-only surfaces live in the shadcn layer as verbatim oklch and are
 * intentionally NOT part of the --m-* hex set. */
export const PALETTE = [
  // Brand primitives (DESIGN-NOTES §7)
  "#E8B83A", // gold
  "#F7E3A8", // gold-soft, the interactive tint
  "#9A5B0F", // gold-text, deep amber for text on light
  "#A8641A", // ring on light
  "#2B2A33", // dusk ink
  "#222129", // the bar in dark mode — still ink
  "#F6EFD9", // flesh cream
  "#5E7F3A", // canopy
  "#4A6A2A", // deep canopy, text on a canopy fill
  "#A9C25D", // unripe lime
  "#D9822B", // ripe blush
  "#CDB38B", // bushveld sand
  "#7C746A", // bark grey
  // Light: the `marula` preset, Neutral column
  "#F8F7F4", // canvas
  "#FFFFFF", // surface, nav, client entry
  "#EFEEEA", // surface-2, sidebar, muted, disabled fill
  "#5F584F", // secondary text
  "#6F685D", // muted text
  "#E2E0DA", // border
  "#8C877C", // input
  "#8A8479", // disabled foreground
  "#6B4308", // on-accent / progress / time-entry text
  "#EFEDE6", // meter track (logged-vs-expected bar)
  // Dark: the `marula` preset, Dark column
  "#34333D", // surface
  "#3A3944", // surface-2, accent, disabled fill
  "#BEB5A2", // secondary text
  "#B0A894", // muted text
  "#43424D", // border
  "#8A8378", // input
  "#9A9384", // disabled foreground
  // Timeline entry tints
  "#EFECE6", // internal note
  "#4A443D", // note text, neutral status text
  "#2F2E37", // system entry
  "#FBEBBF", // time entry
  "#F2CD6B", // time-entry text
  "#F7F3E8", // assistant entry
  "#4A3A1A", // progress + time-entry background (dark)
  "#3A342A", // note background (dark), offline background
  "#E2CFA9", // note text (dark), neutral/offline text (dark)
  "#33302A", // assistant entry (dark)
  // Status pairs
  "#3B5222", // success text
  "#E4EDCB", // success background
  "#D6E5A8", // success text (dark)
  "#2E3A25", // success background (dark)
  "#7A3F0C", // due-today text
  "#F8DCC0", // due-today background
  "#F0B27A", // due-today text (dark)
  "#4D2F14", // due-today background (dark)
  "#B42318", // destructive actions and P1 — the pack's own red
  "#FB8A80", // destructive actions and P1 (dark)
  // Breach — the danger status pair the kit declares and the pack draws
  "#FBE1DE", // breach background
  "#8F1C13", // breach text
  "#4A1A18", // breach background (dark)
  "#F9A097", // breach text (dark)
  // The rest of the kit's status-pair family, carried because the kit owns it
  "#ECE7DF", // new background
  "#3F3D45", // new background (dark)
  "#F3EAD2", // waiting background, also the cream column's muted fill
  "#D8CFBA", // new text (dark), scheduled text (dark)
  "#D5E3C5", // resolved background
  "#2F4A1E", // resolved text
  "#26361A", // resolved background (dark)
  "#BFD99A", // resolved text (dark)
  "#5C4631", // offline text
  // The cream column (marula-warm, the client portal's sand, marketing cream)
  "#FBF7EC", // canvas
  "#E6DCC2", // border
  "#857A66", // input
  "#FBF3DC", // assistant entry
  "#000000", // dark scrim and overlay shadow
] as const;

export type PaletteHex = (typeof PALETTE)[number];

/**
 * The same hexes, addressed BY NAME.
 *
 * PALETTE is the ordered whitelist the CI hex-scan and the tokens test read,
 * so its order is a contract and nothing should index into it. This record is
 * how a consumer names a colour; every entry is drawn from PALETTE, and a
 * test asserts the two stay identical, so a reorder of PALETTE can never
 * silently repoint a chart series at a different hue.
 */
export const TOKEN = {
  gold: "#E8B83A",
  goldSoft: "#F7E3A8",
  goldText: "#9A5B0F",
  ring: "#A8641A",
  // The logged-vs-expected meter's own pair. It exists because the accent tint
  // cannot carry a 3:1 indicator: gold-soft is 1.04:1 on the light canvas and
  // the dark tint is a neutral at 1.19:1. Found by the vision review.
  meterTrack: "#EFEDE6",
  meterFill: "#9A5B0F",
  ink: "#2B2A33",
  bar: "#222129",
  cream: "#F6EFD9",
  canopy: "#5E7F3A",
  canopyDeep: "#4A6A2A",
  lime: "#A9C25D",
  blush: "#D9822B",
  sand: "#CDB38B",
  bark: "#7C746A",
  canvas: "#F8F7F4",
  surface: "#FFFFFF",
  client: "#FFFFFF", // timeline client entry: a surface box with a border
  surface2: "#EFEEEA",
  text: "#2B2A33",
  textSecondary: "#5F584F",
  textMuted: "#6F685D",
  border: "#E2E0DA",
  input: "#8C877C",
  disabledFg: "#8A8479",
  onAccent: "#6B4308",
  surfaceDark: "#34333D",
  surface2Dark: "#3A3944",
  textSecondaryDark: "#BEB5A2",
  textMutedDark: "#B0A894",
  borderDark: "#43424D",
  inputDark: "#8A8378",
  disabledFgDark: "#9A9384",
  note: "#EFECE6",
  noteFg: "#4A443D",
  sys: "#F8F7F4",
  sysDark: "#2F2E37",
  time: "#FBEBBF",
  timeFg: "#6B4308",
  timeFgDark: "#F2CD6B",
  agent: "#F7F3E8",
  timeBgDark: "#4A3A1A",
  noteBgDark: "#3A342A",
  noteFgDark: "#E2CFA9",
  agentDark: "#33302A",
  successFg: "#3B5222",
  successBg: "#E4EDCB",
  successFgDark: "#D6E5A8",
  successBgDark: "#2E3A25",
  dueFg: "#7A3F0C",
  dueBg: "#F8DCC0",
  dueFgDark: "#F0B27A",
  dueBgDark: "#4D2F14",
  dangerFg: "#8F1C13",
  dangerBg: "#FBE1DE",
  dangerFgDark: "#F9A097",
  dangerBgDark: "#4A1A18",
  offlineFg: "#5C4631",
  destructive: "#B42318",
  black: "#000000",
} as const;

export type TokenName = keyof typeof TOKEN;

export const STATUS_TONES = ["success", "warning", "danger", "neutral"] as const;
export type StatusTone = (typeof STATUS_TONES)[number];

export interface Color {
  readonly r: number;
  readonly g: number;
  readonly b: number;
}

/** Parse `#RGB`/`#RRGGBB` into RGB channels. Throws on malformed input. */
export function hexToRgb(hex: string): Color {
  const value = hex.replace("#", "");
  const full =
    value.length === 3
      ? [...value].map((c) => c + c).join("")
      : value;
  if (!/^[0-9a-fA-F]{6}$/.test(full)) {
    throw new Error(`Malformed hex color: ${hex}`);
  }
  return {
    r: parseInt(full.slice(0, 2), 16),
    g: parseInt(full.slice(2, 4), 16),
    b: parseInt(full.slice(4, 6), 16),
  };
}

/** WCAG 2.1 relative luminance of a color. */
export function relativeLuminance(color: Color): number {
  const channel = (raw: number): number => {
    const srgb = raw / 255;
    return srgb <= 0.03928 ? srgb / 12.92 : ((srgb + 0.055) / 1.055) ** 2.4;
  };
  return (
    0.2126 * channel(color.r) +
    0.7152 * channel(color.g) +
    0.0722 * channel(color.b)
  );
}

/**
 * WCAG 2.1 contrast ratio between two hex colors (1 … 21).
 */
export function contrastRatio(foreground: string, background: string): number {
  const l1 = relativeLuminance(hexToRgb(foreground));
  const l2 = relativeLuminance(hexToRgb(background));
  const [lighter, darker] = l1 >= l2 ? [l1, l2] : [l2, l1];
  return (lighter + 0.05) / (darker + 0.05);
}

/** CSS variable name for a status tone's foreground/background pair. */
export function statusVar(tone: StatusTone, part: "fg" | "bg"): string {
  return `--m-status-${tone}-${part}`;
}
