/**
 * useTimeDisplay — the org default zone and the second-clock switch.
 *
 * The chain under test: a record's own zone, then the org's default, then UTC.
 * Never the reader's device: two people must read one record the same way, and
 * a device zone would give them two answers. The reader's own zone is the
 * *other* clock, and only when they have left the switch on.
 */
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { AuthProvider } from "./api";
import { clearTimeDisplayCache, useTimeDisplay } from "./use-time-display";
import { useTimeFormat } from "./use-time-format";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../tests/helpers/mock-api";

function Probe({ site }: { site?: string | null }) {
  const display = useTimeDisplay();
  const time = useTimeFormat();
  return (
    <div>
      <span data-testid="org">{display.orgTimeZone ?? "null"}</span>
      <span data-testid="show">{String(display.showRecordTime)}</span>
      <span data-testid="viewer">{time.timeZone}</span>
      <span data-testid="zone">{display.zoneFor(site, time.timeZone)}</span>
    </div>
  );
}

/** The settings endpoints as the hub returns them: override rows only. */
function mount(opts: {
  orgZone?: string;
  showRecordTime?: boolean;
  viewerZone?: string;
  site?: string | null;
  settingsUp?: boolean;
}): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({
    json: { ...ME_FIXTURE, timezone: opts.viewerZone ?? "Asia/Tokyo" },
  }));
  if (opts.settingsUp !== false) {
    mock.on("GET", "/api/v1/settings/org", () => ({
      json:
        opts.orgZone === undefined
          ? []
          : [{ key: "general.default_timezone", value: opts.orgZone }],
    }));
    mock.on("GET", "/api/v1/settings/user", () => ({
      json:
        opts.showRecordTime === undefined
          ? []
          : [{ key: "general.show_record_time", value: opts.showRecordTime }],
    }));
  }
  mock.install();
  render(
    <AuthProvider>
      <Probe site={opts.site} />
    </AuthProvider>,
  );
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
  clearTimeDisplayCache();
});

describe("useTimeDisplay", () => {
  it("prefers the record's own zone over the org's", async () => {
    mount({
      orgZone: "Europe/London",
      viewerZone: "Asia/Tokyo",
      site: "America/New_York",
    });
    await waitFor(() => expect(screen.getByTestId("org")).toHaveTextContent("Europe/London"));
    expect(screen.getByTestId("zone")).toHaveTextContent("America/New_York");
  });

  it("falls to the org default when the record names no zone", async () => {
    mount({ orgZone: "Europe/London", viewerZone: "Asia/Tokyo", site: null });
    await waitFor(() => expect(screen.getByTestId("org")).toHaveTextContent("Europe/London"));
    expect(screen.getByTestId("zone")).toHaveTextContent("Europe/London");
  });

  it("draws only the reader's clock once the switch is off", async () => {
    mount({
      orgZone: "Europe/London",
      showRecordTime: false,
      viewerZone: "Asia/Tokyo",
      site: "America/New_York",
    });
    await waitFor(() => expect(screen.getByTestId("show")).toHaveTextContent("false"));
    expect(screen.getByTestId("zone")).toHaveTextContent("Asia/Tokyo");
  });

  it("keeps the second clock on by default", async () => {
    mount({ orgZone: "Europe/London", site: "America/New_York" });
    await waitFor(() => expect(screen.getByTestId("org")).toHaveTextContent("Europe/London"));
    expect(screen.getByTestId("show")).toHaveTextContent("true");
  });

  it("never falls back to the device zone when the settings read fails", async () => {
    // The hub is down: the registry defaults are the honest answer, not the
    // reader's device — that would be the bug this wave removes.
    mount({ settingsUp: false, viewerZone: "Asia/Tokyo", site: null });
    await waitFor(() => expect(screen.getByTestId("viewer")).toHaveTextContent("Asia/Tokyo"));
    expect(screen.getByTestId("org")).toHaveTextContent("null");
    expect(screen.getByTestId("zone")).toHaveTextContent("UTC");
  });
});
