// Chart token bridge for Recharts (ADR 0001: Recharts on Tangerine tokens).
//
// Purpose: the ONLY place chart colors come from. Every value traces to the
// @marula-core/ui PALETTE (the same constants tokens.css is generated from), so
// light and dark modes carry parity and no hex is authored outside the token
// set. Recharts needs concrete color strings (SVG presentation attributes do
// not resolve CSS custom properties), which is why the two mode sets are
// materialized here instead of read from CSS variables.
//
// Chart series follow the Tangerine preset ramp (docs/theme.md §6): primary
// series is the Tangerine accent; the preset's slate→orange ramp supplies
// secondary/tertiary distinction in both modes.
//
// Non-purpose: no chart layout — components/dashboards renders.
import { PALETTE } from "@marula-core/ui";
import type { Theme } from "@marula-core/ui";

// PALETTE indices, mirrored from packages/ui/src/tokens.ts (tokens.css §3):
// 0 ink(#333333) 1 graphite(#4B5563) 2 slate(#6B7280) 3 fog(#DCDFE2)
// 4 paper(#FFFFFF) 5 canvas-light(#E8EBED) 6 tangerine(#E05D38)
// 7 destructive(#EF4444) 8-13 chart ramp light+dark-2 14 neutral-bg(#E5E7EB)
// 15 canvas-dark(#1C2433) 16 surface-dark(#2A3040) 17 overlay-dark(#262B38)
// 18 neutral-bg-dark(#2A303E) 19 border-dark(#3D4354) 20 mist(#A3A3A3)
// 21 text-dark(#E5E5E5) 22 success fg light 24 success fg dark
// 30 danger fg light 32 danger fg dark
const INK = PALETTE[0] as string;
const GRAPHITE = PALETTE[1] as string;
const SLATE = PALETTE[2] as string;
const FOG = PALETTE[3] as string;
const PAPER = PALETTE[4] as string;
const TANGERINE = PALETTE[6] as string;
const CHART_1 = PALETTE[8] as string;
const CHART_2_LIGHT = PALETTE[9] as string;
const CHART_2_DARK = PALETTE[13] as string;
const SURFACE_DARK = PALETTE[16] as string;
const BORDER_DARK = PALETTE[19] as string;
const MIST = PALETTE[20] as string;
const TEXT_DARK = PALETTE[21] as string;
const SUCCESS_FG_LIGHT = PALETTE[22] as string;
const SUCCESS_FG_DARK = PALETTE[24] as string;
const DANGER_FG_LIGHT = PALETTE[30] as string;
const DANGER_FG_DARK = PALETTE[32] as string;

export interface ChartTokens {
  /** Primary series — Tangerine stays the interactive/primary accent. */
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
  primary: TANGERINE,
  secondary: CHART_1,
  grid: FOG,
  tick: SLATE,
  tooltipBg: PAPER,
  tooltipText: INK,
  tooltipBorder: MIST,
  success: SUCCESS_FG_LIGHT,
  danger: DANGER_FG_LIGHT,
};

const DARK: ChartTokens = {
  primary: TANGERINE,
  secondary: CHART_2_DARK,
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
