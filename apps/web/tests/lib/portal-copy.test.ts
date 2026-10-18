/**
 * The portal's copy, and the zone it draws in.
 *
 * Two rules are pinned here. An instant follows the reader's zone, so the same
 * booking reads as the 18th to a client in London and the 19th to one in Tokyo.
 * A bare `Format: date` day — `due_on`, `valid_until` — has no instant in it and
 * is read in its own frame, or an invoice due on the 18th bills as the 17th for
 * everyone west of Greenwich.
 *
 * Nothing here depends on the zone the suite runs in.
 */
import { describe, expect, it } from "vitest";
import {
  clockTime,
  longDate,
  raisedAt,
  stamp,
  timeRange,
} from "../../lib/portal-copy";

describe("longDate", () => {
  it("draws an instant in the named zone", () => {
    // 23:30Z is the 18th in Los Angeles and the 19th in Tokyo.
    expect(longDate("2026-10-18T23:30:00Z", "Asia/Tokyo")).toBe("19 Oct 2026");
    expect(longDate("2026-10-18T23:30:00Z", "America/Los_Angeles")).toBe("18 Oct 2026");
  });

  it("reads a bare day in its own frame, whatever the zone", () => {
    for (const zone of ["Asia/Tokyo", "America/Los_Angeles", "UTC"]) {
      expect(longDate("2026-10-18", zone)).toBe("18 Oct 2026");
    }
  });
});

describe("clockTime and timeRange", () => {
  it("draws the clock in the named zone", () => {
    expect(clockTime("2026-10-18T23:30:00Z", "Asia/Tokyo")).toBe("08:30");
    expect(clockTime("2026-10-18T23:30:00Z", "UTC")).toBe("23:30");
  });

  it("draws a visit window in one zone", () => {
    expect(
      timeRange("2026-10-18T09:00:00Z", "2026-10-18T10:30:00Z", "Asia/Tokyo"),
    ).toBe("18:00 to 19:30");
  });
});

describe("stamp", () => {
  // 18:00 on the 18th in Tokyo.
  const now = Date.parse("2026-10-18T09:00:00Z");

  it("names today, yesterday and tomorrow in the reader's zone", () => {
    expect(stamp("2026-10-18T01:00:00Z", "Asia/Tokyo", now)).toBe("Today, 10:00");
    // 14:00Z is 23:00 on the 17th in Tokyo.
    expect(stamp("2026-10-17T14:00:00Z", "Asia/Tokyo", now)).toBe("Yesterday, 23:00");
    expect(stamp("2026-10-19T01:00:00Z", "Asia/Tokyo", now)).toBe("Tomorrow, 10:00");
    expect(stamp("2026-10-20T02:00:00Z", "Asia/Tokyo", now)).toBe("Tue 20 Oct, 11:00");
  });

  it("gives two readers their own word for one instant", () => {
    // 22:00Z is 07:00 tomorrow in Tokyo and 15:00 today in Los Angeles.
    expect(stamp("2026-10-18T22:00:00Z", "Asia/Tokyo", now)).toBe("Tomorrow, 07:00");
    expect(stamp("2026-10-18T22:00:00Z", "America/Los_Angeles", now)).toBe("Today, 15:00");
  });
});

describe("raisedAt", () => {
  const now = Date.parse("2026-10-18T09:00:00Z");

  it("reads as a sentence, in the reader's zone", () => {
    expect(raisedAt("2026-10-17T14:00:00Z", "Asia/Tokyo", now)).toBe("yesterday at 23:00");
    expect(raisedAt("2026-10-18T01:00:00Z", "Asia/Tokyo", now)).toBe("today at 10:00");
    expect(raisedAt("2026-10-20T02:00:00Z", "Asia/Tokyo", now)).toBe("Tue 20 Oct at 11:00");
  });
});
