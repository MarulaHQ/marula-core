"use client";
/**
 * DensitySwitch — Wave 41 lane 2b, decision D7, reshaped against the pack in
 * the lane-10 vision pass.
 *
 * Purpose: pick one of the three density steps.
 *
 * Two shapes, because the setting is read in two places. In a ledger footer the
 * pack draws a bare "Density" label and three small row glyphs — more lines
 * means more room — with the active step a raised tile. That is the default,
 * and it is what every ledger footer uses. In the appearance settings, where
 * the page is EXPLAINING the setting, the wider shape carries each step's name
 * and its measurement, because "Compact" is a word everybody has an opinion
 * about and none of those opinions are 40px: showing the step's own triple is
 * the only way a user knows which assumption the switch is about to break.
 *
 * The measurements are on the control in the settings shape and announced in
 * both: a screen-reader user gets the numbers either way, while a user in a
 * footer gets the glyph.
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

/** The pack's own row glyphs (i-rows-5 / i-rows-4 / i-rows-3): more lines
 *  means more room, so comfortable is the densest-looking stack of five and
 *  dense the sparsest of three. */
const STEP_ICON: Record<Density, string> = {
  comfortable: "M4 4h16M4 8h16M4 12h16M4 16h16M4 20h16",
  compact: "M4 5h16M4 9.7h16M4 14.3h16M4 19h16",
  dense: "M4 6h16M4 12h16M4 18h16",
};

export interface DensitySwitchProps {
  value: Density;
  onChange: (step: Density) => void;
  disabled?: boolean;
  /** Announced as the group's name, and printed beside it unless the caller
   *  turns the printed label off. */
  label?: string;
  /**
   * "icons" (default) is the pack's footer control: a bare label and three
   * row glyphs. "described" is the settings form, which spells out each step
   * and its measurement.
   */
  variant?: "icons" | "described";
  className?: string;
}

export function DensitySwitch({
  value,
  onChange,
  disabled,
  label = "Density",
  variant = "icons",
  className,
}: DensitySwitchProps): React.JSX.Element {
  const described = variant === "described";
  return (
    <div
      className={cn(
        "m-density-switch",
        described ? "m-density-switch--described" : "m-density-switch--icons",
        className,
      )}
    >
      <span className="m-density-switch-label" aria-hidden={!described ? "true" : undefined}>
        {label}
      </span>
      <div
        className="m-density-switch-group"
        role="radiogroup"
        aria-label={described ? undefined : label}
      >
        {DENSITY_STEPS.map((step) => {
          const active = step === value;
          const spec = DENSITY_STEP_SPEC[step];
          return (
            <button
              key={step}
              type="button"
              role="radio"
              aria-checked={active}
              aria-label={described ? undefined : STEP_LABEL[step]}
              disabled={disabled}
              className={cn("m-density-switch-item", active && "is-active")}
              onClick={() => onChange(step)}
            >
              {described ? (
                <>
                  <span className="m-density-switch-step">{STEP_LABEL[step]}</span>
                  <span className="m-density-switch-hint" aria-hidden="true">
                    {STEP_HINT[step]}
                  </span>
                </>
              ) : (
                <>
                  <svg
                    width="16"
                    height="16"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.75"
                    strokeLinecap="round"
                    aria-hidden="true"
                  >
                    <path d={STEP_ICON[step]} />
                  </svg>
                  {/* Announced rather than shown: the glyph says "rows" and a
                      screen-reader user still gets the numbers. */}
                  <span className="sr-only">
                    {`${STEP_LABEL[step]}: ${spec.control} pixel controls, ${spec.row} pixel rows, ${spec.card} pixel card padding`}
                  </span>
                </>
              )}
            </button>
          );
        })}
      </div>
    </div>
  );
}