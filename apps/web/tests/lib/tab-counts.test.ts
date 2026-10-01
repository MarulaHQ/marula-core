// The tab row's figures. Time carries a DURATION, not a count, because "0:45"
// answers a different question from "3 entries".
import { describe, expect, it } from "vitest";
import { formatDuration } from "../../components/ticket-workspace/tab-counts";

describe("workbench tab durations", () => {
  it("renders logged minutes as h:mm", () => {
    expect(formatDuration(0)).toBe("0:00");
    expect(formatDuration(45)).toBe("0:45");
    expect(formatDuration(90)).toBe("1:30");
    expect(formatDuration(605)).toBe("10:05");
    expect(formatDuration(1440)).toBe("24:00");
  });

  it("treats nonsense totals as zero rather than printing NaN", () => {
    expect(formatDuration(Number.NaN)).toBe("0:00");
    expect(formatDuration(-30)).toBe("0:00");
  });
});
