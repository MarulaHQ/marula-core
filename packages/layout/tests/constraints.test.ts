/**
 * @module Tests for the pure resize-constraint mathematics.
 */

import { describe, expect, it } from "vitest";
import {
  DEFAULT_MIN_PANE_PX,
  MIN_SPLIT_PX,
  applyPairResizePx,
  clamp,
  dividerRangePx,
  fractionsToPx,
  pxToFractions,
  renormalizeFractions,
} from "../src/constraints";

const closeToArray = (actual: readonly number[], expected: readonly number[], epsilon = 1e-9) => {
  expect(actual).toHaveLength(expected.length);
  actual.forEach((value, index) => {
    expect(Math.abs(value - (expected[index] as number))).toBeLessThan(epsilon);
  });
};

describe("clamp", () => {
  it("clamps within bounds", () => {
    expect(clamp(5, 0, 10)).toBe(5);
    expect(clamp(-1, 0, 10)).toBe(0);
    expect(clamp(11, 0, 10)).toBe(10);
  });
});

describe("fractionsToPx / pxToFractions", () => {
  it("converts fractions to pixels proportionally", () => {
    closeToArray(fractionsToPx([0.25, 0.75], 400), [100, 300]);
  });

  it("distributes equally when fractions are degenerate", () => {
    closeToArray(fractionsToPx([0, 0], 200), [100, 100]);
    closeToArray(fractionsToPx([0.3, 0.7], 0), [0, 0]);
  });

  it("round-trips through pixels", () => {
    const fractions = [0.2, 0.5, 0.3];
    closeToArray(pxToFractions(fractionsToPx(fractions, 999)), fractions);
  });

  it("normalizes unnormalized pixel inputs to a unit sum", () => {
    const result = pxToFractions([10, 30]);
    closeToArray(result, [0.25, 0.75]);
    closeToArray([result.reduce((a, b) => a + b, 0)], [1]);
  });

  it("returns equal fractions for empty or invalid input", () => {
    closeToArray(pxToFractions([]), []);
    closeToArray(pxToFractions([-1, Number.NaN]), [0.5, 0.5]);
  });
});

describe("dividerRangePx", () => {
  it("honors minimums on both sides", () => {
    const range = dividerRangePx({
      firstPx: 300,
      secondPx: 300,
      first: { minPx: 120 },
      second: { minPx: 200 },
    });
    expect(range.minOffsetPx).toBe(120);
    expect(range.maxOffsetPx).toBe(600 - 200);
  });

  it("honors maximums on both sides", () => {
    const range = dividerRangePx({
      firstPx: 400,
      secondPx: 200,
      first: { minPx: 50, maxPx: 450 },
      second: { minPx: 50, maxPx: 250 },
    });
    expect(range.minOffsetPx).toBe(600 - 250);
    expect(range.maxOffsetPx).toBe(450);
  });

  it("collapses to a deterministic midpoint when constraints are unsatisfiable", () => {
    // Total 150 but mins demand 100 + 90.
    const range = dividerRangePx({
      firstPx: 60,
      secondPx: 90,
      first: { minPx: 100 },
      second: { minPx: 90 },
    });
    expect(range.minOffsetPx).toBe(range.maxOffsetPx);
  });
});

describe("applyPairResizePx", () => {
  const pair = (overrides: Partial<Parameters<typeof applyPairResizePx>[0]>) =>
    applyPairResizePx({
      firstPx: 300,
      secondPx: 300,
      first: { minPx: 120 },
      second: { minPx: 120 },
      targetOffsetPx: 300,
      ...overrides,
    });

  it("moves freely within the feasible range and preserves the total", () => {
    const [first, second] = pair({ targetOffsetPx: 420 });
    expect(first).toBe(420);
    expect(second).toBe(180);
  });

  it("clamps at the second child's minimum", () => {
    const [first, second] = pair({ targetOffsetPx: 550 });
    expect(second).toBe(120);
    expect(first).toBe(480);
  });

  it("clamps at the first child's minimum", () => {
    const [first] = pair({ targetOffsetPx: 40 });
    expect(first).toBe(120);
  });

  it("respects an explicit max on the first child", () => {
    const [first] = pair({ first: { minPx: 120, maxPx: 360 }, targetOffsetPx: 500 });
    expect(first).toBe(360);
  });

  it("pushes the boundary right when the second child's max forces it", () => {
    const [first, second] = pair({
      second: { minPx: 120, maxPx: 200 },
      targetOffsetPx: 250,
    });
    expect(second).toBe(200);
    expect(first).toBe(400);
  });

  it("keeps other siblings mathematically untouched by preserving totals", () => {
    const before = pair({ targetOffsetPx: 310 });
    const after = pair({ targetOffsetPx: 290 });
    expect(before[0] + before[1]).toBe(after[0] + after[1]);
  });
});

describe("renormalizeFractions", () => {
  it("scales proportionally when everything fits", () => {
    const result = renormalizeFractions(
      [0.25, 0.25, 0.5],
      [
        { minPx: 100 },
        { minPx: 100 },
        { minPx: 100 },
      ],
      1200,
    );
    closeToArray(result, [0.25, 0.25, 0.5]);
  });

  it("pins children at their minimums when space shrinks below their share", () => {
    // 1000px: wants 800/200, but both must have >= 350.
    const result = renormalizeFractions(
      [0.8, 0.2],
      [{ minPx: 350 }, { minPx: 350 }],
      1000,
    );
    const px = fractionsToPx(result, 1000);
    expect(px[0]).toBeCloseTo(650, 6);
    expect(px[1]).toBeCloseTo(350, 6);
  });

  it("redistributes leftover space only among unpinned children", () => {
    // 900px: wants [90, 405, 405]; A is pinned at its 200px minimum,
    // B and C share the remainder proportionally (equally here).
    const result = renormalizeFractions(
      [0.1, 0.45, 0.45],
      [{ minPx: 200 }, { minPx: 200 }, { minPx: 200 }],
      900,
    );
    const px = fractionsToPx(result, 900);
    expect(px[0]).toBeCloseTo(200, 6);
    expect(px[1]).toBeCloseTo(350, 6);
    expect(px[2]).toBeCloseTo(350, 6);
  });

  it("respects maximum constraints while redistributing", () => {
    // 1000px equal thirds desired, but A caps at 100 -> B/C absorb the rest.
    const result = renormalizeFractions(
      [1 / 3, 1 / 3, 1 / 3],
      [{ minPx: 20, maxPx: 100 }, { minPx: 20 }, { minPx: 20 }],
      1000,
    );
    const px = fractionsToPx(result, 1000);
    expect(px[0]).toBeCloseTo(100, 6);
    expect(px[1]).toBeGreaterThan(px[2]! - 1e-6);
    expect(px[1]! + px[2]!).toBeCloseTo(900, 6);
  });

  it("falls back to proportional-to-minimum allocation when space cannot fit all minimums", () => {
    const result = renormalizeFractions(
      [0.5, 0.5],
      [{ minPx: 300 }, { minPx: 500 }],
      600,
    );
    const px = fractionsToPx(result, 600);
    expect(px[0]).toBeCloseTo(225, 6);
    expect(px[1]).toBeCloseTo(375, 6);
  });

  it("is idempotent once feasible", () => {
    const fractions = [0.7, 0.3];
    const bounds = [{ minPx: 200 }, { minPx: 250 }];
    const once = renormalizeFractions(fractions, bounds, 1000);
    const twice = renormalizeFractions(once, bounds, 1000);
    closeToArray(once, twice);
  });

  it("throws on length mismatch", () => {
    expect(() => renormalizeFractions([1], [{ minPx: 1 }, { minPx: 1 }], 500)).toThrow();
  });

  it("handles zero-size containers with equal output", () => {
    const result = renormalizeFractions([0.2, 0.8], [{ minPx: 0 }, { minPx: 0 }], 0);
    closeToArray(result, [0.5, 0.5]);
  });
});

describe("engine constants", () => {
  it("keeps documented default extents", () => {
    expect(DEFAULT_MIN_PANE_PX).toBe(120);
    expect(MIN_SPLIT_PX).toBe(80);
  });
});
