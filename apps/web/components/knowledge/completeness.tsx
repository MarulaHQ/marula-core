"use client";
/**
 * CompletenessBlock — "6 of 8 documented", with the reasons underneath.
 *
 * Purpose: the brief's rule that a client's documentation completeness is a
 * count with a reason, never a bare percentage. The count says how far along
 * the client is; the list says what to do next, in the order the legs are
 * declared server-side.
 *
 * Non-purpose: no scoring, no fetching, no colour-only signal. A full set is a
 * green word, not a green bar alone.
 */

import type { Completeness } from "./knowledge-api";
import { CheckIcon } from "./ledger";

export function CompletenessBlock({
  value,
  compact = false,
}: {
  value: Completeness;
  /** Compact drops the gap list: the client hub prints it in full. */
  compact?: boolean;
}): React.JSX.Element {
  const done = value.complete === value.total;
  return (
    <div className="flex flex-col gap-2" data-testid="knowledge-completeness">
      <p className="flex flex-wrap items-baseline gap-x-2 text-[13px]">
        <span className="text-[20px] font-semibold tabular-nums">{value.complete}</span>
        <span style={{ color: "var(--m-text-secondary)" }}>of {value.total} documentation legs</span>
        {done ? (
          <span
            className="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[12px] font-semibold"
            style={{
              background: "var(--m-status-success-bg)",
              color: "var(--m-status-success-fg)",
            }}
          >
            <CheckIcon size={12} />
            Complete
          </span>
        ) : null}
      </p>
      {!compact && value.gaps.length > 0 ? (
        <ul className="flex flex-col gap-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {value.gaps.map((gap) => (
            <li key={gap} className="flex items-start gap-2">
              <span
                aria-hidden="true"
                className="mt-1.5 inline-block h-1.5 w-1.5 shrink-0 rounded-full"
                style={{ background: "var(--m-blush)" }}
              />
              <span>{gap}</span>
            </li>
          ))}
        </ul>
      ) : null}
      {!compact && done ? (
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Every leg has at least one record. The next step is keeping them current, not adding more.
        </p>
      ) : null}
    </div>
  );
}
