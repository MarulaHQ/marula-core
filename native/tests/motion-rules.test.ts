// Motion rules — the theme law, held by the node suite.
//
// Covers: the 120ms ceiling (docs/theme.md §5), the stagger cap that keeps a
// long list from arriving for a second, and the reduced-motion collapse that
// turns every entrance into an instant paint.

import { describe, expect, it } from "vitest";

import { motion, revealSettle, staggerDelay } from "@/ui/motion-rules";

describe("motion law", () => {
  it("keeps every transition at or under the 120ms ceiling", () => {
    expect(motion.duration).toBeLessThanOrEqual(120);
    expect(motion.stagger).toBeLessThan(motion.duration);
  });

  it("rises a distance small enough to read as settling", () => {
    expect(motion.distance).toBeGreaterThan(0);
    expect(motion.distance).toBeLessThanOrEqual(8);
  });

  it("dips a pressed card by a hair, within the ceiling", () => {
    expect(motion.pressScale).toBeLessThan(1);
    expect(motion.pressScale).toBeGreaterThan(0.95);
    expect(motion.pressIn).toBeLessThanOrEqual(motion.duration);
  });
});

describe("stagger", () => {
  it("gives the first item no delay", () => {
    expect(staggerDelay(0)).toBe(0);
  });

  it("steps by the shared value", () => {
    expect(staggerDelay(1)).toBe(motion.stagger);
    expect(staggerDelay(3)).toBe(motion.stagger * 3);
  });

  it("caps so a long list settles inside one beat", () => {
    expect(staggerDelay(50)).toBe(motion.maxStagger);
    expect(revealSettle(50)).toBe(motion.maxStagger + motion.duration);
    // The whole cascade lands inside a quarter second, not a full one.
    expect(revealSettle(50)).toBeLessThanOrEqual(250);
  });

  it("treats a negative or fractional index as the first item", () => {
    expect(staggerDelay(-4)).toBe(0);
    expect(staggerDelay(2.4)).toBe(motion.stagger * 2);
  });

  it("collapses to zero under reduced motion", () => {
    expect(staggerDelay(9, true)).toBe(0);
    expect(revealSettle(9)).toBeGreaterThan(0);
  });
});
