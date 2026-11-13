// Motion rules — the theme law as numbers, RN-free so the node test suite can
// hold the line without a renderer. motion.tsx imports these and adds the
// animated components, the same split pack-rules.ts and pack.tsx use.
//
// docs/theme.md §5: transitions at or under 120ms, on transform and opacity,
// collapsing under reduced motion. Nothing here animates a layout property —
// a skeleton that reflows the row it is standing in is worse than no skeleton.

/** The only timings the app is allowed to use. */
export const motion = {
  /** The ceiling from theme.md §5. No transition in the app runs longer. */
  duration: 120,
  /** Entrance rise in px: enough to read as settling, not as sliding. */
  distance: 4,
  /** Added delay per item in a staggered reveal. */
  stagger: 12,
  /** The longest delay a stagger may hand the last item. */
  maxStagger: 60,
  /** How far a touched card dips, as a scale multiplier. */
  pressScale: 0.985,
  /** The dip itself is quicker than the release: it must feel immediate. */
  pressIn: 90,
} as const;

/**
 * Delay for item `index` in a staggered reveal.
 *
 * Capped at `maxStagger` so a fifty-row list does not keep arriving for a
 * second: rows past the cap enter together, which reads as deliberate rather
 * than slow. A reduced-motion reader gets zero, because a delay is itself a
 * motion cue.
 */
export function staggerDelay(index: number, reduced = false): number {
  if (reduced) return 0;
  if (!Number.isFinite(index) || index <= 0) return 0;
  return Math.min(Math.round(index) * motion.stagger, motion.maxStagger);
}

/** When the last staggered item settles, in ms — the number a review checks. */
export function revealSettle(index: number): number {
  return staggerDelay(index) + motion.duration;
}
