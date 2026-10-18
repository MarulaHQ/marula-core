/**
 * RecordWindow — the member's window beside the approver's.
 *
 * Three laws: the aside says the record's clock, it says nothing when the two
 * zones agree, and the reader's switch turns it off. The aside is not a second
 * opinion about the time — it is the same instant, named in the zone the work
 * was logged in.
 */
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { AuthProvider } from "../../lib/api";
import { clearTimeDisplayCache } from "../../lib/use-time-display";
import { RecordWindow } from "../../components/record-window";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

const START = "2026-10-19T08:00:00Z";
const END = "2026-10-19T10:00:00Z";

function mount(opts: {
  viewerZone?: string;
  orgZone?: string;
  showRecordTime?: boolean;
  zone?: string | null;
}): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({
    json: { ...ME_FIXTURE, timezone: opts.viewerZone ?? "Asia/Tokyo" },
  }));
  mock.on("GET", "/api/v1/settings/org", () => ({
    json: [{ key: "general.default_timezone", value: opts.orgZone ?? "UTC" }],
  }));
  mock.on("GET", "/api/v1/settings/user", () => ({
    json:
      opts.showRecordTime === undefined
        ? []
        : [{ key: "general.show_record_time", value: opts.showRecordTime }],
  }));
  mock.install();
  render(
    <AuthProvider>
      <RecordWindow start={START} end={END} zone={opts.zone ?? null} />
    </AuthProvider>,
  );
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
  clearTimeDisplayCache();
});

describe("RecordWindow", () => {
  it("draws the member's window in the record's zone", async () => {
    mount({ viewerZone: "Asia/Tokyo", zone: "Europe/London" });
    const aside = await screen.findByTestId("record-window");
    // 08:00–10:00 UTC is 09:00–11:00 in London on 19 October.
    expect(aside.textContent).toContain("09:00–11:00");
    expect(aside.textContent).toMatch(/recorded/);
  });

  it("uses the org default when the entry names no zone", async () => {
    mount({ viewerZone: "Asia/Tokyo", orgZone: "America/New_York", zone: null });
    // 08:00 UTC is 04:00 in New York in October. The first paint has no org
    // default yet, so wait for the settled reading rather than the fallback.
    await waitFor(() =>
      expect(screen.getByTestId("record-window").textContent).toContain("04:00–06:00"),
    );
  });

  it("says nothing when the reader is already in the record's zone", async () => {
    mount({ viewerZone: "Europe/London", zone: "Europe/London" });
    await waitFor(() => expect(screen.queryByTestId("record-window")).toBeNull());
  });

  it("says nothing once the reader turns the second clock off", async () => {
    mount({ viewerZone: "Asia/Tokyo", zone: "Europe/London", showRecordTime: false });
    await waitFor(() => expect(screen.queryByTestId("record-window")).toBeNull());
  });
});
