// Brand lockups — the approved Marula identity, from brand/marula-brand-assets.
//
// Purpose: render the fruit mark and the wordmark lockup with the surface
// variant the brand kit specifies (ink on a light surface, cream on a dark
// one) and enforce the minimum sizes that same kit sets: the lockup is never
// below 96px wide, the mark is never below 32px, and under 32px the fruit
// dot replaces the mark because that is the version the favicons use.
//
// Non-purpose: no invented variants and no recolouring. Every file referenced
// here is a byte copy of a kit asset in apps/web/public/ — the per-file source
// path is recorded in design/qa-p41-brand-assets/LANDING.md. The wordmark is
// outlined in the SVGs, so no font has to be installed to render it.
"use client";

import { useTheme } from "@marula-core/ui";

/** Lockup width : height, from the kit lockup viewBox (589 x 200). */
const LOCKUP_RATIO = 589 / 200;

/** The kit's minimum sizes, in px. */
export const LOCKUP_MIN_WIDTH = 96;
export const MARK_MIN_SIZE = 32;

/** Rendered height in px that satisfies LOCKUP_MIN_WIDTH. */
export const LOCKUP_COMPLIANT_HEIGHT = Math.ceil(LOCKUP_MIN_WIDTH / LOCKUP_RATIO);

function lockupSrc(dark: boolean) {
  return dark ? "/lockup-mono-cream.svg" : "/lockup-mono-ink.svg";
}

function markSrc(dark: boolean) {
  return dark ? "/mark-mono-cream.svg" : "/mark-mono-ink.svg";
}

/**
 * The mark alone, at the size that size actually calls for.
 *
 * Under MARK_MIN_SIZE the kit's favicon version takes over: the fruit dot,
 * which drops the leaf and highlight and stays legible at favicon sizes. The
 * fruit dot has no dark-surface variant — gold reads on both.
 */
export function BrandMark({ size = 24 }: { size?: number }) {
  const { theme } = useTheme();
  const dark = theme === "dark";
  if (size < MARK_MIN_SIZE) {
    return (
      <img
        src="/fruit-dot-gold.svg"
        alt=""
        aria-hidden="true"
        style={{ height: size, width: "auto" }}
      />
    );
  }
  return (
    <img
      src={markSrc(dark)}
      alt=""
      aria-hidden="true"
      style={{ height: size, width: "auto" }}
    />
  );
}

/**
 * The mark and the wordmark together.
 *
 * `size` is the requested rendered height in px. A height that would put the
 * lockup under the kit's 96px floor falls back to the mark rather than
 * rendering an undersized lockup — so callers that want the wordmark should
 * pass at least LOCKUP_COMPLIANT_HEIGHT.
 */
export function BrandLockup({
  size = LOCKUP_COMPLIANT_HEIGHT,
  label = "marula",
}: {
  /** Requested rendered height in px; width follows the 589:200 lockup ratio. */
  size?: number;
  label?: string;
}) {
  const { theme } = useTheme();
  const dark = theme === "dark";
  if (size * LOCKUP_RATIO < LOCKUP_MIN_WIDTH) {
    return <BrandMark size={Math.max(size, MARK_MIN_SIZE)} />;
  }
  return (
    <img
      src={lockupSrc(dark)}
      alt={label}
      style={{ height: size, width: "auto" }}
    />
  );
}
