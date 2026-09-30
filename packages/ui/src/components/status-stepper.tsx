"use client";
/**
 * StatusStepper — Wave 41 lane 2b, from DESIGN-NOTES §4.
 *
 * Purpose: show where a ticket is in its workflow and let a person move it by
 * clicking the step they want. Three rules from the spec, all of them
 * load-bearing:
 *
 * 1. The CURRENT step is gold. Gold is the product's word for "this one".
 * 2. An EARLIER step shows a check. A step you have been through is done, and
 *    done in this product is green, not a dimmer version of the current one.
 * 3. Clicking ANY step sets it — not just the next one. A stepper that only
 *    moves forward is a progress bar; a technician who has already called the
 *    client and knows the answer is Resolved should not have to walk it
 *    through Waiting to get there.
 *
 * Non-purpose: this does not decide whether a move is legal. That is the
 * board's transition table, and a stepper that enforced its own rules would
 * disagree with the API. `disabled` is here for the surface that already
 * knows: a status the ticket cannot reach is shown, not offered.
 */
import { cn } from "../lib/cn";

export interface StepperStep {
  id: string;
  label: string;
}

export interface StatusStepperProps {
  steps: StepperStep[];
  /** The step the ticket is on now. */
  currentId: string;
  onSelect?: (id: string) => void;
  disabled?: boolean;
  label?: string;
  className?: string;
}

export function StatusStepper({
  steps,
  currentId,
  onSelect,
  disabled,
  label = "Ticket status",
  className,
}: StatusStepperProps): React.JSX.Element {
  const currentIndex = steps.findIndex((s) => s.id === currentId);
  // A current step the caller does not list (a status from a newer API, or a
  // workflow that has not shipped) renders as "no current step" rather than
  // throwing the last step's gold onto the wrong row.
  const resolved = currentIndex === -1 ? -1 : currentIndex;
  return (
    <ol className={cn("m-stepper", className)} aria-label={label}>
      {steps.map((step, index) => {
        const isCurrent = index === resolved;
        const isDone = resolved > index;
        const interactive = Boolean(onSelect) && !disabled;
        return (
          <li key={step.id} className="m-stepper-item">
            {index > 0 ? (
              <span
                className="m-stepper-rail"
                data-done={isDone ? "true" : undefined}
                aria-hidden="true"
              />
            ) : null}
            <button
              type="button"
              className={cn(
                "m-stepper-step",
                isCurrent && "is-current",
                isDone && "is-done",
              )}
              aria-current={isCurrent ? "step" : undefined}
              disabled={!interactive}
              onClick={interactive ? () => onSelect?.(step.id) : undefined}
            >
              <span className="m-stepper-mark" aria-hidden="true">
                {isDone ? (
                  <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3.2">
                    <path d="M4 12.5 9.5 18 20 6.5" strokeLinecap="round" strokeLinejoin="round" />
                  </svg>
                ) : (
                  index + 1
                )}
              </span>
              <span className="m-stepper-label">{step.label}</span>
            </button>
          </li>
        );
      })}
    </ol>
  );
}
