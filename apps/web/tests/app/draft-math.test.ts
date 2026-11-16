// Draft arithmetic: the one implementation of the preview the builder shows.
import { describe, expect, it } from "vitest";

import {
  contractTotal,
  formatMoneyMinor,
  lineAmountMinor,
  proposedLineIndexes,
  quoteOptionalTotal,
  quoteSubtotal,
  quoteTotals,
  sectionsOf,
  sliceChars,
  taxOf,
  type TaxableLine,
} from "../../components/money/draft-math";

const line = (patch: Partial<TaxableLine> = {}): TaxableLine => ({
  description: "Switch",
  quantity_milli: 1000,
  unit_price_minor: 1000,
  ...patch,
});

describe("draft arithmetic", () => {
  it("multiplies milli-quantities and rounds half to even", () => {
    expect(lineAmountMinor(1500, 1000)).toBe(1500);
    expect(lineAmountMinor(500, 1)).toBe(0);
    expect(lineAmountMinor(1500, 1)).toBe(2);
    expect(lineAmountMinor(-5, 100)).toBe(0);
  });

  it("keeps optional lines out of the total and reports them separately", () => {
    const lines = [line(), line({ optional: true, unit_price_minor: 20_000 })];
    expect(quoteSubtotal(lines)).toBe(1000);
    expect(quoteOptionalTotal(lines)).toBe(20_000);
    expect(quoteTotals(lines, 800)).toEqual({
      subtotalMinor: 1000,
      taxMinor: 80,
      totalMinor: 1080,
      optionalMinor: 20_000,
    });
  });

  it("taxes only the lines the create route will tax", () => {
    // The builder's own lines carry `taxable`; the preview has to read it, or a
    // non-taxable catalogue line shows tax the saved quote will not carry.
    const lines = [
      line({ unit_price_minor: 10_000 }),
      line({ unit_price_minor: 10_000, taxable: false }),
      line({ unit_price_minor: 20_000, optional: true }),
    ];
    expect(quoteTotals(lines, 1000)).toEqual({
      subtotalMinor: 20_000,
      taxMinor: 1000,
      totalMinor: 21_000,
      optionalMinor: 20_000,
    });
  });

  it("rounds tax half to even, the rule the server stores with", () => {
    // 5 minor at 1000 bps is 0.5, and the stored answer is 0 (see
    // `quote::rollup` in core-domain); 15 is 1.5 and rounds to 2.
    expect(taxOf(5, 1000)).toBe(0);
    expect(taxOf(1, 5000)).toBe(0);
    expect(taxOf(15, 1000)).toBe(2);
    expect(taxOf(0, 800)).toBe(0);
  });

  it("builds a contract total from quantities and rates, never a stated total", () => {
    expect(
      contractTotal(
        [
          { label: "Users", quantity_milli: 12_000, unit_price_minor: 3800 },
          { label: "Devices", quantity_milli: 14_000, unit_price_minor: 1200 },
        ],
        null,
      ),
    ).toBe(62_400);
    // No pricing lines: the single amount stands.
    expect(contractTotal([], 45_000)).toBe(45_000);
  });

  it("says a mixed section's subtitle from the lines it holds", () => {
    const grouped = sectionsOf([
      line({ section: "Hardware" }),
      line({ section: "Labour" }),
      line({ section: "Hardware" }),
    ]);
    expect(grouped.map((section) => [section.title, section.lines.length])).toEqual([
      ["Hardware", 2],
      ["Labour", 1],
    ]);
    expect(sectionsOf([line({ section: null })])[0]?.title).toBe("Items");
  });

  it("slices the notes by character, not by byte", () => {
    const text = "Re-cable 2 runs ✓ and test";
    expect(sliceChars(text, 0, 9)).toBe("Re-cable ");
    expect(sliceChars(text, 9, 15)).toBe("2 runs");
  });

  it("maps proposal paths to the rows they tint", () => {
    const indexes = proposedLineIndexes([
      "lines[0].unit_price_minor",
      "lines[2].optional",
      "notes",
      "lines[+]",
    ]);
    expect([...indexes].sort()).toEqual([0, 2]);
  });

  it("formats money without inventing a currency", () => {
    expect(formatMoneyMinor(525_400, "USD")).toContain("5,254.00");
    // An unknown code still formats with the code it was given.
    expect(formatMoneyMinor(100, "ZZZ")).toContain("1.00");
    expect(formatMoneyMinor(100, "ZZZ")).toContain("ZZZ");
  });
});
