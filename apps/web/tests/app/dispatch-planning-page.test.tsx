// Integration test: /dispatch/planning — capacity horizon + skill
// catalog rendering, and the read-only connector surface with its typed
// "not available yet" fallback when the connector API is absent.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import DispatchPlanningPage from "../../app/dispatch/planning/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/dispatch/planning",
}));

const USER_ID = "00000000-0000-4000-8000-000000000001";

function setupApi(opts: { connector?: boolean } = {}): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/dispatch/capacity", () => ({
    json: {
      from: "2026-09-03",
      days: 2,
      unassigned_count: 4,
      technicians: [
        {
          user_id: USER_ID,
          display_name: "Dana Tech",
          days: [
            { date: "2026-09-03", available_minutes: 480, booked_minutes: 420, free_minutes: 60, utilization_percent: 88 },
            { date: "2026-09-04", available_minutes: 480, booked_minutes: 510, free_minutes: -30, utilization_percent: 106 },
          ],
        },
      ],
    },
  }));
  mock.on("GET", "/api/v1/dispatch/skills", () => ({
    json: { skills: [{ skill: "networking", holders: 3, active_holders: 2 }] },
  }));
  if (opts.connector) {
    mock.on("GET", "/api/v1/dispatch-connector/technicians", () => ({
      json: { technicians: [{ user_id: USER_ID, display_name: "Dana Tech", skills: ["networking"] }] },
    }));
    mock.on("GET", "/api/v1/dispatch-connector/schedule", () => ({
      json: { from: "2026-09-03", days: 7, appointments: [] },
    }));
  }
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/dispatch/planning integration", () => {
  it("renders the capacity horizon with over-capacity day and the skill catalog", async () => {
    setupApi();
    render(
      <AuthProvider>
        <DispatchPlanningPage />
      </AuthProvider>,
    );
    await screen.findByText("Dana Tech");
    expect(screen.getByText("4 unassigned in queue")).toBeInTheDocument();
    expect(screen.getByText("106%")).toBeInTheDocument();
    expect(screen.getByText("networking")).toBeInTheDocument();
    cleanup();
  });

  it("shows the typed not-available-yet state when the connector API is absent", async () => {
    setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <DispatchPlanningPage />
      </AuthProvider>,
    );
    await screen.findByText("Dana Tech");
    await user.click(screen.getByRole("button", { name: "Connector status" }));
    await screen.findByText(/dispatch connector surface is not available yet/);
    cleanup();
  });

  it("lists connector technicians and booking counts when present", async () => {
    setupApi({ connector: true });
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <DispatchPlanningPage />
      </AuthProvider>,
    );
    await screen.findByText("Dana Tech");
    await user.click(screen.getByRole("button", { name: "Connector status" }));
    await screen.findByText(/external connector API serves/);
    expect(screen.getAllByText("Dana Tech").length).toBeGreaterThanOrEqual(2);
    cleanup();
  });
});
