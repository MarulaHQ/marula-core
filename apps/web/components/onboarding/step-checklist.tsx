// Wave 26 onboarding step checklist — the wizard's shared step renderer.
//
// Purpose: one checklist vocabulary used by BOTH the org wizard rail and
// the user first-run page: done (accent check), satisfied-but-unrecorded
// (hollow check — the artifact exists, press Mark done), dismissed
// (muted ✓ + "hidden"), pending (numbered dot). Steps are clickable to
// focus. Tokens only, motion ≤120ms transform/opacity.
//
// Non-purpose: no fetching and no mutation logic — parent owns state and
// callbacks; this renders the contract only.

import type { OnboardingStepView } from "../../lib/onboarding";

export function StepChecklist({
  steps,
  labels,
  current,
  onSelect,
}: {
  steps: OnboardingStepView[];
  labels: Record<string, { title: string }>;
  current: string | null;
  onSelect: (step: string) => void;
}) {
  return (
    <ol className="grid gap-1" data-testid="onboarding-step-list">
      {steps.map((s, i) => {
        const isCurrent = current === s.step;
        const state = s.done
          ? "done"
          : s.dismissed
            ? "dismissed"
            : s.satisfied
              ? "satisfied"
              : "pending";
        const color =
          state === "done"
            ? "var(--m-accent)"
            : state === "satisfied"
              ? "var(--m-accent)"
              : state === "dismissed"
                ? "var(--m-text-muted)"
                : "var(--m-text-secondary)";
        return (
          <li key={s.step}>
            <button
              type="button"
              onClick={() => onSelect(s.step)}
              aria-current={isCurrent ? "step" : undefined}
              className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left"
              style={{
                background: isCurrent ? "var(--m-canvas-raised)" : "transparent",
                transition: "background 120ms ease, opacity 120ms ease",
                opacity: state === "dismissed" ? 0.7 : 1,
              }}
              data-testid={`onboarding-rail-${s.step}`}
              data-state={state}
            >
              <span
                aria-hidden="true"
                className="grid h-4 w-4 shrink-0 place-items-center rounded-full text-[10px] font-semibold"
                style={{
                  border:
                    state === "pending"
                      ? "1.5px solid var(--m-border)"
                      : `1.5px solid ${color}`,
                  color,
                }}
              >
                {state === "done" || state === "satisfied" || state === "dismissed"
                  ? "✓"
                  : i + 1}
              </span>
              <span
                className="text-[13px] leading-snug"
                style={{
                  color: state === "pending" ? "var(--m-text)" : color,
                  fontWeight: isCurrent ? 600 : 400,
                }}
              >
                {labels[s.step]?.title ?? s.step}
              </span>
              {state === "dismissed" && (
                <span
                  className="ml-auto text-[11px]"
                  style={{ color: "var(--m-text-muted)" }}
                >
                  hidden
                </span>
              )}
            </button>
          </li>
        );
      })}
    </ol>
  );
}
