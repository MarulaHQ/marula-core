/**
 * Locale-aware numbers.
 *
 * Purpose: one place a count, a byte total or a currency prints with a
 * thousands separator, so a surface never reaches for `toLocaleString()` and
 * leaves a number outside the kit's formatting story.
 *
 * Non-purpose: no zone, no instants (that is `lib/time`), no currency table —
 * callers pass the symbol or the `Intl` options they mean.
 *
 * Unlike a clock, a number has no zone to be right or wrong about: the reader's
 * own numeric locale is a reading they should win, so the locale argument
 * defaults to the device's, exactly as `toLocaleString()` did.
 */

/** Formats a number with the reader's grouping. `—` for a non-finite value. */
export function formatNumber(
  value: number | null | undefined,
  options?: Intl.NumberFormatOptions,
  locale?: string,
): string {
  if (typeof value !== "number" || !Number.isFinite(value)) return "—";
  try {
    return new Intl.NumberFormat(locale, options).format(value);
  } catch {
    return String(value);
  }
}
