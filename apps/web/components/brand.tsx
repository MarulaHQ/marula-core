// Brand lockups — marula-ci asset bundle integration (Wave 17 lane 10).
//
// Purpose: render the marula-ci logo lockups from the copied bundle in
// apps/web/public/ with theme-aware file selection. `marula-ci/source/*.svg`
// stays the editable source of truth; the PNGs here are the rendered
// equivalents shipped by the bundle (no hand edits, hexes live in the SVGs).
// Non-purpose: no marketing pages, no color inventing — navy/white are the
// bundle's own two lockup variants.
"use client";

import { useTheme } from "@marula-core/ui";
import type { Theme } from "@marula-core/ui";

export function BrandLockup({
  size = 20,
  label = "Marula",
}: {
  /** Rendered height in px; width follows the 516:140 lockup ratio. */
  size?: number;
  label?: string;
}) {
  const { theme } = useTheme();
  const dark = theme === "dark";
  return (
    <img
      src={dark ? "/lockup-white@2x.png" : "/lockup-navy@2x.png"}
      alt={label}
      style={{ height: size, width: "auto" }}
    />
  );
}

/**
 * The standalone mark (marula-ci mark tile: ink square, white M, accent
 * marula line). Theme-aware: the bundle's navy ink mark is illegible on dark
 * surfaces, so
 * dark uses mark-white-512 (the lockup-white white-ink convention applied to
 * the standalone mark); navy variant on light.
 */
export function BrandMark({ size = 24 }: { size?: number }) {
  const { theme } = useTheme();
  const dark = theme === "dark";
  return (
    <img
      src={dark ? "/mark-white-512.png" : "/mark-navy-512.png"}
      alt=""
      aria-hidden="true"
      style={{ height: size, width: size }}
    />
  );
}
