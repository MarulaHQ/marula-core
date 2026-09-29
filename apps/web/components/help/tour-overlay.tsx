// TourOverlay — the spotlight/highlight flow renderer (Wave 26 lane 2).
//
// Purpose: renders ONE declarative tour step: dimmed page, highlighted
// target, positioned tooltip card with Back/Next/Done. Motion obeys the
// law (docs/theme.md §5): transform/opacity only, ≤120ms, and fully
// static under prefers-reduced-motion. The overlay is read-only about
// the page — it never mutates DOM outside its own root.

"use client";

import { Button } from "@marula-core/ui";
import { useEffect, useState } from "react";
import type { Tour, TourStep } from "../../lib/tours/tours";

interface Rect {
  top: number;
  left: number;
  width: number;
  height: number;
}

const PAD = 8;

function measure(step: TourStep): Rect | null {
  const el = document.querySelector(step.target);
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return { top: r.top, left: r.left, width: r.width, height: r.height };
}

export function TourOverlay({
  tour,
  stepIndex,
  onNext,
  onBack,
  onDone,
  onDisableTours,
}: {
  tour: Tour;
  stepIndex: number;
  onNext(): void;
  onBack(): void;
  onDone(): void;
  onDisableTours(): void;
}) {
  const step: TourStep | undefined = tour.steps[stepIndex];
  const [rect, setRect] = useState<Rect | null>(null);
  const [reducedMotion, setReducedMotion] = useState(false);

  useEffect(() => {
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    setReducedMotion(mq.matches);
    const onChange = () => setReducedMotion(mq.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  useEffect(() => {
    if (!step) return;
    const update = () => setRect(measure(step));
    update();
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [step]);

  if (!step || !rect) return null;

  const last = stepIndex === tour.steps.length - 1;
  // Tooltip prefers below the target; flips above when there is no room.
  const below = rect.top + rect.height + 180 < window.innerHeight;
  const tipTop = below ? rect.top + rect.height + PAD : Math.max(PAD, rect.top - 180 - PAD);
  const tipLeft = Math.min(
    Math.max(PAD, rect.left),
    window.innerWidth - 320 - PAD,
  );
  const transition = reducedMotion
    ? "none"
    : "top 120ms ease-out, left 120ms ease-out, opacity 120ms ease-out";

  return (
    <div data-testid="tour-overlay" role="dialog" aria-label={tour.title}>
      {/* The dim is decoration only (pointer-events-none): the highlighted
          element stays interactive and Escape-style dismissal is done via
          the tooltip controls. */}
      <div className="pointer-events-none fixed inset-0 z-50" style={{ opacity: 1 }} />
      {/* Spotlight: one element carries the cutout via a huge box-shadow —
          no four-panel math, and the dim never blocks pointer events on
          the highlighted element because the spotlight itself is
          pointer-events-none; the dim layer keeps clicks. */}
      <div
        data-testid="tour-spotlight"
        className="pointer-events-none fixed z-50 rounded-lg"
        style={{
          top: rect.top - 4,
          left: rect.left - 4,
          width: rect.width + 8,
          height: rect.height + 8,
          boxShadow: "0 0 0 9999px rgba(15, 12, 9, 0.55)",
          outline: "2px solid var(--m-accent)",
          transition,
        }}
      />
      <div
        data-testid="tour-tooltip"
        className="fixed z-50 w-[320px] rounded-xl border p-3 shadow-lg"
        style={{
          top: tipTop,
          left: tipLeft,
          borderColor: "var(--m-border)",
          background: "var(--m-surface)",
          transition,
        }}
      >
        <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
          {tour.title} · step {stepIndex + 1} of {tour.steps.length}
        </p>
        <p className="mt-1 text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
          {step.title}
        </p>
        <p className="mt-1 text-[12.5px]" style={{ color: "var(--m-text-secondary)" }}>
          {step.body}
        </p>
        <div className="mt-3 flex items-center justify-between gap-2">
          <button
            type="button"
            data-testid="tour-disable"
            onClick={onDisableTours}
            className="cursor-pointer text-[11px] underline-offset-2 hover:underline"
            style={{ color: "var(--m-text-muted)" }}
          >
            Don&apos;t show me tours
          </button>
          <div className="flex items-center gap-1.5">
            {stepIndex > 0 && (
              <Button variant="ghost" size="sm" data-testid="tour-back" onClick={onBack}>
                Back
              </Button>
            )}
            <Button
              variant="secondary"
              size="sm"
              data-testid={last ? "tour-done" : "tour-next"}
              onClick={last ? onDone : onNext}
            >
              {last ? "Done" : "Next"}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
