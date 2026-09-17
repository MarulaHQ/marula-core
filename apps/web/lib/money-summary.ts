// Money-area shared helpers (Wave 17 lane 5) — status tone mappings and
// currency-honest aggregation for the /billing, /contracts, /sales surfaces.
//
// Money correctness: aggregate tiles sum stored minor-units ONLY within one
// currency; a mixed-currency selection renders "mixed currencies" instead of
// a fake total (docs/qa-money DESIGN-RESEARCH.md — never invent FX client-side).
import type { components } from "@meridian/sdk";

type Invoice = components["schemas"]["Invoice"];
type InvoiceStatus = components["schemas"]["InvoiceStatus"];
type DunningState = components["schemas"]["DunningState"];
type Contract = components["schemas"]["Contract"];
type ContractStatus = components["schemas"]["ContractStatus"];
type DealStatus = components["schemas"]["DealStatus"];
type QuoteStatus = components["schemas"]["QuoteStatus"];

export type Tone = "neutral" | "warning" | "danger" | "success" | "outline";

/** Lifecycle tone: paid success, void danger, issued neutral, draft outline. */
export function invoiceStatusTone(status: InvoiceStatus): Tone {
  if (status === "paid") return "success";
  if (status === "void") return "danger";
  if (status === "draft") return "outline";
  return "neutral";
}

/** Dunning escalation runs neutral → warning (reminder) → danger (overdue+). */
export function dunningTone(state: DunningState): Tone {
  if (state === "reminder_sent") return "warning";
  if (state === "overdue" || state === "final_notice" || state === "collections") return "danger";
  return "neutral";
}

export function contractStatusTone(status: ContractStatus): Tone {
  if (status === "active") return "success";
  if (status === "cancelled") return "danger";
  if (status === "draft") return "outline";
  return "neutral";
}

export const DEAL_STATUS_TONE: Record<DealStatus, "neutral" | "success" | "danger"> & object = {
  open: "neutral",
  won: "success",
  lost: "danger",
};

export const QUOTE_STATUS_TONE: Record<QuoteStatus, "neutral" | "success" | "warning" | "danger"> = {
  draft: "neutral",
  sent: "warning",
  accepted: "success",
  declined: "danger",
  expired: "danger",
};

/** Calendar-aware helper: days until an upcoming term boundary. */
export function daysUntil(dateStr: string, now: number = Date.now()): number | null {
  const t = new Date(`${dateStr}T00:00:00Z`).getTime();
  if (Number.isNaN(t)) return null;
  const today = new Date(now);
  today.setUTCHours(0, 0, 0, 0);
  return Math.round((t - today.getTime()) / 86_400_000);
}

/** Renewal-signal chip for a contract term: warning within 30d, danger past. */
export function termSignal(
  effectiveUntil: string | null | undefined,
  now: number = Date.now(),
): { label: string; tone: Tone } | null {
  if (effectiveUntil == null) return null;
  const days = daysUntil(effectiveUntil, now);
  if (days === null) return null;
  if (days < 0) return { label: `ended ${effectiveUntil}`, tone: "danger" };
  if (days <= 30) {
    return { label: days === 0 ? "ends today" : `ends in ${days}d`, tone: "warning" };
  }
  return null;
}

const moneyFmt = (minor: number, currency: string): string =>
  new Intl.NumberFormat(undefined, { style: "currency", currency }).format(minor / 100);

export function moneyDisplay(minor: number, currency: string): string {
  return moneyFmt(minor, currency);
}

/** Per-currency sum of stored amounts across an entity group. */
export function perCurrencySum(
  amounts: readonly { currency: string; amount: number }[],
): { currency: string | null; value: string } {
  if (amounts.length === 0) return { currency: null, value: "—" };
  const currencies: string[] = [...new Set(amounts.map((a) => a.currency as string))];
  const single = currencies.length === 1 ? (currencies[0] ?? null) : null;
  if (single === null) {
    return { currency: null, value: "mixed currencies" };
  }
  const total = amounts.reduce((acc, a) => acc + a.amount, 0);
  return { currency: single, value: moneyFmt(total, single) };
}
