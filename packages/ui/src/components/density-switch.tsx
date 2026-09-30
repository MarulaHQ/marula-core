"use client";
/**
 * DensitySwitch — Wave 41 lane 2b, decision D7.
 *
 * Purpose: pick one of the three density steps, and say what each one
 * measures. The measurements are on the control because "Compact" is a word
 * everybody already has an opinion about and none of those opinions are 40px
 * — showing the step's own triple is the only way a user knows which of
 * their assumptions the switch is about to break.
 *
 * Radio semantics, not tabs: a density step is a SETTING, one of three, with
 * no panels behind it. Tab semantics would promise content that does not
 * exist.
 *
 * Non-purpose: this does not apply the step. The caller applies it (and
 * persists it) so the switch stays a control and the preference stays in the
 * preference layer with every other one.
 */
import { DENSITY_STEP_SPEC, DENSITY_STEPS, type Density } from "../density";
import { cn } from "../lib/cn";

const STEP_LABEL: Record<Density, string> = {
  comfortable: "Comfortable",
  compact: "Compact",
  dense: "Dense",
};

const STEP_HINT: Record<Density, string> = {
  comfortable: "44px controls, 48px rows",
  compact: "36px controls, 40px rows",
  dense: "32px controls, 32px rows",
};

export interface DensitySwitchProps {
  value: Density;
  onChange: (step: Density) => void;
  disabled?: boolean;
  /** Announced as the group's name. */
  label?: string;
  className?: string;
}

export function DensitySwitch({
  value,
  onChange,
  disabled,
  label = "Density",
  className,
}: DensitySwitchProps): React.JSX.Element {
  return (
    <div className={cn("m-density-switch", className)} role="radiogroup" aria-label={label}>
      {DENSITY_STEPS.map((step) => {
        const active = step === value;
        const spec = DENSITY_STEP_SPEC[step];
        return (
          <button
            key={step}
            type="button"
            role="radio"
            aria-checked={active}
            disabled={disabled}
            className={cn("m-density-switch-item", active && "is-active")}
            onClick={() => onChange(step)}
          >
            <span className="m-density-switch-label">{STEP_LABEL[step]}</span>
            <span className="m-density-switch-hint" aria-hidden="true">
              {STEP_HINT[step]}
            </span>
            {/* Announced rather than shown: a screen-reader user gets the
                measurements too, and a sighted user gets them as the hint. */}
            <span className="sr-only">
              {`${STEP_LABEL[step]}: ${spec.control} pixel controls, ${spec.row} pixel rows, ${spec.card} pixel card padding`}
            </span>
          </button>
        );
      })}
    </div>
  );
}
