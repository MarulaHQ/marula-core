/**
 * @module Pure resize-constraint mathematics for split nodes.
 *
 * Purpose: fraction/pixel conversion, divider movement ranges, two-neighbor
 * resize application, and water-filling renormalization of fractional sizes
 * against per-child pixel bounds. Fully deterministic and unit-tested; no DOM,
 * no React, no I/O.
 * Non-purpose: no cascading redistribution beyond the two neighbors of a
 * dragged divider — that interaction model is an explicit non-goal.
 */

import type { SizeBounds } from "./types";

/** Default minimum extent applied to any child that declares none, in CSS pixels. */
export const DEFAULT_MIN_PANE_PX = 120;

/** Minimum extent applied to nested split containers along either axis, in CSS pixels. */
export const MIN_SPLIT_PX = 80;

/** Largest absolute difference treated as equal when comparing fraction sums. */
const FRACTION_EPSILON = 1e-6;

export function clamp(value: number, low: number, high: number): number {
  return Math.min(Math.max(value, low), high);
}

export function sum(values: readonly number[]): number {
  return values.reduce((total, value) => total + value, 0);
}

function normalizeToUnit(values: readonly number[]): number[] {
  if (values.length === 0) return [];
  const total = sum(values);
  if (!Number.isFinite(total) || total <= FRACTION_EPSILON) {
    return values.map(() => 1 / values.length);
  }
  return values.map((value) => value / total);
}

/**
 * Convert fractional sizes to pixel extents for `innerPx` of flexible space
 * (container size minus divider tracks). Guards against zero/NaN totals by
 * distributing equally.
 */
export function fractionsToPx(fractions: readonly number[], innerPx: number): number[] {
  if (fractions.length === 0) return [];
  if (!Number.isFinite(innerPx) || innerPx <= 0) {
    return fractions.map(() => 0);
  }
  const total = sum(fractions);
  if (!Number.isFinite(total) || total <= FRACTION_EPSILON) {
    const even = innerPx / fractions.length;
    return fractions.map(() => even);
  }
  return fractions.map((fraction) => (fraction / total) * innerPx);
}

/**
 * Convert pixel extents back to fractions summing to exactly 1 (within float
 * precision). Negative or non-finite entries are replaced by equal weighting.
 */
export function pxToFractions(px: readonly number[]): number[] {
  if (px.length === 0) return [];
  const usable = px.every((value) => Number.isFinite(value) && value >= 0) ? px : px.map(() => 1);
  return normalizeToUnit(usable);
}

/**
 * Absolute range, measured from the start of the first child, within which the
 * boundary between two adjacent children may sit.
 *
 * When the combined constraints are unsatisfiable (minimums exceed available
 * space), both bounds collapse to the midpoint of the contradictory interval so
 * callers get one deterministic, stable position.
 */
export function dividerRangePx(params: {
  firstPx: number;
  secondPx: number;
  first: SizeBounds;
  second: SizeBounds;
}): { minOffsetPx: number; maxOffsetPx: number } {
  const { firstPx, secondPx, first, second } = params;
  const total = firstPx + secondPx;
  const low = Math.max(
    0,
    first.minPx,
    second.maxPx !== undefined ? total - second.maxPx : Number.NEGATIVE_INFINITY,
  );
  const high = Math.min(
    total,
    first.maxPx ?? Number.POSITIVE_INFINITY,
    total - Math.max(0, second.minPx),
  );
  if (high < low) {
    const compromise = (low + high) / 2;
    return { minOffsetPx: compromise, maxOffsetPx: compromise };
  }
  return { minOffsetPx: low, maxOffsetPx: high };
}

/**
 * Apply a divider move: returns the new `[firstPx, secondPx]` pair after
 * placing the boundary at `targetOffsetPx` (relative to the pair start),
 * clamped through {@link dividerRangePx}. The pair's total is preserved, so
 * all other siblings are untouched.
 */
export function applyPairResizePx(params: {
  firstPx: number;
  secondPx: number;
  first: SizeBounds;
  second: SizeBounds;
  targetOffsetPx: number;
}): [number, number] {
  const { firstPx, secondPx, first, second, targetOffsetPx } = params;
  const total = firstPx + secondPx;
  const { minOffsetPx, maxOffsetPx } = dividerRangePx({ firstPx, secondPx, first, second });
  const boundary = clamp(targetOffsetPx, minOffsetPx, maxOffsetPx);
  return [boundary, total - boundary];
}

/**
 * Water-filling renormalization: scale `fractions` onto `innerPx` pixels so
 * every child respects its {@link SizeBounds}, redistributing leftover space
 * proportionally among unconstrained children. Bounded sweeps; idempotent once
 * feasible.
 *
 * If the space cannot fit all minimums, returns fractions proportional to each
 * minimum (overflow; the container clips — documented engine behavior).
 */
export function renormalizeFractions(
  fractions: readonly number[],
  bounds: readonly SizeBounds[],
  innerPx: number,
): number[] {
  if (fractions.length === 0) return [];
  if (fractions.length !== bounds.length) {
    throw new Error("renormalizeFractions: fractions and bounds length mismatch");
  }
  const count = fractions.length;
  const safeInner = Number.isFinite(innerPx) && innerPx > 0 ? innerPx : 0;
  const mins: number[] = bounds.map((bound) => Math.max(0, bound.minPx));
  const maxes: number[] = bounds.map((bound) => bound.maxPx ?? Number.POSITIVE_INFINITY);

  const minTotal = sum(mins);
  if (safeInner <= minTotal) {
    return minTotal <= FRACTION_EPSILON
      ? fractions.map(() => 1 / count)
      : mins.map((min) => min / minTotal);
  }

  const fractionTotal = sum(fractions);
  let allocation =
    Number.isFinite(fractionTotal) && fractionTotal > FRACTION_EPSILON
      ? fractions.map((fraction) => (fraction / fractionTotal) * safeInner)
      : fractions.map(() => safeInner / count);

  const pinnedAtMin = new Array<boolean>(count).fill(false);
  const pinnedAtMax = new Array<boolean>(count).fill(false);

  for (let sweep = 0; sweep < count + 2; sweep += 1) {
    let changed = false;
    for (let index = 0; index < count; index += 1) {
      const value = allocation[index] as number;
      if (!pinnedAtMax[index] && value >= (maxes[index] as number)) {
        allocation[index] = maxes[index] as number;
        pinnedAtMax[index] = true;
        changed = true;
      } else if (!pinnedAtMin[index] && value <= (mins[index] as number)) {
        allocation[index] = mins[index] as number;
        pinnedAtMin[index] = true;
        changed = true;
      }
    }

    const freeIndexes: number[] = [];
    let fixedTotal = 0;
    let freeFractionTotal = 0;
    for (let index = 0; index < count; index += 1) {
      if (pinnedAtMin[index] || pinnedAtMax[index]) {
        fixedTotal += allocation[index] as number;
      } else {
        freeIndexes.push(index);
        freeFractionTotal += fractions[index] as number;
      }
    }

    if (freeIndexes.length === 0) break;
    const remaining = safeInner - fixedTotal;
    if (remaining <= FRACTION_EPSILON) {
      for (const index of freeIndexes) {
        allocation[index] = mins[index] as number;
        pinnedAtMin[index] = true;
      }
      changed = true;
    } else {
      const shareProportional = freeFractionTotal > FRACTION_EPSILON;
      const base = shareProportional ? freeFractionTotal : freeIndexes.length;
      for (const index of freeIndexes) {
        const share = shareProportional
          ? ((fractions[index] as number) / base) * remaining
          : remaining / base;
        if (Math.abs((allocation[index] as number) - share) > FRACTION_EPSILON) {
          allocation[index] = share;
          changed = true;
        }
      }
    }
    if (!changed) break;
  }

  return pxToFractions(allocation);
}
