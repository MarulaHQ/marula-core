"use client";
/**
 * Ledger severity marks — lane 9, derived from the surface-pack rules.
 *
 * Purpose: the two severity pills the pack makes a contract for the loans
 * and tools surfaces:
 *
 * - OVERDUE IS RED (breach class), DUE SOON IS BLUSH (attention class), and
 *   NEITHER IS EVER COLOUR ALONE — each pill carries a word and an icon
 *   (DESIGN-NOTES-tickets §9 rule "colour is never alone", stated twice in
 *   the pack for loans and once in the derivation guide).
 * - The icons draw from the kit's Icon door at the sizes the bar uses.
 *
 * Non-purpose: no status logic lives here. Callers decide which mark a row
 * earns; this file only draws it the same way on every surface.
 */
import type { ReactNode } from "react";
import { Icon } from "@marula-core/ui/kit";
import { cn } from "@marula-core/ui";

export function AlertIcon({ className }: { className?: string }): React.JSX.Element {
  return <Icon name="alert" size={18} className={className} />;
}

export function ClockIcon({ className }: { className?: string }): React.JSX.Element {
  return <Icon name="clock" size={18} className={className} />;
}

export function CheckCircleIcon({ className }: { className?: string }): React.JSX.Element {
  return <Icon name="check" size={18} className={className} />;
}

export function OfflineDotIcon({ className }: { className?: string }): React.JSX.Element {
  return <Icon name="cloudoff" size={18} className={className} />;
}

export function MergeIcon({ className }: { className?: string }): React.JSX.Element {
  return <Icon name="merge" size={18} className={className} />;
}

/** The marks a bulk action wears: the pack's bar is icon + label, never
 * label alone, so an action is recognisable before it is read. */
export function OpenIcon(): React.JSX.Element {
  return <Icon name="ext" size={12} />;
}

export function CopyIcon(): React.JSX.Element {
  return <Icon name="copy" size={12} />;
}

export function ReturnIcon(): React.JSX.Element {
  return <Icon name="undo" size={12} />;
}

export function ReceiveIcon(): React.JSX.Element {
  return <Icon name="download" size={12} />;
}

export function MoveIcon(): React.JSX.Element {
  return <Icon name="down" size={12} />;
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
