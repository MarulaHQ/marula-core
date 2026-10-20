"use client";

// Signature pad — the drawing surface for a customer sign-off.
//
// Geometry follows the Marula pack, not a generic canvas widget:
// `internal/design/mockups/marula-app-mockup-images/mobile/{light,dark}/states/
// 08-wrap-up-resolved.png` and `11-wrap-up-signed.png` draw a section
// labelled "Client sign-off", a DASHED rounded box, the placeholder
// "Sign here" at the bottom-LEFT, and "Clear" at the bottom-RIGHT, with the
// stroke in the theme's text colour. There is no ruled baseline in the pack —
// the placeholder is the affordance — so this component does not invent one.
//
// Craft rules honoured (docs/theme.md §5, apple-design): motion is
// transform/opacity only and ≤120ms; the pad is a labelled region, not a
// bare canvas, so a screen reader announces what to do; a typed name is an
// equal path to the same record for anyone who cannot draw.
//
// Pressure is DROPPED, deliberately and for a reason: PointerEvent.pressure
// is only meaningful for `pointerType === "pen"`, and a mouse reports a
// constant 0.5 that would render every desktop signature thinner than the
// signer intended. One weight for all inputs is honest; a per-input width
// that is wrong on the most common input is not.

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import {
  DEFAULT_RASTER,
  describeMark,
  hasInk,
  type Stroke,
  type StrokePoint,
} from "../lib/signature/stroke";
import { rasteriseToBase64 } from "../lib/signature/raster";

export type { Stroke, StrokePoint };

/** The pen weight used everywhere, in CSS pixels. */
const STROKE_WIDTH = 3;

export interface CapturePayload {
  /** Base64 PNG, or "" for a typed-name agreement. */
  imageBase64: string;
  /** Who signed, as they typed it. Always present — a drawn mark identifies
   *  nobody on its own, so the name is half the record. */
  signerName: string;
}

export interface SignaturePadProps {
  /** Called once the signer confirms, with the mark and the typed name. */
  onCapture: (payload: CapturePayload) => void | Promise<void>;
  /** The consent wording the signer is agreeing to. */
  consentText: string;
  /** Consent version shown to the signer; captured with the row. */
  consentVersion: number;
  /** Disables capture (consent still loading, for instance). */
  disabled?: boolean;
  busy?: boolean;
}

/** Resolves a CSS custom property to the colour the browser will actually
 *  paint. A canvas context has no idea what `var(--m-text)` means, so the
 *  token has to be read off an element before it can become ink. */
function tokenOf(host: HTMLElement | null, token: string, fallback: string): string {
  if (host === null) return fallback;
  const value = getComputedStyle(host).getPropertyValue(token).trim();
  return value === "" ? fallback : value;
}

export function SignaturePad({
  onCapture,
  consentText,
  consentVersion,
  disabled = false,
  busy = false,
}: SignaturePadProps): ReactNode {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const strokesRef = useRef<Stroke[]>([]);
  const liveRef = useRef<StrokePoint[]>([]);
  const enabledRef = useRef(!disabled);
  /** Ink colour resolved off the live DOM so a theme swap repaints. */
  const inkRef = useRef<string>(DEFAULT_RASTER.inkColor);
  const [strokes, setStrokes] = useState<Stroke[]>([]);
  const [typedName, setTypedName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [capturing, setCapturing] = useState(false);

  const hasMark = useMemo(() => hasInk(strokes), [strokes]);
  const typedMode = typedName.trim().length > 0;

  const resolveInk = useCallback((): string => {
    const ink = tokenOf(canvasRef.current, "--m-text", DEFAULT_RASTER.inkColor);
    inkRef.current = ink;
    return ink;
  }, []);

  // Repaint when the theme flips: the resolved ink is re-read and the stroke
  // colour changes with it, which is the pack's own rule.
  useEffect(() => {
    resolveInk();
  }, [resolveInk]);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (canvas === null) return;
    const rect = canvas.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = Math.max(Math.round(rect.width * dpr), 1);
    const h = Math.max(Math.round(rect.height * dpr), 1);
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
    }
    const ctx = canvas.getContext("2d");
    if (ctx === null) return;
    const scale = w / rect.width;
    ctx.setTransform(scale, 0, 0, scale, 0, 0);
    ctx.clearRect(0, 0, rect.width, rect.height);
    ctx.strokeStyle = inkRef.current;
    ctx.lineWidth = STROKE_WIDTH;
    ctx.lineCap = "round";
    ctx.lineJoin = "round";

    const all = strokesRef.current;
    for (const stroke of all) {
      if (stroke.length === 0) continue;
      const first = stroke[0]!;
      ctx.beginPath();
      ctx.moveTo(first.x, first.y);
      if (stroke.length === 1) {
        ctx.lineTo(first.x, first.y);
      } else {
        for (let i = 1; i < stroke.length; i += 1) {
          const p = stroke[i]!;
          ctx.lineTo(p.x, p.y);
        }
      }
      ctx.stroke();
    }
  }, []);

  const redraw = useCallback(() => {
    resolveInk();
    draw();
  }, [draw, resolveInk]);

  useEffect(() => {
    redraw();
  }, [redraw, strokes]);

  useEffect(() => {
    const onResize = (): void => redraw();
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [redraw]);

  useEffect(() => {
    enabledRef.current = !disabled;
  }, [disabled]);

  const localPoint = useCallback(
    (event: ReactPointerEvent<HTMLCanvasElement>): StrokePoint => ({
      x: event.clientX - event.currentTarget.getBoundingClientRect().left,
      y: event.clientY - event.currentTarget.getBoundingClientRect().top,
      t: event.timeStamp,
    }),
    [],
  );

  const handleDown = useCallback(
    (event: ReactPointerEvent<HTMLCanvasElement>) => {
      if (!enabledRef.current || busy) return;
      event.currentTarget.setPointerCapture(event.pointerId);
      liveRef.current = [localPoint(event)];
      strokesRef.current = [...strokesRef.current, liveRef.current];
      setStrokes([...strokesRef.current]);
      setError(null);
    },
    [busy, localPoint],
  );

  const handleMove = useCallback(
    (event: ReactPointerEvent<HTMLCanvasElement>) => {
      const live = liveRef.current;
      if (live.length === 0) return;
      // Coalesced samples when the device supplies them: a fast stroke
      // becomes a smooth curve, not a polygon of a handful of vertices.
      const native = event.nativeEvent;
      const coalesced =
        typeof native.getCoalescedEvents === "function" ? native.getCoalescedEvents() : [];
      const rect = event.currentTarget.getBoundingClientRect();
      if (coalesced.length > 0) {
        for (const ev of coalesced) {
          live.push({
            x: ev.clientX - rect.left,
            y: ev.clientY - rect.top,
            t: ev.timeStamp,
          });
        }
      } else {
        live.push(localPoint(event));
      }
      draw();
    },
    [draw, localPoint],
  );

  const endStroke = useCallback(
    (event: ReactPointerEvent<HTMLCanvasElement>) => {
      if (liveRef.current.length === 0) return;
      const canvas = event.currentTarget;
      if (canvas.hasPointerCapture(event.pointerId)) {
        canvas.releasePointerCapture(event.pointerId);
      }
      // Replace the in-progress stroke with its finished self so the
      // committed list is exactly what was drawn.
      const committed = liveRef.current;
      liveRef.current = [];
      strokesRef.current = [...strokesRef.current.slice(0, -1), committed];
      setStrokes([...strokesRef.current]);
    },
    [],
  );

  const handleLeave = useCallback(
    (event: ReactPointerEvent<HTMLCanvasElement>) => {
      if (liveRef.current.length === 0) return;
      const canvas = event.currentTarget;
      if (canvas.hasPointerCapture(event.pointerId)) {
        canvas.releasePointerCapture(event.pointerId);
      }
      liveRef.current = [];
      setStrokes([...strokesRef.current.slice(0, -1)]);
    },
    [],
  );

  const clear = useCallback(() => {
    strokesRef.current = [];
    liveRef.current = [];
    setStrokes([]);
    setError(null);
    redraw();
  }, [redraw]);

  /** Removes the most recent stroke. Per-stroke, not per-point: "undo" on a
   *  drawing surface means "take back that gesture", and a customer who
   *  wants a clean pad presses Clear. */
  const undo = useCallback(() => {
    const next = strokesRef.current.slice(0, -1);
    strokesRef.current = next;
    liveRef.current = [];
    setStrokes([...next]);
    setError(null);
  }, []);

  const capture = useCallback(async () => {
    setError(null);
    setCapturing(true);
    try {
      const typed = typedName.trim();
      if (typedMode) {
        // A typed agreement is the SAME record without the ink: the name the
        // customer typed IS the sign-off, which is why `image_base64` is
        // empty and the server still requires a name.
        await onCapture({ imageBase64: "", signerName: typed });
        return;
      }
      if (!hasInk(strokesRef.current)) {
        setError("The customer needs to sign, or type their name to agree instead.");
        return;
      }
      if (typed === "") {
        setError("Type the customer's name under the signature so the record says who signed.");
        return;
      }
      const b64 = await rasteriseToBase64(strokesRef.current, {
        ...DEFAULT_RASTER,
        inkColor: inkRef.current,
      });
      await onCapture({ imageBase64: b64, signerName: typed });
    } catch (err) {
      setError(err instanceof Error ? err.message : "The signature could not be captured.");
    } finally {
      setCapturing(false);
    }
  }, [onCapture, typedMode, typedName]);

  const onTypedKeyDown = useCallback(
    (event: KeyboardEvent<HTMLInputElement>) => {
      if (event.key !== "Enter") return;
      event.preventDefault();
      void capture();
    },
    [capture],
  );

  const description = useMemo(() => describeMark(strokes), [strokes]);
  const idle = !hasMark && !typedMode;

  return (
    <section aria-labelledby="sign-off-heading" data-testid="signature-pad">
      <div
        className="mb-1.5 text-sm font-bold"
        id="sign-off-heading"
        style={{ color: "var(--m-text)" }}
      >
        Client sign-off
      </div>

      <div
        className="relative rounded-xl border border-dashed"
        style={{ borderColor: "var(--m-border)", background: "var(--m-bg-subtle)" }}
      >
        <canvas
          ref={canvasRef}
          role="img"
          aria-label={
            hasMark
              ? `Signature drawn: ${description}. Undo takes back the last stroke.`
              : "Empty signature pad. The customer draws here with a finger, stylus or mouse."
          }
          data-testid="signature-canvas"
          className={`block h-33 w-full touch-none rounded-xl ${disabled || busy ? "cursor-default" : "cursor-crosshair"}`}
          onPointerDown={handleDown}
          onPointerMove={handleMove}
          onPointerUp={endStroke}
          onPointerLeave={handleLeave}
          onPointerCancel={handleLeave}
        />
        {!hasMark && (
          <span
            aria-hidden
            className="pointer-events-none absolute bottom-2 left-3 text-xs"
            style={{ color: "var(--m-text-muted)" }}
            data-testid="signature-placeholder"
          >
            Sign here
          </span>
        )}
        <button
          type="button"
          className="absolute bottom-2 right-2 cursor-pointer rounded border-none bg-transparent px-1.5 py-0.5 text-xs"
          style={{ color: "var(--m-text-muted)" }}
          onClick={clear}
          disabled={disabled || busy || !hasMark}
          data-testid="signature-clear"
        >
          Clear
        </button>
      </div>

      {strokes.length > 0 && (
        <button
          type="button"
          className="mt-1.5 cursor-pointer rounded border-none bg-transparent px-0 py-0.5 text-xs underline"
          style={{ color: "var(--m-text-muted)" }}
          onClick={undo}
          disabled={disabled || busy}
          data-testid="signature-undo"
        >
          Undo last stroke
        </button>
      )}

      <label
        className="mt-2 block text-xs"
        htmlFor="sign-typed-name"
        style={{ color: "var(--m-text-secondary)" }}
      >
        Customer&rsquo;s name
      </label>
      <input
        id="sign-typed-name"
        type="text"
        value={typedName}
        placeholder="Dana Whitfield"
        disabled={disabled || busy}
        onChange={(e) => setTypedName(e.target.value)}
        onKeyDown={onTypedKeyDown}
        data-testid="signature-typed-name"
        className="mt-1 w-full rounded border px-2 py-1.5 text-sm outline-none"
        style={{
          borderColor: "var(--m-border)",
          background: "var(--m-surface)",
          color: "var(--m-text)",
        }}
      />

      <p
        className="mt-1 text-xs"
        style={{ color: "var(--m-text-muted)" }}
        data-testid="signature-hint"
      >
        Typing a name and pressing Sign off records the same agreement without ink — for a customer who
        cannot draw.
      </p>

      <p
        className="mt-2 text-xs leading-relaxed"
        style={{ color: "var(--m-text-muted)" }}
        data-testid="signature-consent"
      >
        {consentText}
        <span className="ml-1 whitespace-nowrap opacity-70"> (v{consentVersion})</span>
      </p>

      <div className="mt-2 flex items-center gap-2">
        <button
          type="button"
          className="cursor-pointer rounded border px-2.5 py-1 text-xs font-semibold"
          style={{
            borderColor: idle ? "var(--m-border)" : "var(--m-accent)",
            background: idle ? "var(--m-bg-subtle)" : "var(--m-accent)",
            color: idle ? "var(--m-text-muted)" : "var(--m-accent-fg)",
          }}
          disabled={disabled || busy || idle}
          onClick={() => void capture()}
          data-testid="signature-confirm"
        >
          {capturing || busy ? "Signing…" : "Sign off"}
        </button>
        <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
          {capturing ? "Recording" : hasMark ? "Ready" : typedMode ? "Typed agreement" : "Waiting for ink"}
        </span>
      </div>

      {error !== null && (
        <p
          className="mt-1.5 text-xs"
          role="alert"
          style={{ color: "var(--m-status-danger-fg)" }}
          data-testid="signature-error"
        >
          {error}
        </p>
      )}
    </section>
  );
}
