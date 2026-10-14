/**
 * The pre-paint theme bootstrap as an inline script body — one source of
 * truth for every layout that carries it.
 *
 * Purpose: the app paints from localStorage's resolved preference (stored
 * choice, else the system's light/dark) BEFORE the first paint, writing the
 * KIT's own attribute values (`neutral` for the app's light — the kit's
 * default theme — and `dark`), so a visitor whose device is dark never sees
 * the light canvas flash first. The preset and density legs mirror
 * `resolveInitialPreset` / `resolveInitialDensity` and persist nothing.
 *
 * This module is deliberately import-free and carries no `"use client"` mark:
 * it is evaluated inside `dangerouslySetInnerHTML` on the server, so a server
 * layout can call it without dragging a client boundary into the root. The
 * `applyResolvedAppearance` twin in `theme.ts` re-applies the same contract if
 * anything later re-creates `<html>` (React's hydration recovery on authed
 * routes); `packages/ui/tests/theme-bootstrap.test.ts` pins the two together.
 *
 * Non-purpose: no theme switching — the switchers (`applyTheme`,
 * `setThemePreset`, `applyDensity`) own persistence and the account-saved
 * values; this only decides the FIRST paint.
 */
const THEME_STORAGE_KEY = "marula-theme";
const PRESET_STORAGE_KEY = "marula-theme-preset";
const DENSITY_STORAGE_KEY = "marula-density";

/** The density steps the bootstrap validates, in the shipped order. */
const DENSITY_STEPS = ["compact", "dense", "comfortable"] as const;
const DEFAULT_DENSITY = "comfortable";

export function themeBootstrapScript(): string {
  return `(function(){try{var t=localStorage.getItem(${JSON.stringify(
    THEME_STORAGE_KEY,
  )});if(t!=="light"&&t!=="dark"){t=window.matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light";}document.documentElement.dataset.theme=(t==="light"?"neutral":"dark");var p=localStorage.getItem(${JSON.stringify(
    PRESET_STORAGE_KEY,
  )});if(p){document.documentElement.dataset.preset=p;}var d=localStorage.getItem(${JSON.stringify(
    DENSITY_STORAGE_KEY,
  )});if(d===${DENSITY_STEPS.map((step) => JSON.stringify(step)).join(
    "||d===",
  )}){document.documentElement.dataset.density=d;}else{document.documentElement.dataset.density=${JSON.stringify(
    DEFAULT_DENSITY,
  )};}}catch(e){}})();`;
}

/** Exported for the parity test that pins these literals to the modules that
 * own the same keys behind the switchers. */
export const BOOTSTRAP_KEYS = {
  theme: THEME_STORAGE_KEY,
  preset: PRESET_STORAGE_KEY,
  density: DENSITY_STORAGE_KEY,
  densitySteps: DENSITY_STEPS,
  defaultDensity: DEFAULT_DENSITY,
} as const;
