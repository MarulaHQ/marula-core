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
  isTimeZone,
  offsetLabel,
  resolveTimeZone,
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
