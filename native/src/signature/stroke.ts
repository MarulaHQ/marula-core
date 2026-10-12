// The captured mark's data shape, shared by the native pad and its rasteriser.
//
// Purpose: one list of strokes of timestamped points. Deliberately the same
// shape the web pad keeps (`apps/web/lib/signature/stroke.ts`) so a mark
// captured on one platform rasterises identically on the other — the trim
// box, aspect rule and centring all match, and only the drawing primitive
// differs (canvas on web, SVG paths here).
//
// Non-purpose: no persistence. A finished mark becomes PNG bytes and is
// submitted; the vector lives in component state for undo/clear and is
// dropped on submit, because two copies of the same mark is two things to
// keep consistent.

/** One sample of one stroke. `t` is the device clock in ms — evidence about
 *  the device, not a trusted server timestamp. */
export interface StrokePoint {
  x: number;
  y: number;
  t: number;
}

/** One pen-down-to-pen-up gesture. */
export type Stroke = readonly StrokePoint[];

export const BASE_LINE_WIDTH = 3;
