/**
 * @module Draggable, keyboard-operable divider between two adjacent children
 * of a split.
 *
 * Purpose: pointer-drag resizing with live preview (layout updates every
 * frame), full keyboard operation (arrows, Home/End, Shift for coarse steps),
 * double-click to equalize the pair, and separator ARIA semantics. All size
 * math is delegated to the pure constraint module; this file only measures the
 * DOM and maps input events onto it.
 * Non-purpose: no drag-and-drop of panes or tabs and no floating docks.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import type {
  KeyboardEvent as ReactKeyboardEvent,
  PointerEvent as ReactPointerEvent,
} from "react";
import { applyPairResizePx, clamp, dividerRangePx } from "../constraints";
import { subtreeAxisMinPx } from "../tree";
import type { NodeId, SizeBounds, SplitDirection, SplitNode } from "../types";

export interface DividerProps {
  /** Owning split node. */
  split: SplitNode;
  /**
   * Index of the boundary: it sits between `children[index]` and
   * `children[index + 1]`, so `0 <= index < children.length - 1`.
   */
  index: number;
  /** Effective render direction of the owning split (may be flipped when collapsed). */
  direction: SplitDirection;
  /** Receives replacement fractions for the whole split after each resize step. */
  onResize(sizes: number[]): void;
}

interface PairMeasurement {
  firstPx: number;
  secondPx: number;
  originPx: number;
  range: { minOffsetPx: number; maxOffsetPx: number };
}

const KEYBOARD_STEP_PX = 12;
const KEYBOARD_COARSE_STEP_PX = 48;

export function Divider({ split, index, direction, onResize }: DividerProps) {
  const elementRef = useRef<HTMLDivElement | null>(null);
  const [dragging, setDragging] = useState(false);
  const dragRef = useRef<PairMeasurement | null>(null);
  const frameRef = useRef<number | null>(null);

  const firstChild = split.children[index];
  const secondChild = split.children[index + 1];
  if (firstChild === undefined || secondChild === undefined) {
    throw new Error(`Divider ${index} out of bounds for split ${split.id}`);
  }

  const axis = direction === "row" ? "x" : "y";

  const measurePair = useCallback((): PairMeasurement | null => {
    const element = elementRef.current;
    if (element === null) return null;
    const previous = previousElementOf(element);
    const next = nextElementOf(element);
    if (previous === null || next === null) return null;

    const firstRect = previous.getBoundingClientRect();
    const secondRect = next.getBoundingClientRect();
    const firstPx = axis === "x" ? firstRect.width : firstRect.height;
    const secondPx = axis === "x" ? secondRect.width : secondRect.height;
    if (firstPx <= 0 && secondPx <= 0) return null;

    const bounds: [SizeBounds, SizeBounds] = [
      { minPx: subtreeAxisMinPx(firstChild, direction) },
      { minPx: subtreeAxisMinPx(secondChild, direction) },
    ];
    return {
      firstPx,
      secondPx,
      originPx: axis === "x" ? firstRect.left : firstRect.top,
      range: dividerRangePx({
        firstPx,
        secondPx,
        first: bounds[0] as SizeBounds,
        second: bounds[1] as SizeBounds,
      }),
    };
  }, [axis, direction, firstChild, secondChild]);

  const commitBoundaryOffset = useCallback(
    (measurement: PairMeasurement, targetOffsetPx: number) => {
      const boundsFirst: SizeBounds = { minPx: subtreeAxisMinPx(firstChild, direction) };
      const boundsSecond: SizeBounds = { minPx: subtreeAxisMinPx(secondChild, direction) };
      const [nextFirstPx, nextSecondPx] = applyPairResizePx({
        firstPx: measurement.firstPx,
        secondPx: measurement.secondPx,
        first: boundsFirst,
        second: boundsSecond,
        targetOffsetPx,
      });

      // Preserve the pair's combined fraction so all other siblings keep
      // their exact shares; only these two fractions move.
      const combinedFraction =
        (split.sizes[index] as number) + (split.sizes[index + 1] as number);
      const pairTotal = nextFirstPx + nextSecondPx;
      const sizes = [...split.sizes];
      if (pairTotal <= 0) return;
      sizes[index] = (combinedFraction * nextFirstPx) / pairTotal;
      sizes[index + 1] = (combinedFraction * nextSecondPx) / pairTotal;
      onResize(sizes);
    },
    [direction, firstChild, index, onResize, secondChild, split.sizes],
  );

  const handlePointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      const measurement = measurePair();
      if (measurement === null) return;
      event.currentTarget.setPointerCapture(event.pointerId);
      dragRef.current = measurement;
      setDragging(true);
      event.preventDefault();
    },
    [measurePair],
  );

  const handlePointerMove = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (dragRef.current === null) return;
      const pointerPosition = axis === "x" ? event.clientX : event.clientY;
      if (frameRef.current !== null) cancelAnimationFrame(frameRef.current);
      frameRef.current = requestAnimationFrame(() => {
        frameRef.current = null;
        const measurement = dragRef.current;
        if (measurement === null) return;
        const target = pointerPosition - measurement.originPx;
        commitBoundaryOffset(measurement, target);
      });
    },
    [axis, commitBoundaryOffset],
  );

  const endDrag = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    if (dragRef.current === null) return;
    dragRef.current = null;
    setDragging(false);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  }, []);

  useEffect(() => {
    return () => {
      if (frameRef.current !== null) cancelAnimationFrame(frameRef.current);
    };
  }, []);

  const handleKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLDivElement>) => {
      const measurement = measurePair();
      if (measurement === null) return;

      let delta: number | null = null;
      if (direction === "row") {
        if (event.key === "ArrowLeft") delta = -1;
        else if (event.key === "ArrowRight") delta = 1;
      } else {
        if (event.key === "ArrowUp") delta = -1;
        else if (event.key === "ArrowDown") delta = 1;
      }

      let target: number | null = null;
      if (delta !== null) {
        const step = event.shiftKey ? KEYBOARD_COARSE_STEP_PX : KEYBOARD_STEP_PX;
        const currentOffset = measurement.firstPx;
        target = currentOffset + step * delta;
      } else if (event.key === "Home") {
        target = measurement.range.minOffsetPx;
      } else if (event.key === "End") {
        target = measurement.range.maxOffsetPx;
      }
      if (target === null) return;

      event.preventDefault();
      commitBoundaryOffset(measurement, target);
    },
    [commitBoundaryOffset, direction, measurePair],
  );

  const handleDoubleClick = useCallback(() => {
    const combinedFraction =
      (split.sizes[index] as number) + (split.sizes[index + 1] as number);
    const sizes = [...split.sizes];
    sizes[index] = combinedFraction / 2;
    sizes[index + 1] = combinedFraction / 2;
    onResize(sizes);
  }, [index, onResize, split.sizes]);

  const percentNow = Math.round(clamp((split.sizes[index] as number) * 100, 0, 100));
  const firstId: NodeId = firstChild.id;
  const secondId: NodeId = secondChild.id;

  return (
    <div
      ref={elementRef}
      className="marula-divider"
      data-direction={direction}
      data-dragging={dragging ? "true" : undefined}
      role="separator"
      tabIndex={0}
      aria-orientation={direction === "row" ? "vertical" : "horizontal"}
      aria-label={`Resize panes (${percentNow} percent)`}
      aria-controls={`${firstId} ${secondId}`}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percentNow}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={endDrag}
      onLostPointerCapture={endDrag}
      onKeyDown={handleKeyDown}
      onDoubleClick={handleDoubleClick}
    />
  );
}

function previousElementOf(element: HTMLElement): HTMLElement | null {
  let candidate = element.previousElementSibling;
  while (candidate !== null && !(candidate instanceof HTMLElement)) {
    candidate = candidate.previousElementSibling;
  }
  return candidate;
}

function nextElementOf(element: HTMLElement): HTMLElement | null {
  let candidate = element.nextElementSibling;
  while (candidate !== null && !(candidate instanceof HTMLElement)) {
    candidate = candidate.nextElementSibling;
  }
  return candidate;
}
