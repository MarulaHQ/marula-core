// Wave 29 (`p29-keyboard-shortcuts`) — the shortcut standard's types.
//
// The registry binds every user-facing command to ONE chord. Similar or
// primary functions share the same command (and therefore the same chord)
// ACROSS surfaces: the verbs (new / navigate / pane-cycle / copy-link /
// close / share / palette) are surface-agnostic. A per-surface reshaping of
// a shared command does not exist by construction — the consistency lint
// (lib/commands/consistency.ts) enforces the remaining invariants at
// authoring time, in tests, never at runtime.

/** The surface-agnostic verb vocabulary. Each verb is one command shape. */
export type CommandVerb =
  | "new"
  | "navigate"
  | "pane-cycle"
  | "copy-link"
  | "close"
  | "share"
  | "palette"
  | "popout";

/** Parsed, canonical chord — lowercased key + explicit modifiers. */
export interface Chord {
  /** `alt` is the registry's activation key; `mod` = Cmd on darwin / Ctrl elsewhere. */
  modifiers: ReadonlyArray<"alt" | "mod" | "shift">;
  /** Lowercased key representation: letters, digits 1–9, "?". */
  key: string;
  /** What the overlay + cheat sheet render, e.g. "Alt+N". */
  display: string;
}

/** What a command receives when executed. */
export interface CommandContext {
  /** pathname of the surface the chord was pressed on. */
  pathname: string;
  /** Click the SAME control the user would have clicked (click parity). */
  click: (selector: string) => boolean;
  /** Navigate exactly like a clicked link would. */
  navigate: (href: string) => void;
  /** Copy text (best-effort; never throws). */
  copy: (text: string) => boolean;
}

/** One registered command. Immutable at runtime — registered once here. */
export interface CommandDef {
  /** Namespaced id: `<verb>.<subject>` — unique by lint. */
  id: string;
  verb: CommandVerb;
  /** Short label for the hold-Alt overlay. */
  label: string;
  /** One-sentence description for the cheat sheet. */
  description: string;
  /** Canonical chord string: modifiers joined by "+" around the key, e.g. "alt+n". */
  chord: string;
  /**
   * Route prefixes the command is valid on. `[]` = valid everywhere (global).
   * Segments may contain `[id]` meaning "exactly one non-empty segment"
   * (any value), so "/tickets/[id]" matches "/tickets/a2f".
   */
  surfaces: ReadonlyArray<string>;
  run: (ctx: CommandContext) => void;
}

/** Per-user shortcut profile (typed setting `shortcuts.profile`). */
export type ShortcutProfile = "default" | "all_off";
