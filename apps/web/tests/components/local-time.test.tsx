/**
 * LocalTime and RecordTime — the two ways a surface draws a clock.
 *
 * The rules under test: a reading belongs to the reader's account zone, never
 * the machine's; a record that names its own zone gets a second reading only
 * when the two zones differ and the reader has left the switch on; and a
 * missing value draws a dash rather than the epoch.
 *
 * The instant is in July so the two zones below are on their summer offsets —
 * London at UTC+1, Tokyo at UTC+9 — and neither matches the suite's own zone.
 *
 * The second clock arrives with the settings read, one state update after the
 * account zone does. Every assertion therefore waits on the reading it is about,
 * inside one `waitFor`, so a re-render between the query and the check cannot
 * leave the test holding a node React has already replaced.
 */
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { AuthProvider } from "../../lib/api";
import { clearTimeDisplayCache } from "../../lib/use-time-display";
import { LocalTime, RecordTime } from "../../components/local-time";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

/** 12:30Z — 13:30 in London, 21:30 in Tokyo. */
const INSTANT = "2026-07-15T12:30:00Z";

function mount(opts: {
  viewerZone?: string;
  orgZone?: string;
  showRecordTime?: boolean;
  ui: React.ReactElement;
}): void {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({
    json: { ...ME_FIXTURE, timezone: opts.viewerZone ?? "Asia/Tokyo" },
  }));
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
  mock.install();
  render(<AuthProvider>{opts.ui}</AuthProvider>);
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
  clearTimeDisplayCache();
});

describe("LocalTime", () => {
  it("draws the reader's account zone, not the machine's", async () => {
    mount({ ui: <LocalTime value={INSTANT} kind="time" /> });
    // 21:30 is Tokyo. The machine's zone is whatever the leg runs in, and UTC
    // would draw 12:30.
    await screen.findByText("21:30");
  });

  it("moves with the account zone", async () => {
    mount({ viewerZone: "Europe/London", ui: <LocalTime value={INSTANT} kind="time" /> });
    await screen.findByText("13:30");
  });

  it("draws a dash for a value that is not there, never the epoch", async () => {
    mount({ ui: <LocalTime value={null} /> });
    await screen.findByText("—");
    expect(screen.queryByText("1970")).not.toBeInTheDocument();
  });
});

describe("RecordTime", () => {
  const site = "America/New_York"; // 08:30 on the same day

  it("draws both clocks when the record's zone differs", async () => {
    mount({
      showRecordTime: true,
      ui: <RecordTime value={INSTANT} zone={site} kind="time" />,
    });
    await waitFor(() => {
      // The second reading carries the record zone's abbreviation with it.
      expect(screen.getByText(/08:30/)).toBeInTheDocument();
      expect(screen.getByText("21:30")).toBeInTheDocument();
    });
  });

  it("draws one clock when the reader has turned the second one off", async () => {
    mount({
      showRecordTime: false,
      ui: <RecordTime value={INSTANT} zone={site} kind="time" />,
    });
    await waitFor(() => {
      expect(screen.getByText("21:30")).toBeInTheDocument();
      expect(screen.queryByText(/08:30/)).not.toBeInTheDocument();
    });
  });

  it("says nothing extra when both zones agree", async () => {
    mount({
      showRecordTime: true,
      ui: <RecordTime value={INSTANT} zone="Asia/Tokyo" kind="time" />,
    });
    await waitFor(() => expect(screen.getAllByText("21:30")).toHaveLength(1));
  });
});
