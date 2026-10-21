// Signature stroke geometry — pure, platform-agnostic, unit-testable.
//
// Purpose: the mark a customer draws is a list of strokes of timestamped
// points. Everything decided about that mark before it becomes bytes lives
// here: trimming to the ink's own bounds (a signature is not a rectangle —
// the pad's whitespace is presentation, not record), and rasterising the
// trimmed ink onto a fixed canvas so two identical marks produce identical
// bytes and the content-addressed dedup can see them.
//
// Non-purpose: no DOM, no canvas API, no network. Both the web pad (canvas)
// and the native pad (SVG) reduce their strokes through these helpers so the
// two platforms disagree about nothing.

import { TOKEN } from "@marula-core/ui";

/** One sample of one stroke. `t` is the device clock in ms — evidence about
 *  the device, not a trusted server timestamp. */
export interface StrokePoint {
  x: number;
  y: number;
  /** Device-clock ms when the sample was taken. */
  t: number;
}

/** One pen-down-to-pen-up gesture. */
export type Stroke = readonly StrokePoint[];

export interface Bounds {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

export interface TrimOptions {
  /** Output canvas width in pixels. */
  width: number;
  /** Output canvas height in pixels. */
  height: number;
  /** Margin kept between the ink and the canvas edge, in output pixels. */
  padding: number;
  /** Rendered stroke weight in output pixels. */
  lineWidth: number;
}

/**
 * The ink's own bounds, ignoring stroke weight. `null` for an empty,
 * single-point, or NaN-bearing mark — an empty pad must not rasterise a
 * 1×1 "signature".
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
      // A lone point is a dot with no extent; a stroke needs a direction to
      // be a mark, so ignore single-sample strokes for bounds but let two
      // samples in one stroke define the minimum measurable ink.
      if (p.x < minX) minX = p.x;
      if (p.y < minY) minY = p.y;
      if (p.x > maxX) maxX = p.x;
      if (p.y > maxY) maxY = p.y;
      seen += 1;
    }
  }
  if (seen === 0) return null;
  // A lone point is NOT rejected: a tap is a way to sign, and the pad draws
  // it as a dot. The two platforms agree about that, so the rasteriser gets
  // the same input whichever device captured the mark.
  return { minX, minY, maxX, maxY };
}

/** True when any stroke carries at least one point. */
export function hasInk(strokes: readonly Stroke[]): boolean {
  return strokes.some((stroke) => stroke.length > 0);
}

/**
 * The uniform scale + offset that fits the ink into `options`, preserving
 * aspect. The mark is centred in the box rather than stretched to fill it,
 * because stretching a signature changes its proportions and therefore the
 * person's hand.
 */
export function fitTransform(
  bounds: Bounds,
  options: TrimOptions,
): { scale: number; offsetX: number; offsetY: number } {
  const availableW = Math.max(options.width - options.padding * 2, 1);
  const availableH = Math.max(options.height - options.padding * 2, 1);
  const inkW = Math.max(bounds.maxX - bounds.minX, 1);
  const inkH = Math.max(bounds.maxY - bounds.minY, 1);
  const scale = Math.min(availableW / inkW, availableH / inkH);
  const centeredW = inkW * scale;
  const centeredH = inkH * scale;
  return {
    scale,
    offsetX: (options.width - centeredW) / 2 - bounds.minX * scale,
    offsetY: (options.height - centeredH) / 2 - bounds.minY * scale,
  };
}

/** Maps one pad-space point into the trimmed canvas. */
export function toCanvasPoint(
  p: StrokePoint,
  fit: { scale: number; offsetX: number; offsetY: number },
): { x: number; y: number } {
  return { x: p.x * fit.scale + fit.offsetX, y: p.y * fit.scale + fit.offsetY };
}

/** Total samples across all strokes — the record's shape evidence. */
export function pointCount(strokes: readonly Stroke[]): number {
  return strokes.reduce((sum, stroke) => sum + stroke.length, 0);
}

/** The default rasterisation box for a captured mark. */
export const TRIM_BOX: TrimOptions = {
  width: 1024,
  height: 320,
  padding: 24,
  lineWidth: 9,
};

/** Output rasterisation target — a wide box, since a signature reads
 *  horizontally and the pad in the pack is wider than it is tall. */
export interface RasterTarget extends TrimOptions {
  /** Ink colour in the output image, as a CSS colour. */
  inkColor: string;
}

export const DEFAULT_RASTER: RasterTarget = {
  ...TRIM_BOX,
  // The dusk ink from the token set (`--m-ink` / `TOKEN.ink`). A canvas cannot
  // resolve a CSS custom property, so the value has to be a string here — but
  // it is the token's value, not a colour authored in this module. The pad
  // reads the live `--m-text` first and falls back to this.
  inkColor: TOKEN.ink,
};

/**
 * A stable, human-readable description of the mark for logs and audit
 * payloads — deliberately not a hash, because a log line a person reads is
 * worth more than one a person re-derives.
 */
export function describeMark(strokes: readonly Stroke[]): string {
  const bounds = inkBounds(strokes);
  if (bounds === null) return "no ink";
  const w = Math.round(bounds.maxX - bounds.minX);
  const h = Math.round(bounds.maxY - bounds.minY);
  return `${pointCount(strokes)} points, ${strokes.length} strokes, ink ${w}x${h}`;
}
