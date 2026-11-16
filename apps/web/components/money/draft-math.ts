// Draft arithmetic for the quote and contract builders.
//
// Purpose: ONE implementation of the preview the builder shows, so the header,
// the summary column, the change-proposal card and the tests cannot disagree
// about a total. Quantities are milli-units and prices are minor units, and the
// rounding is half-to-even, matching `quote_lines.amount_minor` on the server.
//
// Non-purpose: it never decides a price. The catalogue, the person and the
// draft supply those; this only multiplies and adds.

import type { ContractDraft, ContractPriceLine, DraftLine, QuoteDraft } from "../../lib/money-authoring";

/** Quantity (milli) × price (minor), half-to-even, positive operands. */
export function lineAmountMinor(quantityMilli: number, unitPriceMinor: number): number {
  const quantity = Math.max(0, Math.round(quantityMilli));
  const price = Math.max(0, Math.round(unitPriceMinor));
  const numerator = quantity * price;
  const quotient = Math.floor(numerator / 1000);
  const remainder = numerator - quotient * 1000;
  if (remainder * 2 > 1000) return quotient + 1;
  if (remainder * 2 === 1000 && quotient % 2 !== 0) return quotient + 1;
  return quotient;
}

export function quoteSubtotal(lines: DraftLine[]): number {
  return lines
    .filter((line) => line.optional !== true)
    .reduce((sum, line) => sum + lineAmountMinor(line.quantity_milli, line.unit_price_minor), 0);
}

/** Optional extras: priced, shown, and deliberately outside the total. */
export function quoteOptionalTotal(lines: DraftLine[]): number {
  return lines
    .filter((line) => line.optional === true)
    .reduce((sum, line) => sum + lineAmountMinor(line.quantity_milli, line.unit_price_minor), 0);
}

export function taxOf(taxableMinor: number, taxRateBps: number): number {
  const rate = Math.max(0, Math.round(taxRateBps));
  return Math.round((Math.max(0, taxableMinor) * rate) / 10000);
}

export type QuoteTotals = {
  subtotalMinor: number;
  taxMinor: number;
  totalMinor: number;
  optionalMinor: number;
};

export function quoteTotals(lines: DraftLine[], taxRateBps: number): QuoteTotals {
  // Taxable is per line on the create route; a describing draft has no
  // taxability flag, so every line is taxable and the rate is the builder's.
  const subtotalMinor = quoteSubtotal(lines);
  const taxMinor = taxOf(subtotalMinor, taxRateBps);
  return {
    subtotalMinor,
    taxMinor,
    totalMinor: subtotalMinor + taxMinor,
    optionalMinor: quoteOptionalTotal(lines),
  };
}

export function contractTotal(pricing: ContractPriceLine[] | undefined, fallbackMinor: number | null): number {
  if (pricing === undefined || pricing.length === 0) return Math.max(0, fallbackMinor ?? 0);
  return pricing.reduce(
    (sum, line) => sum + lineAmountMinor(line.quantity_milli, line.unit_price_minor),
    0,
  );
}

/** The draft total the change card shows. Contract drafts bill one amount. */
export function draftTotal(kind: "quote" | "contract", draft: QuoteDraft | ContractDraft): number {
  if (kind === "quote") return quoteSubtotal((draft as QuoteDraft).lines);
  const contract = draft as ContractDraft;
  return contractTotal(contract.pricing, contract.value_minor ?? null);
}

export type LineSection = { title: string; lines: { line: DraftLine; index: number }[] };

/** Group the item table the way the pack does: a section row, then its lines. */
export function sectionsOf(lines: DraftLine[]): LineSection[] {
  const order: string[] = [];
  const grouped = new Map<string, { line: DraftLine; index: number }[]>();
  lines.forEach((line, index) => {
    const title = line.section?.trim() !== undefined && line.section?.trim() !== "" ? line.section.trim() : "Items";
    if (!grouped.has(title)) {
      grouped.set(title, []);
      order.push(title);
    }
    grouped.get(title)?.push({ line, index });
  });
  return order.map((title) => ({ title, lines: grouped.get(title) ?? [] }));
}

/**
 * Character offsets → a slice, so the notes tab can highlight the exact words.
 * Character offsets, not bytes: the server counts characters.
 */
export function sliceChars(text: string, start: number, end: number): string {
  return Array.from(text).slice(start, end).join("");
}

/** The source text a span points at, or null when the source is gone. */
export function sourceText(
  sources: { id?: string; text: string }[],
  sourceId: string,
): string | null {
  return sources.find((source) => source.id === sourceId)?.text ?? null;
}

/** Money for display only. The total itself is computed here, never formatted from prose. */
export function formatMoneyMinor(amountMinor: number, currency: string): string {
  try {
    return new Intl.NumberFormat(undefined, {
      style: "currency",
      currency,
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    }).format(amountMinor / 100);
  } catch {
    return `${(amountMinor / 100).toFixed(2)} ${currency}`;
  }
}

/** Which lines a proposal touches, for the tinted rows. */
export function proposedLineIndexes(paths: string[]): Set<number> {
  const indexes = new Set<number>();
  for (const path of paths) {
    const match = /^lines\[(\d+)\]/.exec(path);
    if (match !== null) indexes.add(Number(match[1]));
  }
  return indexes;
}
