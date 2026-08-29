// Chart token bridge for Recharts (ADR 0001: Recharts on Workbench tokens).
//
// Purpose: the ONLY place chart colors come from. Every value traces to the
// @meridian/ui PALETTE (the same constants tokens.css is generated from), so
// light and dark modes carry parity and no hex is authored outside the token
// set. Recharts needs concrete color strings (SVG presentation attributes do
// not resolve CSS custom properties), which is why the two mode sets are
// materialized here instead of read from CSS variables.
//
// Non-purpose: no chart layout — components/dashboards renders.
import { PALETTE } from "@meridian/ui";
import type { Theme } from "@meridian/ui";

// PALETTE indices, mirrored from packages/ui/src/tokens.ts (tokens.css §3):
// 0 ink, 1 graphite, 2 slate, 3 mist, 4 fog, 5 paper, 6 canvas-light,
// 7 canvas-dark, 8 surface-dark, 9 border-dark, 10 text-dark, 11 ember,
// 12 ember-dark, 13 success fg light, 15 success fg dark, 21 danger fg
// light, 23 danger fg dark.
const INK = PALETTE[0] as string;
const GRAPHITE = PALETTE[1] as string;
const SLATE = PALETTE[2] as string;
const MIST = PALETTE[3] as string;
const FOG = PALETTE[4] as string;
const PAPER = PALETTE[5] as string;
const SURFACE_DARK = PALETTE[8] as string;
const BORDER_DARK = PALETTE[9] as string;
const TEXT_DARK = PALETTE[10] as string;
const EMBER = PALETTE[11] as string;
const EMBER_DARK = PALETTE[12] as string;
const SUCCESS_FG_LIGHT = PALETTE[13] as string;
const SUCCESS_FG_DARK = PALETTE[15] as string;
const DANGER_FG_LIGHT = PALETTE[21] as string;
const DANGER_FG_DARK = PALETTE[23] as string;

export interface ChartTokens {
  /** Primary series — Ember stays the interactive/primary accent. */
  readonly primary: string;
  /** Secondary series (contrast series on multi-series charts). */
  readonly secondary: string;
  readonly grid: string;
  readonly tick: string;
  readonly tooltipBg: string;
  readonly tooltipText: string;
  readonly tooltipBorder: string;
  readonly success: string;
  readonly danger: string;
}

const LIGHT: ChartTokens = {
  primary: EMBER,
  secondary: SLATE,
  grid: FOG,
  tick: MIST,
  tooltipBg: PAPER,
  tooltipText: INK,
  tooltipBorder: MIST,
  success: SUCCESS_FG_LIGHT,
  danger: DANGER_FG_LIGHT,
};

const DARK: ChartTokens = {
  primary: EMBER_DARK,
  secondary: MIST,
  grid: BORDER_DARK,
  tick: MIST,
  tooltipBg: SURFACE_DARK,
  tooltipText: TEXT_DARK,
  tooltipBorder: GRAPHITE,
  success: SUCCESS_FG_DARK,
  danger: DANGER_FG_DARK,
};

/** Token set for the active theme. Picking happens through the standard
 * `useTheme()` hook so the existing data-theme switch drives parity. */
export function chartTokensFor(theme: Theme): ChartTokens {
  return theme === "dark" ? DARK : LIGHT;
}
