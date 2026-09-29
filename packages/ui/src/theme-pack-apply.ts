"use client";
/**
 * Theme-pack application (Wave 24 leg B).
 *
 * Purpose: turn a VALIDATED custom pack manifest into live CSS. Packs are
 * restyle-only by construction (the ingestion seam coerces + schema-validates
 * them server-side), so apply is one injected <style> element per pack id
 * scoped by the `data-preset` attribute on the document root. Activation is
 * exclusive: exactly one preset attribute value at a time.
 * Non-purpose: validation (backend), settings persistence, distribution.
 */
import { presetCss } from "./theme-tokens";

export const PACK_STYLE_PREFIX = "m-theme-pack-";
export const PRESET_STORAGE_KEY = "marula-theme-preset";

export interface PackTokens {
  slug: string;
  light: Record<string, string>;
  dark: Record<string, string>;
}

/** Injects / replaces one pack's style element. Safe without DOM (noop). */
export function applyPackStyles(pack: PackTokens): void {
  if (typeof document === "undefined") return;
  const id = packStyleId(pack.slug);
  let styleTag = document.getElementById(id) as HTMLStyleElement | null;
  if (!styleTag) {
    styleTag = document.createElement("style");
    styleTag.id = id;
    document.head.appendChild(styleTag);
  }
  styleTag.textContent = presetCss(pack.slug, pack.light, pack.dark);
}

export function removePackStyles(slug: string): void {
  if (typeof document === "undefined") return;
  document.getElementById(packStyleId(slug))?.remove();
}

export function packStyleId(slug: string): string {
  return `${PACK_STYLE_PREFIX}${slug}`;
}

/** Activates one theme id (builtin preset or installed pack slug) by
 * setting the `data-preset` attribute; persists the device preference.
 * `null` clears back to the tokens.css Tangerine baseline. */
export function setThemePreset(id: string | null): void {
  if (typeof document === "undefined") return;
  if (id) {
    document.documentElement.setAttribute("data-preset", id);
    try {
      localStorage.setItem(PRESET_STORAGE_KEY, id);
    } catch {
      // Storage unavailable; the attribute still applies for the session.
    }
  } else {
    document.documentElement.removeAttribute("data-preset");
    try {
      localStorage.setItem(PRESET_STORAGE_KEY, "");
    } catch {
      // Storage unavailable.
    }
  }
}

/** The persisted device preset (empty string = Tangerine baseline). */
export function resolveInitialPreset(): string | null {
  if (typeof window === "undefined") return null;
  try {
    const stored = localStorage.getItem(PRESET_STORAGE_KEY);
    return stored ? stored : null;
  } catch {
    return null;
  }
}
