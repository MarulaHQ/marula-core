import { describe, expect, it } from "vitest";
import { slaReading } from "../../lib/ticket-ops";
const M = 60_000, H = 3_600_000, D = 86_400_000;
// Offset by a few seconds so a fixture never sits ON a unit boundary: the
// countdown reads the wall clock at assert time, and a bare 30m floors to 29m
// the moment a millisecond passes.
const at = (ms: number) => ({
  due_at: new Date(Date.now() + ms + (ms >= 0 ? 5_000 : -5_000)).toISOString(),
  resolved_at: null,
});
describe("countdown units", () => {
  it("reads in the largest unit that still says something", () => {
    expect(slaReading(at(30 * M))?.label).toBe("30m left");
    expect(slaReading(at(90 * M))?.label).toBe("1h 30m left");
    expect(slaReading(at(5 * H))?.label).toBe("5h left");
    expect(slaReading(at(30 * H))?.label).toBe("30h left");
    // Past 48h it steps up to days: the first live capture printed
    // "303h 28m left", which is twelve and a half days as an hour count.
    expect(slaReading(at(49 * H))?.label).toBe("2d 1h left");
    expect(slaReading(at(72 * H))?.label).toBe("3d left");
    expect(slaReading(at(303 * H))?.label).toBe("12d 15h left");
    expect(slaReading(at(-9 * H))?.label).toBe("9h over");
    expect(slaReading(at(-3 * H))?.label).toBe("3h over");
    expect(slaReading(at(-40 * D))?.label).toBe("40d over");
  });
});
