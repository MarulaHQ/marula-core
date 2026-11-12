// NavCounter — the one count chip the shell draws.
//
// Purpose: every number on the navigation chrome (unread notifications on the
// bell, open tickets on the rail) is this component, so the badges cannot drift
// into two sizes, two colours or two caps. The fill is the inverted pair
// (`primary` / `primary-foreground`): ink with a gold figure in light, gold with
// an ink figure in dark, which is the mockup's count badge in both themes.
//
// Non-purpose: no fetching, no accessible name. The tappable parent owns both —
// it reports the number to a screen reader (the chip is aria-hidden, so the
// count is announced once, not twice) and decides where the chip sits.
//
// Rules it enforces for its callers:
//   - null/undefined/NaN/0 renders NOTHING. A count that has not loaded is not
//     a zero, and a fake zero is a lie the chrome tells quietly.
//   - above `cap` it reads "99+", so a large number cannot widen the chrome.
//   - tabular figures, so a count going 9 -> 10 -> 11 does not jitter.

export function NavCounter({
  count,
  cap = 99,
  className = "",
}: {
  /** The number to show. `null` while it is unknown or unavailable. */
  count: number | null | undefined;
  /** Largest number drawn in full; above it the chip reads `<cap>+`. */
  cap?: number;
  /** Position the chip (corner-anchored, inline, margin). */
  className?: string;
}) {
  if (typeof count !== "number" || !Number.isFinite(count) || count <= 0) return null;
  const whole = Math.floor(count);
  return (
    <span
      aria-hidden="true"
      className={`pointer-events-none inline-flex h-4.5 min-w-4.5 items-center justify-center rounded-full bg-primary px-1 text-xs font-semibold leading-none tabular-nums text-primary-foreground ${className}`}
    >
      {whole > cap ? `${cap}+` : whole}
    </span>
  );
}
