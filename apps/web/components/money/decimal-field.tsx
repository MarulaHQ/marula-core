"use client";
/**
 * A number the person types: a quantity in milli-units or a rate in minor
 * units.
 *
 * Purpose: the field keeps the raw text while it is being typed, and commits
 * the parsed number on every keystroke. A value re-rendered from the committed
 * number alone cannot be typed into — "2." collapses to "2" and drops the dot,
 * and "1" in a price field becomes "1.00" so the next key makes "1.005". The
 * text is rebuilt from the committed value only when the two disagree: on
 * blur, or when a change proposal replaces the line under the field.
 *
 * Non-purpose: no rounding policy and no arithmetic. The scale says what the
 * number is in; the caller's math decides what it means.
 */
import { useEffect, useState } from "react";

function formatted(value: number, scale: number, decimals: number | undefined): string {
  if (decimals === undefined) return String(value / scale);
  return (value / scale).toFixed(decimals);
}

/** The text as a committed value, or null while it is not a number. */
function parsed(text: string, scale: number): number | null {
  const trimmed = text.trim();
  if (trimmed === "") return null;
  const value = Number(trimmed);
  if (!Number.isFinite(value)) return null;
  return Math.round(value * scale);
}

export function DecimalField({
  label,
  value,
  scale,
  width,
  decimals,
  onCommit,
}: {
  /** The field's accessible name, which says which row it belongs to. */
  label: string;
  /** The committed number, in the same unit as `scale`. */
  value: number;
  /** 1000 for milli-units, 100 for minor currency units. */
  scale: number;
  width: number;
  /** Fixed decimals for display; omitted keeps the shortest exact text. */
  decimals?: number;
  onCommit: (value: number) => void;
}): React.JSX.Element {
  const [text, setText] = useState(() => formatted(value, scale, decimals));

  // A value that changed underneath the text (a proposal applied, a line
  // replaced) wins; text that already parses to the value is the person's own
  // typing and is left alone.
  useEffect(() => {
    setText((current) =>
      parsed(current, scale) === value ? current : formatted(value, scale, decimals),
    );
  }, [value, scale, decimals]);

  return (
    <input
      className="m-input m-dsc-num"
      style={{ width }}
      inputMode="decimal"
      aria-label={label}
      value={text}
      onChange={(event) => {
        const next = event.target.value;
        setText(next);
        const committed = parsed(next, scale);
        if (committed !== null) onCommit(committed);
      }}
      onBlur={() => setText(formatted(value, scale, decimals))}
    />
  );
}

/**
 * The same field for a surface that lays its own table: the caller supplies
 * the accessible label and, optionally, a width. `id` is accepted so an
 * existing form can pass one; the field is labelled by `label`.
 */
export function DecimalInput({
  value,
  scale,
  onCommit,
  decimals,
  label,
  width = 110,
}: {
  value: number;
  scale: number;
  onCommit: (value: number) => void;
  decimals?: number;
  label?: string;
  id?: string;
  width?: number;
}): React.JSX.Element {
  return (
    <DecimalField
      label={label ?? "Amount"}
      value={value}
      scale={scale}
      decimals={decimals}
      width={width}
      onCommit={onCommit}
    />
  );
}
