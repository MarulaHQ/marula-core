// Pack rules — the pure geometry/severity contract, RN-free so the node
// test suite can hold the line. pack.tsx re-exports these beside the
// components; screens import from either.

import { darkTokens, lightTokens } from "./tokens";

/** Pack geometry, straight from the mockups' CSS. Not theme-restylable. */
export const pack = {
  radiusControl: 12,
  radiusPrimary: 14,
  radiusSheet: 24,
  radiusCard: 16,
  primaryHeight: 52,
  secondaryHeight: 44,
  controlHeight: 48,
  rowHeight: 44,
  tabBarHeight: 90,
  timerPillHeight: 56,
  scanButtonSize: 60,
  chipHeight: 36,
  factsRowHeight: 44,
  minWidthText: 12,
} as const;

/** Areas a screen may reserve at its bottom for the pinned chrome. */
export const bottomReserve = {
  /** Tab bar (90) — plus room so the timer pill (56) never half-hides a row. */
  tabs: pack.tabBarHeight + pack.timerPillHeight + 20,
  /** Action bar only (no timer pill — a ticket replaces both with the bar). */
  actions: 98,
} as const;

export type SlaVerdict = "breach" | "due";

/** The severity contract's colour side: red is breach-only. */
export function slaVerdictColors(verdict: SlaVerdict): {
  light: { text: string; background: string };
  dark: { text: string; background: string };
} {
  return verdict === "breach"
    ? { light: lightTokens.danger, dark: darkTokens.danger }
    : { light: lightTokens.warning, dark: darkTokens.warning };
}

/** Stop-and-log's rounding rule (pack: "Rounded up to the next 15 minutes"). */
export function roundUpTo(minutes: number, step = 15): number {
  if (minutes <= 0) return step;
  return Math.ceil(minutes / step) * step;
}

/** `0:30` style primary-button label (pack: "Stop and log 0:30"). */
export function shortClock(minutes: number): string {
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return `${hours}:${String(rest).padStart(2, "0")}`;
}

/** Tabular 00:25:05 from elapsed milliseconds. */
export function clockText(totalMs: number): string {
  const seconds = Math.floor(totalMs / 1000);
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const secs = seconds % 60;
  return [hours, minutes, secs].map((part) => String(part).padStart(2, "0")).join(":");
}
