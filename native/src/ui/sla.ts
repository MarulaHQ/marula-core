// SLA urgency, shared by every list that shows a due clock.
//
// Purpose: one rule for when a row earns a pill. A far-out due date shows no
// pill at all (a neutral chip on every row reads as status noise and
// collides with the status word already in the subtitle). Overdue and
// due-soon are the two states worth color. Pure and injectable-clock so the
// rule is unit-tested without a device.
//
// Non-purpose: no formatting of the underlying timestamp.

export type SlaSeverity = "overdue" | "due_soon";

export interface SlaState {
  severity: SlaSeverity;
  label: string;
}

export const DUE_SOON_WINDOW_MS = 4 * 60 * 60 * 1000;

export function slaState(dueAt: string | null | undefined, now: number): SlaState | undefined {
  if (dueAt === null || dueAt === undefined) return undefined;
  const due = Date.parse(dueAt);
  if (Number.isNaN(due)) return undefined;
  const delta = due - now;
  if (delta < 0) return { severity: "overdue", label: "OVERDUE" };
  if (delta < DUE_SOON_WINDOW_MS) return { severity: "due_soon", label: "DUE SOON" };
  return undefined;
}
