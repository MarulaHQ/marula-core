// Unit tests for the shared format helpers used by the daily-landing
// surfaces (/today due buckets, compact durations).
import { describe, expect, it } from "vitest";
import { dueBucket, dueLabel, fmtMins } from "../../lib/format";

const NOW = new Date("2026-09-17T14:00:00").getTime();

describe("fmtMins", () => {
  it("formats hours, mixed spans, and zero", () => {
    expect(fmtMins(45)).toBe("45m");
    expect(fmtMins(60)).toBe("1h");
    expect(fmtMins(390)).toBe("6h 30m");
    expect(fmtMins(0)).toBe("0m");
    expect(fmtMins(-5)).toBe("0m");
  });
});

describe("dueLabel", () => {
  it("counts up for overdue deadlines", () => {
    const twoHoursAgo = new Date(NOW - 2 * 3600_000).toISOString();
    expect(dueLabel(twoHoursAgo, NOW)).toEqual({ text: "overdue 2h", overdue: true });
  });

  it("counts down for future deadlines", () => {
    const inThreeDays = new Date(NOW + 3 * 86_400_000 + 5 * 3600_000).toISOString();
    expect(dueLabel(inThreeDays, NOW)).toEqual({ text: "3d 5h", overdue: false });
    const inNinetySecs = new Date(NOW + 90_000).toISOString();
    expect(dueLabel(inNinetySecs, NOW)).toEqual({ text: "2m", overdue: false });
  });
});

describe("dueBucket", () => {
  it("buckets deadlines relative to the local day", () => {
    expect(dueBucket(new Date(NOW - 3600_000).toISOString(), NOW)).toBe("overdue");
    expect(dueBucket(new Date(NOW + 3600_000).toISOString(), NOW)).toBe("today");
    expect(dueBucket(new Date("2026-09-18T09:00:00").toISOString(), NOW)).toBe("later");
    expect(dueBucket(null, NOW)).toBe("none");
    expect(dueBucket(undefined, NOW)).toBe("none");
    expect(dueBucket("not-a-date", NOW)).toBe("none");
  });
});
