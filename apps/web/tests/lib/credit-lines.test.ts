import { describe, expect, it } from "vitest";
import {
  creditLinesMatchTotal,
  creditLinesTotal,
  isUuid,
} from "../../lib/credit-lines";

describe("credit lines", () => {
  it("sum in minor units", () => {
    expect(creditLinesTotal([{ amount_minor: 600 }, { amount_minor: 400 }])).toBe(1000);
  });

  it("match only the exact total", () => {
    const lines = [{ amount_minor: 600 }, { amount_minor: 400 }];
    expect(creditLinesMatchTotal(lines, 1000)).toBe(true);
    expect(creditLinesMatchTotal(lines, 900)).toBe(false);
  });

  it("accept a uuid and refuse anything else", () => {
    expect(isUuid("01a20052-3989-70fd-9418-18cc11372056")).toBe(true);
    expect(isUuid("not-a-row")).toBe(false);
  });
});
