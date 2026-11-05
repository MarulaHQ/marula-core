// CardSizeControl — the width, height and order of one dashboard card.
//
// Purpose: placement is a control, not a drag handle. Every option this renders
// comes from the `GET /api/report-catalog` payload (`spans`, `heights`); nothing
// here holds a list of allowed values. The grid's twelve columns live in this
// file because CSS owns the measurement (crates/core-reports/src/card.rs says
// so): a density change moves the pixels, not the stored contract.
//
// Keyboard: each group is a radiogroup with one tab stop on the checked option.
// Arrow keys move the selection and the focus together. Enter and Space select,
// because these are buttons.
//
// Non-purpose: no persistence and no fetching — the page owns both.
"use client";

import type { KeyboardEvent } from "react";
import { useId, useRef } from "react";
import { Button } from "@marula-core/ui";
import type { CardSize } from "../../lib/dashboards";
import { humanSlug } from "../../lib/dashboards";

/** Span slug to the columns it occupies. The payload names, the CSS measures. */
const SPAN_CLASS: Record<string, string> = {
  quarter: "col-span-12 sm:col-span-6 xl:col-span-3",
  third: "col-span-12 sm:col-span-6 xl:col-span-4",
  half: "col-span-12 xl:col-span-6",
  full: "col-span-12",
};

/** Height slug to the smallest block it draws. */
const HEIGHT_CLASS: Record<string, string> = {
  compact: "min-h-[136px]",
  regular: "min-h-[264px]",
  tall: "min-h-[400px]",
};

/**
 * The grid columns for a span. A slug with no class falls back to the full row
 * rather than a sliver: the payload is validated before a card is stored, so
 * this is a last resort for a vocabulary that moved under the client.
 */
export function cardSpanClass(span: string): string {
  return SPAN_CLASS[span] ?? SPAN_CLASS.full ?? "col-span-12";
}

/** The minimum height for a height slug, with the same fallback rule. */
export function cardHeightClass(height: string): string {
  return HEIGHT_CLASS[height] ?? HEIGHT_CLASS.regular ?? "min-h-[264px]";
}

/** A slug as a person reads it: `half` is "Half", `extra_wide` is "Extra wide". */
export function sizeLabel(slug: string): string {
  return humanSlug(slug);
}

/** The header button that opens the size panel for one card. */
export function CardSizeToggle({
  open,
  onToggle,
  panelId,
  title,
}: {
  open: boolean;
  onToggle: () => void;
  panelId: string;
  title: string;
}): React.JSX.Element {
  return (
    <Button
      variant="ghost"
      size="sm"
      aria-expanded={open}
      aria-controls={panelId}
      aria-label={open ? `Close size controls for ${title}` : `Size and position for ${title}`}
      onClick={onToggle}
    >
      Size
    </Button>
  );
}

/**
 * The open size panel: width, height, and the two order buttons. `size` is the
 * card's current size, so a change to one half leaves the other alone.
 */
export function CardSizePanel({
  title,
  spans,
  heights,
  size,
  onResize,
  onMove,
  canMoveEarlier,
  canMoveLater,
  testId,
}: {
  title: string;
  spans: readonly string[];
  heights: readonly string[];
  size: CardSize;
  onResize: (next: CardSize) => void;
  onMove: (direction: -1 | 1) => void;
  canMoveEarlier: boolean;
  canMoveLater: boolean;
  testId: string;
}): React.JSX.Element {
  return (
    <div
      className="m-rise grid gap-3 rounded-[var(--m-radius-container)] border p-3"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface-2)" }}
      data-testid={testId}
    >
      <SizeChoices
        legend={`${title} card width`}
        options={spans}
        value={size.span}
        onChange={(span) => onResize({ span, height: size.height })}
        preview
      />
      <SizeChoices
        legend={`${title} card height`}
        options={heights}
        value={size.height}
        onChange={(height) => onResize({ span: size.span, height })}
      />
      <div className="flex flex-wrap items-center gap-2">
        <Button
          variant="secondary"
          size="sm"
          disabled={!canMoveEarlier}
          aria-label={`Move ${title} earlier`}
          onClick={() => onMove(-1)}
        >
          Earlier
        </Button>
        <Button
          variant="secondary"
          size="sm"
          disabled={!canMoveLater}
          aria-label={`Move ${title} later`}
          onClick={() => onMove(1)}
        >
          Later
        </Button>
        <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
          Cards keep this order as the grid wraps.
        </p>
      </div>
    </div>
  );
}

/**
 * One radiogroup. The checked option is the tab stop; arrows move the selection
 * and the focus, which is how a native radio group behaves.
 */
function SizeChoices({
  legend,
  options,
  value,
  onChange,
  preview = false,
}: {
  legend: string;
  options: readonly string[];
  value: string;
  onChange: (next: string) => void;
  /** Draw a footprint bar whose width follows the option's place in the list. */
  preview?: boolean;
}): React.JSX.Element {
  const refs = useRef<Array<HTMLButtonElement | null>>([]);
  const id = useId();

  const step = (direction: -1 | 1) => {
    const from = options.indexOf(value);
    const index = (from + direction + options.length) % options.length;
    const next = options[index];
    if (next === undefined) return;
    onChange(next);
    refs.current[index]?.focus();
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      event.preventDefault();
      step(1);
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      event.preventDefault();
      step(-1);
    }
  };

  return (
    <div className="grid gap-1">
      <span
        id={`${id}-label`}
        className="text-[10px] font-semibold uppercase tracking-wide"
        style={{ color: "var(--m-text-muted)" }}
      >
        {legend}
      </span>
      <div
        role="radiogroup"
        aria-labelledby={`${id}-label`}
        className="flex flex-wrap gap-1"
        onKeyDown={onKeyDown}
      >
        {options.map((option, index) => {
          const active = option === value;
          return (
            <button
              key={option}
              ref={(node) => {
                refs.current[index] = node;
              }}
              type="button"
              role="radio"
              aria-checked={active}
              tabIndex={active ? 0 : -1}
              onClick={() => onChange(option)}
              className="inline-flex cursor-pointer items-center gap-1.5 rounded-[var(--m-radius-control)] px-2 py-1 text-[12px] font-medium transition-colors duration-100"
              style={{
                background: active ? "var(--m-surface)" : "transparent",
                color: active ? "var(--m-text)" : "var(--m-text-secondary)",
                border: `1px solid ${active ? "var(--m-border-strong)" : "var(--m-border)"}`,
              }}
            >
              {preview ? (
                <span
                  aria-hidden="true"
                  className="inline-flex h-2 w-5 items-center overflow-hidden rounded-xs"
                  style={{ background: "var(--m-border)" }}
                >
                  <span
                    className="h-2 rounded-xs"
                    style={{
                      width: `${Math.round(((index + 1) / options.length) * 100)}%`,
                      background: active ? "var(--m-accent)" : "var(--m-text-muted)",
                    }}
                  />
                </span>
              ) : null}
              {sizeLabel(option)}
            </button>
          );
        })}
      </div>
    </div>
  );
}
