// Command targets — the controls a command clicks, declared so a rename is a
// red check instead of a dead chord (Wave 47 lane A25).
//
// Two constructors, `testid()` and `attr()`. Both keep the selector and the
// markup fact it resolves to in one object: `run` clicks `target.selector`,
// and `scripts/check-command-targets.mjs` greps the source tree for
// `data-testid="<name>"` (or the attribute pair) from the same declaration.
// One declaration, two consumers, no second list to drift.
//
// `presence` is the policy for "target legitimately absent":
//   - "required" (the default) — the surfaces this command declares promise
//     the control. A miss is a broken command and reports `failed`.
//   - "optional" — the author says the control can be absent by design
//     (nothing is open to close). A miss reports `inapplicable`.
// Both are non-silent; the difference is severity, which the dispatcher keeps.

import { failed, inapplicable, RAN } from "./outcome";
import type { CommandContext, CommandOutcome, CommandTarget, TargetPresence } from "./types";

/** A `data-testid` control. */
export function testid(
  name: string,
  presence: TargetPresence = "required",
  description = `[data-testid="${name}"] control`,
): CommandTarget {
  return {
    selector: `[data-testid="${name}"]`,
    testId: name,
    attribute: null,
    presence,
    description,
  };
}

/** A control marked by any other attribute (e.g. `data-shortcuts-close="1"`). */
export function attr(
  name: string,
  value: string,
  presence: TargetPresence = "required",
  description = `[${name}="${value}"] control`,
): CommandTarget {
  return {
    selector: `[${name}="${value}"]`,
    testId: null,
    attribute: { name, value },
    presence,
    description,
  };
}

/**
 * Click a declared target and say what happened. The one place where
 * "selector matched nothing" turns into an outcome: `failed` for a required
 * target, `inapplicable` for one the author declared optional.
 */
export function clickTarget(ctx: CommandContext, target: CommandTarget): CommandOutcome {
  const result = ctx.click(target.selector);
  if (result.kind === "clicked") return RAN;
  const reason = `${target.description} is not on ${ctx.pathname}`;
  return target.presence === "required"
    ? failed(reason, target.selector)
    : inapplicable(reason, target.selector);
}
