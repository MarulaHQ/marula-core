"use client";
/**
 * SurfaceFooter — the data-surface template's 52px strip.
 *
 * Purpose: state what the reader is looking at, and offer the next rows.
 * `DESIGN-NOTES-tickets.md` §3 puts four things on this strip and gives it a
 * fixed height: the count as `**{shown}** of **{total}** {noun}`, a
 * `Load N more` link button when more rows exist, a density slot, and the J/K
 * keyboard hints. §11 makes it a law for every surface that copies the template,
 * which is most of them.
 *
 * It exists because it was the one part of the template the kit had no component
 * for, and every surface that needed it re-derived it: `LedgerFooter` in
 * `apps/web/components/knowledge/ledger.tsx`, a hand-built `m-table-foot` in
 * `app/tickets/page.tsx`, a second footer in `app/boards/page.tsx`, a third
 * shape in the asset ledgers, and "Load 50 more" written out twice by hand in
 * `audit` and `provenance`. Four spellings of one strip is how the surfaces
 * drifted.
 *
 * Two honesty rules, because a count is a claim:
 *
 * 1. `total` is optional, and when it is absent this prints `N {noun}` rather
 *    than inventing a denominator. A list that knows only "there is more" must
 *    not print "12 of 40,213" and imply it counted the queue.
 * 2. The load-more button appears exactly when the caller can serve another
 *    page. A button that cannot move the list is worse than no button.
 *
 * Non-purpose: this fetches nothing and decides nothing. The caller owns the
 * cursor, the rows and the density preference, because the same footer has to
 * sit under a server-paged table and a fully loaded one.
 */
import type { ReactNode } from "react";
import { cn } from "../lib/cn";
import { formatNumber } from "../lib/number";

/** One keyboard pair, the way the pack prints them: `J K` + `move`. */
export interface SurfaceFooterHint {
  /** Space-separated keycaps: "J K", "X", "F". */
  keys: string;
  label: string;
}

export interface SurfaceFooterProps {
  /** Rows the surface is currently showing. */
  shown: number;
  /**
   * The real total, when the surface knows it. Omit it when the API reports
   * only "there is another page" — the footers that guess a denominator are the
   * reason this prop is optional rather than required.
   */
  total?: number;
  /** Singular noun, e.g. "ticket", "entry", "article". */
  noun: string;
  /** Override the plural when "…s" is wrong, e.g. "entries". */
  nounPlural?: string;
  /** Present when another page exists. Omit when the walk is exhausted. */
  onLoadMore?: () => void;
  /** The next page's size, for the button's own label. Defaults to 50. */
  loadMoreCount?: number;
  /** True while the next page is in flight. */
  loadingMore?: boolean;
  /** Overrides the whole button label when the shape is unusual. */
  loadMoreLabel?: string;
  /** The density control. This is where `DensitySwitch` belongs. */
  density?: ReactNode;
  /** The J/K chips. Rendered after the density control, as the pack draws them. */
  hints?: ReadonlyArray<SurfaceFooterHint>;
  /** Anything else that belongs on the strip, before the spacer. */
  children?: ReactNode;
  /** Right-aligned trailing content, after the hints. */
  trailing?: ReactNode;
  /** `data-testid` on the count itself, for the surfaces whose tests read it. */
  countTestId?: string;
  className?: string;
  /** Announced to assistive tech as the strip's purpose. */
  label?: string;
}

export function SurfaceFooter({
  shown,
  total,
  noun,
  nounPlural,
  onLoadMore,
  loadMoreCount = 50,
  loadingMore = false,
  loadMoreLabel,
  density,
  hints,
  children,
  trailing,
  countTestId,
  className,
  label = "List footer",
}: SurfaceFooterProps): React.JSX.Element {
  const many = nounPlural ?? `${noun}s`;
  const word = shown === 1 ? noun : many;
  return (
    <div className={cn("m-surface-footer", className)} role="group" aria-label={label}>
      <span className="m-surface-footer-count" data-testid={countTestId}>
        {/* Real spaces, not only the flex gap: a screen reader hears
            "12 of 12 tickets", and a reader that drops the gap still reads
            words rather than "12of12tickets". */}
        <b className="m-surface-footer-num">{formatNumber(shown)}</b>{" "}
        {total === undefined ? null : (
          <>
            of <b className="m-surface-footer-num">{formatNumber(total)}</b>{" "}
          </>
        )}
        {word}
      </span>
      {onLoadMore === undefined ? null : (
        <button
          type="button"
          className="m-surface-footer-load"
          onClick={onLoadMore}
          disabled={loadingMore}
          aria-busy={loadingMore}
        >
          {loadingMore
            ? "Loading…"
            : (loadMoreLabel ?? `Load ${formatNumber(loadMoreCount)} more`)}
        </button>
      )}
      {children}
      <span className="m-surface-footer-gap" />
      {density}
      {hints === undefined || hints.length === 0 ? null : (
        <div className="m-surface-footer-hints">
          {hints.map((hint) => (
            <span key={`${hint.keys}:${hint.label}`} className="m-surface-footer-hint">
              {hint.keys
                .split(" ")
                .filter(Boolean)
                .map((key, i) => (
                  <kbd key={`${key}-${String(i)}`} className="m-surface-footer-kbd">
                    {key}
                  </kbd>
                ))}
              <span>{hint.label}</span>
            </span>
          ))}
        </div>
      )}
      {trailing}
    </div>
  );
}
