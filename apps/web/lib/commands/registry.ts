// The command registry — the ONE source of truth for every keyboard command.
//
// Two views render from this and nothing else: the hold-Alt overlay
// (commands valid on the CURRENT surface) and the ?-cheat-sheet dialog (the
// FULL registry, grouped). No second copy of the chord data exists anywhere.
//
// Chord conventions (recorded for the landing record):
// - Alt is the activation key per user directive: Alt+letter/digit chords
//   carry commands. The pre-existing search palette keeps Cmd/Ctrl+K
//   (Wave 15; the one documented exception — it predates the registry and
//   is whitelisted in lib/commands/consistency.ts).
// - OS/browser-reserved combos (Ctrl+T/W/L/N, Cmd+Q, Alt+F4/Tab, Alt+arrows…)
//   are forbidden and rejected by the lint at authoring time.
// - Alt+1..9 mirror the rail's own section order (rail-model.ts), so there
//   is one navigation tree whether you click or press.

import { RAIL_SECTIONS } from "@/components/shell/rail-model";
import { openPopout } from "@/lib/popout/popout";
import { paramsFor, popoutRouteFor } from "@/lib/popout/registry";
import { openNativeWindow } from "@/lib/desktop/windows";
import type { CommandContext, CommandDef, CommandVerb } from "./types";

/** Click-parity target for surfaces that expose their own create control. */
const NEW_RECORD_CONTROL = '[data-testid="new-record"]';

/** Shell overlays mark their close control with this attribute; pressing a
 * "close" command clicks the SAME control a user would click. */
export const CLOSE_CONTROL = '[data-shortcuts-close="1"]';

/** Global commands (empty surfaces = every surface). */
const GLOBAL_COMMANDS: CommandDef[] = [
  {
    id: "palette.search",
    verb: "palette",
    label: "Search & commands",
    description: "Open the command palette — unified search across tickets, clients, devices, and more.",
    chord: "mod+k",
    surfaces: [],
    run: (ctx) => ctx.navigate("/search"),
  },
  {
    id: "close.topmost",
    verb: "close",
    label: "Close overlay",
    description: "Close the topmost Marula overlay or panel, same as its own close control.",
    chord: "alt+x",
    surfaces: [],
    run: (ctx) => ctx.click(CLOSE_CONTROL),
  },
  {
    id: "pane-cycle.section",
    verb: "pane-cycle",
    label: "Next page in section",
    description: "Cycle to the next page within the current rail section (wraps around).",
    chord: "alt+j",
    surfaces: [],
    run: (ctx) => {
      const section =
        RAIL_SECTIONS.find(
          (s) => ctx.pathname === s.href || s.match.some((m) => ctx.pathname.startsWith(m)),
        ) ?? null;
      if (!section) return;
      const hrefs = section.links.map((l) => l.href);
      const active = hrefs.findIndex((h) => ctx.pathname === h || ctx.pathname.startsWith(`${h}/`));
      const next = active === -1 ? hrefs[0] : hrefs[(active + 1) % hrefs.length];
      if (next) ctx.navigate(next);
    },
  },
];

/** Rail navigation: Alt+1..9 mirror the rail sections in order. Built once
 * so the registry, the groups, and the dispatcher share the same objects. */
const RAIL_COMMANDS: ReadonlyArray<CommandDef> = RAIL_SECTIONS.slice(0, 9).map((section, index) => ({
  id: `navigate.${section.id}`,
  verb: "navigate" as CommandVerb,
  label: `Go to ${section.label}`,
  description: `Jump to the ${section.label} section, same as the rail's ${section.label} entry.`,
  chord: `alt+${index + 1}`,
  surfaces: [],
  run: (ctx: CommandContext) => ctx.navigate(section.href),
}));

/**
 * Per-surface new-record commands — the same verb, the same chord, one
 * command shape across surfaces. The lint FAILS a per-surface reshaping
 * (same id re-registered with a different chord, or a duplicate chord), so
 * these rows can never drift apart.
 */
const NEW_RECORD_COMMANDS: CommandDef[] = [
  {
    id: "new.clients",
    verb: "new",
    label: "New client",
    description: "Open the new-client dialog, exactly as the New client button does.",
    chord: "alt+n",
    // Non-destructive: the list page owns the create dialog.
    surfaces: ["/clients"],
    run: (ctx) => ctx.click(NEW_RECORD_CONTROL),
  },
  {
    id: "new.ticket",
    verb: "new",
    label: "New ticket",
    description: "Open this board's new-ticket dialog, exactly as the New ticket button does.",
    chord: "alt+n",
    surfaces: ["/boards/[id]"],
    run: (ctx) => ctx.click(NEW_RECORD_CONTROL),
  },
];

/** Copy-link on record-detail surfaces: the URL is the link a click would. */
const COPY_LINK_COMMANDS: CommandDef[] = [
  {
    id: "copy-link.record",
    verb: "copy-link",
    label: "Copy link",
    description: "Copy the URL of the record you are on (ticket, client, or device detail).",
    chord: "alt+c",
    surfaces: ["/tickets/[id]", "/clients/[id]", "/assets/[id]"],
    run: (ctx) => ctx.copy(window.location.href),
  },
];

/**
 * Popout commands (Wave 38): open the current surface as a route-keyed
 * secondary window (multi-monitor), exactly as the in-context "Pop out"
 * affordance does. Blocked pop-ups surface the same honest inline hint.
 */
const POPOUT_COMMANDS: CommandDef[] = [
  {
    id: "popout.ticket",
    verb: "popout",
    label: "Pop out ticket",
    description: "Open this ticket in its own window (compact shell, for a second monitor).",
    chord: "alt+p",
    surfaces: ["/tickets/[id]", "/tickets/classic/[id]"],
    run: () => {
      const def = popoutRouteFor(window.location.pathname);
      if (def === null) return;
      openPopout(def.key, paramsFor(def, window.location.pathname));
    },
  },
  {
    id: "popout.surface",
    verb: "popout",
    label: "Pop out surface",
    description: "Open this surface (dispatch board, stock lookup, decisions) in its own window.",
    chord: "alt+shift+p",
    surfaces: ["/dispatch", "/stock", "/approvals"],
    run: () => {
      const def = popoutRouteFor(window.location.pathname);
      if (def === null) return;
      openPopout(def.key, paramsFor(def, window.location.pathname));
    },
  },
];

/**
 * Desktop fleet commands (Wave 38 lane 2): open the quick-action windows
 * the tray also offers. They run everywhere in the desktop shell; on plain
 * web the native opener honestly returns null and the command is a no-op
 * that never lies (the desktop-only labeling keeps the cheat sheet honest —
 * plain-web users see "desktop" in the description).
 */
const FLEET_COMMANDS: CommandDef[] = [
  {
    id: "popout.dispatch-native",
    verb: "popout",
    label: "Open dispatch board window",
    description:
      "Desktop: open the dispatch board as its own app window (remembered position per window).",
    chord: "alt+d",
    surfaces: [],
    run: () => {
      void openNativeWindow("dispatch-board", "dispatch");
    },
  },
  {
    id: "popout.stock-native",
    verb: "popout",
    label: "Open stock lookup window",
    description: "Desktop: open the stock lookup as its own app window (remembered position per window).",
    chord: "alt+s",
    surfaces: [],
    run: () => {
      void openNativeWindow("stock-lookup", "stock");
    },
  },
];

/** The registry, in dispatch order: surface-scoped rows first, then global. */
export const COMMAND_REGISTRY: ReadonlyArray<CommandDef> = [
  ...NEW_RECORD_COMMANDS,
  ...COPY_LINK_COMMANDS,
  ...POPOUT_COMMANDS,
  ...FLEET_COMMANDS,
  ...GLOBAL_COMMANDS,
  ...RAIL_COMMANDS,
];

/** Commands valid on the given pathname — what the hold-Alt overlay lists. */
export function commandsForSurface(pathname: string): ReadonlyArray<CommandDef> {
  return COMMAND_REGISTRY.filter(
    (command) => command.surfaces.length === 0 || command.surfaces.some((s) => surfaceCovers(s, pathname)),
  );
}

/** Full registry grouped for the ?-cheat sheet. */
export function registryGroups(): ReadonlyArray<{ title: string; commands: ReadonlyArray<CommandDef> }> {
  return [
    { title: "Everywhere", commands: GLOBAL_COMMANDS },
    {
      title: `Navigation (Alt+1…${Math.min(RAIL_SECTIONS.length, 9)})`,
      commands: RAIL_COMMANDS,
    },
    { title: "On specific surfaces", commands: [...NEW_RECORD_COMMANDS, ...COPY_LINK_COMMANDS] },
    { title: "Multi-window (pop out)", commands: POPOUT_COMMANDS },
    { title: "Desktop windows", commands: FLEET_COMMANDS },
  ];
}

/** Resolve a chord to its command on a surface; surface rows win over global. */
export function commandForChord(chordSpec: string, pathname: string): CommandDef | null {
  return (
    COMMAND_REGISTRY.find(
      (command) =>
        command.chord === chordSpec &&
        (command.surfaces.length === 0 || command.surfaces.some((s) => surfaceCovers(s, pathname))),
    ) ?? null
  );
}

/** The chords the hold-Alt overlay presents as "meta" rows. */
export const META_ROWS: ReadonlyArray<{ chord: string; label: string; display: string }> = [
  { chord: "alt", label: "Hold to reveal these shortcuts", display: "Hold Alt" },
  { chord: "?", label: "Full cheat sheet", display: "?" },
];

export function metaRows(): ReadonlyArray<{ chord: string; label: string; display: string }> {
  return META_ROWS;
}

/** Segment-pattern coverage: "/tickets/[id]" covers "/tickets/abc"; empty lists match everywhere. */
export function surfaceCovers(surface: string, pathname: string): boolean {
  const prefix = surface.replace(/\[id\]$/, "");
  return pathname === surface || (prefix !== "/" && pathname.startsWith(prefix));
}
