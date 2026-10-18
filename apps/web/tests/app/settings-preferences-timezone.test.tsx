/**
 * /settings/preferences — the time-zone control.
 *
 * The endpoint (`PATCH /api/v1/users/me`) predates this lane but only
 * onboarding called it, so the zone could not be changed afterwards. The law
 * here: choosing a zone writes that zone, the card reports the save, and the
 * trigger shows what was stored rather than falling back to a placeholder.
 */
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AuthProvider } from "../../lib/api";
import UserPreferencesPage from "../../app/settings/preferences/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/settings/preferences",
}));

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: { ...ME_FIXTURE, timezone: "UTC" } }));
  mock.on("GET", "/api/v1/settings/user", () => ({ json: [] }));
  mock.on("GET", "/api/v1/settings/org", () => ({ json: [] }));
  mock.on("PATCH", "/api/v1/users/me", ({ body }) => ({
    json: { ...ME_FIXTURE, ...(body as Record<string, unknown>) },
  }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/settings/preferences time zone", () => {
  it("writes the chosen zone through the profile endpoint", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <UserPreferencesPage />
      </AuthProvider>,
    );

    const trigger = await screen.findByLabelText("Your time zone");
    expect(trigger).toHaveTextContent("UTC");

    await user.click(trigger);
    await user.click(await screen.findByRole("option", { name: /^Asia\/Tokyo/ }));

    await waitFor(() => expect(mock.callsTo("PATCH", "/api/v1/users/me").length).toBe(1));
    expect(mock.lastBody("PATCH", "/api/v1/users/me")).toEqual({ timezone: "Asia/Tokyo" });
    expect(await screen.findByTestId("pref-timezone-saved")).toBeInTheDocument();
    await waitFor(() => expect(trigger).toHaveTextContent("Asia/Tokyo"));
    cleanup();
  });

  it("puts the saved zone back when the server refuses it", async () => {
    const mock = setupApi();
    mock.on("PATCH", "/api/v1/users/me", () => ({
      status: 422,
      json: { error: { message: "timezone must be 1–64 characters" } },
    }));
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <UserPreferencesPage />
      </AuthProvider>,
    );

    const trigger = await screen.findByLabelText("Your time zone");
    await user.click(trigger);
    await user.click(await screen.findByRole("option", { name: /^Asia\/Tokyo/ }));

    expect(await screen.findByTestId("pref-timezone-error")).toBeInTheDocument();
    await waitFor(() => expect(trigger).toHaveTextContent("UTC"));
    cleanup();
  });
});
