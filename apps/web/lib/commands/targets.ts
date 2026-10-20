// Command targets — the controls a command clicks, declared so a rename is a
// red check instead of a dead chord (Wave 47 lane A25).
//
// Two constructors, `testid()` and `attr()`. Both keep the selector and the
// markup fact it resolves to in one object: `run` clicks `target.selector`,
// and `scripts/check-command-targets.ts` imports the real registry and
// resolves each declaration against the source tree. One declaration, two
// consumers, no second list to drift.
//
// `presence` is the policy for "target legitimately absent":
//   - "required" (the default) — the surfaces this command declares promise
//     the control. A miss is a broken command and reports `failed`.
//   - "optional" — the author says the control can be absent by design
//     (nothing is open to close). A miss reports `inapplicable`.
// Both are non-silent; the difference is severity, which the dispatcher keeps.
//
// `via` is where the static check looks for the control:
//   - "surface" (the default) — inside the route directory of every surface
//     the command declares. A rename on ONE page is red even when another
//     page kept the testid, which is exactly how `new-record` died on
//     /clients while /boards still rendered it.
//   - "shared" — the control lives in a shared component the route imports;
//     existence anywhere in the source is the strongest a text scan can say.
//     Declaring it here is a written decision, not a silent exception.

import { failed, inapplicable, RAN } from "./outcome";
import type {
  CommandContext,
  CommandOutcome,
  CommandTarget,
  TargetPresence,
} from "./types";

export interface TargetOptions {
  /** Absence policy. Default `required`. */
  presence?: TargetPresence;
  /** What the control is, in words, for the failure message. */
  description?: string;
  /** Where the static check resolves the control. Default `surface`. */
  via?: "surface" | "shared";
}

/** A `data-testid` control. */
export function testid(name: string, options: TargetOptions = {}): CommandTarget {
  return {
    selector: `[data-testid="${name}"]`,
    testId: name,
    attribute: null,
    presence: options.presence ?? "required",
    via: options.via ?? "surface",
    description: options.description ?? `the [data-testid="${name}"] control`,
  };
}

/** A control marked by any other attribute (e.g. `data-shortcuts-close="1"`). */
export function attr(name: string, value: string, options: TargetOptions = {}): CommandTarget {
  return {
    selector: `[${name}="${value}"]`,
    testId: null,
    attribute: { name, value },
    presence: options.presence ?? "required",
    via: options.via ?? "surface",
    description: options.description ?? `the [${name}="${value}"] control`,
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
