// The found sheet's warranty pill: what it says, and the invariant behind it.
//
// Two things are worth pinning. The sentences, because a pill that reads
// "Ends in -11 days" is a bug a screenshot finds and a unit test does not.
// And the fact that neither surface derives the horizon itself — the native
// sheet and the web ledger both read the API's `state` + `days_remaining`, so
// they cannot drift apart about which side of the window an asset sits on.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import type { WarrantyHorizon } from "@/core/warranty";
import { warrantyPairKey, warrantySentence } from "@/core/warranty";

const here = path.dirname(fileURLToPath(import.meta.url));

function horizon(overrides: Partial<WarrantyHorizon>): WarrantyHorizon {
  return {
    state: "active",
    ends_on: "2040-01-15",
    days_remaining: 4843,
    ...overrides,
  };
}

/** Drops `//` and block comments so an assertion about code is not answered by
 * a sentence about the past. */
function stripComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1");
}

describe("the warranty pill sentence", () => {
  it("reads the pack's phrasing while a horizon is ending", () => {
    expect(warrantySentence(horizon({ state: "ending", days_remaining: 41 }))).toBe(
      "Ends in 41 days",
    );
  });

  it("counts backwards once a horizon has lapsed", () => {
    expect(
      warrantySentence(horizon({ state: "ended", days_remaining: -45, ends_on: "2026-08-28" })),
    ).toBe("Ended 45 days ago");
  });

  it("says today rather than zero days in either direction", () => {
    expect(warrantySentence(horizon({ state: "ending", days_remaining: 0 }))).toBe("Ends today");
    expect(warrantySentence(horizon({ state: "ended", days_remaining: 0 }))).toBe("Ended today");
  });

  it("carries the date while a horizon is covered", () => {
    expect(warrantySentence(horizon({ state: "active", days_remaining: 4843 }))).toBe(
      "Covered until 2040-01-15",
    );
  });

  it("rounds a fractional day count the way a human reads it", () => {
    expect(warrantySentence(horizon({ state: "ending", days_remaining: 41.4 }))).toBe(
      "Ends in 41 days",
    );
  });
});

describe("the warranty pill pair", () => {
  it("keeps red for a lapsed horizon and amber for a close one", () => {
    expect(warrantyPairKey(horizon({ state: "ending" }))).toBe("warning");
    expect(warrantyPairKey(horizon({ state: "ended" }))).toBe("danger");
  });

  it("reads green while covered, so nothing on the sheet is ringing an alarm", () => {
    expect(warrantyPairKey(horizon({ state: "active" }))).toBe("success");
  });
});

describe("the ledger derives nothing", () => {
  // The old assets page computed the horizon from purchase_date +
  // useful_life_months with its own copy of the 90-day window. It now reads
  // the API's horizon, so this asserts the ledger has no horizon arithmetic
  // left in it. Comments are stripped first — the file explains what it used
  // to do, and a sentence about the past is not arithmetic.
  const source = stripComments(
    fs.readFileSync(path.resolve(here, "../../apps/web/app/assets/page.tsx"), "utf8"),
  );

  it("does not read the purchase inputs the horizon is derived from", () => {
    expect(source).not.toContain("purchase_date");
    expect(source).not.toContain("useful_life_months");
  });

  it("has no window constant of its own to fall out of step", () => {
    expect(source).not.toContain("WARRANTY_WINDOW_DAYS");
    expect(source).not.toContain("setMonth");
  });

  it("reads the horizon off the payload", () => {
    expect(source).toContain("warrantyMark(asset.warranty)");
  });
});
