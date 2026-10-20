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
//
// Every row declares what it clicks (`targets`) and returns a CommandOutcome
// (Wave 47 lane A25). A target whose control is missing reports `failed` with
// the command id and the selector; a target declared `optional` reports
// `inapplicable`. Neither is silent, and neither is a boolean nobody reads.

import { RAIL_SECTIONS } from "@/components/shell/rail-model";
import { openPopout } from "@/lib/popout/popout";
import { paramsFor, popoutRouteFor } from "@/lib/popout/registry";
import { openNativeWindow } from "@/lib/desktop/windows";
import { failed, inapplicable, RAN, reportCommandOutcome } from "./outcome";
import { attr, clickTarget, testid } from "./targets";
import type { CommandContext, CommandDef, CommandOutcome, CommandVerb } from "./types";

/**
 * Click-parity target for surfaces that expose their own create control.
 * Required: `/clients` and `/boards/[id]` both promise the button, so a miss
 * is a defect, not an empty state. This is the exact control a Wave 41 pack
 * port renamed out from under Alt+N.
 */
const NEW_RECORD_CONTROL = testid(
  "new-record",
  "required",
  "the New record button this surface's create dialog hangs off",
);

/** Shell overlays mark their close control with this attribute; pressing a
 * "close" command clicks the SAME control a user would click. Optional: most
 * of the time nothing is open, and that is not a broken shortcut. */
const CLOSE_CONTROL = attr(
  "data-shortcuts-close",
  "1",
  "optional",
  "the topmost overlay's own close control",
);

/** Open the current surface as a popout window, honestly. */
function popoutOutcome(ctx: CommandContext): CommandOutcome {
  const def = popoutRouteFor(ctx.pathname);
  if (def === null) {
    return inapplicable(`no popout route is registered for ${ctx.pathname}`);
  }
  const result = openPopout(def.key, paramsFor(def, ctx.pathname));
  if (result.ok) return RAN;
  return inapplicable(`the browser blocked the "${def.key}" window`);
}

/** Open a desktop-native window, honestly. Plain web has no factory: that is
 * declared, not an error. A factory refusal is reported with its command id. */
function nativeOutcome(
  commandId: string,
  ctx: CommandContext,
  routeKey: string,
  pathSuffix: string,
): CommandOutcome {
  const pending = openNativeWindow(routeKey, pathSuffix);
  if (pending === null) {
    return inapplicable(
      `the desktop shell is not running — "${routeKey}" has no native window factory on plain web`,
    );
  }
  void pending.then((result) => {
    if (result !== null && !result.ok) {
      reportCommandOutcome({
        command: commandId,
        pathname: ctx.pathname,
        outcome: failed(`the desktop shell refused to open "${routeKey}"`),
      });
    }
  });
  return RAN;
}

/** Global commands (empty surfaces = every surface). */
const GLOBAL_COMMANDS: CommandDef[] = [
  {
    id: "palette.search",
    verb: "palette",
    label: "Search & commands",
    description: "Open the command palette — unified search across tickets, clients, devices, and more.",
    chord: "mod+k",
    surfaces: [],
    targets: [],
    run: (ctx) => {
      ctx.navigate("/search");
      return RAN;
    },
  },
  {
    id: "close.topmost",
    verb: "close",
    label: "Close overlay",
    description: "Close the topmost Marula overlay or panel, same as its own close control.",
    chord: "alt+x",
    surfaces: [],
    targets: [CLOSE_CONTROL],
    run: (ctx) => clickTarget(ctx, CLOSE_CONTROL),
  },
  {
    id: "pane-cycle.section",
    verb: "pane-cycle",
    label: "Next page in section",
    description: "Cycle to the next page within the current rail section (wraps around).",
    chord: "alt+j",
    surfaces: [],
    targets: [],
    run: (ctx) => {
      const section =
        RAIL_SECTIONS.find(
          (s) => ctx.pathname === s.href || s.match.some((m) => ctx.pathname.startsWith(m)),
        ) ?? null;
      if (!section) return inapplicable(`no rail section covers ${ctx.pathname}`);
      const hrefs = section.links.map((l) => l.href);
      const active = hrefs.findIndex((h) => ctx.pathname === h || ctx.pathname.startsWith(`${h}/`));
      const next = active === -1 ? hrefs[0] : hrefs[(active + 1) % hrefs.length];
      if (!next) return inapplicable(`the ${section.label} section has no pages to cycle`);
      ctx.navigate(next);
      return RAN;
    },
  },
];

/** Rail navigation: Alt+1..9 mirror the rail sections in order. Built once
 * so the registry, the groups, and the dispatcher share the same objects. */
const RAIL_COMMANDS: ReadonlyArray<CommandDef> = RAIL_SECTIONS.slice(0, 9).map(
  (section, index): CommandDef => ({
    id: `navigate.${section.id}`,
    verb: "navigate" as CommandVerb,
    label: `Go to ${section.label}`,
    description: `Jump to the ${section.label} section, same as the rail's ${section.label} entry.`,
    chord: `alt+${index + 1}`,
    surfaces: [],
    targets: [],
    run: (ctx: CommandContext) => {
      ctx.navigate(section.href);
      return RAN;
    },
  }),
);

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
    targets: [NEW_RECORD_CONTROL],
    run: (ctx) => clickTarget(ctx, NEW_RECORD_CONTROL),
  },
  {
    id: "new.ticket",
    verb: "new",
    label: "New ticket",
    description: "Open this board's new-ticket dialog, exactly as the New ticket button does.",
    chord: "alt+n",
    surfaces: ["/boards/[id]"],
    targets: [NEW_RECORD_CONTROL],
    run: (ctx) => clickTarget(ctx, NEW_RECORD_CONTROL),
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
    targets: [],
    // Clipboard is best-effort by contract (jsdom, denied permission, http):
    // a false here is an environment fact, not a broken command.
    run: (ctx) => {
      ctx.copy(window.location.href);
      return RAN;
    },
  },
];

/**
 * Popout commands (Wave 38): open the current surface as a route-keyed
 * secondary window (multi-monitor), exactly as the in-context "Pop out"
 * affordance does. A blocked pop-up is reported as inapplicable rather than
 * pretended into success.
 */
const POPOUT_COMMANDS: CommandDef[] = [
  {
    id: "popout.ticket",
    verb: "popout",
    label: "Pop out ticket",
    description: "Open this ticket in its own window (compact shell, for a second monitor).",
    chord: "alt+p",
    surfaces: ["/tickets/[id]", "/tickets/classic/[id]"],
    targets: [],
    run: popoutOutcome,
  },
  {
    id: "popout.surface",
    verb: "popout",
    label: "Pop out surface",
    description: "Open this surface (dispatch board, stock lookup, decisions) in its own window.",
    chord: "alt+shift+p",
    surfaces: ["/dispatch", "/stock", "/approvals"],
    targets: [],
    run: popoutOutcome,
  },
];

/**
 * Desktop fleet commands (Wave 38 lane 2): open the quick-action windows
 * the tray also offers. They run everywhere in the desktop shell; on plain
 * web the native opener honestly returns null, and that is reported as
 * inapplicable — the desktop-only labeling keeps the cheat sheet honest.
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
    targets: [],
    run: (ctx) => nativeOutcome("popout.dispatch-native", ctx, "dispatch-board", "dispatch"),
  },
  {
    id: "popout.stock-native",
    verb: "popout",
    label: "Open stock lookup window",
    description: "Desktop: open the stock lookup as its own app window (remembered position per window).",
    chord: "alt+s",
    surfaces: [],
    targets: [],
    run: (ctx) => nativeOutcome("popout.stock-native", ctx, "stock-lookup", "stock"),
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
