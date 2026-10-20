// Command outcomes — how a chord says what it did (Wave 47 lane A25).
//
// Why this module exists. `ctx.click` used to return a boolean that three
// registry rows ignored. When the Wave 41 pack port renamed `new-record`, the
// chord kept being advertised in the hold-Alt overlay and the cheat sheet and
// did nothing, and nothing anywhere said so. The silence was the bug.
//
// The contract now:
//   - a command returns a `CommandOutcome` (types.ts) — `void` is not an option
//   - a miss is a `failed` outcome carrying the command id and the selector
//   - the dispatcher reports every outcome through ONE function below
//
// Reporting has three channels on purpose, because each one catches a reader
// the others miss:
//   1. `console.error` / `console.debug` — the person with devtools open
//   2. a `marula:shortcut-outcome` DOM event — a test or E2E that listens
//   3. `data-testid="shortcut-last-outcome"` in the shortcut layer — a debugger
//      reading the DOM, and a component test asserting the outcome without
//      parsing a console
//
// Production is never interrupted. A keyboard chord that throws out of a window
// keydown listener cannot reach the user as a message; it only adds an
// uncaught error on top of the broken command. The loud channel is an error
// line plus an inspectable marker, which is also what an E2E can assert.

import type { CommandOutcome } from "./types";

/** Fired on `window` for every command press, whatever the outcome. */
export const COMMAND_OUTCOME_EVENT = "marula:shortcut-outcome";

/** One press: which command, on which surface, and what it did. */
export interface CommandOutcomeDetail {
  command: string;
  pathname: string;
  outcome: CommandOutcome;
}

declare global {
  interface Window {
    /**
     * Last failures this page saw, newest last, capped. An E2E reads it after
     * pressing a chord instead of racing an event listener:
     * `page.evaluate(() => window.__marulaCommandFailures)`.
     */
    __marulaCommandFailures?: CommandOutcomeDetail[];
  }
}

const FAILURE_MEMORY = 20;

/** The shared success outcome. Frozen: every `ran` is the same fact. */
export const RAN: CommandOutcome = Object.freeze({ status: "ran" });

/** The author declared this command can legitimately have nothing to act on. */
export function inapplicable(reason: string, selector?: string): CommandOutcome {
  return selector === undefined
    ? { status: "inapplicable", reason }
    : { status: "inapplicable", reason, selector };
}

/** The author promised a target on this surface and it was not there. */
export function failed(reason: string, selector?: string): CommandOutcome {
  return selector === undefined
    ? { status: "failed", reason }
    : { status: "failed", reason, selector };
}

/** One line naming the command, the surface, and the selector. */
export function describeOutcome(detail: CommandOutcomeDetail): string {
  const { command, pathname, outcome } = detail;
  if (outcome.status === "ran") return `${command} ran on ${pathname}`;
  const target = outcome.selector === undefined ? "" : ` target=${outcome.selector}`;
  return `${command} ${outcome.status} on ${pathname}${target}: ${outcome.reason}`;
}

/**
 * Report one press. Never throws: the point is to leave a trace, not to break
 * the surface the user is on. `failed` is an error line, `inapplicable` is a
 * debug line, `ran` is silent apart from the event.
 */
export function reportCommandOutcome(detail: CommandOutcomeDetail): void {
  const { outcome } = detail;
  if (outcome.status === "failed") {
    console.error(`[shortcuts] COMMAND FAILED — ${describeOutcome(detail)}`);
  } else if (outcome.status === "inapplicable") {
    console.debug(`[shortcuts] not applicable — ${describeOutcome(detail)}`);
  }
  if (typeof window === "undefined") return;
  if (outcome.status === "failed") {
    const seen = window.__marulaCommandFailures ?? [];
    window.__marulaCommandFailures = [...seen, detail].slice(-FAILURE_MEMORY);
  }
  window.dispatchEvent(new CustomEvent<CommandOutcomeDetail>(COMMAND_OUTCOME_EVENT, { detail }));
}
