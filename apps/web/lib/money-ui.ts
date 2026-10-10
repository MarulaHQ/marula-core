// Money-cluster UI grammar (Wave 41 lane 8) — the words and numbers the
// money surfaces speak, taken from the Marula surface pack's approvals
// reference (brand/packs/core-surface-pack/DESIGN-NOTES-approvals.md §3–§7
// and source/approvals.html) and the quoting surface
// (DESIGN-NOTES-quoting.md).
//
// Purpose: the cluster is one family, so the sub-navigation order, the H:MM
// duration grammar, the ageing buckets, the colour meaning of a money status
// and the "look fine" bulk-pass rule each live in ONE place and every route
// reads them. A wrong colour or a wrong duration on a money surface costs real
// money, which is why this is a lib with tests rather than markup repeated
// seven times.
//
// Non-purpose: no formatting of amounts (that is lib/format.ts `money`, and it
// stays the only one), no fetch, no JSX — the components read this and paint.
// The colour meaning itself lives in the --m-* tokens; this only names it.

/**
 * The money sub-navigation.
 *
 * The quoting surface names five money documents in one category — quotes,
 * contracts, time approvals, invoices, credit notes (`DESIGN-NOTES-quoting.md`)
 * — and says outright that "the final navigation is not decided" and that "the
 * approvals screen should adopt the same tab row once navigation is settled".
 * The approvals surface, which its own notes call the reference for the whole
 * money cluster, ships the row below plus Pipeline and Catalogue. Two canonical
 * references therefore disagree, and the citing note defers to a decision that
 * has not happened, so this row stays as the approvals reference draws it.
 *
 * One reconciliation was tried and reverted: adding Contracts, the quoting
 * surface's second money document. `/contracts` and `/contracts/[id]` are
 * owned by the clients lane and wear neither `MoneyHeader` nor this
 * sub-navigation, so joining them would put a money nav item on a page that
 * drops the cluster's navigation — a dead end. When contracts adopts the money
 * frame it joins this row; until then it is a cross-lane change, not a
 * fidelity fix.
 */
export type MoneyItemId =
  | "approvals"
  | "invoices"
  | "credit-notes"
  | "quotes"
  | "pipeline"
  | "catalogue";

export interface MoneyNavItem {
  id: MoneyItemId;
  label: string;
  href: string;
}

export const MONEY_NAV: readonly MoneyNavItem[] = [
  { id: "approvals", label: "Approvals", href: "/approvals" },
  { id: "invoices", label: "Invoices", href: "/billing" },
  { id: "credit-notes", label: "Credit notes", href: "/billing/credit-notes" },
  { id: "quotes", label: "Quotes", href: "/sales/quotes" },
  { id: "pipeline", label: "Pipeline", href: "/sales/pipeline" },
  { id: "catalogue", label: "Catalogue", href: "/sales/catalog" },
];

/** Which sub-nav item a pathname belongs to. Invoice detail is still Invoices. */
export function moneyItemFor(pathname: string): MoneyItemId | null {
  if (pathname === "/approvals") return "approvals";
  if (pathname === "/billing" || pathname.startsWith("/billing/")) {
    return pathname.startsWith("/billing/credit-notes") ? "credit-notes" : "invoices";
  }
  if (pathname === "/sales/quotes" || pathname.startsWith("/sales/quotes/")) return "quotes";
  if (pathname === "/sales/pipeline") return "pipeline";
  if (pathname === "/sales/catalog") return "catalogue";
  return null;
}

/**
 * The pack's duration grammar: H:MM, hours zero-padded, so a column of them
 * lines up on the colon (19:15, 2:00, 0:45). A spot above four hours is
 * written as hours plus a day count, because 27:30 is not a duration anyone
 * reads at a glance.
 */
export function hms(minutes: number): string {
  const total = Number.isFinite(minutes) ? Math.max(0, Math.round(minutes)) : 0;
  const h = Math.floor(total / 60);
  const m = total % 60;
  return `${h}:${m.toString().padStart(2, "0")}`;
}

/** Whole days from a date to `now`, negative when the date has passed. */
export function daysPast(dateIso: string | null | undefined, now: number = Date.now()): number | null {
  if (dateIso === null || dateIso === undefined || dateIso === "") return null;
  const t = new Date(`${dateIso}T00:00:00Z`).getTime();
  if (Number.isNaN(t)) return null;
  const today = new Date(now);
  today.setUTCHours(0, 0, 0, 0);
  return Math.round((today.getTime() - t) / 86_400_000);
}

/**
 * Ageing buckets for a money row. Overdue is blush, not red (pack §7 and the
 * cluster's colour table): a sent invoice running late is a fact about money,
 * not a breach, and red stays reserved for a breached SLA.
 */
export type AgeingBucket = "overdue" | "due-soon" | "current" | "settled" | "none";

export interface AgeingInput {
  /** Issued invoices only carry a due-date bucket; drafts and voids never do. */
  status: string;
  dueOn: string | null | undefined;
  /** Balance still owed. A settled invoice is not "due soon", it is settled. */
  unpaidMinor: number;
  /** Due soon means inside this many days. */
  dueSoonWithinDays?: number;
  now?: number;
}

export function ageingBucket({
  status,
  dueOn,
  unpaidMinor,
  dueSoonWithinDays = 3,
  now = Date.now(),
}: AgeingInput): AgeingBucket {
  if (status === "paid" || status === "void") return "settled";
  if (status !== "issued") return "none";
  if (unpaidMinor <= 0) return "settled";
  const days = daysPast(dueOn, now);
  if (days === null) return "none";
  if (days > 0) return "overdue";
  if (days >= -dueSoonWithinDays) return "due-soon";
  return "current";
}

/** The words on the pill: "Overdue 12 days", "Due in 2 days", "Due today". */
export function ageingLabel(
  dueOn: string | null | undefined,
  bucket: AgeingBucket,
  now: number = Date.now(),
): string | null {
  if (bucket === "settled" || bucket === "none") return null;
  const days = daysPast(dueOn, now);
  if (days === null) return null;
  if (bucket === "overdue") {
    return days === 0 ? "Overdue today" : `Overdue ${days} ${days === 1 ? "day" : "days"}`;
  }
  if (days === 0) return "Due today";
  if (days === -1) return "Due tomorrow";
  return `Due in ${-days} days`;
}

/** A money status, in the pack's order of risk (pack §7 colour table). */
export type MoneyStatus =
  | "draft"
  | "issued"
  | "paid"
  | "void"
  | "part-paid"
  | "open"
  | "sent"
  | "accepted"
  | "declined"
  | "expired";

export interface MoneyPill {
  label: string;
  /** The meaning, named as a token family — never a hex. */
  family: "neutral" | "gold" | "blush" | "green";
  /** Pack: Paid carries a check, Overdue carries a clock. */
  icon?: "check" | "clock";
}

/**
 * Invoice status → its pill. Sent is a gold tint, Paid is green, and an
 * invoice with money against it reads Part paid rather than Sent, because the
 * collection posture is not the same one. Anything still fully owed keeps gold
 * here and lets the AGEING pill beside it carry the late signal in blush.
 */
export function invoicePill(status: string, paidMinor: number, totalMinor: number): MoneyPill {
  if (status === "paid") return { label: "Paid", family: "green", icon: "check" };
  if (status === "void") return { label: "Void", family: "neutral" };
  if (status === "draft") return { label: "Draft", family: "neutral" };
  if (paidMinor > 0 && paidMinor < totalMinor) return { label: "Part paid", family: "gold" };
  return { label: "Sent", family: "gold" };
}

/** Quote lifecycle: the same pill rules, one step earlier. An expired quote is
 *  money that lapsed, so it takes blush like any other lapsed amount — red is
 *  never the expiry colour. */
export function quotePill(status: string): MoneyPill {
  switch (status) {
    case "draft":
      return { label: "Draft", family: "neutral" };
    case "sent":
      return { label: "Sent", family: "gold" };
    case "accepted":
      return { label: "Accepted", family: "green", icon: "check" };
    case "declined":
      return { label: "Declined", family: "neutral" };
    case "expired":
      return { label: "Expired", family: "blush" };
    default:
      return { label: status, family: "neutral" };
  }
}

/** Deal stages, for the pipeline board headers. */
export function dealPill(status: string): MoneyPill {
  if (status === "won") return { label: "Won", family: "green", icon: "check" };
  if (status === "lost") return { label: "Lost", family: "neutral" };
  return { label: "Open", family: "neutral" };
}

/**
 * One entry's share of a bulk pass, as a plain rule (pack §4): not your own
 * time, a real description, no longer than four hours, and — when the audit
 * has been granted — not one the audit flagged. It is a rule, not a model: it
 * works with no grant at all, and the pack says so on purpose.
 */
export function looksFine(entry: {
  own: boolean;
  minutes: number;
  notes: string | null | undefined;
  /** True when the audit granted a verdict AND it flagged this entry. */
  flagged?: boolean;
  maxMinutes?: number;
  minNoteLength?: number;
}): boolean {
  const maxMinutes = entry.maxMinutes ?? 240;
  const minNoteLength = entry.minNoteLength ?? 20;
  if (entry.own) return false;
  if (entry.flagged === true) return false;
  if (entry.minutes > maxMinutes) return false;
  const notes = (entry.notes ?? "").trim();
  return notes.length > minNoteLength;
}

/** Why an entry was not in the pass, in one short clause for the pane. */
export function notFineReason(entry: {
  own: boolean;
  minutes: number;
  notes: string | null | undefined;
  flagged?: boolean;
}): string {
  if (entry.own) return "It is your own time — another approver signs that off.";
  if (entry.flagged === true) return "The audit flagged it, so it waits for a person.";
  if (entry.minutes > 240) return `It is ${hms(entry.minutes)} long — a long entry wants a look.`;
  if ((entry.notes ?? "").trim().length <= 20) return "There is not enough written to sign off blind.";
  return "It is not in the pass.";
}

/**
 * The live note under a payment amount (pack §7): paying the balance closes the
 * invoice, paying less leaves it open with the new balance spelled out. The
 * approver never has to do the arithmetic to know which one they are about to
 * trigger.
 */
export function payNote(
  amountMinor: number,
  balanceMinor: number,
  formatAmount: (minor: number) => string,
): { settles: boolean; text: string } {
  const after = Math.max(0, balanceMinor - amountMinor);
  if (after === 0) {
    return {
      settles: true,
      text: "The invoice will be marked Paid. A payment can be reversed from the invoice.",
    };
  }
  return {
    settles: false,
    text: `Balance after this payment: ${formatAmount(after)}. The invoice stays open.`,
  };
}

/** The over-balance block, worded as what to do instead (pack §7). */
export function overBalanceMessage(
  amountMinor: number,
  balanceMinor: number,
  formatAmount: (minor: number) => string,
): string | null {
  if (amountMinor <= 0) return "Enter the amount received.";
  if (amountMinor > balanceMinor) {
    return `That is more than the balance of ${formatAmount(balanceMinor)}. Record the balance, then a credit for the rest.`;
  }
  return null;
}

/**
 * The reject reasons an approver picks, in plain English, each mapped to the
 * typed code the queue already POSTs (pack §5). The tech reads the label, the
 * API reads the code, and neither drifts.
 */
export const REJECT_REASONS: readonly { code: string; label: string }[] = [
  { code: "missing_notes", label: "Description is too vague" },
  { code: "duplicate_entry", label: "Time overlaps another entry" },
  { code: "incorrect_duration", label: "Longer than the work justifies" },
  { code: "wrong_ticket", label: "Wrong ticket or client" },
  { code: "non_billable_work", label: "Should not be billable" },
  { code: "policy_violation", label: "Breaks a policy" },
  { code: "other", label: "Other (explain in the note)" },
];

export function rejectReasonLabel(code: string | null | undefined): string {
  return REJECT_REASONS.find((r) => r.code === code)?.label ?? (code ?? "Rejected");
}

/** The unpaid-reason vocabulary, as the tech wrote it in words. */
export const UNBILLABLE_REASON_LABELS: Readonly<Record<string, string>> = {
  warranty: "covered by warranty",
  courtesy: "courtesy / goodwill",
  in_retainer_block: "inside a retainer block",
  sla_covered: "covered by the SLA",
  internal: "internal work",
  mistake: "mistake — do not bill",
};
