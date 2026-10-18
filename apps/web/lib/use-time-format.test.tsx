/**
 * useTimeFormat — which zone a surface draws in.
 *
 * The account's stored zone wins. When it is missing or not something Intl can
 * format in, the device's zone takes over; before mount the hook reports
 * `pending` and draws UTC so the server's HTML and the first client render
 * agree.
 */
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { AuthProvider } from "./api";
import { useTimeFormat } from "./use-time-format";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../tests/helpers/mock-api";

function Probe() {
  const time = useTimeFormat();
  return (
    <div>
      <span data-testid="zone">{time.timeZone}</span>
      <span data-testid="source">{time.source}</span>
      <span data-testid="device">{time.deviceTimeZone ?? "null"}</span>
      <span data-testid="formatted">{time.format("2026-10-18T12:30:00Z")}</span>
      <span data-testid="day">{time.dayKey("2026-10-18T22:30:00Z")}</span>
    </div>
  );
}

function mount(timezone: string): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: { ...ME_FIXTURE, timezone } }));
  mock.install();
  render(
    <AuthProvider>
      <Probe />
    </AuthProvider>,
  );
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("useTimeFormat", () => {
  it("draws in the account's zone and says so", async () => {
    mount("Asia/Tokyo");
    await waitFor(() => expect(screen.getByTestId("source")).toHaveTextContent("user"));
    expect(screen.getByTestId("zone")).toHaveTextContent("Asia/Tokyo");
    expect(screen.getByTestId("formatted")).toHaveTextContent("21:30");
    expect(screen.getByTestId("day")).toHaveTextContent("2026-10-19");
  });

  it("falls back to the device zone when the stored value is not a zone", async () => {
    mount("Mars/Olympus_Mons");
    // vitest.config.ts pins TZ=UTC, so the device zone is a known value here.
    await waitFor(() => expect(screen.getByTestId("source")).toHaveTextContent("browser"));
    expect(screen.getByTestId("device")).toHaveTextContent("UTC");
    expect(screen.getByTestId("zone")).toHaveTextContent("UTC");
    expect(screen.getByTestId("formatted")).toHaveTextContent("12:30");
  });
});
