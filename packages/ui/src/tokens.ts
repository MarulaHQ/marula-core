/**
 * Typed token API mirroring tokens.css.
 * Purpose: give TS consumers (and tests) a typed view of the Workbench
 * palette, status colors, and WCAG math without re-parsing CSS.
 * Non-purpose: this file defines no new colors — every hex must also exist in
 * tokens.css; a test enforces the two stay in sync.
 */

/** Every hex permitted anywhere in Meridian UI (docs/theme.md §3). */
export const PALETTE = [
  // Greyscale ramp + surfaces
  "#12161C",
  "#3A424C",
  "#55606E",
  "#9AA4B2",
  "#D3D8DF",
  "#FFFFFF",
  "#F4F5F7",
  // Dark mode anchors
  "#101216",
  "#171A1F",
  "#2A2F37",
  "#E8EBEF",
  // Ember
  "#D64B0A",
  "#FF8A4C",
  // Semantic status — light / dark pairs
  "#15803D",
  "#DCFCE7",
  "#4ADE80",
  "#14532D",
  "#B45309",
  "#FEF3C7",
  "#FBBF24",
  "#78350F",
  "#B91C1C",
  "#FEE2E2",
  "#F87171",
  "#7F1D1D",
  "#E5E7EB",
  "#262B33",
] as const;

export type PaletteHex = (typeof PALETTE)[number];

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
