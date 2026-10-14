/**
 * Light/dark theme switching via the `data-theme` attribute on <html>.
 * Purpose: apply, read, and reactively track the active Workbench theme with
 * localStorage persistence. Non-purpose: does not touch component styling —
 * tokens.css reacts to the attribute on its own.
 */
import { useSyncExternalStore } from "react";
import { resolveInitialPreset, setThemePreset } from "./theme-pack-apply";
import { resolveInitialDensity } from "./density";

export type Theme = "light" | "dark";

const STORAGE_KEY = "marula-theme";
const ATTRIBUTE = "data-theme";

/**
 * The DOM carries the KIT's theme names. The kit's default theme is `neutral`
 * and its warm cream theme is `light`, so the app's historical `light` (the
 * neutral canvas) must be written as `neutral` or the cream block would win.
 * The app's own vocabulary stays `light` / `dark`; only the attribute value is
 * translated, in one place.
 */
function toDomTheme(theme: Theme): string {
  return theme === "light" ? "neutral" : "dark";
}

function fromDomTheme(value: string | null): Theme | null {
  if (value === "dark") return "dark";
  if (value === "neutral" || value === "light") return "light";
  return null;
}

function isValidTheme(value: unknown): value is Theme {
  return value === "light" || value === "dark";
}

/** Apply a theme to the document root and persist it. Safe without DOM (SSR). */
export function applyTheme(theme: Theme): void {
  if (typeof document !== "undefined") {
    document.documentElement.setAttribute(ATTRIBUTE, toDomTheme(theme));
  }
  try {
    localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    // Storage unavailable (private mode); attribute still applies for session.
  }
}

/**
 * Resolve the initial theme: stored preference, else system preference.
 */
export function resolveInitialTheme(): Theme {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (isValidTheme(stored)) {
      return stored;
    }
  } catch {
    // Fall through to system preference.
  }
  if (typeof window !== "undefined" && window.matchMedia) {
    return window.matchMedia("(prefers-color-scheme: dark)").matches
      ? "dark"
      : "light";
  }
  return "light";
}

/**
 * Apply the RESOLVED theme (stored preference, else system preference) to the
 * document root and persist nothing.
 *
 * This is the in-TypeScript twin of the pre-paint bootstrap script
 * (`theme-bootstrap.ts`): same keys, same resolved value, same kit attribute
 * literal. It exists because the script runs once, before first paint, and
 * anything that later re-creates the <html> element (React's hydration
 * recovery on an authed route) takes the attribute with it — the DOM is left
 * on the tokens.css baseline until the next write, which is the wrong-mode
 * flash on the staff app's login route. Callers that must never write a
 * preference (public entry screens, the recovery leg) use this instead of
 * `applyTheme`.
 */
export function applyResolvedTheme(): void {
  if (typeof document === "undefined") return;
  document.documentElement.setAttribute(ATTRIBUTE, toDomTheme(resolveInitialTheme()));
}

/**
 * The full pre-paint contract, twin of `themeBootstrapScript`: resolved
 * theme, preset and density attributes on <html>, nothing persisted. The
 * preset and density legs reuse the shipped resolvers so one vocabulary
 * covers the script, the recovery and the visible switchers.
 */
export function applyResolvedAppearance(): void {
  if (typeof document === "undefined") return;
  applyResolvedTheme();
  const preset = resolveInitialPreset();
  if (preset === null || preset === "") {
    document.documentElement.removeAttribute("data-preset");
  } else {
    document.documentElement.setAttribute("data-preset", preset);
  }
  document.documentElement.setAttribute("data-density", resolveInitialDensity());
}

function subscribe(callback: () => void): () => void {
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  const observer = new MutationObserver(callback);
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: [ATTRIBUTE],
  });
  media.addEventListener("change", callback);
  return () => {
    observer.disconnect();
    media.removeEventListener("change", callback);
  };
}

function currentTheme(): Theme {
  return fromDomTheme(document.documentElement.getAttribute(ATTRIBUTE)) ?? "light";
}

/**
 * Reactive current theme. Reads the `data-theme` attribute so external code
 * that calls `applyTheme` (or sets the attribute) stays in sync.
 */
export function useTheme(): { theme: Theme; setTheme: (theme: Theme) => void } {
  const theme = useSyncExternalStore(
    subscribe,
    currentTheme,
    (): Theme => "light",
  );
  return { theme, setTheme: applyTheme };
}

// --- Wave 24: theme presets (data-preset attribute) ---------------------------

/** Reuses the same MutationsObserver machinery as useTheme for preset swaps:
 * the preset is purely a current-attribute read, so subscribers get fresh
 * values whenever data-preset changes. */
function subscribePreset(callback: () => void): () => void {
  const observer = new MutationObserver(callback);
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["data-preset"],
  });
  return () => observer.disconnect();
}

function currentPreset(): string | null {
  return document.documentElement.getAttribute("data-preset");
}

/** Reactive active theme preset id (null = the tokens.css baseline). */
export function useThemePreset(): {
  preset: string | null;
  setPreset: (id: string | null) => void;
} {
  const preset = useSyncExternalStore(
    subscribePreset,
    currentPreset,
    (): string | null => null,
  );
  return { preset, setPreset: setPresetAttribute as (id: string | null) => void };
}

function setPresetAttribute(id: string | null): void {
  // Delegate to the pack-apply setter (attribute + persistence).
  setThemePreset(id);
}