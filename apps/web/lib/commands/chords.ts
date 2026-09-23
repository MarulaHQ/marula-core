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

/** KeyboardEvent → canonical chord spec, with platform-aware mod. */
export function chordFromEvent(event: KeyboardEvent, modIsMeta: boolean): string | null {
  const parts: string[] = [];
  if (event.altKey) parts.push("alt");
  const hasMod = modIsMeta ? event.metaKey : event.ctrlKey;
  if (hasMod) parts.push("mod");
  if (event.shiftKey) parts.push("shift");
  // Digits arrive as "Digit1" on some layouts and "1" on others; "?" only
  // arrives via shift+/ on US layouts. Letters and digits lowercase to
  // their base key.
  const raw = event.key;
  if (raw.length !== 1) return null;
  parts.push(raw.toLowerCase());
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
