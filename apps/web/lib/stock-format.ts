// Stock helpers (Wave 32 lane 4): quantity milli math, movement-kind
// labels, and status tones shared by the /stock, /purchasing, tools, and
// loans surfaces.
//
// Non-purpose: no API calls and no component state — pure formatting only,
// so every surface renders the same number the same way.

/** 1500 milli of a metre reads "1.5 m"; whole counts read "3 each". */
export function formatQtyMilli(qtyMilli: number, unit: string): string {
  const value = qtyMilli / 1000;
  const text = Number.isInteger(value)
    ? String(value)
    : value.toFixed(3).replace(/0+$/, "").replace(/\.$/, "");
  return unit === "" ? text : `${text} ${unit}`;
}

/** Parses a user-typed quantity ("1.5", "3") into milli; null when invalid. */
export function parseQtyToMilli(input: string): number | null {
  const trimmed = input.trim();
  if (trimmed === "") return null;
  const value = Number(trimmed);
  if (Number.isNaN(value) || value < 0) return null;
  return Math.round(value * 1000);
}

/** Signed quantity for ledger rows: "+12 each" / "−3 each". */
export function formatSignedQty(qtyMilli: number, unit: string): string {
  const sign = qtyMilli >= 0 ? "+" : "−";
  return `${sign}${formatQtyMilli(Math.abs(qtyMilli), unit)}`;
}

export const MOVEMENT_KIND_LABELS: Record<string, string> = {
  receive: "Received",
  consume: "Used",
  adjust: "Adjusted",
  transfer: "Transferred",
  "ship-out": "Shipped out",
  checkout: "Checked out",
  checkin: "Checked in",
  loan: "Loaned out",
  loan_return: "Loan returned",
};

export function movementLabel(kind: string): string {
  return MOVEMENT_KIND_LABELS[kind] ?? kind;
}

export type Tone = "success" | "warning" | "danger" | "neutral";

export function serialStatusTone(status: string): Tone {
  switch (status) {
    case "in_stock":
      return "success";
    case "checked_out":
    case "loaned":
      return "neutral";
    case "out":
      return "warning";
    default:
      return "neutral";
  }
}

export function serialStatusLabel(status: string): string {
  switch (status) {
    case "in_stock":
      return "in stock";
    case "checked_out":
      return "checked out";
    case "loaned":
      return "on loan";
    case "out":
      return "out";
    default:
      return status;
  }
}

export function checkoutStatusTone(status: string, overdue: boolean): Tone {
  if (overdue) return "danger";
  switch (status) {
    case "open":
      return "neutral";
    case "returned":
      return "success";
    default:
      return "neutral";
  }
}

export function loanStatusTone(status: string, overdue: boolean): Tone {
  if (overdue) return "danger";
  switch (status) {
    case "open":
      return "neutral";
    case "returned":
      return "success";
    default:
      return "neutral";
  }
}

/** "in 3 days" / "2 days overdue" / "today" for due dates (date or RFC3339). */
export function duePhrase(dueAt: string, now: Date = new Date()): string {
  const due = new Date(dueAt.length === 10 ? `${dueAt}T23:59:59` : dueAt);
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const startOfDue = new Date(due.getFullYear(), due.getMonth(), due.getDate());
  const days = Math.round((startOfDue.getTime() - startOfToday.getTime()) / 86_400_000);
  if (days === 0) return "today";
  if (days === 1) return "tomorrow";
  if (days === -1) return "1 day overdue";
  if (days < 0) return `${Math.abs(days)} days overdue`;
  return `in ${days} days`;
}

/** End-of-day RFC3339 for a date input value (`YYYY-MM-DD`). */
export function endOfDayIso(dateValue: string): string {
  const date = new Date(`${dateValue}T23:59:59`);
  return date.toISOString();
}

/** Today + N days as a `YYYY-MM-DD` input value. */
export function dateInputValue(offsetDays = 0): string {
  const d = new Date();
  d.setDate(d.getDate() + offsetDays);
  return d.toISOString().slice(0, 10);
}
