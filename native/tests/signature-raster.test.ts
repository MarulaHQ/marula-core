// Tests for the dependency-free rasteriser that turns a drawn mark into the
// pixel buffer the PNG encoder consumes.
//
// The invariants that matter: a fast stroke is a continuous line (the brush
// is stamped along the segment, not only at the samples), the ink's own
// bounds are used (whitespace is not the record), and aspect survives.

import { describe, expect, it } from "vitest";

import { DEFAULT_RASTER, describeMark, hasInk, inkBounds, rasterise, type RasterBox } from "../src/signature/raster";
import type { Stroke } from "../src/signature/stroke";

function stroke(...points: [number, number][]): Stroke {
  return points.map(([x, y], index) => ({ x, y, t: index }));
}

/** A small box so the assertions stay readable. */
const BOX: RasterBox = { width: 64, height: 64, padding: 4, radius: 2, inkIndex: 1 };

describe("inkBounds", () => {
  it("is null for an empty pad", () => {
    expect(inkBounds([])).toBeNull();
    expect(inkBounds([[]])).toBeNull();
  });

  // A single sample is a TAP, not noise: it has bounds that collapse to a
  // point, and the rasteriser draws it as a dot.
  it("reports a collapsed box for a single sample rather than treating it as ink-less", () => {
    expect(inkBounds([[{ x: 4, y: 4, t: 0 }]])).toEqual({ minX: 4, minY: 4, maxX: 4, maxY: 4 });
  });

  it("reports the ink's own box", () => {
    expect(inkBounds([stroke([10, 20], [60, 30])])).toEqual({
      minX: 10,
      minY: 20,
      maxX: 60,
      maxY: 30,
    });
  });
});

describe("hasInk", () => {
  it("is false until a stroke exists", () => {
    expect(hasInk([])).toBe(false);
    expect(hasInk([stroke([0, 0], [5, 5])])).toBe(true);
  });
});

describe("rasterise", () => {
  it("throws when there is nothing to draw, so a blank pad cannot upload", () => {
    expect(() => rasterise([], BOX)).toThrow(/nothing to rasterise/);
  });

  it("produces a buffer of exactly width × height", () => {
    const pixels = rasterise([stroke([10, 10], [40, 40])], BOX);
    expect(pixels.length).toBe(BOX.width * BOX.height);
  });

  it("marks more than the two endpoints for a two-point stroke", () => {
    const pixels = rasterise([stroke([10, 10], [50, 10])], BOX);
    const lit = [...pixels].filter((v) => v === 1).length;
    // A 40px gap stamped every ~0.75px is far more than two dots.
    expect(lit).toBeGreaterThan(40);
  });

  it("keeps a fast stroke continuous, with no unlit gap across its own span", () => {
    // Two samples 200 pad-pixels apart: without segment stamping this would
    // rasterise as two unrelated dots and the signature would be invisible.
    const wide: RasterBox = { ...BOX, width: 256, height: 64, padding: 8 };
    const pixels = rasterise([stroke([8, 32], [248, 32])], wide);
    let firstLit = -1;
    let lastLit = -1;
    for (let x = 0; x < wide.width; x += 1) {
      const column = pixels[32 * wide.width + x] === 1;
      if (column) {
        if (firstLit === -1) firstLit = x;
        lastLit = x;
      }
    }
    expect(firstLit).toBeGreaterThanOrEqual(0);
    // Every column between the first and last lit column must be lit, or
    // the mark has a hole in the middle of itself.
    for (let x = firstLit; x <= lastLit; x += 1) {
      expect(pixels[32 * wide.width + x]).toBe(1);
    }
  });

  it("respects the configured padding by keeping the ink inside it", () => {
    // A dense zig-zag that clearly exceeds the box interior.
    const points: [number, number][] = [];
    for (let i = 0; i <= 40; i += 1) points.push([i * 4, i % 2 === 0 ? 0 : 40]);
    const pixels = rasterise([stroke(...points)], BOX);
    let minX = Infinity;
    let maxX = -Infinity;
    for (let y = 0; y < BOX.height; y += 1) {
      for (let x = 0; x < BOX.width; x += 1) {
        if (pixels[y * BOX.width + x] === 1) {
          if (x < minX) minX = x;
          if (x > maxX) maxX = x;
        }
      }
    }
    expect(minX).toBeGreaterThanOrEqual(BOX.padding - 2);
    expect(maxX).toBeLessThanOrEqual(BOX.width - BOX.padding + 2);
  });

  it("keeps a short stroke's ink inside a horizontal band rather than filling the box", () => {
    const pixels = rasterise([stroke([20, 32], [34, 32])], BOX);
    // A horizontal stroke lights up rows, not the whole canvas: count rows.
    let litRows = 0;
    for (let y = 0; y < BOX.height; y += 1) {
      for (let x = 0; x < BOX.width; x += 1) {
        if (pixels[y * BOX.width + x] === 1) {
          litRows += 1;
          break;
        }
      }
    }
    expect(litRows).toBeGreaterThan(0);
    // The brush is a few rows tall at this scale, nowhere near the full box.
    expect(litRows).toBeLessThan(BOX.height * 0.5);
  });

  it("draws a visible dot for a lone tap, because a tap is a way to sign", () => {
    // Returning "nothing to rasterise" here once made the pad refuse a
    // capture the customer had already made.
    const pixels = rasterise([[{ x: 32, y: 32, t: 0 }]], BOX);
    expect([...pixels].filter((v) => v === 1).length).toBeGreaterThan(0);
  });

  it("uses the canonical trim box for the real payload", () => {
    // The shape the server receives: a wide box, matching the web's own.
    expect(DEFAULT_RASTER.width).toBe(1024);
    expect(DEFAULT_RASTER.height).toBe(320);
    expect(DEFAULT_RASTER.width).toBeGreaterThan(DEFAULT_RASTER.height);
  });
});

describe("describeMark", () => {
  it("names an empty pad as no ink", () => {
    expect(describeMark([])).toBe("no ink");
  });

  it("counts the samples and strokes", () => {
    expect(describeMark([stroke([0, 0], [10, 0], [20, 0])])).toBe("3 points, 1 strokes, ink 20x0");
  });

  it("names a single tap as a point rather than a zero-extent box", () => {
    expect(describeMark([[{ x: 4, y: 8, t: 0 }]])).toBe("1 points, 1 strokes, a single point");
  });
});
