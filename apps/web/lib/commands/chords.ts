// Chord parsing + event matching + route matching for the command registry.
//
// Pure functions only — no React, no router — so the consistency lint and
// the runtime dispatcher share one exact definition of "matches".

import type { Chord } from "./types";

/**
 * Parse a canonical chord string ("alt+n", "mod+k", "alt+shift+?").
 * Throws on an unknown modifier or an empty key — authoring mistakes must
 * blow up the registry tests, never the runtime.
 */
export function parseChord(spec: string): Chord {
  const parts = spec.split("+").map((p) => p.trim().toLowerCase());
  if (parts.length < 2 || parts.some((p) => p === "")) {
    throw new Error(`chord "${spec}" is malformed`);
  }
  if (parts.some((p) => p === "")) throw new Error(`chord "${spec}" is malformed`);
  const key = parts[parts.length - 1] as string;
  const modifierSpecs = parts.slice(0, -1);
  const known = new Set<string>(["alt", "mod", "shift"]);
  for (const m of modifierSpecs) {
    if (!known.has(m)) {
      throw new Error(`chord "${spec}" has unknown modifier "${m}"`);
    }
  }
  const modifiers = modifierSpecs as Chord["modifiers"];
  return { modifiers, key, display: displayChord(spec) };
}

/** Render a canonical chord for humans: "alt+n" → "Alt+N". */
export function displayChord(spec: string): string {
  return spec
    .split("+")
    .map((part) => (part === "?" ? "?" : part.charAt(0).toUpperCase() + part.slice(1)))
    .join("+");
}

/**
 * True when this platform's `mod` modifier is Meta (⌘) rather than Control.
 *
 * The dispatcher and the two chord renderers must agree, so the platform
 * test lives here once. Callers in a browser can omit the argument.
 */
export function modIsMeta(platform?: string): boolean {
  if (platform !== undefined) return /mac/i.test(platform);
  if (typeof navigator === "undefined") return false;
  return /mac/i.test(navigator.platform ?? "");
}

/**
 * The base key a chord is authored against, read from the PHYSICAL key.
 *
 * `event.code` names the key, not the character it produced, so a chord fires
 * on the same key under every layout — and, the reason this exists, under
 * whatever character the OS composes for a held modifier. On macOS Option+2
 * arrives as `event.key === "™"` and Option+N as a dead key, so a parser that
 * reads `event.key` can never match `alt+2` or `alt+n`. `event.code` reads
 * `Digit2` and `KeyN` regardless.
 *
 * Letters and digits come from the code; everything else keeps `event.key`,
 * because the punctuation codes are layout names ("Slash") while the chord
 * vocabulary is characters ("?").
 */
function baseKeyFromEvent(event: KeyboardEvent): string | null {
  const code = event.code ?? "";
  if (code.length === 4 && code.startsWith("Key")) return code.slice(3).toLowerCase();
  if (code.length === 6 && code.startsWith("Digit")) return code.slice(5);
  const raw = event.key;
  // "?" only arrives via shift+/ on US layouts, and dead keys arrive as
  // "Dead" (length 4) — both stay the character they always were.
  if (raw.length !== 1) return null;
  return raw.toLowerCase();
}

/** KeyboardEvent → canonical chord spec, with platform-aware mod. */
export function chordFromEvent(event: KeyboardEvent, modIsMeta: boolean): string | null {
  const parts: string[] = [];
  if (event.altKey) parts.push("alt");
  const hasMod = modIsMeta ? event.metaKey : event.ctrlKey;
  if (hasMod) parts.push("mod");
  if (event.shiftKey) parts.push("shift");
  const key = baseKeyFromEvent(event);
  if (key === null) return null;
  parts.push(key);
  return parts.join("+");
}

/** True when the event's modifier state exactly matches the chord's. */
export function eventMatchesChord(chordSpec: string, event: KeyboardEvent, modIsMeta: boolean): boolean {
  return chordFromEvent(event, modIsMeta) === chordSpec;
}

/** Format a surface segment match: "[id]" = exactly one non-empty segment. */
function segmentMatches(pattern: string, actual: string): boolean {
  if (pattern === "[id]") return actual.length > 0;
  return pattern === actual;
}

/**
 * Does a command's surface list cover this pathname?
 * Empty list = global command (valid everywhere).
 */
export function matchesSurface(surfaces: ReadonlyArray<string>, pathname: string): boolean {
  if (surfaces.length === 0) return true;
  for (const surface of surfaces) {
    if (surfaceMatchesOne(surface, pathname)) return true;
  }
  return false;
}

/** "/tickets/[id]" matches "/tickets/a2f" but not "/tickets" or "/tickets/a/b". */
export function surfaceMatchesOne(surface: string, pathname: string): boolean {
  const patternParts = surface.split("/").filter((p) => p !== "");
  const pathParts = (pathname.split("?")[0] ?? pathname).split("/").filter((p) => p !== "");
  if (patternParts.length !== pathParts.length) return false;
  return patternParts.every((p, i) => segmentMatches(p, pathParts[i] ?? "\u0000"));
}
