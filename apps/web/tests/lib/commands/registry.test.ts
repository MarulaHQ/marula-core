// Wave 29 lane 1 — registry consistency lint (authoring-time enforcement).
//
// The contract: conflicts are settled HERE, in tests, by a lint-style
// consistency check — the same command re-registered with a different
// chord, a per-surface reshaping of a shared command, an OS/browser-
// reserved combo, or a duplicate chord FAILS. Nothing reconciles at
// runtime.
import { describe, expect, it } from "vitest";
import {
  SHORTCUT_LINT_CODE,
  RESERVED_CHORD_LIST,
  lintRegistry,
} from "../../../lib/commands/consistency";
import { parseChord, surfaceMatchesOne } from "../../../lib/commands/chords";
import { RAN } from "../../../lib/commands/outcome";
import {
  COMMAND_REGISTRY,
  commandForChord,
  commandsForSurface,
  registryGroups,
} from "../../../lib/commands/registry";
import { RAIL_SECTIONS } from "../../../components/shell/rail-model";
import type { CommandContext, CommandDef } from "../../../lib/commands/types";

function fake(overrides: Partial<CommandDef> & { id: string; chord: string }): CommandDef {
  return {
    verb: "navigate",
    label: overrides.id,
    description: "",
    surfaces: [],
    targets: [],
    run: () => RAN,
    ...overrides,
  };
}

describe("the shipped registry is lint-clean", () => {
  it("passes every consistency invariant", () => {
    expect(lintRegistry(COMMAND_REGISTRY)).toEqual([]);
  });

  it("every rail section has exactly one alt-digit chord, in rail order", () => {
    const rails = COMMAND_REGISTRY.filter((c) => c.verb === "navigate");
    expect(rails.map((c) => c.chord)).toEqual(
      RAIL_SECTIONS.slice(0, 9).map((_, i) => `alt+${i + 1}`),
    );
  });

  it("uses Alt as the activation key for command chords (mod+k is the one whitelisted exception)", () => {
    for (const command of COMMAND_REGISTRY) {
      if (command.chord === "mod+k") continue;
      expect(command.chord.startsWith("alt+")).toBe(true);
    }
  });

  it("carries non-empty typed labels for the overlay + cheat sheet", () => {
    for (const command of COMMAND_REGISTRY) {
      expect(command.label.length).toBeGreaterThan(0);
      expect(command.description.length).toBeGreaterThan(0);
      expect(registryGroups().some((g) => g.commands.includes(command))).toBe(true);
    }
  });
});

describe("the lint rejects what must never reach runtime", () => {
  it("rejects an OS/browser-reserved combo", () => {
    expect(RESERVED_CHORD_LIST).toContain("mod+w");
    const violations = lintRegistry([fake({ id: "navigate.bad", chord: "mod+w" })]);
    expect(violations).toContainEqual({
      code: SHORTCUT_LINT_CODE.RESERVED_CHORD,
      message: violations[0]?.message,
    });
    expect(violations[0]?.message).toContain("reserved");
  });

  it("rejects the same chord bound twice — no per-surface reshaping", () => {
    const violations = lintRegistry([
      fake({ id: "new.clients", chord: "alt+n" }),
      fake({ id: "new.ticket", chord: "alt+n" }),
    ]);
    expect(violations.some((v) => v.code === SHORTCUT_LINT_CODE.DUPLICATE_CHORD)).toBe(true);
  });

  it("rejects a re-registered id (a second registration with a different chord)", () => {
    const violations = lintRegistry([
      fake({ id: "new.clients", chord: "alt+n" }),
      fake({ id: "new.clients", chord: "alt+m", surfaces: ["/clients"] }),
    ]);
    expect(violations.some((v) => v.code === SHORTCUT_LINT_CODE.DUPLICATE_ID)).toBe(true);
  });

  it("rejects malformed chords and verb-id drift", () => {
    expect(lintRegistry([fake({ id: "navigate.x", chord: "banana" })]).some(
      (v) => v.code === SHORTCUT_LINT_CODE.MALFORMED_CHORD,
    )).toBe(true);
    expect(
      lintRegistry([fake({ id: "drifted.x", chord: "alt+m", verb: "navigate" })]).some(
        (v) => v.code === SHORTCUT_LINT_CODE.VERB_SHAPE,
      ),
    ).toBe(true);
  });
});

describe("chord parsing + matching", () => {
  it("renders human chords", () => {
    expect(parseChord("alt+n").display).toBe("Alt+N");
    expect(parseChord("mod+k").display).toBe("Mod+K");
  });

  it("matches segment patterns exactly", () => {
    expect(surfaceMatchesOne("/tickets/[id]", "/tickets/a2f")).toBe(true);
    expect(surfaceMatchesOne("/tickets/[id]", "/tickets")).toBe(false);
    expect(surfaceMatchesOne("/tickets/[id]", "/tickets/a/b")).toBe(false);
    expect(surfaceMatchesOne("/clients", "/clients")).toBe(true);
    expect(surfaceMatchesOne("/clients", "/clients/abc")).toBe(false);
  });
});

describe("dispatch resolution", () => {
  function ctx(pathname: string): CommandContext & { navigated: string[] } {
    const navigated: string[] = [];
    return {
      pathname,
      navigated,
      navigate: (href) => {
        navigated.push(href);
      },
      click: () => {
        navigated.push("click");
        return { kind: "clicked" };
      },
      copy: () => true,
    };
  }

  it("resolves Alt+1..9 to the rail sections", () => {
    const c = ctx("/tickets");
    const first = commandForChord("alt+1", "/tickets");
    expect(first?.id).toBe(`navigate.${RAIL_SECTIONS[0]?.id ?? ""}`);
    first?.run(c);
    expect(c.navigated).toEqual([RAIL_SECTIONS[0]!.href]);
  });

  it("new resolves per surface and stays absent where no create exists", () => {
    expect(commandForChord("alt+n", "/clients")?.id).toBe("new.clients");
    expect(commandForChord("alt+n", "/boards/xyz")?.id).toBe("new.ticket");
    // The create verb does NOT follow you to a surface without new rows:
    // no command does what a click cannot.
    expect(commandForChord("alt+n", "/reports")).toBeNull();
  });

  it("does not advertise a list-page command on a record subroute (Wave 47 lane A25)", () => {
    // "/clients" is the list page. The detail page renders no create control,
    // so prefix matching used to offer Alt+N where it could only fail.
    expect(commandForChord("alt+n", "/clients/abc")).toBeNull();
    expect(commandsForSurface("/clients/abc").some((c) => c.id === "new.clients")).toBe(false);
    // A declared [id] surface still covers a real record path.
    expect(commandForChord("alt+c", "/clients/abc")?.id).toBe("copy-link.record");
  });

  it("copy-link rides record detail surfaces only", () => {
    expect(commandForChord("alt+c", "/tickets/30000000-0000-4000-8000-000000000001")?.id).toBe(
      "copy-link.record",
    );
    expect(commandForChord("alt+c", "/tickets")).toBeNull();
  });

  it("surface-scoped commands appear in the overlay inventory, list pages keep globals", () => {
    const onClients = commandsForSurface("/clients");
    expect(onClients.some((c) => c.id === "new.clients")).toBe(true);
    const onTickets = commandsForSurface("/tickets");
    expect(onClients.some((c) => c.id === "palette.search")).toBe(true);
    expect(onTickets.some((c) => c.id === "copy-link.record")).toBe(false);
  });
});
