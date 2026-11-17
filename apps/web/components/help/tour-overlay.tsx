// TourOverlay — the spotlight/highlight flow renderer (Wave 26 lane 2;
// step-to-step motion per qa-w71-tours RESEARCH.md §5a, user ruling
// 2026-11-17).
//
// Purpose: renders ONE declarative tour step: dimmed page, highlighted
// target, positioned tooltip card with Skip/Back/Next/Done. Motion obeys
// the law (docs/theme.md §5 and its tour carve-out): transform/opacity
// ONLY. Layout boxes (top/left/width/height) land on their new values in
// one frame and a transform carries the eye across the jump:
//  - step change: spotlight and card glide 240ms at the brand default
//    easing — 300ms when the jump exceeds 400px, never above 400ms. The
//    spotlight's layout box is set to the incoming rect instantly and a
//    translate+scale transform (origin 0 0) starts from the outgoing
//    rect's delta; the card re-anchors by pure translate in the same
//    window, so card and cutout move together.
//  - card copy: fades out to opacity 0 / 4px down over 150ms, content
//    swaps, fades in from the same offset over 150ms — overlapped with
//    the glide, one card element, two phases.
//  - entrance: the dim fade IS the spotlight's opacity (the dim is its
//    own box-shadow), 200ms; the card pops from scale 0.96 / 8px below
//    over 240ms with the brand spring easing.
//  - exit: Done / Skip tour / Escape / the global switch reverse the
//    entrance over 200ms BEFORE the parent callback unmounts the
//    overlay; the parent API is unchanged.
// Resize and scroll remeasures apply in one frame and cancel any glide
// in flight — a scroll during a tour never glides. Everything above
// collapses to instant under prefers-reduced-motion. The overlay is
// read-only about the page — it never mutates DOM outside its own root.

"use client";

import { Button } from "@marula-core/ui";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import type { Tour, TourStep } from "../../lib/tours/tours";

interface Rect {
  top: number;
  left: number;
  width: number;
  height: number;
}

const PAD = 8;

// Brand spec §7 easings; durations from the 2026-11-17 tour ruling.
const EASE_DEFAULT = "cubic-bezier(0.65, 0, 0.35, 1)";
const EASE_SPRING = "cubic-bezier(0.34, 1.56, 0.64, 1)";
const GLIDE_MS = 240;
const GLIDE_LONG_MS = 300;
const GLIDE_LONG_PX = 400;
const SWAP_MS = 150;
const SWAP_SHIFT_PX = 4;
const ENTER_DIM_MS = 200;
const ENTER_CARD_MS = 240;
const EXIT_MS = 200;

function measure(step: TourStep): Rect | null {
  const el = document.querySelector(step.target);
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return { top: r.top, left: r.left, width: r.width, height: r.height };
}

// Card anchor for a rect: prefers below the target; flips above when
// there is no room. Run for the outgoing rect too, so the card's glide
// delta is measured against where the card actually was.
function tipFor(rect: Rect): { top: number; left: number } {
  const below = rect.top + rect.height + 180 < window.innerHeight;
  const top = below ? rect.top + rect.height + PAD : Math.max(PAD, rect.top - 180 - PAD);
  const left = Math.min(
    Math.max(PAD, rect.left),
    window.innerWidth - 320 - PAD,
  );
  return { top, left };
}

export function TourOverlay({
  tour,
  stepIndex,
  onNext,
  onBack,
  onDone,
  onSkipTour,
  onDisableTours,
}: {
  tour: Tour;
  stepIndex: number;
  onNext(): void;
  onBack(): void;
  onDone(): void;
  onSkipTour?(): void;
  onDisableTours(): void;
}) {
  const step: TourStep | undefined = tour.steps[stepIndex];
  const [rect, setRect] = useState<Rect | null>(null);
  // Null until the matchMedia effect has read the OS preference, so no
  // animation ever plays on a guess.
  const [reducedMotion, setReducedMotion] = useState<boolean | null>(null);
  // Card copy swap: which step's copy is on screen, and which phase of
  // the out/in transition it is in. During "leaving" the OLD step's copy
  // is still rendered; the content swap happens at the midpoint.
  const [shownIndex, setShownIndex] = useState(stepIndex);
  const [swapPhase, setSwapPhase] = useState<"idle" | "leaving" | "entering">("idle");

  const spotRef = useRef<HTMLDivElement | null>(null);
  const cardRef = useRef<HTMLDivElement | null>(null);
  const rectRef = useRef<Rect | null>(null);
  // The outgoing rect, stashed when stepIndex changes and consumed by
  // the glide below. Null on mount and on remeasures — both apply in
  // one frame.
  const pendingGlideRef = useRef<Rect | null>(null);
  // In-flight enter/glide animations, so a remeasure can cancel them
  // and stay instant.
  const animsRef = useRef<Animation[]>([]);
  const enteredRef = useRef(false);
  const prevGlideStepRef = useRef<number | null>(null);
  const prevSwapStepRef = useRef<number | null>(null);
  const swapTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const exitTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const exitingRef = useRef(false);

  useEffect(() => {
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    setReducedMotion(mq.matches);
    const onChange = () => setReducedMotion(mq.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  // Measure the current step's target. A step change stashes the
  // outgoing rect for the glide; resize/scroll remeasures do NOT — they
  // must land in one frame, cancelling any glide in flight.
  useEffect(() => {
    if (!step) return;
    const changed =
      prevGlideStepRef.current !== null && prevGlideStepRef.current !== stepIndex;
    prevGlideStepRef.current = stepIndex;
    if (changed) pendingGlideRef.current = rectRef.current;
    const update = () => {
      const r = measure(step);
      if (!r) return;
      for (const a of animsRef.current) a.cancel();
      animsRef.current = [];
      rectRef.current = r;
      setRect(r);
    };
    update();
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [step, stepIndex]);

  // Card copy swap, two phases of SWAP_MS: out (old copy to opacity 0,
  // 4px down), swap, in (new copy from the same offset to identity).
  // Reduced or not-yet-measured motion swaps in one frame.
  useEffect(() => {
    if (reducedMotion !== false) {
      if (swapTimerRef.current) clearTimeout(swapTimerRef.current);
      swapTimerRef.current = null;
      prevSwapStepRef.current = stepIndex;
      setShownIndex(stepIndex);
      setSwapPhase("idle");
      return;
    }
    if (prevSwapStepRef.current === stepIndex) return;
    prevSwapStepRef.current = stepIndex;
    if (swapTimerRef.current) clearTimeout(swapTimerRef.current);
    setSwapPhase("leaving");
    swapTimerRef.current = setTimeout(() => {
      swapTimerRef.current = null;
      setShownIndex(stepIndex);
      setSwapPhase("entering");
    }, SWAP_MS);
  }, [stepIndex, reducedMotion]);

  // Entrance and glide, run after the DOM box changed and before paint:
  // first box = entrance animations; a step change = transform glide
  // from the outgoing rect's delta to identity; a remeasure = nothing
  // (its listener already cancelled any glide).
  useLayoutEffect(() => {
    if (!rect) return;
    const from = pendingGlideRef.current;
    pendingGlideRef.current = null;
    const spot = spotRef.current;
    const card = cardRef.current;

    if (from === null) {
      if (enteredRef.current) return;
      enteredRef.current = true;
      if (reducedMotion !== false) return;
      if (spot) {
        animsRef.current.push(
          spot.animate([{ opacity: 0 }, { opacity: 1 }], {
            duration: ENTER_DIM_MS,
            easing: EASE_DEFAULT,
          }),
        );
      }
      if (card) {
        animsRef.current.push(
          card.animate(
            [
              { opacity: 0, transform: "translateY(8px) scale(0.96)" },
              { opacity: 1, transform: "none" },
            ],
            { duration: ENTER_CARD_MS, easing: EASE_SPRING },
          ),
        );
      }
      return;
    }

    if (reducedMotion !== false) return; // the box already jumped; leave it there
    const dx = from.left - rect.left;
    const dy = from.top - rect.top;
    const sx = rect.width > 0 ? from.width / rect.width : 1;
    const sy = rect.height > 0 ? from.height / rect.height : 1;
    const opts = {
      duration: Math.hypot(dx, dy) > GLIDE_LONG_PX ? GLIDE_LONG_MS : GLIDE_MS,
      easing: EASE_DEFAULT,
    };
    const anims: Animation[] = [];
    if (spot) {
      anims.push(
        spot.animate(
          [
            { transform: `translate(${dx}px, ${dy}px) scale(${sx}, ${sy})` },
            { transform: "none" },
          ],
          opts,
        ),
      );
    }
    if (card) {
      const oldTip = tipFor(from);
      const newTip = tipFor(rect);
      anims.push(
        card.animate(
          [
            {
              transform: `translate(${oldTip.left - newTip.left}px, ${oldTip.top - newTip.top}px)`,
            },
            { transform: "none" },
          ],
          opts,
        ),
      );
    }
    animsRef.current = anims;
  }, [rect, reducedMotion]);

  // Exit: reverse the entrance over EXIT_MS, THEN fire the parent
  // callback so the unmount lands after the animation. Reduced motion
  // closes immediately. One exit per overlay.
  const requestExit = useCallback(
    (exit: () => void) => {
      if (exitingRef.current) return;
      exitingRef.current = true;
      if (swapTimerRef.current) clearTimeout(swapTimerRef.current);
      swapTimerRef.current = null;
      if (reducedMotion !== false || !spotRef.current || !cardRef.current) {
        exit();
        return;
      }
      spotRef.current.animate([{ opacity: 1 }, { opacity: 0 }], {
        duration: EXIT_MS,
        easing: EASE_DEFAULT,
        fill: "forwards",
      });
      cardRef.current.animate(
        [
          { opacity: 1, transform: "none" },
          { opacity: 0, transform: `translateY(${SWAP_SHIFT_PX}px)` },
        ],
        { duration: EXIT_MS, easing: EASE_DEFAULT, fill: "forwards" },
      );
      exitTimerRef.current = setTimeout(exit, EXIT_MS);
    },
    [reducedMotion],
  );

  // Escape ends the current tour only (the global switch keeps its own
  // control). onSkipTour is the target; onDone covers a parent that has
  // not grown the prop yet — the same effect in the scheduler.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") requestExit(onSkipTour ?? onDone);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [requestExit, onSkipTour, onDone]);

  // Timer hygiene if the parent unmounts us mid-animation.
  useEffect(
    () => () => {
      if (swapTimerRef.current) clearTimeout(swapTimerRef.current);
      if (exitTimerRef.current) clearTimeout(exitTimerRef.current);
    },
    [],
  );

  if (!step || !rect) return null;

  const last = stepIndex === tour.steps.length - 1;
  const tip = tipFor(rect);
  const displayed = reducedMotion === false ? shownIndex : stepIndex;
  const shownStep = tour.steps[displayed] ?? step;
  const swapTransition = `opacity ${SWAP_MS}ms ${EASE_DEFAULT}, transform ${SWAP_MS}ms ${EASE_DEFAULT}`;
  const contentStyle: CSSProperties =
    reducedMotion === false && swapPhase === "leaving"
      ? {
          opacity: 0,
          transform: `translateY(${SWAP_SHIFT_PX}px)`,
          transition: swapTransition,
        }
      : reducedMotion === false && swapPhase === "entering"
        ? { opacity: 1, transform: "none", transition: swapTransition }
        : { opacity: 1, transform: "none", transition: "none" };

  return (
    <div data-testid="tour-overlay" role="dialog" aria-label={tour.title}>
      {/* The dim is decoration only (pointer-events-none): the highlighted
          element stays interactive and Escape-style dismissal is done via
          the tooltip controls. */}
      <div className="pointer-events-none fixed inset-0 z-50" style={{ opacity: 1 }} />
      {/* Spotlight: one element carries the cutout via a huge box-shadow —
          no four-panel math, and the dim never blocks pointer events on
          the highlighted element because the spotlight itself is
          pointer-events-none; the dim layer keeps clicks. Its box is set
          to the incoming rect in one frame; the step glide is a transform
          from the outgoing rect's delta (origin 0 0) back to identity. */}
      <div
        data-testid="tour-spotlight"
        ref={spotRef}
        className="pointer-events-none fixed z-50 rounded-lg"
        style={{
          top: rect.top - 4,
          left: rect.left - 4,
          width: rect.width + 8,
          height: rect.height + 8,
          boxShadow: "0 0 0 9999px rgba(15, 12, 9, 0.55)",
          outline: "2px solid var(--m-accent)",
          transformOrigin: "0 0",
        }}
      />
      <div
        data-testid="tour-tooltip"
        ref={cardRef}
        className="fixed z-50 w-[320px] rounded-xl border p-3 shadow-lg"
        style={{
          top: tip.top,
          left: tip.left,
          borderColor: "var(--m-border)",
          background: "var(--m-surface)",
        }}
      >
        <div style={contentStyle}>
          <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            {tour.title} · step {displayed + 1} of {tour.steps.length}
          </p>
          <p className="mt-1 text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
            {shownStep.title}
          </p>
          <p className="mt-1 text-[12.5px]" style={{ color: "var(--m-text-secondary)" }}>
            {shownStep.body}
          </p>
        </div>
        <div className="mt-3 flex items-center justify-between gap-2">
          <button
            type="button"
            data-testid="tour-disable"
            onClick={() => requestExit(onDisableTours)}
            className="cursor-pointer text-[11px] underline-offset-2 hover:underline"
            style={{ color: "var(--m-text-muted)" }}
          >
            Don&apos;t show me tours
          </button>
          <div className="flex items-center gap-1.5">
            <Button
              variant="ghost"
              size="sm"
              data-testid="tour-skip"
              onClick={() => requestExit(onSkipTour ?? onDone)}
            >
              Skip tour
            </Button>
            {stepIndex > 0 && (
              <Button variant="ghost" size="sm" data-testid="tour-back" onClick={onBack}>
                Back
              </Button>
            )}
            <Button
              variant="secondary"
              size="sm"
              data-testid={last ? "tour-done" : "tour-next"}
              onClick={last ? () => requestExit(onDone) : onNext}
            >
              {last ? "Done" : "Next"}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
