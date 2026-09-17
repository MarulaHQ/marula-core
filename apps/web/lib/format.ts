// Shared formatting helpers for Wave 9 staff surfaces, extended by Wave 17
// lane 1 with the daily-landing time vocabulary (durations, due countdowns).
export function money(minor: number, currency: string): string {
  return new Intl.NumberFormat(undefined, { style: "currency", currency }).format(minor / 100);
}

/** Minutes to a compact hours string ("6h 30m" / "45m" / "0m"). */
export function fmtMins(mins: number): string {
  if (!Number.isFinite(mins) || mins <= 0) return "0m";
  const h = Math.floor(mins / 60);
  const m = Math.round(mins % 60);
  if (h === 0) return `${m}m`;
  if (m === 0) return `${h}h`;
  return `${h}h ${m}m`;
}

/**
 * Compact relative countdown from `now` ("2d 4h", "3h", "overdue 5h").
 * `now` is injectable so callers and tests stay deterministic.
 */
export function dueLabel(
  iso: string,
  now: number = Date.now(),
): { text: string; overdue: boolean } {
  const due = new Date(iso).getTime();
  const diffMs = due - now;
  const mins = Math.round(Math.abs(diffMs) / 60_000);
  const d = Math.floor(mins / 1440);
  const h = Math.floor((mins % 1440) / 60);
  const span = d > 0 ? `${d}d ${h}h` : h > 0 ? `${h}h` : `${mins}m`;
  return diffMs < 0
    ? { text: `overdue ${span}`, overdue: true }
    : { text: span, overdue: false };
}

/** Which due bucket a deadline falls into, for the glanceable queue groups. */
export type DueBucket = "overdue" | "today" | "later" | "none";

export function dueBucket(
  iso: string | null | undefined,
  now: number = Date.now(),
): DueBucket {
  if (iso === null || iso === undefined) return "none";
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return "none";
  if (t < now) return "overdue";
  // "Due today" = before the end of the caller's local day (23:59:59.999).
  const endOfDay = new Date(now);
  endOfDay.setHours(23, 59, 59, 999);
  return t <= endOfDay.getTime() ? "today" : "later";
}
