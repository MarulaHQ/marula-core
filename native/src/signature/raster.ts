// Rasterises a captured mark into the pixel buffer the PNG encoder consumes.
//
// Purpose: the native half of the same contract the web honours — strokes in,
// trimmed PNG bytes out — implemented without a canvas because React Native
// has none. The geometry (trim to the ink's own bounds, preserve aspect,
// centre in the box) is deliberately IDENTICAL to the web's
// `apps/web/lib/signature/stroke.ts` so the two platforms rasterise the same
// mark to the same shape.
//
// Non-purpose: no anti-aliasing. A bilevel image has no partial pixels, so a
// rasterised edge is jagged. That is the correct trade for a signature paid
// for in offline-queue bytes: the mark is unmistakable at any zoom a
// signature is read at, and the payload stays small enough to hold on a
// device with no signal.

import type { Stroke } from "./stroke";

export interface RasterBox {
  width: number;
  height: number;
  /** Blank pixels kept around the ink. */
  padding: number;
  /** Brush radius in output pixels. */
  radius: number;
  /** Palette index written as ink. */
  inkIndex: 0 | 1;
}

export const DEFAULT_RASTER: RasterBox = {
  width: 1024,
  height: 320,
  padding: 24,
  radius: 5,
  inkIndex: 1,
};

interface Bounds {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

/**
 * The ink's own bounds, or null when there is nothing to draw.
 *
 * A LONE POINT IS A MARK, not a degenerate case: a customer who taps the pad
 * once has signed, and the rasteriser must draw that dot. Returning null for
 * it here once made `rasterise` throw "nothing to rasterise" on a single tap,
 * which is a capture the surface refused for no reason.
 */
export function inkBounds(strokes: readonly Stroke[]): Bounds | null {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  let seen = 0;
  for (const stroke of strokes) {
    for (const p of stroke) {
      if (!Number.isFinite(p.x) || !Number.isFinite(p.y)) continue;
      if (p.x < minX) minX = p.x;
      if (p.y < minY) minY = p.y;
      if (p.x > maxX) maxX = p.x;
      if (p.y > maxY) maxY = p.y;
      seen += 1;
    }
  }
  if (seen === 0) return null;
  return { minX, minY, maxX, maxY };
}

/** True when at least one stroke carries a point. */
export function hasInk(strokes: readonly Stroke[]): boolean {
  return strokes.some((stroke) => stroke.some((p) => Number.isFinite(p.x) && Number.isFinite(p.y)));
}

export function pointCount(strokes: readonly Stroke[]): number {
  return strokes.reduce((sum, stroke) => sum + stroke.length, 0);
}

/** Same shape as the web's `describeMark` — a person-readable log line. */
export function describeMark(strokes: readonly Stroke[]): string {
  const bounds = inkBounds(strokes);
  if (bounds === null) return "no ink";
  const w = Math.round(bounds.maxX - bounds.minX);
  const h = Math.round(bounds.maxY - bounds.minY);
  const shape = w === 0 && h === 0 ? "a single point" : `ink ${w}x${h}`;
  return `${pointCount(strokes)} points, ${strokes.length} strokes, ${shape}`;
}

/** Draws a filled disc, clipped to the buffer. */
function disc(
  pixels: Uint8Array,
  width: number,
  height: number,
  cx: number,
  cy: number,
  radius: number,
  inkIndex: 0 | 1,
): void {
  const x0 = Math.max(Math.floor(cx - radius), 0);
  const x1 = Math.min(Math.ceil(cx + radius), width - 1);
  const y0 = Math.max(Math.floor(cy - radius), 0);
  const y1 = Math.min(Math.ceil(cy + radius), height - 1);
  const r2 = radius * radius;
  for (let y = y0; y <= y1; y += 1) {
    for (let x = x0; x <= x1; x += 1) {
      const dx = x + 0.5 - cx;
      const dy = y + 0.5 - cy;
      if (dx * dx + dy * dy <= r2) pixels[y * width + x] = inkIndex;
    }
  }
}

/**
 * Consecutive samples can be arbitrarily far apart on a fast stroke, so the
 * brush is stamped along the segment rather than only at the samples —
 * otherwise a quick flick rasterises as a dotted line of invisible dots.
 */
function segment(
  pixels: Uint8Array,
  width: number,
  height: number,
  x0: number,
  y0: number,
  x1: number,
  y1: number,
  radius: number,
  inkIndex: 0 | 1,
): void {
  const dx = x1 - x0;
  const dy = y1 - y0;
  const distance = Math.hypot(dx, dy);
  const step = Math.max(radius / 3, 0.75);
  const steps = Math.max(1, Math.ceil(distance / step));
  for (let i = 0; i <= steps; i += 1) {
    const t = i / steps;
    disc(pixels, width, height, x0 + dx * t, y0 + dy * t, radius, inkIndex);
  }
}

/**
 * Maps the ink into `box`, preserving aspect and centring — the same rule as
 * the web's `fitTransform`, so the two platforms agree about the mark.
 */
export function rasterise(strokes: readonly Stroke[], box: RasterBox): Uint8Array {
  const bounds = inkBounds(strokes);
  if (bounds === null) throw new Error("nothing to rasterise");
  const { width, height, padding, radius, inkIndex } = box;

  const availableW = Math.max(width - padding * 2, 1);
  const availableH = Math.max(height - padding * 2, 1);
  // A single point has no extent to scale against, so give it a nominal one.
  // Without this the fit transform would drive the dot off the canvas.
  const inkW = Math.max(bounds.maxX - bounds.minX, 1);
  const inkH = Math.max(bounds.maxY - bounds.minY, 1);
  const scale = Math.min(availableW / inkW, availableH / inkH);
  const offsetX = (width - inkW * scale) / 2 - bounds.minX * scale;
  const offsetY = (height - inkH * scale) / 2 - bounds.minY * scale;

  const pixels = new Uint8Array(width * height);
  const at = (p: { x: number; y: number }) => ({
    x: p.x * scale + offsetX,
    y: p.y * scale + offsetY,
  });

  for (const stroke of strokes) {
    if (stroke.length === 0) continue;
    let prev = at(stroke[0]!);
    disc(pixels, width, height, prev.x, prev.y, radius, inkIndex);
    for (let i = 1; i < stroke.length; i += 1) {
      const next = at(stroke[i]!);
      segment(pixels, width, height, prev.x, prev.y, next.x, next.y, radius, inkIndex);
      prev = next;
    }
  }
  return pixels;
}
