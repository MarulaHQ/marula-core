// Pure-logic tests for the captured mark's geometry.
//
// These are the invariants the two platforms share, proof that "trim to the
// ink's own bounds, preserve aspect, centre in the box" means the same thing
// to the web pad and the native pad.

import { describe, expect, it } from "vitest";

import {
  DEFAULT_RASTER,
  TRIM_BOX,
  describeMark,
  fitTransform,
  hasInk,
  inkBounds,
  pointCount,
  toCanvasPoint,
  type Stroke,
} from "./stroke";

function stroke(...points: [number, number][]): Stroke {
  return points.map(([x, y], index) => ({ x, y, t: index }));
}

describe("inkBounds", () => {
  it("returns the ink's own box, ignoring where it sits", () => {
    const bounds = inkBounds([
      [
        { x: 40, y: 20, t: 0 },
        { x: 120, y: 70, t: 1 },
      ],
    ]);
    expect(bounds).toEqual({ minX: 40, minY: 20, maxX: 120, maxY: 70 });
  });

  it("is null for an empty pad", () => {
    expect(inkBounds([])).toBeNull();
    expect(inkBounds([[]])).toBeNull();
  });

  it("ignores non-finite samples instead of poisoning every bound", () => {
    const bounds = inkBounds([
      [
        { x: 10, y: 10, t: 0 },
        { x: Number.NaN, y: 40, t: 1 },
        { x: 30, y: 90, t: 2 },
      ],
    ]);
    expect(bounds).toEqual({ minX: 10, minY: 10, maxX: 30, maxY: 90 });
  });
});

describe("hasInk", () => {
  it("is false while the pad is untouched and true once a stroke lands", () => {
    expect(hasInk([])).toBe(false);
    expect(hasInk([stroke([0, 0], [10, 10])])).toBe(true);
  });

  it("is true for a single tap, because a tap is a way to sign", () => {
    expect(hasInk([[{ x: 12, y: 30, t: 0 }]])).toBe(true);
  });
});

describe("fitTransform", () => {
  it("preserves aspect rather than stretching a signature", () => {
    const bounds = { minX: 0, minY: 0, maxX: 200, maxY: 100 };
    const fit = fitTransform(bounds, { ...TRIM_BOX, width: 400, height: 400, padding: 0 });
    // A 2:1 mark in a 1:1 box must scale by the tighter axis, not both.
    expect(fit.scale).toBe(2);
  });

  it("centres the ink inside the box", () => {
    // A wide, short mark in a square box.
    const bounds = { minX: 0, minY: 0, maxX: 360, maxY: 40 };
    const options = { ...TRIM_BOX, width: 400, height: 400, padding: 0 };
    const fit = fitTransform(bounds, options);
    const topLeft = toCanvasPoint({ x: bounds.minX, y: bounds.minY, t: 0 }, fit);
    const bottomRight = toCanvasPoint({ x: bounds.maxX, y: bounds.maxY, t: 0 }, fit);
    const inkW = bottomRight.x - topLeft.x;
    const inkH = bottomRight.y - topLeft.y;
    expect(topLeft.x).toBeCloseTo((400 - inkW) / 2, 6);
    expect(topLeft.y).toBeCloseTo((400 - inkH) / 2, 6);
  });

  it("leaves the configured padding clear on the tighter axis", () => {
    const bounds = { minX: 0, minY: 0, maxX: 100, maxY: 100 };
    const padding = 24;
    const fit = fitTransform(bounds, { ...TRIM_BOX, padding });
    const start = toCanvasPoint({ x: 0, y: 0, t: 0 }, fit);
    const end = toCanvasPoint({ x: 100, y: 100, t: 0 }, fit);
    // The box is 1024x320, so the HEIGHT binds for a square mark: the ink
    // spans exactly the height minus both paddings, and the slack sits on the
    // width. Asserting the binding axis is what makes this a padding test
    // rather than a coincidence.
    expect(end.y - start.y).toBeCloseTo(TRIM_BOX.height - padding * 2, 6);
    expect(start.y).toBeCloseTo(padding, 6);
    expect(end.x - start.x).toBeCloseTo(TRIM_BOX.height - padding * 2, 6);
  });

  it("maps the same pad point to the same pixel for identical input", () => {
    const bounds = { minX: 5, minY: 5, maxX: 95, maxY: 55 };
    const a = fitTransform(bounds, TRIM_BOX);
    const b = fitTransform(bounds, TRIM_BOX);
    expect(toCanvasPoint({ x: 50, y: 50, t: 0 }, a)).toEqual(
      toCanvasPoint({ x: 50, y: 50, t: 0 }, b),
    );
  });
});

describe("the rasterisation contract both platforms honour", () => {
  it("is a wide box, since a signature reads horizontally", () => {
    expect(DEFAULT_RASTER.width).toBeGreaterThan(DEFAULT_RASTER.height);
  });

  it("uses the pack's ink colour, not a browser default", () => {
    // docs/theme.md: dusk ink #2B2A33 is the text colour and the light
    // primary. The pack draws the stroke in the theme's text colour.
    expect(DEFAULT_RASTER.inkColor).toBe("#2B2A33");
  });
});

describe("pointCount and describeMark", () => {
  it("counts every sample across strokes", () => {
    expect(pointCount([stroke([0, 0], [1, 1]), stroke([2, 2], [3, 3], [4, 4])])).toBe(5);
  });

  it("describes a mark a person can read in a log line", () => {
    expect(describeMark([stroke([0, 0], [100, 50])])).toBe("2 points, 1 strokes, ink 100x50");
  });

  it("names an empty pad as no ink rather than printing zeros", () => {
    expect(describeMark([])).toBe("no ink");
  });
});
