// Unit tests: money-area summaries (Wave 17 lane 5) — the money-correctness
// glue: per-currency sums never invent FX, term renewal signals stay honest.
import { describe, expect, it } from "vitest";
import { daysUntil, perCurrencySum, termSignal } from "../../lib/money-summary";

describe("perCurrencySum", () => {
  it("sums within one currency and reports it", () => {
    const r = perCurrencySum([
      { currency: "USD", amount: 1200 },
      { currency: "USD", amount: 800 },
    ]);
    expect(r.currency).toBe("USD");
    expect(r.value).toBe("$20.00");
  });

  it("never invents a mixed-currency total", () => {
    const r = perCurrencySum([
      { currency: "USD", amount: 1000 },
      { currency: "EUR", amount: 2000 },
    ]);
    expect(r.currency).toBeNull();
    expect(r.value).toBe("mixed currencies");
  });

  it("is empty-honest", () => {
    expect(perCurrencySum([])).toEqual({ currency: null, value: "—" });
  });
});

describe("termSignal", () => {
  const NOW = Date.parse("2026-09-17T00:00:00Z");

  it("warns inside 30 days", () => {
    const s = termSignal("2026-10-10", NOW);
    expect(s?.tone).toBe("warning");
    expect(s?.label).toContain("ends in");
  });

  it("flags a term that already ended as danger", () => {
    const s = termSignal("2026-01-01", NOW);
    expect(s?.tone).toBe("danger");
  });

  it("stays quiet for open-ended or far terms", () => {
    expect(termSignal(null, NOW)).toBeNull();
    expect(termSignal("2027-06-01", NOW)).toBeNull();
  });
});

describe("daysUntil", () => {
  it("counts calendar days to the boundary", () => {
    expect(daysUntil("2026-09-18", Date.parse("2026-09-17T12:00:00Z"))).toBe(1);
  });

  it("returns null for garbage input", () => {
    expect(daysUntil("not-a-date")).toBeNull();
  });
});
