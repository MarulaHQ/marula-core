/**
 * Density switching via the `data-density` attribute on <html> — Wave 41
 * lane 2b, decision D7.
 *
 * Purpose: apply, read and reactively track the account's density step, with
 * localStorage persistence for the pre-paint bootstrap. One attribute on the
 * document root moves every control that reads `--m-density-control`,
 * `--m-density-row` or `--m-card-padding`, so the step is a per-user
 * preference rather than a per-surface switch.
 *
 * Non-purpose: this declares no measurements. The step triples live in
 * tokens.css and are asserted by packages/ui/tests/tokens.test.ts, so a CSS
 * value and the label shown beside it cannot drift apart silently. The
 * ladder was set by design.md §9.2 and the Marula plan: `comfortable` is the
 * default, and `dense` reproduces the pre-Marula values so the data-heavy
 * fleet, stock and billing tables do not regress.
 */
import { useSyncExternalStore } from "react";

/** The three steps of D7. */
export type Density = "comfortable" | "compact" | "dense";

/** Rendered in the order a chooser shows them: roomiest first. */
export const DENSITY_STEPS: readonly Density[] = ["comfortable", "compact", "dense"] as const;

/** Marula's own step, and what an account with no saved choice lands on. */
export const DEFAULT_DENSITY: Density = "comfortable";

export const DENSITY_STORAGE_KEY = "marula-density";
const ATTRIBUTE = "data-density";

/**
 * The measurements each step produces, for a control's own description and
 * for a settings page that says which step does what. These mirror
 * tokens.css; the test asserts the two agree.
 */
export const DENSITY_STEP_SPEC: Record<Density, { control: number; row: number; card: number }> = {
  comfortable: { control: 44, row: 48, card: 24 },
  compact: { control: 36, row: 40, card: 16 },
  dense: { control: 32, row: 32, card: 12 },
};

export function isDensity(value: unknown): value is Density {
  return value === "comfortable" || value === "compact" || value === "dense";
}

/** Apply a step to the document root and persist it. Safe without DOM (SSR). */
export function applyDensity(density: Density): void {
  if (typeof document !== "undefined") {
    document.documentElement.setAttribute(ATTRIBUTE, density);
  }
  try {
    localStorage.setItem(DENSITY_STORAGE_KEY, density);
  } catch {
    // Storage unavailable (private mode); the attribute still applies for
    // this session, which is the same degradation the theme switch accepts.
  }
}

/** Resolve the initial step: a stored preference, else the Marula default. */
export function resolveInitialDensity(): Density {
  try {
    const stored = localStorage.getItem(DENSITY_STORAGE_KEY);
    if (isDensity(stored)) return stored;
  } catch {
    // Fall through to the default.
  }
  return DEFAULT_DENSITY;
}

function subscribe(callback: () => void): () => void {
  const observer = new MutationObserver(callback);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: [ATTRIBUTE] });
  return () => observer.disconnect();
}

function currentDensity(): Density {
  const attr = document.documentElement.getAttribute(ATTRIBUTE);
  return isDensity(attr) ? attr : DEFAULT_DENSITY;
}

function serverDensity(): Density {
  return DEFAULT_DENSITY;
}

/** Subscribe to the active density step. */
export function useDensity(): Density {
  return useSyncExternalStore(subscribe, currentDensity, serverDensity);
}
