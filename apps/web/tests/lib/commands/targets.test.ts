// Wave 47 lane A25 (`p47-silent-command-noops`) — the outcome contract.
//
// The bug this suite exists for: `ctx.click` returned a boolean, three
// registry rows ignored it, and a renamed `new-record` left Alt+N advertised
// and dead. These tests pin the replacement: a miss is a `failed` outcome that
// carries the selector, a declared-optional miss is `inapplicable`, and the
// dispatcher's reporter leaves a trace a person (or an E2E) can read.
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  COMMAND_OUTCOME_EVENT,
  RAN,
  describeOutcome,
  failed,
  inapplicable,
  reportCommandOutcome,
} from "../../../lib/commands/outcome";
import type { CommandOutcomeDetail } from "../../../lib/commands/outcome";
import { COMMAND_REGISTRY } from "../../../lib/commands/registry";
import { attr, clickTarget, testid } from "../../../lib/commands/targets";
import type { ClickResult, CommandContext } from "../../../lib/commands/types";

function ctx(pathname: string, click: (selector: string) => ClickResult): CommandContext {
  return {
    pathname,
    click,
    navigate: () => {},
    copy: () => true,
  };
}

const hit = (): ClickResult => ({ kind: "clicked" });
const miss = (selector: string): ClickResult => ({ kind: "absent", selector });

afterEach(() => {
  delete window.__marulaCommandFailures;
  vi.restoreAllMocks();
});

describe("a target declaration is the selector its command clicks", () => {
  it("builds a required testid selector by default", () => {
    const target = testid("new-record");
    expect(target.selector).toBe('[data-testid="new-record"]');
    expect(target.presence).toBe("required");
    expect(target.testId).toBe("new-record");
  });

  it("builds an attribute selector and keeps presence explicit", () => {
    const target = attr("data-shortcuts-close", "1", { presence: "optional" });
    expect(target.selector).toBe('[data-shortcuts-close="1"]');
    expect(target.presence).toBe("optional");
    expect(target.attribute).toEqual({ name: "data-shortcuts-close", value: "1" });
  });
});

describe("a miss is an outcome, not a boolean", () => {
  it("reports ran when the control was clicked", () => {
    const outcome = clickTarget(ctx("/clients", hit), testid("new-record"));
    expect(outcome).toEqual(RAN);
  });

  it("reports failed with the command's selector when a required target is absent", () => {
    const target = testid("new-record");
    const outcome = clickTarget(ctx("/clients", miss), target);
    expect(outcome.status).toBe("failed");
    if (outcome.status !== "failed") throw new Error("unreachable");
    expect(outcome.selector).toBe('[data-testid="new-record"]');
    expect(outcome.reason).toContain("/clients");
  });

  it("reports inapplicable — not failed — when the author declared the target optional", () => {
    const target = attr("data-shortcuts-close", "1", {
      presence: "optional",
      description: "the topmost overlay's close control",
    });
    const outcome = clickTarget(ctx("/clients", miss), target);
    expect(outcome.status).toBe("inapplicable");
    if (outcome.status !== "inapplicable") throw new Error("unreachable");
    expect(outcome.reason).toContain("close control");
  });
});

describe("the shipped registry declares what it clicks", () => {
  const byId = (id: string) => COMMAND_REGISTRY.find((c) => c.id === id);

  it("new.clients and new.ticket both require the shared new-record control", () => {
    for (const id of ["new.clients", "new.ticket"]) {
      const command = byId(id);
      expect(command?.targets).toHaveLength(1);
      expect(command?.targets[0]?.testId).toBe("new-record");
      expect(command?.targets[0]?.presence).toBe("required");
    }
  });

  it("close.topmost declares its target optional: nothing open is not a broken chord", () => {
    const close = byId("close.topmost");
    expect(close?.targets).toHaveLength(1);
    expect(close?.targets[0]?.presence).toBe("optional");
  });

  it("commands that do not click declare an empty target list, none of them silently", () => {
    for (const command of COMMAND_REGISTRY) {
      expect(Array.isArray(command.targets)).toBe(true);
      if (command.targets.length > 0) {
        for (const target of command.targets) {
          expect(target.selector.length).toBeGreaterThan(0);
          expect(target.description.length).toBeGreaterThan(0);
        }
      }
    }
  });
});

describe("reporting leaves a trace the person debugging can read", () => {
  it("fails loudly: an error line, the failure memory, and the DOM event", () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    const seen: CommandOutcomeDetail[] = [];
    const listen = (event: Event) => seen.push((event as CustomEvent<CommandOutcomeDetail>).detail);
    window.addEventListener(COMMAND_OUTCOME_EVENT, listen);

    reportCommandOutcome({
      command: "new.clients",
      pathname: "/clients",
      outcome: failed("the control is not on /clients", '[data-testid="new-record"]'),
    });

    expect(error).toHaveBeenCalledTimes(1);
    const line = String(error.mock.calls[0]?.[0] ?? "");
    expect(line).toContain("new.clients");
    expect(line).toContain('[data-testid="new-record"]');
    expect(window.__marulaCommandFailures).toHaveLength(1);
    expect(window.__marulaCommandFailures?.[0]?.command).toBe("new.clients");
    expect(seen[0]?.outcome.status).toBe("failed");

    window.removeEventListener(COMMAND_OUTCOME_EVENT, listen);
  });

  it("keeps the failure memory bounded", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    for (let i = 0; i < 25; i += 1) {
      reportCommandOutcome({ command: `new.${i}`, pathname: "/clients", outcome: failed("gone") });
    }
    expect(window.__marulaCommandFailures).toHaveLength(20);
  });

  it("says 'not applicable' at debug level, never as an error", () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    const debug = vi.spyOn(console, "debug").mockImplementation(() => {});
    reportCommandOutcome({
      command: "close.topmost",
      pathname: "/clients",
      outcome: inapplicable("nothing is open"),
    });
    expect(error).not.toHaveBeenCalled();
    expect(debug).toHaveBeenCalledTimes(1);
    expect(window.__marulaCommandFailures ?? []).toHaveLength(0);
  });

  it("names the command, surface and selector in one line", () => {
    const line = describeOutcome({
      command: "new.clients",
      pathname: "/clients",
      outcome: failed("missing", '[data-testid="new-record"]'),
    });
    expect(line).toBe('new.clients failed on /clients target=[data-testid="new-record"]: missing');
  });
});
