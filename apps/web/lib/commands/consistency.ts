// Authoring-time consistency lint for the command registry.
//
// The contract (Wave 29): conflicts are settled HERE, at registry-authoring
// time, inside the lane's tests — never migrated to runtime. Every
// violation below is a test failure waiting; the runtime never reconciles
// chords, scopes, or duplicate registrations.

import { parseChord } from "./chords";
import type { CommandDef } from "./types";

export const SHORTCUT_LINT_CODE = {
  DUPLICATE_ID: "duplicate-id",
  DUPLICATE_CHORD: "duplicate-chord",
  RESERVED_CHORD: "reserved-chord",
  MALFORMED_CHORD: "malformed-chord",
  VERB_SHAPE: "verb-shape",
} as const;
export type ShortcutLintCode = (typeof SHORTCUT_LINT_CODE)[keyof typeof SHORTCUT_LINT_CODE];

export interface ShortcutLintViolation {
  code: ShortcutLintCode;
  message: string;
}

/**
 * Chords that belong to the OS or the browser and therefore MUST NOT be
 * registered: browser history (Alt+arrows), tab/window lifecycle
 * (Ctrl/Cmd+T/W/L/N/Q), OS close/menus (Alt+F4, Cmd+Q, Alt+Tab), address
 * bar and search (Ctrl/Cmd+L/K in some browsers — the app's pre-existing
 * Cmd/Ctrl+K search palette is the ONE documented exception, registered
 * with allowBrowserHonesty in the registry and whitelisted here).
 */
const RESERVED_CHORDS: ReadonlySet<string> = new Set([
  // OS-reserved
  "alt+tab",
  "alt+f4",
  "mod+q",
  "mod+m",
  "mod+h",
  "mod+space",
  "ctrl+alt+arrowup",
  "alt+arrowup",
  "alt+arrowdown",
  "alt+arrowleft",
  "alt+arrowright",
  // Browser-reserved (Chrome/Firefox/Safari): tab + window lifecycle, history.
  "mod+t",
  "mod+w",
  "mod+l",
  "mod+n",
  "mod+shift+n",
  "mod+shift+t",
  "ctrl+t",
  "ctrl+w",
  "ctrl+l",
  "ctrl+n",
  "ctrl+shift+n",
  "ctrl+shift+t",
  "alt+home",
]);
export const RESERVED_CHORD_LIST: ReadonlyArray<string> = [...RESERVED_CHORDS];

/** The pre-existing search palette is the one documented exception: it
 * predates the registry (Wave 15), is honest about it in its label, and no
 * OS/browser binding claims Cmd/Ctrl+K on this stack. */
const WHITELISTED_CHORDS: ReadonlySet<string> = new Set(["mod+k"]);

/** True when two surface lists can both be valid on one surface.
 * Global (empty list) overlaps everything; surface strings overlap when
 * equal, or when one's [id]-prefix covers the other. */
function surfacesOverlap(a: ReadonlyArray<string>, b: ReadonlyArray<string>): boolean {
  if (a.length === 0 || b.length === 0) return true;
  for (const s of a) {
    for (const t of b) {
      if (t === s) return true;
      const prefixT = t.replace(/\[id\]$/, "");
      const prefixS = s.replace(/\[id\]$/, "");
      if (prefixT !== "/" && s.startsWith(prefixT)) return true;
      if (prefixS !== "/" && t.startsWith(prefixS)) return true;
    }
  }
  return false;
}

/**
 * Lint the registry. Returns every violation; an empty array means the
 * registry is consistent.
 */
export function lintRegistry(commands: ReadonlyArray<CommandDef>): ShortcutLintViolation[] {
  const violations: ShortcutLintViolation[] = [];
  const seenIds = new Map<string, string>();
  const seenChords = new Map<string, string>();

  for (const command of commands) {
    // (1) unique id, namespaced by verb
    if (seenIds.has(command.id)) {
      violations.push({
        code: SHORTCUT_LINT_CODE.DUPLICATE_ID,
        message: `command id "${command.id}" is already registered`,
      });
    }
    seenIds.set(command.id, command.chord);

    // (2) chord is well-formed and parseable
    try {
      parseChord(command.chord);
    } catch (e) {
      violations.push({
        code: SHORTCUT_LINT_CODE.MALFORMED_CHORD,
        message: `${command.id}: ${(e as Error).message}`,
      });
      continue;
    }

    // (3) no OS/browser-reserved combo
    if (RESERVED_CHORDS.has(command.chord) && !WHITELISTED_CHORDS.has(command.chord)) {
      violations.push({
        code: SHORTCUT_LINT_CODE.RESERVED_CHORD,
        message: `${command.id}: chord "${command.chord}" is reserved by the OS/browser — Alt is the activation key; pick a free Alt-family chord`,
      });
    }

    // (4) the standard's core rule: SIMILAR/PRIMARY FUNCTIONS SHARE THE
    // SAME COMMAND ACROSS SURFACES — two commands with disjoint surface
    // sets MAY share a chord (that IS Alt+N = new record everywhere). A
    // conflict is a chord that would be ambiguous on ONE surface: same id
    // again with a different chord (dupe-id below), or a chord whose
    // commands' surface sets overlap.
    const prior = seenChords.get(command.chord);
    if (prior !== undefined) {
      const earlier = commands.find((c) => c.id === prior);
      if (earlier && surfacesOverlap(earlier.surfaces, command.surfaces)) {
        violations.push({
          code: SHORTCUT_LINT_CODE.DUPLICATE_CHORD,
          message: `${command.id}: chord "${command.chord}" is ambiguous on an overlapping surface with "${prior}"`,
        });
      }
    }
    seenChords.set(command.chord, command.id);

    // (5) verb-shape honesty: id prefix must match the verb (labels ride
    // the verb vocabulary; a renamed id that flips the verb is a reshaping).
    if (!command.id.startsWith(`${command.verb}.`)) {
      violations.push({
        code: SHORTCUT_LINT_CODE.VERB_SHAPE,
        message: `${command.id}: id must start with the verb "${command.verb}."`,
      });
    }
  }

  return violations;
}
