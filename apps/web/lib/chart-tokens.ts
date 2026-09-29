// Chart token bridge for Recharts (ADR 0001: Recharts on Marula tokens).
//
// Purpose: the ONLY place chart colors come from. Every value traces to the
// @marula-core/ui TOKEN record (the same constants tokens.css is generated
// from), so light and dark modes carry parity and no hex is authored outside
// the token set. Recharts needs concrete color strings (SVG presentation
// attributes do not resolve CSS custom properties), which is why the two mode
// sets are materialized here instead of read from CSS variables.
//
// Chart series follow the brand ramp (docs/theme.md §6): gold, canopy, blush,
// bark, lime. Gold leads in BOTH modes on the light side because gold-on-light
// is a fill, and on the dark side because gold on ink is 7.7:1 — one leading
// hue, two grounds, so a chart reads the same in either mode.
//
// Non-purpose: no chart layout — components/dashboards renders.
import { TOKEN } from "@marula-core/ui";
import type { Theme } from "@marula-core/ui";

export interface ChartTokens {
  /** Leading series: the ring gold, which is the accent family. */
  readonly primary: string;
  /** Secondary series (canopy green) — the contrast series. */
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
  primary: TOKEN.ring,
  secondary: TOKEN.canopy,
  grid: TOKEN.border,
  tick: TOKEN.bark,
  tooltipBg: TOKEN.surface,
  tooltipText: TOKEN.ink,
  tooltipBorder: TOKEN.border,
  success: TOKEN.successFg,
  danger: TOKEN.dangerFg,
};

const DARK: ChartTokens = {
  primary: TOKEN.gold,
  secondary: TOKEN.lime,
  grid: TOKEN.borderDark,
  tick: TOKEN.textSecondaryDark,
  tooltipBg: TOKEN.surfaceDark,
  tooltipText: TOKEN.cream,
  tooltipBorder: TOKEN.borderDark,
  success: TOKEN.successFgDark,
  danger: TOKEN.dangerFgDark,
};

/** Token set for the active theme. Picking happens through the standard
 *  `useTheme()` hook so the existing data-theme switch drives parity. */
export function chartTokensFor(theme: Theme): ChartTokens {
  return theme === "dark" ? DARK : LIGHT;
}
