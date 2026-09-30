// MergeBanner — a ticket that has been folded into another one.
//
// Purpose: say what happened, once, in the place the user is already looking,
// and offer the way back. A merged ticket stays queryable by doctrine (the row
// is closed and never re-opens), so the alternative to a banner is a ticket
// that looks live and silently is not.
//
// Tone: the neutral/sand pair. A merge is history, not a failure — it is not
// red, and it is not a warning, because nothing is at risk and nothing is due.
// The destructive pair belongs to a genuine breach, P1 and destructive actions
// only (DESIGN-NOTES §5).
"use client";

import { cn } from "@marula-core/ui";
import Link from "next/link";

export interface MergeBannerProps {
  /** The ticket that survived the merge. */
  survivorId: string;
  /** The survivor's number, when the surface knows it. */
  survivorLabel?: string | null;
  /** Shown instead of the link when the surviving ticket is not in view. */
  unreachableHint?: string;
  className?: string;
}

export function MergeBanner({
  survivorId,
  survivorLabel,
  unreachableHint = "The surviving ticket is not in this view.",
  className,
}: MergeBannerProps): React.JSX.Element {
  return (
    <div
      className={cn("m-merge-banner", className)}
      role="status"
      data-testid="merge-banner"
    >
      <span className="m-merge-banner-icon" aria-hidden="true">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <path d="M6 3v6a4 4 0 0 0 4 4h8" strokeLinecap="round" strokeLinejoin="round" />
          <path d="M15 10l4 4-4 4" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </span>
      <span className="m-merge-banner-text">
        This ticket was merged and is closed. Its history is kept.
      </span>
      {survivorLabel ? (
        <Link href={`/tickets/${survivorId}`} className="m-merge-banner-link">
          Open {survivorLabel}
        </Link>
      ) : (
        <span className="m-merge-banner-link is-muted">{unreachableHint}</span>
      )}
    </div>
  );
}
