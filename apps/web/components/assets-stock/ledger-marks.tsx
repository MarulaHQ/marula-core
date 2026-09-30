"use client";
/**
 * Ledger severity marks — lane 9, derived from the surface-pack rules.
 *
 * Purpose: one inline-SVG icon set plus the two severity pills the pack
 * makes a contract for the loans and tools surfaces:
 *
 * - OVERDUE IS RED (breach class), DUE SOON IS BLUSH (attention class), and
 *   NEITHER IS EVER COLOUR ALONE — each pill carries a word and an icon
 *   (DESIGN-NOTES-tickets §9 rule "colour is never alone", stated twice in
 *   the pack for loans and once in the derivation guide).
 * - The icons are inline SVG paths in the repo's 18px stroke style, matching
 *   the nav-rail icon set — no icon font, no dependency.
 *
 * Non-purpose: no status logic lives here. Callers decide which mark a row
 * earns; this file only draws it the same way on every surface.
 */
import type { ReactNode } from "react";
import { cn } from "@marula-core/ui";

export function AlertIcon({ className }: { className?: string }): React.JSX.Element {
  return (
    <svg
      aria-hidden="true"
      className={className}
      width="12"
      height="12"
      viewBox="0 0 12 12"
      fill="none"
    >
      <path
        d="M6 1.5 11 10H1L6 1.5Z"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinejoin="round"
      />
      <path d="M6 4.6v2.6" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
      <circle cx="6" cy="8.8" r="0.55" fill="currentColor" />
    </svg>
  );
}

export function ClockIcon({ className }: { className?: string }): React.JSX.Element {
  return (
    <svg
      aria-hidden="true"
      className={className}
      width="12"
      height="12"
      viewBox="0 0 12 12"
      fill="none"
    >
      <circle cx="6" cy="6" r="4.5" stroke="currentColor" strokeWidth="1.3" />
      <path d="M6 3.8V6l1.6 1.2" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
    </svg>
  );
}

export function CheckCircleIcon({ className }: { className?: string }): React.JSX.Element {
  return (
    <svg
      aria-hidden="true"
      className={className}
      width="12"
      height="12"
      viewBox="0 0 12 12"
      fill="none"
    >
      <circle cx="6" cy="6" r="4.5" stroke="currentColor" strokeWidth="1.3" />
      <path
        d="m4.2 6.2 1.3 1.3 2.4-2.6"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

export function OfflineDotIcon({ className }: { className?: string }): React.JSX.Element {
  return (
    <svg
      aria-hidden="true"
      className={className}
      width="12"
      height="12"
      viewBox="0 0 12 12"
      fill="none"
    >
      <circle cx="6" cy="6" r="2.4" stroke="currentColor" strokeWidth="1.3" />
      <path d="M2 2l8 8" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
    </svg>
  );
}

export function MergeIcon({ className }: { className?: string }): React.JSX.Element {
  return (
    <svg
      aria-hidden="true"
      className={className}
      width="12"
      height="12"
      viewBox="0 0 12 12"
      fill="none"
    >
      <path
        d="M3 1.5v2.2c0 1.6.9 2.5 2.4 3.1L8 8.1M9 1.5v2.2c0 1.2-.4 2-1.2 2.6M3 10.5V8.6M9 10.5V8.6"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinecap="round"
      />
      <circle cx="3" cy="9.6" r="1.1" stroke="currentColor" strokeWidth="1.2" />
      <circle cx="9" cy="9.6" r="1.1" stroke="currentColor" strokeWidth="1.2" />
    </svg>
  );
}

/** The marks a bulk action wears: the pack's bar is icon + label, never
 * label alone, so an action is recognisable before it is read. */
export function OpenIcon(): React.JSX.Element {
  return (
    <svg aria-hidden="true" width="12" height="12" viewBox="0 0 12 12" fill="none">
      <path d="M4.5 2.5h5v5" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M9.5 2.5 3 9" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
    </svg>
  );
}

export function CopyIcon(): React.JSX.Element {
  return (
    <svg aria-hidden="true" width="12" height="12" viewBox="0 0 12 12" fill="none">
      <rect x="4.2" y="4.2" width="5.6" height="5.6" rx="1.4" stroke="currentColor" strokeWidth="1.2" />
      <path d="M7.8 2.4H3.4a1.4 1.4 0 0 0-1.4 1.4v4.4" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
    </svg>
  );
}

export function ReturnIcon(): React.JSX.Element {
  return (
    <svg aria-hidden="true" width="12" height="12" viewBox="0 0 12 12" fill="none">
      <path d="M7.4 2.6 3.2 6l4.2 3.4" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M3.4 6h5.2a2.6 2.6 0 0 1 0 5.2H7" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
    </svg>
  );
}

export function ReceiveIcon(): React.JSX.Element {
  return (
    <svg aria-hidden="true" width="12" height="12" viewBox="0 0 12 12" fill="none">
      <path d="M6 1.8v5.4" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
      <path d="M3.6 4.8 6 7.2l2.4-2.4" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M2.2 8.4v1.2a1.4 1.4 0 0 0 1.4 1.4h4.8a1.4 1.4 0 0 0 1.4-1.4V8.4" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
    </svg>
  );
}

export function MoveIcon(): React.JSX.Element {
  return (
    <svg aria-hidden="true" width="12" height="12" viewBox="0 0 12 12" fill="none">
      <path d="M6 1.8v8.4" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
      <path d="M3.4 7.6 6 10.2l2.6-2.6" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M2.4 3.6h7.2" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
    </svg>
  );
}

/** Raised-severity pill shell: icon + words, tinted by tone. */
function SeverityPill({
  tone,
  icon,
  children,
  className,
  testId,
}: {
  tone: "red" | "blush" | "green" | "neutral";
  icon: ReactNode;
  children: string;
  className?: string;
  testId?: string;
}) {
  return (
    <span
      data-testid={testId}
      className={cn(
        "m-ledger-sev",
        tone === "red" && "m-ledger-sev-red",
        tone === "blush" && "m-ledger-sev-blush",
        tone === "green" && "m-ledger-sev-green",
        tone === "neutral" && "m-ledger-sev-neutral",
        className,
      )}
    >
      {icon}
      <span>{children}</span>
    </span>
  );
}

/** A genuinely overdue return: the loans view's RED, never used for anything softer. */
export function OverdueMark({
  label,
  className,
  testId,
}: {
  label: string;
  className?: string;
  testId?: string;
}) {
  return (
    <SeverityPill
      tone="red"
      icon={<AlertIcon />}
      className={className}
      testId={testId}
    >
      {label}
    </SeverityPill>
  );
}

/** Due inside the warning window: blush, with a clock and the words. */
export function DueSoonMark({
  label,
  className,
  testId,
}: {
  label: string;
  className?: string;
  testId?: string;
}) {
  return (
    <SeverityPill
      tone="blush"
      icon={<ClockIcon />}
      className={className}
      testId={testId}
    >
      {label}
    </SeverityPill>
  );
}

/** Returned / healthy: green with a check and the words. */
export function ReturnedMark({
  label,
  className,
  testId,
}: {
  label: string;
  className?: string;
  testId?: string;
}) {
  return (
    <SeverityPill
      tone="green"
      icon={<CheckCircleIcon />}
      className={className}
      testId={testId}
    >
      {label}
    </SeverityPill>
  );
}

/** Quiet neutral state that still wants its icon: held, in stock, idle. */
export function NeutralMark({
  label,
  icon,
  className,
  testId,
}: {
  label: string;
  icon?: ReactNode;
  className?: string;
  testId?: string;
}) {
  return (
    <SeverityPill
      tone="neutral"
      icon={icon}
      className={className}
      testId={testId}
    >
      {label}
    </SeverityPill>
  );
}

/** A due/overdue phrase that resolves to the mark the pack draws. */
export function loanDueMark(
  dueAt: string,
  overdue: boolean,
  phrase: string,
  testId?: string,
): ReactNode {
  if (overdue) {
    return <OverdueMark label={`Overdue · ${phrase}`} testId={testId} />;
  }
  return <DueSoonMark label={`Due ${phrase}`} testId={testId} />;
}
