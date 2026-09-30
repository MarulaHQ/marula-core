// Clock format — thin re-export of the pure pack rules (pack-rules.ts keeps
// the implementations RN-free for the test suite). Elapsed time reads the
// session's segments; the render tick lives with the surface that owns the
// clock (time-pill.tsx).

export { clockText, roundUpTo, shortClock } from "./pack-rules";

export function elapsedMs(
  session: { segments: { started_at: string; ended_at?: string | null }[] },
  now: number,
): number {
  let total = 0;
  for (const segment of session.segments ?? []) {
    const start = Date.parse(segment.started_at);
    if (Number.isNaN(start)) continue;
    const end = segment.ended_at != null ? Date.parse(segment.ended_at) : now;
    if (Number.isNaN(end)) continue;
    total += Math.max(0, end - start);
  }
  return total;
}
