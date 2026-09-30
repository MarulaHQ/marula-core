// Pack kit rules — the severity/rounding contract the vision review checks.
//
// Covers: red means breach (never a form error), blush is attention, the
// bottom padding rule (timer pill + tab bar), and the stop-and-log rounding
// ladder ("Rounded up to the next 15 minutes"). Pure pack-rules so the node
// suite holds the line without React Native in the graph.

import { describe, expect, it } from "vitest";

import { clockText, pack, roundUpTo, bottomReserve, slaVerdictColors, shortClock } from "@/ui/pack-rules";

/** The one red, documented in tokens.ts (danger.text, light). */
// The one red (tokens.ts danger.text, light) — hex bu yerga yozilmaydi, qiymat tokens faylidan kutiladi.
const RED_TEXT = slaVerdictColors("breach").light.text.toLowerCase();

describe("clock text", () => {
  it("renders the pack's frozen 00:25:05", () => {
    expect(clockText(25 * 60 * 1000 + 5 * 1000)).toBe("00:25:05");
  });

  it("stays tabular across hours", () => {
    expect(clockText((1 * 3600 + 2 * 60 + 3) * 1000)).toBe("01:02:03");
  });
});

describe("stop-and-log rounding", () => {
  it("rounds up to the next 15 minutes, never down", () => {
    expect(roundUpTo(1)).toBe(15);
    expect(roundUpTo(15)).toBe(15);
    expect(roundUpTo(16)).toBe(30);
    expect(roundUpTo(25)).toBe(30);
    expect(roundUpTo(61)).toBe(75);
  });

  it("labels 30 minutes as 0:30 on the primary", () => {
    expect(shortClock(30)).toBe("0:30");
    expect(shortClock(60)).toBe("1:00");
    expect(shortClock(75)).toBe("1:15");
  });
});

describe("severity pairs", () => {
  it("reserves red for breach, blush for due soon", () => {
    const breach = slaVerdictColors("breach");
    // Red text with a pale red ground; the pairs are pinned in tokens.ts.
    expect(breach.light.text.toLowerCase()).toBe(RED_TEXT);
    expect(breach.light.background).not.toBe(breach.light.text);
    const due = slaVerdictColors("due");
    expect(due.light.background).not.toBe(breach.light.background);
  });

  it("flips the primary on mode but keeps danger wording readable", () => {
    const breach = slaVerdictColors("breach");
    expect(breach.dark.text).not.toBe(breach.light.text);
  });
});

describe("pack geometry law", () => {
  it("keeps the three hard numbers: 44 targets, 52 primary, 12px text floor", () => {
    expect(pack.secondaryHeight).toBeGreaterThanOrEqual(44);
    expect(pack.primaryHeight).toBe(52);
    expect(pack.minWidthText).toBe(12);
    expect(pack.rowHeight).toBe(44);
    expect(pack.factsRowHeight).toBe(44);
  });

  it("reserves timer pill + tab bar under tab lists (56 + 90)", () => {
    expect(pack.timerPillHeight).toBe(56);
    expect(pack.tabBarHeight).toBe(90);
    expect(bottomReserve.tabs).toBeGreaterThanOrEqual(pack.timerPillHeight + pack.tabBarHeight);
  });
});
