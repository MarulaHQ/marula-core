// Rasterises a captured mark to PNG bytes.
//
// Purpose: the only place the drawn mark becomes the payload that rides the
// content-addressed store. Uses the browser's own canvas encoder so the
// bytes are a real, magic-byte-valid PNG rather than a bespoke encoder.
//
// Non-purpose: no stroke storage — the PNG IS the payload. The vector stays
// in component state for undo/clear and is dropped once captured, because a
// second copy of the mark is a second thing to keep consistent.

import {
  fitTransform,
  hasInk,
  inkBounds,
  toCanvasPoint,
  type RasterTarget,
  type Stroke,
} from "./stroke";

/**
 * Draws the trimmed ink onto a canvas at `target`'s size and returns the
 * raw PNG bytes (not the data URL) so the caller can hash and upload.
 *
 * Ink is drawn with round caps and joins so a slow, shaky stroke renders as
 * a continuous mark rather than a chain of separate segments.
 */
export async function rasterise(
  strokes: readonly Stroke[],
  target: RasterTarget,
): Promise<Uint8Array> {
  if (!hasInk(strokes)) throw new Error("nothing to rasterise");
  const bounds = inkBounds(strokes);
  if (bounds === null) throw new Error("nothing to rasterise");

  const canvas = document.createElement("canvas");
  canvas.width = target.width;
  canvas.height = target.height;
  const ctx = canvas.getContext("2d");
  if (ctx === null) throw new Error("a 2d canvas is required to rasterise a signature");

  // Transparent, not the surface: the payload is ink, and where it lands is
  // the renderer's business, not the capture's.
  ctx.clearRect(0, 0, target.width, target.height);
  const fit = fitTransform(bounds, target);

  ctx.strokeStyle = target.inkColor;
  ctx.fillStyle = target.inkColor;
  ctx.lineWidth = target.lineWidth;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";

  for (const stroke of strokes) {
    if (stroke.length === 0) continue;
    if (stroke.length === 1) {
      // A tap is a dot, not an invisible point.
      const p = toCanvasPoint(stroke[0]!, fit);
      ctx.beginPath();
      ctx.arc(p.x, p.y, target.lineWidth / 2, 0, Math.PI * 2);
      ctx.fill();
      continue;
    }
    ctx.beginPath();
    const first = toCanvasPoint(stroke[0]!, fit);
    ctx.moveTo(first.x, first.y);
    for (let i = 1; i < stroke.length; i += 1) {
      const p = toCanvasPoint(stroke[i]!, fit);
      ctx.lineTo(p.x, p.y);
    }
    ctx.stroke();
  }

  return new Promise<Uint8Array>((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (blob === null) {
        reject(new Error("the browser refused to encode the signature image"));
        return;
      }
      blob.arrayBuffer().then(
        (buffer) => resolve(new Uint8Array(buffer)),
        (err: unknown) => reject(err instanceof Error ? err : new Error(String(err))),
      );
    }, "image/png");
  });
}

/** Base64 for the JSON body — the upload route takes one string field. */
export function toBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

/** Convenience: the whole mark to a base64 PNG in one call. */
export async function rasteriseToBase64(
  strokes: readonly Stroke[],
  target: RasterTarget,
): Promise<string> {
  return toBase64(await rasterise(strokes, target));
}
