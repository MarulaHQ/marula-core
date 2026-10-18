/**
 * Time / DualTime and the zone helpers behind them.
 *
 * The laws under test: a reading is drawn in the zone it was given, never in
 * the machine's zone; a dual reading says the record's zone out loud; and a
 * stored zone Intl rejects resolves to a real zone rather than to a blank.
 */
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { DualTime, Time } from "../../src/components/time";
import {
  browserTimeZone,
  dayKeyInTimeZone,
  formatDayKey,
  formatInTimeZone,
  formatWallClock,
  isTimeZone,
  offsetLabel,
  resolveTimeZone,
  shiftDayKey,
  timeZoneOffsetMinutes,
  timeZoneOptions,
} from "../../src/lib/time";

const INSTANT = "2026-10-18T12:30:00Z";

describe("Time", () => {
  it("draws the instant in the zone it is given", () => {
    render(<Time value={INSTANT} timeZone="Africa/Johannesburg" />);
    expect(screen.getByText("14:30")).toBeInTheDocument();
  });

  it("tracks the zone's own daylight rule", () => {
    // July: New York is UTC-4, so noon UTC is 08:30 there.
    render(<Time value="2026-07-18T12:30:00Z" timeZone="America/New_York" />);
    expect(screen.getByText("08:30")).toBeInTheDocument();
  });

  it("carries a machine-readable instant", () => {
    render(<Time value={INSTANT} timeZone="Asia/Tokyo" />);
    expect(screen.getByText("21:30")).toHaveAttribute("datetime", "2026-10-18T12:30:00.000Z");
  });
});

describe("DualTime", () => {
  it("reads in the viewer's zone, then names the record's", () => {
    render(<DualTime value={INSTANT} zone="UTC" viewerZone="Africa/Johannesburg" />);
    expect(screen.getByText("14:30")).toBeInTheDocument();
    expect(screen.getByText("12:30 UTC")).toBeInTheDocument();
  });

  it("does not say the same clock twice", () => {
    const { container } = render(<DualTime value={INSTANT} zone="UTC" viewerZone="UTC" />);
    expect(container.querySelector(".m-dual-time-alt")).toBeNull();
    expect(screen.getByText("12:30")).toBeInTheDocument();
  });

  it("carries the date when the record's day is not the reader's", () => {
    // 22:30 UTC is already Monday in Tokyo, so a bare "22:30" under a Monday
    // heading would read as a mistake.
    render(
      <DualTime value="2026-10-18T22:30:00Z" zone="UTC" viewerZone="Asia/Tokyo" />,
    );
    const alt = screen.getByText(/UTC/, { selector: ".m-dual-time-alt" });
    expect(alt.textContent).toMatch(/Oct/);
    expect(alt.textContent).toMatch(/22:30/);
  });

  it("keeps the reading short when both zones share the day", () => {
    render(<DualTime value={INSTANT} zone="UTC" viewerZone="Africa/Johannesburg" />);
    const alt = screen.getByText(/UTC/, { selector: ".m-dual-time-alt" });
    expect(alt.textContent).not.toMatch(/Oct/);
  });
});

describe("kind table", () => {
  // Every shape a surface draws, asserted against a zone the machine is not in.
  // A shape that drifts here is a shape that drifts on fifty screens.
  const SHAPES: Array<[Parameters<typeof Time>[0]["kind"], string]> = [
    ["time", "14:30"],
    ["date", "18 Oct 2026"],
    ["datetime", "18 Oct 2026, 14:30"],
    ["weekday", "Sun 18 Oct, 14:30"],
    ["day", "Sun 18 Oct"],
    ["dayMonth", "18 Oct"],
    ["dayTime", "18 Oct, 14:30"],
    ["longDate", "Sunday 18 October"],
    ["month", "Oct"],
    ["monthYear", "October 2026"],
    ["weekdayOnly", "Sun"],
    ["weekdayDay", "Sun 18"],
  ];

  it.each(SHAPES)("draws %s as %s", (kind, expected) => {
    const { unmount } = render(
      <Time value={INSTANT} timeZone="Africa/Johannesburg" kind={kind} />,
    );
    expect(screen.getByText(expected)).toBeInTheDocument();
    unmount();
  });

  it("pins the shape so two readers see one format", () => {
    // The zone moves with the reader; the order of the fields does not.
    render(<Time value={INSTANT} timeZone="Africa/Johannesburg" kind="dayTime" />);
    expect(screen.getByText("18 Oct, 14:30")).toBeInTheDocument();
  });
});

describe("formatDayKey", () => {
  it("reads a bare day in its own frame, not the reader's", () => {
    // `2026-10-15` is a Thursday. Midnight UTC of that day is still the 15th
    // in every zone from UTC-12 to UTC+12, so the weekday cannot drift.
    expect(formatDayKey("2026-10-15")).toBe("Thu 15 Oct");
    expect(formatDayKey("2026-10-15", "weekdayDay")).toBe("Thu 15");
    expect(formatDayKey("2026-10-15", "weekdayOnly")).toBe("Thu");
  });

  it("refuses a value that is not a day key", () => {
    expect(formatDayKey("2026-10-15T09:00:00Z")).toBe("—");
    expect(formatDayKey(null)).toBe("—");
  });
});

describe("zone helpers", () => {
  it("rejects a name Intl cannot format in and keeps a real alias", () => {
    expect(isTimeZone("Mars/Olympus_Mons")).toBe(false);
    expect(isTimeZone("")).toBe(false);
    // An alias older clients stored: Intl formats it even though
    // `supportedValuesOf` lists only the canonical name.
    expect(isTimeZone("Asia/Calcutta")).toBe(true);
  });

  it("falls back through preferred, given fallback, then the device", () => {
    expect(resolveTimeZone("Asia/Tokyo", "Europe/London")).toBe("Asia/Tokyo");
    expect(resolveTimeZone("Not/AZone", "Europe/London")).toBe("Europe/London");
    expect(resolveTimeZone(null, "also/not-real")).toBe(browserTimeZone());
  });

  it("prices a zone's offset at the instant, not by name", () => {
    const october = new Date("2026-10-18T00:00:00Z");
    expect(timeZoneOffsetMinutes("Africa/Johannesburg", october)).toBe(120);
    expect(timeZoneOffsetMinutes("America/New_York", october)).toBe(-240);
    expect(offsetLabel(-210)).toBe("UTC-03:30");
    expect(offsetLabel(120)).toBe("UTC+02:00");
  });

  it("keys a day in the draw zone, not UTC", () => {
    expect(dayKeyInTimeZone("2026-10-18T22:30:00Z", "Asia/Tokyo")).toBe("2026-10-19");
    expect(dayKeyInTimeZone("2026-10-18T22:30:00Z", "UTC")).toBe("2026-10-18");
  });

  it("offers the stored alias as an option so the picker never loses it", () => {
    const values = timeZoneOptions(new Date(), "Asia/Calcutta").map((option) => option.value);
    expect(values).toContain("Asia/Calcutta");
    expect(values).toContain("UTC");
  });
});

describe("shiftDayKey", () => {
  it("moves a key by whole days across month and year ends", () => {
    expect(shiftDayKey("2026-10-15", -1)).toBe("2026-10-14");
    expect(shiftDayKey("2026-11-01", -1)).toBe("2026-10-31");
    expect(shiftDayKey("2027-01-01", -1)).toBe("2026-12-31");
    expect(shiftDayKey("2026-02-28", 1)).toBe("2026-03-01");
  });

  it("leaves a value that is not a day key alone", () => {
    expect(shiftDayKey("2026-10-15T09:00:00Z", -1)).toBe("2026-10-15T09:00:00Z");
  });
});

describe("formatWallClock", () => {
  it("draws the reader's own clock, in the pinned format", () => {
    // Deliberately the machine's zone: this is the reading that is *not* about
    // a record, so only the shape is asserted, never an absolute hour.
    const local = new Date(2026, 9, 18, 9, 5);
    expect(formatWallClock(local, "time")).toBe("09:05");
    expect(formatWallClock(local, "day")).toBe("Sun 18 Oct");
    expect(formatWallClock(local, "longDate")).toBe("Sunday 18 October");
  });
});

describe("a missing value", () => {
  it("draws a dash in every formatter, never the epoch", () => {
    // `new Date(null)` is 1970, so each of these has to turn null away first.
    for (const missing of [null, undefined, ""] as const) {
      expect(formatInTimeZone(missing, "UTC", "datetime")).toBe("—");
      expect(formatWallClock(missing)).toBe("—");
      expect(formatDayKey(missing)).toBe("—");
    }
  });
});
