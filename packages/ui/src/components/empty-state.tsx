"use client";
/**
 * EmptyState — one sentence plus one action. No illustrations (theme.md §6).
 */
import type { ReactNode } from "react";

export function EmptyState({
  message,
  action,
}: {
  message: string;
  /** Single primary action button for the empty state. */
  action?: ReactNode;
}): React.JSX.Element {
  return (
    <div className="m-empty">
      <p>{message}</p>
      {action}
    </div>
  );
}
