// Wave 26 onboarding step actions — the skip contract, rendered.
//
// Purpose: the two recording buttons every step shows. "Skip for now" is
// the DEFAULT-safe action (records nothing server-side; the step stays
// pending and resumable); "Never show this again" persists a recorded
// dismissal that never resets on reload and never silently re-nags.
//
// Non-purpose: no step logic; parents own the mutation calls.

import { Button } from "@meridian/ui";

export function StepActions({
  onSkip,
  onNever,
  busy,
}: {
  onSkip: () => void;
  onNever: () => void;
  busy?: boolean;
}) {
  return (
    <div className="flex flex-wrap items-center gap-3">
      <Button
        variant="ghost"
        size="sm"
        disabled={busy}
        onClick={onSkip}
        data-testid="onboarding-skip-for-now"
      >
        Skip for now
      </Button>
      <Button
        variant="ghost"
        size="sm"
        disabled={busy}
        onClick={onNever}
        data-testid="onboarding-never-show"
        style={{ color: "var(--m-text-muted)" }}
      >
        Never show this again
      </Button>
    </div>
  );
}
