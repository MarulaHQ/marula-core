/**
 * formatNumber — the kit's numeric formatting, and the boundary it draws.
 *
 * A number has no zone, so the reader's own locale stays the reading they win.
 * That is the one place the kit deliberately does not pin: the shape of a date
 * is a comprehension hazard, the grouping of a number is not.
 */
import { describe, expect, it } from "vitest";
import { formatNumber } from "../../src/lib/number";

describe("formatNumber", () => {
  it("groups the same way toLocaleString did", () => {
    expect(formatNumber(1048576)).toBe((1048576).toLocaleString());
    expect(formatNumber(0)).toBe("0");
  });

  it("takes Intl options when a surface means them", () => {
    expect(
      formatNumber(3.4, {
        style: "currency",
        currency: "USD",
        minimumFractionDigits: 2,
        maximumFractionDigits: 2,
      }),
    ).toBe("$3.40");
  });

  it("draws a dash for a value that is not a number", () => {
    expect(formatNumber(null)).toBe("—");
    expect(formatNumber(undefined)).toBe("—");
    expect(formatNumber(Number.NaN)).toBe("—");
  });
});
