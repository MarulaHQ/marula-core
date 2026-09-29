/**
 * Light/dark theme switching via the `data-theme` attribute on <html>.
 * Purpose: apply, read, and reactively track the active Workbench theme with
 * localStorage persistence. Non-purpose: does not touch component styling —
 * tokens.css reacts to the attribute on its own.
 */
import { useSyncExternalStore } from "react";
import { setThemePreset } from "./theme-pack-apply";

export type Theme = "light" | "dark";

const STORAGE_KEY = "marula-theme";
const ATTRIBUTE = "data-theme";

function isValidTheme(value: unknown): value is Theme {
  return value === "light" || value === "dark";
}

/** Apply a theme to the document root and persist it. Safe without DOM (SSR). */
export function applyTheme(theme: Theme): void {
  if (typeof document !== "undefined") {
    document.documentElement.setAttribute(ATTRIBUTE, theme);
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
  const attr = document.documentElement.getAttribute(ATTRIBUTE);
  return isValidTheme(attr) ? attr : "light";
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

/** Reactive active theme preset id (null = Tangerine baseline). */
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