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

/**
 * What a click produced. Deliberately NOT a boolean: a caller that ignores
 * `false` is how `new.clients` stayed advertised and dead for months after a
 * Wave 41 pack port renamed its control (Wave 47 lane A25). A typed result
 * carries the selector that missed, so the failure can name it.
 */
export type ClickResult = { kind: "clicked" } | { kind: "absent"; selector: string };

/** What a command receives when executed. */
export interface CommandContext {
  /** pathname of the surface the chord was pressed on. */
  pathname: string;
  /** Click the SAME control the user would have clicked (click parity). */
  click: (selector: string) => ClickResult;
  /** Navigate exactly like a clicked link would. */
  navigate: (href: string) => void;
  /** Copy text (best-effort; never throws). */
  copy: (text: string) => boolean;
}

/**
 * Every command reports what happened. There is no `void` and no boolean: the
 * type is what stops a command from ending in silence.
 *
 * - `ran` — it did the thing.
 * - `inapplicable` — the author DECLARED absence possible (nothing to close,
 *   no native window on plain web, popup blocked). Expected, not a defect.
 * - `failed` — the author promised a target on a declared surface and it was
 *   not there. A renamed or deleted control. Always reported as an error.
 */
export type CommandOutcome =
  | { status: "ran" }
  | { status: "inapplicable"; reason: string; selector?: string }
  | { status: "failed"; reason: string; selector?: string };

/** Whether a command's control must exist on the surfaces it declares. */
export type TargetPresence = "required" | "optional";

/**
 * A control a command clicks, declared on the command row so it can be
 * resolved against the source tree without a DOM (scripts/check-command-targets.mjs).
 * `selector` is what runs; `testId` / `attribute` are what the static check
 * looks for in markup.
 */
export interface CommandTarget {
  /** CSS selector handed to `ctx.click`. */
  selector: string;
  /** `data-testid` name when the selector is testid-shaped. */
  testId: string | null;
  /** Attribute name/value when the selector is attribute-shaped. */
  attribute: { name: string; value: string } | null;
  /** `required`: absence is a defect. `optional`: absence is declared. */
  presence: TargetPresence;
  /** Where the static check resolves the control: the command's route
   * directories, or anywhere in the source for a shared component. */
  via: "surface" | "shared";
  /** What the control is, in words, for the failure message. */
  description: string;
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
  /**
   * Controls this command clicks. Empty for commands that navigate, copy or
   * open windows. Declared rather than inlined into `run` so the static check
   * (scripts/check-command-targets.mjs) can resolve every one of them against
   * the source tree, and so "may this be absent?" is a written decision.
   */
  targets: ReadonlyArray<CommandTarget>;
  /** Reports what happened. Cannot return void — see `CommandOutcome`. */
  run: (ctx: CommandContext) => CommandOutcome;
}

/** Per-user shortcut profile (typed setting `shortcuts.profile`). */
export type ShortcutProfile = "default" | "all_off";
