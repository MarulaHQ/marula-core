// Renewal and severity signals for the client, project and contract records
// (Wave 41 lane 7 `p41-ui-clients`).
//
// Purpose: one place that decides how a date reads, so a renewal is the same
// pill on the client list, the client's contract summary, the contract's own
// record and the projects ledger. The rule the pack fixes and this module
// carries:
//
//   **A renewal is blush, never red.** A term inside the renewal window is
//   attention, not a breach — red is reserved for a genuine SLA breach, a P1
//   and a destructive action. An *elapsed* term is different: the contract
//   has run past its end, which is a real commercial event, so that one is
//   allowed red, and only once it is actually past.
//
// Two windows, both stated on screen rather than implied:
//   - `RENEWAL_WINDOW_DAYS` (90) is the attention window. It drives the
//     list's blush pill and the header shortcut of the same name.
//   - `RENEWAL_LEAD_DAYS` (60) is the operational one: the renewal quote
//     should be out this long before the end. The contract tab prints it as
//     a date, because "a renewal is missed when the lead time is not written
//     down" (`DESIGN-NOTES-clients.md` §3).
//
// Non-purpose: this does not decide whether a contract should renew. It reads
// dates it is given and returns words plus a tone, because colour is never
// alone in this product (DERIVATION-GUIDE rule 6).

const DAY_MS = 86_400_000;

/** The attention window. A term inside it is worth acting on today. */
export const RENEWAL_WINDOW_DAYS = 90;

/** How long before the end a renewal quote should be out. */
export const RENEWAL_LEAD_DAYS = 60;

export type RenewalTone = "due" | "warning" | "danger" | null;

export interface RenewalSignal {
  tone: RenewalTone;
  /** The pill's words. Always carries the number, never colour alone. */
  label: string;
  /** Whole days to the end. Negative once the term has elapsed. */
  days: number;
}

/** Calendar-aware day count: dates are calendar days, not 24-hour spans. */
export function daysUntil(dateStr: string, now: number = Date.now()): number | null {
  const t = new Date(`${dateStr}T00:00:00Z`).getTime();
  if (Number.isNaN(t)) return null;
  const today = new Date(now);
  today.setUTCHours(0, 0, 0, 0);
  return Math.round((t - today.getTime()) / DAY_MS);
}

/**
 * The renewal signal for one term end, in the pack's shape.
 *
 * `effectiveUntil` null means an open-ended term: there is nothing to renew
 * and nothing to say, so this returns null and the surface prints "open".
 */
export function renewalSignal(
  effectiveUntil: string | null | undefined,
  now: number = Date.now(),
): RenewalSignal | null {
  if (effectiveUntil == null || effectiveUntil === "") return null;
  const days = daysUntil(effectiveUntil, now);
  if (days === null) return null;
  if (days < 0) {
    return { tone: "danger", label: `ended ${Math.abs(days)}d ago`, days };
  }
  if (days === 0) return { tone: "due", label: "ends today", days };
  if (days <= RENEWAL_WINDOW_DAYS) {
    return { tone: "due", label: `${days} days left`, days };
  }
  return null;
}

/** Whether a term end is inside the attention window, for the list's own
 * filter without needing to read the pill. */
export function insideRenewalWindow(
  effectiveUntil: string | null | undefined,
  now: number = Date.now(),
): boolean {
  const signal = renewalSignal(effectiveUntil, now);
  return signal !== null && signal.days >= 0 && signal.days <= RENEWAL_WINDOW_DAYS;
}

/**
 * The date a renewal quote should go out: `RENEWAL_LEAD_DAYS` before the
 * term ends. Printed as a real date so the lead time is written down rather
 * than assumed. Null when there is no term end to count back from.
 */
export function renewalQuoteDueDate(
  effectiveUntil: string | null | undefined,
): string | null {
  if (effectiveUntil == null || effectiveUntil === "") return null;
  const t = new Date(`${effectiveUntil}T00:00:00Z`);
  if (Number.isNaN(t.getTime())) return null;
  t.setUTCDate(t.getUTCDate() - RENEWAL_LEAD_DAYS);
  return t.toISOString().slice(0, 10);
}

/** A date for the pack's surface: `12 Dec 2026`. Not locale-dependent, so a
 * captured surface and the pack read identically. */
export function formatPackDate(iso: string | null | undefined): string | null {
  if (iso == null || iso === "") return null;
  const MONTHS = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun",
    "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
  ];
  const d = new Date(`${iso.slice(0, 10)}T00:00:00Z`);
  if (Number.isNaN(d.getTime())) return null;
  return `${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]} ${d.getUTCFullYear()}`;
}

/** "Mar 2021" — the month a relationship started, for the record header. */
export function formatPackMonth(iso: string | null | undefined): string | null {
  if (iso == null || iso === "") return null;
  const MONTHS = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun",
    "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
  ];
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return null;
  return `${MONTHS[d.getUTCMonth()]} ${d.getFullYear()}`;
}

/** A plain-language project posture: at risk is BLUSH, overdue is RED only
 * once the due date has actually passed, and the words are in both cases. */
export function projectPosture(
  dueDate: string | null | undefined,
  status: string,
  now: number = Date.now(),
): { tone: RenewalTone; label: string } | null {
  if (dueDate == null || dueDate === "") return null;
  if (status === "completed" || status === "cancelled") return null;
  const days = daysUntil(dueDate, now);
  if (days === null) return null;
  if (days < 0) return { tone: "danger", label: `overdue ${Math.abs(days)}d` };
  if (days === 0) return { tone: "due", label: "due today" };
  if (days <= 14) return { tone: "due", label: `due in ${days}d` };
  return null;
}