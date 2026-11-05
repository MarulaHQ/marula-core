"use client";
/**
 * CardContractMeta — a stored card's window, refresh and size, as it will run.
 *
 * One component for both surfaces. The saved-report row and the dashboard card
 * print the same three facts because both read the same stored `CardSpec`
 * (`crates/core-reports/src/card.rs`, migration 0180) through one formatter in
 * `lib/dashboards`. Nothing here is inferred from a viz or a default: if a
 * definition stores no card, the row says so instead of inventing one.
 *
 * The `data-card-*` attributes carry the raw members so a test, a support
 * screen and the browser can all read what was stored rather than a rendering.
 *
 * Non-purpose: no picking, no editing. The size controls live in
 * `card-size-control.tsx`; a definition's card is edited by saving the report.
 */
import { cardReadout, type StoredCard } from "../../lib/dashboards";

export interface CardContractMetaProps {
  /** The stored card, or null/absent when the definition has none. */
  card: StoredCard | null | undefined;
  /** `cell` stacks the window over its meta line; `inline` is one row. */
  variant?: "cell" | "inline";
  /** What to print when no card is stored. */
  emptyCopy?: string;
  testId?: string;
}

export function CardContractMeta({
  card,
  variant = "inline",
  emptyCopy = "No card stored — runs with the trailing 30 days.",
  testId,
}: CardContractMetaProps) {
  const readout = cardReadout(card);

  if (readout === null) {
    return (
      <span
        className="text-[12px]"
        style={{ color: "var(--m-text-muted)" }}
        data-testid={testId}
        data-card="none"
      >
        {emptyCopy}
      </span>
    );
  }

  const attrs = {
    "data-testid": testId,
    "data-card": "stored",
    "data-card-window-days": card?.window_days,
    "data-card-refresh-minutes": card?.refresh?.every_minutes,
    "data-card-reach-days": card?.refresh?.reach_days ?? 0,
    "data-card-span": card?.size?.span,
    "data-card-height": card?.size?.height,
  } as const;

  if (variant === "cell") {
    return (
      <div className="grid gap-0.5" {...attrs}>
        <span className="whitespace-nowrap text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {readout.window}
        </span>
        <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
          {readout.refresh} · {readout.size}
        </span>
        {readout.reach !== null ? (
          <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            {readout.reach}
          </span>
        ) : null}
      </div>
    );
  }

  return (
    <span className="inline-flex flex-wrap items-baseline gap-x-2 gap-y-1 text-[12px]" {...attrs}>
      <span style={{ color: "var(--m-text-secondary)" }}>{readout.window}</span>
      <Dot />
      <span style={{ color: "var(--m-text-secondary)" }}>{readout.refresh}</span>
      <Dot />
      <span style={{ color: "var(--m-text-secondary)" }}>{readout.size}</span>
      {readout.reach !== null ? (
        <>
          <Dot />
          <span style={{ color: "var(--m-text-muted)" }}>{readout.reach}</span>
        </>
      ) : null}
    </span>
  );
}

function Dot() {
  return (
    <span aria-hidden="true" style={{ color: "var(--m-text-muted)" }}>
      ·
    </span>
  );
}
